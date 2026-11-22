//! `voxtype meeting todoist` and `meeting actions --send todoist`, through
//! the real binary, against a fake Todoist on a loopback port. Nothing here
//! reaches the real Todoist.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use voxtype::meeting::data::{ActionItem, MeetingMetadata, MeetingSummary};
use voxtype::meeting::{MeetingStorage, StorageConfig};

type Seen = Arc<Mutex<Vec<(String, String, String)>>>;

/// A fake Todoist API: registration, tokens and tasks. Records (path,
/// authorization header, body) for each request.
fn fake_todoist() -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let (mut length, mut auth) = (0, String::new());
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                if lower.starts_with("authorization:") {
                    auth = h.trim().to_string();
                }
                if h == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let n = {
                let mut seen = log.lock().unwrap();
                seen.push((path.clone(), auth, String::from_utf8(body).unwrap()));
                seen.len()
            };
            let reply = match path.as_str() {
                "/oauth/register" => r#"{"client_id":"tdd_test"}"#.to_string(),
                "/oauth/access_token" => r#"{"access_token":"at1","refresh_token":"rt1","expires_in":3600,"scope":"task:add"}"#.to_string(),
                "/api/v1/tasks" => format!(r#"{{"id":"task{n}"}}"#),
                _ => "{}".to_string(),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    (base, seen)
}

struct Scratch {
    dir: tempfile::TempDir,
    api: String,
}

impl Scratch {
    fn new(api: String, extra_config: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["config/voxtype", "data/voxtype", "runtime", "state"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        std::fs::write(
            dir.path().join("config/voxtype/config.toml"),
            format!("[meeting]\nenabled = true\n\n[meeting.actions]\n{extra_config}\n"),
        )
        .unwrap();
        Self { dir, api }
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.dir.path().join(sub)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
        cmd.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("XDG_RUNTIME_DIR", self.path("runtime"))
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env("VOXTYPE_TODOIST_API", &self.api)
            .env("VOXTYPE_TODOIST_APP", "https://app.todoist.invalid")
            .env("RUST_LOG", "warn")
            .args(args)
            .stdin(Stdio::null());
        cmd
    }

    fn voxtype(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn meeting(&self) -> String {
        let storage = MeetingStorage::open(StorageConfig {
            storage_path: self.path("data/voxtype/meetings"),
            retain_audio: false,
            max_meetings: 0,
            my_name: None,
        })
        .unwrap();
        let metadata = MeetingMetadata::new(Some("Release sync".into()));
        storage.create_meeting(&metadata).unwrap();
        storage
            .save_summary(
                &metadata.id,
                &MeetingSummary {
                    summary: "Shipping.".into(),
                    key_points: vec![],
                    action_items: vec![ActionItem {
                        description: "Tag the release".into(),
                        assignee: Some("Dana".into()),
                        ..Default::default()
                    }],
                    decisions: vec![],
                    chapters: vec![],
                    generated_at: chrono::Utc::now(),
                    model: None,
                },
            )
            .unwrap();
        metadata.id.to_string()
    }
}

fn json(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "voxtype failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn param(url: &str, key: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    let raw = query
        .split('&')
        .find_map(|p| p.strip_prefix(&format!("{key}=")))
        .unwrap();
    // Only %XX escapes appear in what voxtype builds.
    let mut out = Vec::new();
    let b = raw.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(&raw[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

#[test]
fn connect_in_the_browser_then_send_tasks_then_disconnect() {
    let (api, seen) = fake_todoist();
    let s = Scratch::new(api, "");
    let id = s.meeting();

    let status = json(&s.voxtype(&["meeting", "todoist", "status", "--json"]));
    assert_eq!(status["connected"], false);
    assert_eq!(status["method"], serde_json::Value::Null);

    let mut child = s
        .command(&[
            "meeting",
            "todoist",
            "connect",
            "--no-browser",
            "--json",
            "--timeout",
            "30",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let first: serde_json::Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(first["event"], "authorize");
    let url = first["url"].as_str().unwrap();
    assert!(url.starts_with("https://app.todoist.invalid/oauth/authorize?"));
    assert_eq!(param(url, "client_id"), "tdd_test");
    assert_eq!(param(url, "scope"), "task:add");
    assert_eq!(param(url, "code_challenge_method"), "S256");
    let redirect = param(url, "redirect_uri");
    assert_eq!(
        redirect,
        format!("http://localhost:{}/callback", first["port"])
    );

    // Be the browser: Todoist sends it back with a code.
    let back = format!(
        "{}?code=c0de&state={}",
        redirect.replace("localhost", "127.0.0.1"),
        param(url, "state")
    );
    let page = ureq::get(&back).call().unwrap().into_string().unwrap();
    assert!(page.contains("connected to Todoist"), "{page}");

    let done: serde_json::Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(done["event"], "connected", "{done}");
    assert_eq!(done["scope"], "task:add");
    assert!(child.wait().unwrap().success());

    let file = s.path("state/voxtype/todoist.json");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains("\"at1\"") && saved.contains("tdd_test"));
    assert!(!saved.contains("secret"));
    let config = std::fs::read_to_string(s.path("config/voxtype/config.toml")).unwrap();
    assert!(!config.contains("at1"), "no token in config.toml");

    let status = json(&s.voxtype(&["meeting", "todoist", "status", "--json"]));
    assert_eq!(status["connected"], true);
    assert_eq!(status["method"], "oauth");

    let sent = json(&s.voxtype(&["meeting", "actions", &id, "--send", "todoist", "--json"]));
    assert_eq!(sent["results"][0]["status"], "sent", "{sent}");
    let task = seen
        .lock()
        .unwrap()
        .iter()
        .find(|(p, _, _)| p == "/api/v1/tasks")
        .cloned()
        .unwrap();
    assert_eq!(task.1, "Authorization: Bearer at1");
    let body: serde_json::Value = serde_json::from_str(&task.2).unwrap();
    assert_eq!(body["content"], "Tag the release (Dana)");

    let gone = json(&s.voxtype(&["meeting", "todoist", "disconnect", "--json"]));
    assert_eq!(gone["disconnected"], true);
    assert!(!file.exists());
    let out = s.voxtype(&["meeting", "actions", &id, "--send", "todoist", "--resend"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("todoist connect"));
}

#[test]
fn a_personal_token_file_works_without_connecting() {
    let (api, seen) = fake_todoist();
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("token");
    std::fs::write(&token, "personal-abc\n").unwrap();
    let s = Scratch::new(
        api,
        &format!("todoist_token_file = \"{}\"", token.display()),
    );
    let id = s.meeting();
    let status = json(&s.voxtype(&["meeting", "todoist", "status", "--json"]));
    assert_eq!(status["method"], "token_file");
    let sent = json(&s.voxtype(&["meeting", "actions", &id, "--send", "todoist", "--json"]));
    assert_eq!(sent["results"][0]["status"], "sent", "{sent}");
    assert_eq!(
        seen.lock().unwrap()[0].1,
        "Authorization: Bearer personal-abc"
    );
    assert!(!s.path("state/voxtype/todoist.json").exists());
}
