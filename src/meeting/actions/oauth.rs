//! Connecting a Todoist account: OAuth 2.0 authorization code with PKCE,
//! as a public client.
//!
//! A desktop app cannot keep a client secret, so voxtype has none. On first
//! connect it registers itself with Todoist (RFC 7591 dynamic client
//! registration, `token_endpoint_auth_method: none`) and keeps the client
//! ID with the tokens. The browser returns to a loopback address on one of
//! a few fixed ports, and PKCE (S256) ties the code to this process. The
//! only scope asked for is `task:add`: voxtype can add tasks and can read
//! nothing.

use super::todoist::{TodoistError, TokenStore, Tokens};
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Ports the browser can come back to. Registered together, so any free
/// one works.
pub const PORTS: [u16; 5] = [47815, 47816, 47817, 47818, 47819];

/// Add tasks, nothing else.
pub const SCOPE: &str = "task:add";

/// Where Todoist is. `VOXTYPE_TODOIST_API` and `VOXTYPE_TODOIST_APP`
/// point voxtype at a test server instead.
#[derive(Debug, Clone)]
pub struct Endpoints {
    /// Registration, tokens and tasks
    pub api: String,
    /// The consent page
    pub app: String,
}

impl Endpoints {
    pub fn from_env() -> Self {
        let get = |var: &str, default: &str| {
            std::env::var(var)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| default.to_string())
                .trim_end_matches('/')
                .to_string()
        };
        Self {
            api: get("VOXTYPE_TODOIST_API", super::todoist::API_BASE),
            app: get("VOXTYPE_TODOIST_APP", "https://app.todoist.com"),
        }
    }
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/callback")
}

/// Why connecting failed, each saying what to do.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error(
        "Every port voxtype can receive Todoist's answer on ({first}-{last}) is in use. Free one and run voxtype meeting todoist connect again.",
        first = PORTS[0],
        last = PORTS[PORTS.len() - 1]
    )]
    PortsBusy,
    #[error(
        "Todoist would not accept a local redirect address ({uri}): {detail}. Connecting from this machine needs a different setup; please report this."
    )]
    RedirectRefused { uri: String, detail: String },
    #[error("Todoist access was not granted ({0}).")]
    Denied(String),
    #[error(
        "No answer from Todoist within {0} seconds. Run voxtype meeting todoist connect again. If Todoist showed an error about the redirect address, please report it."
    )]
    TimedOut(u64),
    #[error(transparent)]
    Todoist(#[from] TodoistError),
}

/// What happens while connecting, for a terminal or a GUI to show.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Open this in a browser; it was already handed to `open`.
    Authorize { url: String, port: u16 },
}

/// Everything a connect needs, so tests can supply their own.
pub struct Connect<'a> {
    pub endpoints: Endpoints,
    pub store: &'a TokenStore,
    pub ports: &'a [u16],
    pub timeout: Duration,
    /// Hands the consent page to a browser
    pub open: &'a dyn Fn(&str),
}

/// `len` bytes from the kernel's random source.
fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        // SAFETY: writes at most len - filled bytes into buf after filled.
        let n = unsafe { libc::getrandom(buf[filled..].as_mut_ptr().cast(), len - filled, 0) };
        if n > 0 {
            filled += n as usize;
        } else if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            panic!("getrandom failed: {}", std::io::Error::last_os_error());
        }
    }
    buf
}

