//! Find the text model Omarchy runs locally, through Omarchy's documented
//! contract for third-party clients only:
//!
//! - The Omarchy manual ("Local models"): "Any OpenAI-compatible client can
//!   connect too: `status` shows each model's address (the first is
//!   `http://127.0.0.1:11435/v1`), the recipe name is the model, and the key
//!   is in `~/.config/omarchy/agent-local/key`."
//! - `omarchy-models status --json`, whose shape omarchy-local-ai's
//!   engine.md documents: `{models: {<recipe>: {kind, unit, endpoint, ...}},
//!   last, ...}`.
//!
//! The preferred model is the one named in config, else `last` (the last
//! started text model) when it is running, else another running text model,
//! the order Omarchy's own `omarchy agent --local` uses. When the
//! `omarchy-models` command is missing or fails, the documented loopback
//! ports from 11435 up are asked for `/v1/models` instead, and only ports
//! whose listening socket belongs to this user get the key.
//!
//! Voxtype never starts, installs or downloads a model. It only reads
//! state and talks to a model that is already running.

use super::{Endpoint, LlmError, Provider};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Omarchy's model command (`omarchy models`).
pub const MODELS_COMMAND: &str = "omarchy-models";

/// Where Omarchy keeps the key, as its manual names it.
pub const KEY_FILE_DISPLAY: &str = "~/.config/omarchy/agent-local/key";

/// The first local model's port; each further model gets the next one.
pub const FIRST_PORT: u16 = 11435;

/// How many ports from [`FIRST_PORT`] the fallback asks.
const PROBE_PORTS: u16 = 10;

/// Omarchy's own scripts directory, which a systemd user unit's PATH lacks.
const OMARCHY_BIN_DIR: &str = "/usr/share/omarchy/bin";

/// How to get a model running, said in every "none running" message.
pub const START_HINT: &str = "Start one with `omarchy models start <recipe>` or from the local \
models panel (Super + Ctrl + Alt + X); `omarchy models list` shows which models fit this \
machine. Voxtype never starts or downloads a model itself.";

/// Where discovery looks. Replaced in tests.
#[derive(Debug, Clone)]
pub struct OmarchyEnv {
    /// `omarchy-models`, when found.
    pub models_command: Option<PathBuf>,
    /// Omarchy's key file.
    pub key_file: Option<PathBuf>,
    /// Ports the fallback asks.
    pub probe_ports: Vec<u16>,
    /// How long `omarchy-models status --json` may take.
    pub status_timeout: Duration,
    /// Send the key only to a port whose listener belongs to this user.
    pub check_port_owner: bool,
}

impl OmarchyEnv {
    pub fn from_process() -> Self {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .or_else(dirs::home_dir);
        Self {
            models_command: find_program(MODELS_COMMAND),
            key_file: home.map(|h| h.join(".config/omarchy/agent-local/key")),
            probe_ports: (FIRST_PORT..FIRST_PORT + PROBE_PORTS).collect(),
            status_timeout: Duration::from_secs(10),
            check_port_owner: true,
        }
    }

    /// Finds nothing: no command, no key, no ports.
    pub fn disabled() -> Self {
        Self {
            models_command: None,
            key_file: None,
            probe_ports: Vec::new(),
            status_timeout: Duration::from_secs(1),
            check_port_owner: true,
        }
    }
}

/// `name` on PATH, then in Omarchy's bin directory, then /usr/bin.
fn find_program(name: &str) -> Option<PathBuf> {
    which::which(name).ok().or_else(|| {
        [OMARCHY_BIN_DIR, "/usr/bin"]
            .iter()
            .map(|d| Path::new(d).join(name))
            .find(|p| p.is_file())
    })
}

/// Is this an Omarchy install? Judged only by Omarchy's public commands:
/// `omarchy-models` or `omarchy` can be found.
pub fn is_omarchy_install() -> bool {
    find_program(MODELS_COMMAND).is_some() || find_program("omarchy").is_some()
}

/// One model from `status --json`.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusModel {
    pub recipe: String,
    /// `text` when the field is missing, as the engine defaults it.
    pub kind: String,
    pub unit: String,
    pub endpoint: String,
}

impl StatusModel {
    fn is_text(&self) -> bool {
        self.kind == "text"
    }
}

/// What `status --json` says, as far as voxtype needs it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub models: Vec<StatusModel>,
    pub last: Option<String>,
}

/// Parse `omarchy-models status --json`. The engine changes often, so
/// unknown fields are ignored and odd ones are read as missing.
pub fn parse_status(text: &str) -> Result<Status, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let models = v
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| "no `models` object".to_string())?;
    let s = |m: &Value, k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut out: Vec<StatusModel> = models
        .iter()
        .map(|(id, m)| StatusModel {
            recipe: m
                .get("recipe")
                .and_then(Value::as_str)
                .filter(|r| !r.is_empty())
                .unwrap_or(id)
                .to_string(),
            kind: m
                .get("kind")
                .and_then(Value::as_str)
                .filter(|k| !k.is_empty())
                .unwrap_or("text")
                .to_string(),
            unit: s(m, "unit"),
            endpoint: s(m, "endpoint"),
        })
        .collect();
    out.sort_by(|a, b| a.recipe.cmp(&b.recipe));
    Ok(Status {
        models: out,
        last: v
            .get("last")
            .and_then(Value::as_str)
            .filter(|l| !l.is_empty())
            .map(str::to_string),
    })
}

/// Choose the model to use. `wanted` is a recipe from config.
pub fn pick<'a>(status: &'a Status, wanted: Option<&str>) -> Result<&'a StatusModel, LlmError> {
    let text: Vec<&StatusModel> = status.models.iter().filter(|m| m.is_text()).collect();
    let usable = |m: &StatusModel| {
        let ok =
            m.unit == "active" && !m.endpoint.is_empty() && super::is_loopback_url(&m.endpoint);
        if m.unit == "active" && !m.endpoint.is_empty() && !ok {
            tracing::debug!(
                "Omarchy model {} has a non-loopback endpoint; voxtype does not use it",
                m.recipe
            );
        }
        ok
    };

    if let Some(wanted) = wanted.map(str::trim).filter(|w| !w.is_empty()) {
        let Some(m) = text.iter().copied().find(|m| m.recipe == wanted) else {
            return Err(LlmError::NotRunning(format!(
                "Omarchy has no text model '{wanted}' set up. `omarchy models list` shows \
                 the models that fit this machine. Voxtype never starts or downloads a model itself."
            )));
        };
        if usable(m) {
            return Ok(m);
        }
        return Err(LlmError::NotRunning(match m.unit.as_str() {
            "activating" => format!(
                "Omarchy's model '{wanted}' is still starting. Try again once the local \
                 models panel shows it running."
            ),
            "installing" => format!(
                "Omarchy's model '{wanted}' is still being set up. Try again once it is running."
            ),
            state => format!(
                "Omarchy's model '{wanted}' is not running ({}). Start it with \
                 `omarchy models start {wanted}` or from the local models panel \
                 (Super + Ctrl + Alt + X). Voxtype never starts a model itself.",
                if state.is_empty() {
                    "state unknown"
                } else {
                    state
                }
            ),
        }));
    }

    let running: Vec<&StatusModel> = text.iter().copied().filter(|m| usable(m)).collect();
    if let Some(last) = status.last.as_deref() {
        if let Some(m) = running.iter().copied().find(|m| m.recipe == last) {
            return Ok(m);
        }
    }
    if let Some(m) = running.first().copied() {
        return Ok(m);
    }
    let set_up: Vec<&str> = text
        .iter()
        .filter(|m| m.unit != "installing" && m.unit != "removing")
        .map(|m| m.recipe.as_str())
        .collect();
    Err(LlmError::NotRunning(if set_up.is_empty() {
        "No Omarchy text model is set up. Set one up with `omarchy models use best text` \
         (`omarchy models list` shows which fit this machine). Voxtype never starts or \
         downloads a model itself."
            .to_string()
    } else {
        // Name a real recipe to start: the last started one when it is set
        // up, else the first.
        let example = status
            .last
            .as_deref()
            .filter(|l| set_up.contains(l))
            .unwrap_or(set_up[0]);
        format!(
            "No Omarchy text model is running (set up here: {}). Start one with \
             `omarchy models start {example}` or from the local models panel \
             (Super + Ctrl + Alt + X). Voxtype never starts a model itself.",
            set_up.join(", ")
        )
    }))
}

/// Whether Omarchy's key file can be opened. Opens it without reading it.
pub fn key_readable(env: &OmarchyEnv) -> Result<PathBuf, LlmError> {
    let path = env.key_file.clone().ok_or_else(|| LlmError::Key {
        path: PathBuf::from(KEY_FILE_DISPLAY),
        reason: "HOME is not set".to_string(),
    })?;
    std::fs::File::open(&path).map_err(|e| LlmError::Key {
        path: path.clone(),
        reason: match e.kind() {
            std::io::ErrorKind::NotFound => {
                "it does not exist; Omarchy writes it when a model is first set up".to_string()
            }
            kind => kind.to_string(),
        },
    })?;
    Ok(path)
}