/// base64url without padding (RFC 4648 section 5).
pub fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 3);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// The PKCE pair: a verifier, and its S256 challenge.
pub fn pkce() -> (String, String) {
    use sha2::{Digest, Sha256};
    let verifier = base64url(&random_bytes(48));
    let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                    .ok_or(())
                {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The query of a request target, as pairs.
fn query(target: &str) -> Vec<(String, String)> {
    target
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect()
}

/// The consent page's address.
pub fn authorize_url(
    app: &str,
    client_id: &str,
    redirect: &str,
    state: &str,
    challenge: &str,
) -> String {
    format!(
        "{app}/oauth/authorize?response_type=code&client_id={}&scope={}&state={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256",
        encode(client_id),
        encode(SCOPE),
        encode(state),
        encode(redirect),
        encode(challenge)
    )
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()
}

fn todoist_error(e: ureq::Error) -> (Option<String>, TodoistError) {
    match e {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let code = v.get("error").and_then(|e| e.as_str()).map(str::to_string);
            let message = v
                .get("error_description")
                .and_then(|e| e.as_str())
                .map(str::to_string)
                .or_else(|| code.clone())
                .unwrap_or_else(|| body.chars().take(200).collect());
            (code, TodoistError::Http { status, message })
        }
        ureq::Error::Transport(t) => (None, TodoistError::Network(t.to_string())),
    }
}

/// Register voxtype as a public client with every port's redirect.
fn register(endpoints: &Endpoints, ports: &[u16]) -> Result<String, ConnectError> {
    #[derive(Deserialize)]
    struct Reply {
        client_id: String,
    }
    let uris: Vec<String> = ports.iter().map(|&p| redirect_uri(p)).collect();
    let reply = agent()
        .post(&format!("{}/oauth/register", endpoints.api))
        .send_json(serde_json::json!({
            "client_name": "Voxtype",
            "client_uri": "https://voxtype.io",
            "redirect_uris": uris,
            "scope": SCOPE,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }));
    match reply {
        Ok(r) => Ok(r
            .into_json::<Reply>()
            .map_err(|e| TodoistError::Network(e.to_string()))?
            .client_id),
        Err(e) => match todoist_error(e) {
            (Some(code), TodoistError::Http { message, .. })
                if code == "invalid_client_metadata" =>
            {
                Err(ConnectError::RedirectRefused {
                    uri: redirect_uri(ports[0]),
                    detail: message,
                })
            }
            (_, e) => Err(e.into()),
        },
    }
}

/// Listen on the first free port, on IPv4 and, where it can, IPv6
/// loopback: browsers may try either for "localhost".
fn listen(ports: &[u16]) -> Result<(u16, Vec<TcpListener>), ConnectError> {
    for &port in ports {
        let Ok(v4) = TcpListener::bind(("127.0.0.1", port)) else {
            continue;
        };
        let mut listeners = vec![v4];
        if let Ok(v6) = TcpListener::bind(("::1", port)) {
            listeners.push(v6);
        }
        for l in &listeners {
            l.set_nonblocking(true).map_err(|e| {
                TodoistError::Network(format!("could not listen on port {port}: {e}"))
            })?;
        }
        return Ok((port, listeners));
    }
    Err(ConnectError::PortsBusy)
}

const PAGE_OK: &str = "<!doctype html><meta charset=utf-8><title>Voxtype</title><body style=\"font-family:sans-serif;margin:3em\"><h2>Voxtype is connected to Todoist.</h2><p>You can close this tab.</p>";
const PAGE_DENIED: &str = "<!doctype html><meta charset=utf-8><title>Voxtype</title><body style=\"font-family:sans-serif;margin:3em\"><h2>Voxtype was not connected.</h2><p>You can close this tab.</p>";

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// What the browser brought back.
enum Callback {
    Code(String),
    Denied(String),
}

/// Wait for the browser's return to /callback with our `state`. Anything
/// else (a favicon, a stale tab with another state) is answered and
/// ignored.
fn wait_for_callback(
    listeners: &[TcpListener],
    state: &str,
    deadline: Instant,
    timeout: Duration,
) -> Result<Callback, ConnectError> {
    loop {
        for listener in listeners {
            let mut stream = match listener.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => continue,
            };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let target = line.split_whitespace().nth(1).unwrap_or("");
            if !target.starts_with("/callback") {
                respond(&mut stream, "404 Not Found", "");
                continue;
            }
            let q = query(target);
            let get = |k: &str| q.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
            if get("state").as_deref() != Some(state) {
                respond(&mut stream, "400 Bad Request", PAGE_DENIED);
                continue;
            }
            if let Some(error) = get("error") {
                respond(&mut stream, "200 OK", PAGE_DENIED);
                return Ok(Callback::Denied(error));
            }
            if let Some(code) = get("code").filter(|c| !c.is_empty()) {
                respond(&mut stream, "200 OK", PAGE_OK);
                return Ok(Callback::Code(code));
            }
            respond(&mut stream, "400 Bad Request", PAGE_DENIED);
        }
        if Instant::now() >= deadline {
            return Err(ConnectError::TimedOut(timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Trade the code for tokens, proving with the PKCE verifier that this is
/// the process that asked.
fn exchange(
    endpoints: &Endpoints,
    client_id: &str,
    code: &str,
    redirect: &str,
    verifier: &str,
) -> Result<Tokens, ConnectError> {
    #[derive(Deserialize)]
    struct Reply {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<i64>,
        scope: Option<String>,
    }
    let reply: Reply = agent()
        .post(&format!("{}/oauth/access_token", endpoints.api))
        .send_form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("redirect_uri", redirect),
            ("code_verifier", verifier),
        ])
        .map_err(|e| ConnectError::Todoist(todoist_error(e).1))?
        .into_json()
        .map_err(|e| TodoistError::Network(e.to_string()))?;
    Ok(Tokens {
        access_token: reply.access_token,
        refresh_token: reply.refresh_token,
        expires_at: reply.expires_in.map(|s| chrono::Utc::now().timestamp() + s),
        client_id: Some(client_id.to_string()),
        scope: reply.scope.or_else(|| Some(SCOPE.to_string())),
        redirect_uris: Vec::new(),
    })
}

/// Connect: register if needed, send the user to the consent page, wait
/// for the browser to come back, and save the tokens. Nothing is saved
/// unless it all works, except a new registration, which is reused next
/// time.
pub fn connect(c: &Connect, progress: &mut dyn FnMut(Progress)) -> Result<Tokens, ConnectError> {
    let wanted: Vec<String> = c.ports.iter().map(|&p| redirect_uri(p)).collect();
    let saved = c.store.load()?.unwrap_or_default();
    let client_id = match &saved.client_id {
        Some(id) if saved.redirect_uris == wanted => id.clone(),
        _ => {
            let id = register(&c.endpoints, c.ports)?;
            // Keep the registration; a connection already working keeps
            // working until this one replaces it.
            let registered = Tokens {
                client_id: Some(id.clone()),
                redirect_uris: wanted.clone(),
                ..saved.clone()
            };
            c.store.save(&registered)?;
            id
        }
    };
    let (port, listeners) = listen(c.ports)?;
    let redirect = redirect_uri(port);
    let (verifier, challenge) = pkce();
    let state = base64url(&random_bytes(24));
    let url = authorize_url(&c.endpoints.app, &client_id, &redirect, &state, &challenge);
    progress(Progress::Authorize {
        url: url.clone(),
        port,
    });
    (c.open)(&url);
    let deadline = Instant::now() + c.timeout;
    let code = match wait_for_callback(&listeners, &state, deadline, c.timeout)? {
        Callback::Code(code) => code,
        Callback::Denied(why) => return Err(ConnectError::Denied(why)),
    };
    drop(listeners);
    let mut tokens = exchange(&c.endpoints, &client_id, &code, &redirect, &verifier)?;
    tokens.redirect_uris = wanted;
    c.store.save(&tokens)?;
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::{Arc, Mutex};

    #[test]
    fn base64url_matches_rfc_4648_vectors() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn the_pkce_challenge_is_s256_of_the_verifier() {
        // RFC 7636 appendix B.
        use sha2::{Digest, Sha256};
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            base64url(&Sha256::digest(verifier.as_bytes())),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let (v, c) = pkce();
        assert_eq!(v.len(), 64);
        assert!(v
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
        assert_eq!(c, base64url(&Sha256::digest(v.as_bytes())));
        assert_ne!(pkce().0, v, "fresh every time");
    }

    #[test]
    fn queries_decode() {
        assert_eq!(
            query("/callback?code=a%2Fb+c&state=x&empty"),
            [
                ("code".to_string(), "a/b c".to_string()),
                ("state".to_string(), "x".to_string()),
                ("empty".to_string(), String::new())
            ]
        );
        assert_eq!(decode("100%"), "100%");
        let url = authorize_url(
            "https://app.example",
            "tdd_1",
            &redirect_uri(47815),
            "s t",
            "ch",
        );
        assert_eq!(
            url,
            "https://app.example/oauth/authorize?response_type=code&client_id=tdd_1&scope=task%3Aadd&state=s%20t&redirect_uri=http%3A%2F%2Flocalhost%3A47815%2Fcallback&code_challenge=ch&code_challenge_method=S256"
        );
    }

    /// A fake Todoist: registration and token endpoints, recording each
    /// request body by path.
    struct FakeTodoist {
        base: String,
        seen: Arc<Mutex<Vec<(String, String)>>>,
        _thread: std::thread::JoinHandle<()>,
    }

    fn fake_todoist(register_status: u16, register_body: &'static str) -> FakeTodoist {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut length = 0;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    if h == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                log.lock()
                    .unwrap()
                    .push((path.clone(), String::from_utf8(body).unwrap()));
                let (status, reply) = match path.as_str() {
                    "/oauth/register" => (register_status, register_body),
                    "/oauth/access_token" => (
                        200,
                        r#"{"access_token":"at","refresh_token":"rt","expires_in":3600,"token_type":"Bearer","scope":"task:add"}"#,
                    ),
                    _ => (404, "{}"),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
            }
        });
        FakeTodoist {
            base,
            seen,
            _thread: thread,
        }
    }

    /// A port nothing listens on, for one test's redirect listener, which
    /// `connect` binds later itself. Each is handed out once per run, from
    /// below Linux's ephemeral range (32768 and up by default), so neither
    /// another test nor an outgoing connection is given the same one.
    ///
    /// That alone did not make these tests reliable. The flake they had
    /// (`PortsBusy` about once in five full runs) came from re-binding a
    /// port a test had just used: when `connect` closed its listener, a test
    /// elsewhere in the process that was forking a child at that moment
    /// held a copy of the socket until the child exec'd, so the port still
    /// read as LISTEN. `ss` showed the socket under the forking test's
    /// thread. So a test that connects twice either takes a fresh port for
    /// the second `connect` or waits for the old one with
    /// [`wait_until_free`].
    fn free_port() -> u16 {
        use std::sync::atomic::{AtomicU16, Ordering};
        static NEXT: AtomicU16 = AtomicU16::new(0);
        // Spread runs out a little, for two test processes side by side;
        // every port stays below 32768.
        let base = 20000 + (std::process::id() % 64) as u16 * 100;
        for _ in 0..6000 {
            let port = base + NEXT.fetch_add(1, Ordering::Relaxed) % 6000;
            if TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return port;
            }
        }
        panic!("no free test port below the ephemeral range");
    }

    /// Wait for a port a test's last `connect` listened on to be free again.
    /// The listener is closed when `connect` returns, but a test elsewhere
    /// in this process that forks a child at that moment holds a copy of it
    /// until the child execs, and the port reads as busy for that long.
    fn wait_until_free(port: u16) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while TcpListener::bind(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "port {port} stayed busy");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Act as the browser: follow the consent page straight back to the
    /// redirect with `answer` (a query string, `{state}` filled in).
    fn browser(answer: &'static str) -> impl Fn(&str) {
        move |url: &str| {
            let q = query(url);
            let get = |k: &str| q.iter().find(|(key, _)| key == k).unwrap().1.clone();
            let redirect = get("redirect_uri").replace("localhost", "127.0.0.1");
            let state = get("state");
            let target = format!("{redirect}?{}", answer.replace("{state}", &encode(&state)));
            std::thread::spawn(move || {
                // A wrong state first, as a stale tab would send: ignored.
                let wrong = target.replace("state=", "state=stale");
                let _ = ureq::get(&wrong).call();
                let _ = ureq::get(&target).call();
            });
        }
    }

    fn store() -> (tempfile::TempDir, TokenStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("todoist.json"));
        (dir, store)
    }

    #[test]
    fn connecting_registers_a_public_client_and_saves_tokens() {
        let fake = fake_todoist(
            201,
            r#"{"client_id":"tdd_new","token_endpoint_auth_method":"none"}"#,
        );
        let (_dir, store) = store();
        let ports = [free_port()];
        let open = browser("code=the%20code&state={state}");
        let mut events = Vec::new();
        let tokens = connect(
            &Connect {
                endpoints: Endpoints {
                    api: fake.base.clone(),
                    app: "https://app.example".into(),
                },
                store: &store,
                ports: &ports,
                timeout: Duration::from_secs(10),
                open: &open,
            },
            &mut |p| events.push(p),
        )
        .unwrap();
        assert_eq!(tokens.access_token, "at");
        assert_eq!(store.load().unwrap(), Some(tokens.clone()));
        assert_eq!(tokens.client_id.as_deref(), Some("tdd_new"));
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        assert_eq!(tokens.redirect_uris, [redirect_uri(ports[0])]);

        let [Progress::Authorize { url, port }] = &events[..] else {
            panic!("{events:?}")
        };
        assert_eq!(*port, ports[0]);
        assert!(url.starts_with("https://app.example/oauth/authorize?"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("scope=task%3Aadd"));

        let seen = fake.seen.lock().unwrap().clone();
        let register: serde_json::Value = serde_json::from_str(&seen[0].1).unwrap();
        assert_eq!(seen[0].0, "/oauth/register");
        assert_eq!(register["token_endpoint_auth_method"], "none");
        assert_eq!(register["scope"], "task:add");
        assert_eq!(register["redirect_uris"][0], redirect_uri(ports[0]));
        assert_eq!(register["grant_types"][1], "refresh_token");

        // The code exchange proves the verifier behind the challenge, and
        // sends no secret.
        assert_eq!(seen[1].0, "/oauth/access_token");
        let form = query(&format!("?{}", seen[1].1));
        let get = |k: &str| {
            form.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("grant_type").as_deref(), Some("authorization_code"));
        assert_eq!(get("code").as_deref(), Some("the code"));
        assert_eq!(get("client_id").as_deref(), Some("tdd_new"));
        assert_eq!(get("redirect_uri"), Some(redirect_uri(ports[0])));
        assert!(get("client_secret").is_none());
        use sha2::{Digest, Sha256};
        let challenge = query(url)
            .into_iter()
            .find(|(k, _)| k == "code_challenge")
            .unwrap()
            .1;
        assert_eq!(
            base64url(&Sha256::digest(get("code_verifier").unwrap().as_bytes())),
            challenge
        );

        // Connecting again reuses the registration, so on the same port.
        wait_until_free(ports[0]);
        let open = browser("code=again&state={state}");
        connect(
            &Connect {
                endpoints: Endpoints {
                    api: fake.base.clone(),
                    app: "https://app.example".into(),
                },
                store: &store,
                ports: &ports,
                timeout: Duration::from_secs(10),
                open: &open,
            },
            &mut |_| {},
        )
        .unwrap();
        let paths: Vec<String> = fake
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.0.clone())
            .collect();
        assert_eq!(
            paths,
            [
                "/oauth/register",
                "/oauth/access_token",
                "/oauth/access_token"
            ]
        );
    }

    #[test]
    fn a_refused_redirect_says_so_and_saves_nothing() {
        let fake = fake_todoist(
            400,
            r#"{"error":"invalid_client_metadata","error_description":"redirect_uris must be HTTPS"}"#,
        );
        let (_dir, store) = store();
        let err = connect(
            &Connect {
                endpoints: Endpoints {
                    api: fake.base.clone(),
                    app: "https://app.example".into(),
                },
                store: &store,
                ports: &[free_port()],
                timeout: Duration::from_secs(5),
                open: &|_| {},
            },
            &mut |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, ConnectError::RedirectRefused { .. }), "{err}");
        assert!(err.to_string().contains("redirect_uris must be HTTPS"));
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn denial_timeout_and_busy_ports_fail_without_tokens() {
        let fake = fake_todoist(201, r#"{"client_id":"tdd_x"}"#);
        let (_dir, store) = store();
        let endpoints = Endpoints {
            api: fake.base.clone(),
            app: "https://app.example".into(),
        };
        let ports = [free_port()];
        let open = browser("error=access_denied&state={state}");
        let err = connect(
            &Connect {
                endpoints: endpoints.clone(),
                store: &store,
                ports: &ports,
                timeout: Duration::from_secs(10),
                open: &open,
            },
            &mut |_| {},
        )
        .unwrap_err();
        assert!(
            matches!(err, ConnectError::Denied(ref e) if e == "access_denied"),
            "{err}"
        );
        let saved = store.load().unwrap().unwrap();
        assert!(saved.access_token.is_empty(), "registered, not connected");
        assert_eq!(saved.client_id.as_deref(), Some("tdd_x"));

        // A port of its own, not the one the denial just used: while that
        // listener closes, any test forking a process at that moment holds
        // a copy of it until the child execs, and the port reads as busy.
        let err = connect(
            &Connect {
                endpoints: endpoints.clone(),
                store: &store,
                ports: &[free_port()],
                timeout: Duration::from_millis(200),
                open: &|_| {},
            },
            &mut |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, ConnectError::TimedOut(_)), "{err}");

        let busy = TcpListener::bind("127.0.0.1:0").unwrap();
        let busy_port = busy.local_addr().unwrap().port();
        let err = connect(
            &Connect {
                endpoints,
                store: &store,
                ports: &[busy_port],
                timeout: Duration::from_secs(1),
                open: &|_| {},
            },
            &mut |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, ConnectError::PortsBusy), "{err}");
        assert!(store.load().unwrap().unwrap().access_token.is_empty());
    }

    #[test]
    fn stray_requests_are_answered_and_ignored() {
        let ports = [free_port()];
        let (port, listeners) = listen(&ports).unwrap();
        let t = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET /favicon.ico HTTP/1.1\r\n\r\n").unwrap();
            let mut reply = String::new();
            let _ = s.read_to_string(&mut reply);
            assert!(reply.starts_with("HTTP/1.1 404"), "{reply}");
            let _ = ureq::get(&format!("http://127.0.0.1:{port}/callback?state=ok&code=c")).call();
        });
        let got = wait_for_callback(
            &listeners,
            "ok",
            Instant::now() + Duration::from_secs(10),
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(matches!(got, Callback::Code(ref c) if c == "c"));
        t.join().unwrap();
    }
}