/// Find the running Omarchy text model to talk to.
pub fn discover(env: &OmarchyEnv, wanted: Option<&str>) -> Result<Endpoint, LlmError> {
    let from_status = env
        .models_command
        .as_deref()
        .map(|cmd| run_status(cmd, env.status_timeout).and_then(|t| parse_status(&t)));
    let (recipe, base_url) = match from_status {
        Some(Ok(status)) => {
            let m = pick(&status, wanted)?;
            (m.recipe.clone(), m.endpoint.clone())
        }
        other => {
            if let Some(Err(e)) = other {
                tracing::debug!("{MODELS_COMMAND} status --json gave no answer ({e}); asking the documented ports");
            }
            probe_ports(env, wanted)?
        }
    };
    let key_file = key_readable(env)?;
    Ok(Endpoint {
        provider: Provider::Omarchy,
        base_url: base_url.trim_end_matches('/').to_string(),
        model: recipe,
        key_file: Some(key_file),
    })
}

/// Run `omarchy-models status --json`, killed after `timeout`.
pub fn run_status(cmd: &Path, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(cmd)
        .args(["status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run {}: {e}", cmd.display()))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}s", timeout.as_secs()));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let text = reader.join().unwrap_or_default();
    if !status.success() {
        return Err(format!("exited with {status}"));
    }
    Ok(text)
}

/// The fallback: ask each documented port for `/v1/models`.
fn probe_ports(env: &OmarchyEnv, wanted: Option<&str>) -> Result<(String, String), LlmError> {
    let key_file = key_readable(env).map_err(|_| {
        LlmError::NotRunning(format!(
            "Omarchy's local models were not found ({MODELS_COMMAND} is not available and \
             {KEY_FILE_DISPLAY} does not exist). Set a text model up with \
             `omarchy models use best text`. Voxtype never starts or downloads a model itself."
        ))
    })?;
    let key = super::read_key(&key_file)?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_millis(500))
        .timeout(Duration::from_secs(3))
        .build();
    for &port in &env.probe_ports {
        if env.check_port_owner && !listener_is_ours(port) {
            continue;
        }
        let base = format!("http://127.0.0.1:{port}/v1");
        let Ok(response) = agent
            .get(&format!("{base}/models"))
            .set("Authorization", &format!("Bearer {key}"))
            .call()
        else {
            continue;
        };
        let Ok(v) = response.into_json::<Value>() else {
            continue;
        };
        let ids: Vec<&str> = v["data"]
            .as_array()
            .map(|a| a.iter().filter_map(|m| m["id"].as_str()).collect())
            .unwrap_or_default();
        let found = match wanted.map(str::trim).filter(|w| !w.is_empty()) {
            Some(w) => ids.iter().find(|id| **id == w),
            None => ids.first(),
        };
        if let Some(id) = found {
            return Ok((id.to_string(), base));
        }
    }
    Err(LlmError::NotRunning(match wanted {
        Some(w) if !w.trim().is_empty() => {
            format!("Omarchy's model '{w}' is not running. {START_HINT}")
        }
        _ => format!("No Omarchy text model is running. {START_HINT}"),
    }))
}

/// Whether a socket listening on loopback `port` belongs to this user, so
/// the key never goes to another user's process on a shared machine.
#[cfg(target_os = "linux")]
fn listener_is_ours(port: u16) -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    ["/proc/net/tcp", "/proc/net/tcp6"].iter().any(|file| {
        std::fs::read_to_string(file)
            .map(|t| proc_net_has_listener(&t, port, uid))
            .unwrap_or(false)
    })
}

#[cfg(not(target_os = "linux"))]
fn listener_is_ours(_port: u16) -> bool {
    false
}

/// Whether a `/proc/net/tcp{,6}` table has a socket listening on `port`
/// owned by `uid`.
fn proc_net_has_listener(table: &str, port: u16, uid: u32) -> bool {
    table.lines().skip(1).any(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        // sl local_address rem_address st tx:rx tr:when retrnsmt uid ...
        let (Some(local), Some(state), Some(owner)) = (cols.get(1), cols.get(3), cols.get(7))
        else {
            return false;
        };
        let listen = *state == "0A";
        let port_matches = local
            .rsplit_once(':')
            .and_then(|(_, p)| u16::from_str_radix(p, 16).ok())
            == Some(port);
        listen && port_matches && owner.parse::<u32>() == Ok(uid)
    })
}

#[cfg(test)]
mod tests {
    use super::super::fake_server::{FakeServer, Reply};
    use super::*;

    /// The shape `omarchy-models status --json` printed on 2026-11-21
    /// (omarchy-local-ai 0.9.0), with a running text model, a stopped one,
    /// an image model and a setup in progress.
    fn fixture(endpoint: &str) -> String {
        serde_json::json!({
            "models": {
                "qwen3.6-35b-a3b-llamacpp": {
                    "recipe": "qwen3.6-35b-a3b-llamacpp", "kind": "text",
                    "name": "Qwen3.6 35B A3B", "runtime": "llamacpp", "ctx": 131072,
                    "unit": "active", "sub_state": "running", "port": 11436,
                    "endpoint": endpoint, "fits": true, "observed": null
                },
                "qwen3.8-27b-gufo": {
                    "recipe": "qwen3.8-27b-gufo", "kind": "text", "ctx": 262144,
                    "unit": "inactive", "port": 11435,
                    "endpoint": "http://127.0.0.1:11435/v1", "fits": true
                },
                "flux2-klein-4b-sdcpp": {
                    "recipe": "flux2-klein-4b-sdcpp", "kind": "image", "ctx": null,
                    "unit": "active", "port": 11437,
                    "endpoint": "http://127.0.0.1:11437/v1"
                },
                "ace-step-1.5-turbo-acestep": {
                    "recipe": "ace-step-1.5-turbo-acestep", "kind": "music",
                    "unit": "installing", "port": null, "endpoint": "",
                    "install": {"phase": "failed", "done_bytes": 0, "total_bytes": 0}
                }
            },
            "share_error": "",
            "last": "qwen3.8-27b-gufo",
            "ram_total_gib": 91.9,
            "best": null,
            "link": {"state": "none"}
        })
        .to_string()
    }

    #[test]
    fn parses_the_documented_shape() {
        let status = parse_status(&fixture("http://127.0.0.1:11436/v1")).unwrap();
        assert_eq!(status.models.len(), 4);
        assert_eq!(status.last.as_deref(), Some("qwen3.8-27b-gufo"));
        let image = status
            .models
            .iter()
            .find(|m| m.recipe == "flux2-klein-4b-sdcpp")
            .unwrap();
        assert_eq!(image.kind, "image");
        // A row without `kind` is text, as the engine defaults it.
        let bare = parse_status(
            r#"{"models":{"m":{"unit":"active","endpoint":"http://127.0.0.1:1/v1"}}}"#,
        )
        .unwrap();
        assert_eq!(bare.models[0].kind, "text");
        assert_eq!(bare.models[0].recipe, "m");
        assert!(parse_status("not json").is_err());
        assert!(parse_status("{}").is_err());
    }

    #[test]
    fn picks_a_running_text_model_and_never_an_image_one() {
        let status = parse_status(&fixture("http://127.0.0.1:11436/v1")).unwrap();
        // `last` is stopped, so the running text model wins; the running
        // image model is never a candidate.
        assert_eq!(
            pick(&status, None).unwrap().recipe,
            "qwen3.6-35b-a3b-llamacpp"
        );
        let mut with_last = status.clone();
        with_last.last = Some("qwen3.6-35b-a3b-llamacpp".into());
        assert_eq!(
            pick(&with_last, None).unwrap().recipe,
            "qwen3.6-35b-a3b-llamacpp"
        );
    }

    #[test]
    fn a_named_model_must_be_running() {
        let status = parse_status(&fixture("http://127.0.0.1:11436/v1")).unwrap();
        let err = pick(&status, Some("qwen3.8-27b-gufo")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("is not running (inactive)"), "{msg}");
        assert!(
            msg.contains("omarchy models start qwen3.8-27b-gufo"),
            "{msg}"
        );
        let err = pick(&status, Some("nope")).unwrap_err();
        assert!(err.to_string().contains("no text model 'nope'"), "{err}");
        // An image model is not a text model, even by name.
        assert!(pick(&status, Some("flux2-klein-4b-sdcpp")).is_err());
    }

    #[test]
    fn no_running_model_says_how_to_start_one() {
        let mut status = parse_status(&fixture("http://127.0.0.1:11436/v1")).unwrap();
        for m in &mut status.models {
            if m.kind == "text" {
                m.unit = "inactive".into();
            }
        }
        let msg = pick(&status, None).unwrap_err().to_string();
        assert!(msg.contains("No Omarchy text model is running"), "{msg}");
        // `last` is set up, so it is the one named.
        assert!(
            msg.contains("omarchy models start qwen3.8-27b-gufo"),
            "{msg}"
        );
        assert!(msg.contains("never starts"), "{msg}");

        let empty = Status::default();
        let msg = pick(&empty, None).unwrap_err().to_string();
        assert!(msg.contains("omarchy models use best text"), "{msg}");
    }

    #[test]
    fn a_non_loopback_endpoint_is_never_used() {
        let status = parse_status(&fixture("http://192.168.1.20:11436/v1")).unwrap();
        assert!(matches!(pick(&status, None), Err(LlmError::NotRunning(_))));
    }

    fn fake_command(dir: &Path, json: &str) -> PathBuf {
        let fixture = dir.join("status.json");
        std::fs::write(&fixture, json).unwrap();
        let cmd = dir.join("omarchy-models");
        crate::test_support::write_script(
            &cmd,
            &format!(
                "#!/bin/sh\n[ \"$1 $2\" = \"status --json\" ] || exit 9\ncat '{}'\n",
                fixture.display()
            ),
        );
        cmd
    }

    #[test]
    fn discovers_through_the_status_command() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key");
        std::fs::write(&key, "fixture-key\n").unwrap();
        let env = OmarchyEnv {
            models_command: Some(fake_command(
                dir.path(),
                &fixture("http://127.0.0.1:11436/v1/"),
            )),
            key_file: Some(key.clone()),
            probe_ports: Vec::new(),
            status_timeout: Duration::from_secs(5),
            check_port_owner: true,
        };
        let ep = discover(&env, None).unwrap();
        assert_eq!(ep.provider, Provider::Omarchy);
        assert_eq!(ep.model, "qwen3.6-35b-a3b-llamacpp");
        assert_eq!(ep.base_url, "http://127.0.0.1:11436/v1");
        assert_eq!(ep.key_file.as_deref(), Some(key.as_path()));
        assert!(ep.is_local());

        // No key file: a clear error naming the file, not its content.
        let mut no_key = env.clone();
        no_key.key_file = Some(dir.path().join("missing-key"));
        let err = discover(&no_key, None).unwrap_err();
        assert!(matches!(err, LlmError::Key { .. }), "{err}");
    }

    #[test]
    fn without_the_command_the_documented_ports_are_asked() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key");
        std::fs::write(&key, "fixture-key\n").unwrap();
        let server = FakeServer::start(vec![Reply::models(&["qwen3.8-27b-gufo"])]);
        let env = OmarchyEnv {
            models_command: None,
            key_file: Some(key),
            probe_ports: vec![server.port()],
            status_timeout: Duration::from_secs(1),
            // The test owns the listener, so the /proc check passes too.
            check_port_owner: cfg!(target_os = "linux"),
        };
        let ep = discover(&env, None).unwrap();
        assert_eq!(ep.model, "qwen3.8-27b-gufo");
        assert_eq!(ep.base_url, server.base_url());
        let req = &server.requests()[0];
        assert_eq!(req.path, "/v1/models");
        assert_eq!(req.authorization.as_deref(), Some("Bearer fixture-key"));
    }

    #[test]
    fn without_a_key_file_no_port_is_asked() {
        let server = FakeServer::start(vec![Reply::models(&["m"])]);
        let env = OmarchyEnv {
            models_command: None,
            key_file: Some(PathBuf::from("/nonexistent/voxtype-test/key")),
            probe_ports: vec![server.port()],
            status_timeout: Duration::from_secs(1),
            check_port_owner: false,
        };
        let err = discover(&env, None).unwrap_err();
        assert!(matches!(err, LlmError::NotRunning(_)), "{err}");
        assert!(server.requests().is_empty());
    }

    #[test]
    fn proc_net_listener_ownership() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 0100007F:2CCB 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 1 1 0 100 0 0 10 0\n\
   1: 0100007F:2CCC 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1001        0 1 1 0 100 0 0 10 0\n\
   2: 0100007F:2CCD 0100007F:9999 01 00000000:00000000 00:00000000 00000000  1000        0 1 1 0 100 0 0 10 0\n";
        assert!(proc_net_has_listener(table, 11467, 1000)); // 0x2CCB
        assert!(!proc_net_has_listener(table, 11468, 1000), "another user's");
        assert!(!proc_net_has_listener(table, 11469, 1000), "not listening");
        assert!(!proc_net_has_listener(table, 11435, 1000));
    }
}
