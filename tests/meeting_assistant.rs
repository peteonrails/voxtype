//! `voxtype meeting ask`, `meeting assistant` and `meeting summarize` end to
//! end, through the real binary.
//!
//! The agents are shell scripts on a private PATH that record their argv
//! and stdin and print canned answers; no real agent is ever run. The
//! notification test starts its own `dbus-daemon` and serves a fake
//! `org.freedesktop.Notifications` on it, so nothing touches the user's
//! session bus or desktop.

#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use voxtype::meeting::{
    AudioSource, MeetingMetadata, MeetingStorage, StorageConfig, Transcript, TranscriptSegment,
};
use zbus::zvariant::OwnedValue;
use zbus::SignalContext;

struct World {
    dir: tempfile::TempDir,
}

fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn claude_stream(deltas: &[&str], result: &str) -> String {
    let mut lines = vec![r#"{"type":"system","subtype":"init"}"#.to_string()];
    for d in deltas {
        lines.push(
            serde_json::json!({"type": "stream_event", "event": {"type": "content_block_delta",
                "index": 0, "delta": {"type": "text_delta", "text": d}}})
            .to_string(),
        );
    }
    lines.push(
        serde_json::json!({"type": "result", "subtype": "success", "is_error": false, "result": result})
            .to_string(),
    );
    lines.join("\n") + "\n"
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let w = Self { dir };
        for d in [
            "bin", "out", "meetings", "state", "config", "data", "runtime",
        ] {
            std::fs::create_dir_all(w.p(d)).unwrap();
        }
        // Only these tools are on PATH, so no real agent or
        // omarchy-default-agent on this machine can be found.
        for tool in ["sh", "cat", "cp", "ls", "grep", "env", "sleep", "sort"] {
            for base in ["/usr/bin", "/bin"] {
                let src = Path::new(base).join(tool);
                if src.exists() {
                    let _ = std::os::unix::fs::symlink(&src, w.p("bin").join(tool));
                    break;
                }
            }
        }
        let out = w.p("out");
        script(
            &w.p("bin").join("claude"),
            &format!(
                r#"printf '%s\n' "$@" > '{o}/claude.argv'
cat > '{o}/claude.stdin'
cat '{o}/claude.reply'"#,
                o = out.display()
            ),
        );
        std::fs::write(
            out.join("claude.reply"),
            claude_stream(
                &["Steve said ", "we ship Friday [00:41]."],
                "Steve said we ship Friday [00:41].",
            ),
        )
        .unwrap();

        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ollama = format!("http://{}", dead.local_addr().unwrap());
        drop(dead);
        std::fs::write(
            w.p("config.toml"),
            format!(
                r#"[meeting]
enabled = true
storage_path = "{}"

[meeting.summary]
ollama_url = "{}"

[meeting.assistant]
open_command = "env | grep ^VOXTYPE_ | sort > '{}/opened'"
"#,
                w.p("meetings").display(),
                ollama,
                out.display()
            ),
        )
        .unwrap();
        w
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn meeting(&self) -> String {
        let storage = MeetingStorage::open(StorageConfig {
            storage_path: self.p("meetings"),
            retain_audio: false,
            max_meetings: 0,
        })
        .unwrap();
        let mut metadata = MeetingMetadata::new(Some("Release sync".into()));
        metadata.storage_path = Some(storage.create_meeting(&metadata).unwrap());
        let mut t = Transcript::new();
        for (i, (secs, who, text)) in [(41u64, "Remote", "We ship Friday."), (52, "You", "Agreed.")]
            .iter()
            .enumerate()
        {
            let mut s = TranscriptSegment::new(
                i as u32,
                secs * 1000,
                secs * 1000 + 2000,
                text.to_string(),
                0,
            );
            s.source = if *who == "You" {
                AudioSource::Microphone
            } else {
                AudioSource::Loopback
            };
            s.speaker_id = Some(who.to_string());
            t.add_segment(s);
        }
        storage.save_transcript(&metadata.id, &t).unwrap();
        metadata.complete();
        storage.update_meeting(&metadata).unwrap();
        storage
            .set_speaker_label(&metadata.id, "Remote", "Steve")
            .unwrap();
        metadata.id.to_string()
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("VOXTYPE_") {
                cmd.env_remove(name);
            }
        }
        cmd.env("PATH", self.p("bin"))
            .env("HOME", self.dir.path())
            .env("XDG_STATE_HOME", self.p("state"))
            .env("XDG_CONFIG_HOME", self.p("config"))
            .env("XDG_DATA_HOME", self.p("data"))
            .env("XDG_RUNTIME_DIR", self.p("runtime"))
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .stdin(Stdio::null())
            .arg("--config")
            .arg(self.p("config.toml"))
            .args(args);
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn json_lines(o: &Output) -> Vec<serde_json::Value> {
    text(&o.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

#[test]
fn ask_refuses_until_allowed_then_streams_and_records() {
    let w = World::new();
    let id = w.meeting();

    let o = w.run(&["meeting", "ask", "latest", "What did Steve say?"]);
    assert!(!o.status.success());
    let err = text(&o.stderr);
    assert!(
        err.contains("voxtype meeting assistant allow claude"),
        "{err}"
    );
    assert!(!w.p("out/claude.stdin").exists(), "nothing was sent");

    let o = w.run(&["meeting", "ask", "latest", "What did Steve say?", "--json"]);
    assert!(!o.status.success());
    let lines = json_lines(&o);
    assert_eq!(lines[0]["event"], "error");
    assert_eq!(lines[0]["code"], "consent_required");

    let o = w.run(&["meeting", "assistant", "allow", "claude"]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    assert!(w.p("state/voxtype/assistant-consent.json").exists());

    let o = w.run(&["meeting", "ask", "latest", "What did Steve say?"]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    assert_eq!(text(&o.stdout), "Steve said we ship Friday [00:41].\n");
    assert!(text(&o.stderr).contains("Answered by Claude"));
    let prompt = std::fs::read_to_string(w.p("out/claude.stdin")).unwrap();
    assert!(
        prompt.contains("[00:41] Steve: We ship Friday.\n[00:52] You: Agreed.\n"),
        "{prompt}"
    );

    // A unique ID prefix works, as for every other meeting command.
    let o = w.run(&[
        "meeting",
        "ask",
        &id[..8],
        "What did Steve say?",
        "--last",
        "1m",
        "--json",
    ]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    let lines = json_lines(&o);
    assert_eq!(lines[0]["event"], "start");
    assert_eq!(lines[0]["agent"], "claude");
    assert_eq!(lines[0]["agent_name"], "Claude");
    assert_eq!(lines[0]["meeting_id"], id.as_str());
    assert_eq!(lines[0]["last_secs"], 60);
    let deltas: String = lines
        .iter()
        .filter(|l| l["event"] == "delta")
        .map(|l| l["text"].as_str().unwrap())
        .collect();
    assert_eq!(deltas, "Steve said we ship Friday [00:41].");
    let answer = lines.last().unwrap();
    assert_eq!(answer["event"], "answer");
    assert_eq!(answer["answer"], "Steve said we ship Friday [00:41].");
    assert_eq!(answer["citations"][0]["ms"], 41_000);

    let o = w.run(&["meeting", "show", "latest", "--json"]);
    assert!(o.status.success());
    let shown: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let asks = shown["asks"].as_array().unwrap();
    assert_eq!(asks.len(), 2);
    assert_eq!(asks[1]["id"], answer["id"]);
    assert_eq!(asks[1]["last_secs"], 60);

    let o = w.run(&["meeting", "assistant", "status", "--json"]);
    let status: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(status["agent"], "claude");
    assert_eq!(status["source"], "path");
    assert_eq!(status["consent"], true);
    assert_eq!(status["backend"], "agent");
    assert_eq!(status["omarchy_default"], serde_json::Value::Null);
    assert_eq!(status["auto_summary"], true);

    let o = w.run(&["meeting", "assistant", "revoke", "claude"]);
    assert!(o.status.success());
    let o = w.run(&["meeting", "ask", "latest", "Again?"]);
    assert!(!o.status.success());
}

#[test]
fn local_flag_never_runs_the_agent() {
    let w = World::new();
    w.meeting();
    let o = w.run(&["meeting", "ask", "latest", "q", "--local", "--json"]);
    assert!(!o.status.success());
    let lines = json_lines(&o);
    assert_eq!(lines.last().unwrap()["code"], "local_failed");
    assert!(!w.p("out/claude.stdin").exists());
}

#[test]
fn summarize_saves_and_regenerates() {
    let w = World::new();
    w.meeting();
    w.run(&["meeting", "assistant", "allow", "claude"]);
    std::fs::write(
        w.p("out/claude.reply"),
        claude_stream(
            &[],
            r#"{"overview": "Ship Friday.", "action_items": [{"description": "Tag it", "owner": "Steve"}], "chapters": [{"start": "00:41", "title": "Date"}]}"#,
        ),
    )
    .unwrap();
    let o = w.run(&["meeting", "summarize", "latest", "--format", "json"]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    let s: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(s["summary"], "Ship Friday.");
    assert_eq!(s["chapters"][0]["start_ms"], 41_000);

    // Saved: a second run does not call the agent.
    std::fs::remove_file(w.p("out/claude.stdin")).unwrap();
    let o = w.run(&["meeting", "summarize", "latest"]);
    assert!(o.status.success());
    assert!(text(&o.stderr).contains("Saved summary"));
    assert!(text(&o.stdout).contains("- [00:41] Date"));
    assert!(!w.p("out/claude.stdin").exists());

    let o = w.run(&["meeting", "summarize", "latest", "--regenerate"]);
    assert!(o.status.success());
    assert!(w.p("out/claude.stdin").exists());

    let o = w.run(&["meeting", "show", "latest", "--json"]);
    let shown: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(shown["metadata"]["summary"]["summary"], "Ship Friday.");
}

// --- --notify over a private bus -----------------------------------------

struct PrivateBus {
    child: std::process::Child,
    address: String,
}

impl PrivateBus {
    fn start() -> Option<Self> {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take()?)
            .read_line(&mut line)
            .ok()?;
        let address = line.trim().to_string();
        (!address.is_empty()).then_some(Self { child, address })
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Answers every notification with `answer` after a moment.
struct FakeServer {
    answer: String,
    log: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl FakeServer {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        app_name: String,
        _replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        _hints: HashMap<String, OwnedValue>,
        _expire_timeout: i32,
    ) -> u32 {
        self.log.lock().unwrap().push(format!(
            "notify {} | {} | {} | {}",
            app_name,
            summary,
            body,
            actions.join(",")
        ));
        let answer = self.answer.clone();
        let ctxt = ctxt.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = FakeServer::action_invoked(&ctxt, 7, &answer).await;
        });
        7
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into()]
    }

    fn close_notification(&self, id: u32) {
        self.log.lock().unwrap().push(format!("close {id}"));
    }

    #[zbus(signal)]
    async fn action_invoked(
        ctxt: &SignalContext<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

#[tokio::test(flavor = "multi_thread")]
async fn notify_shows_the_answer_and_open_runs_the_command() {
    let Some(bus) = PrivateBus::start() else {
        eprintln!("skipping: dbus-daemon is not available");
        return;
    };
    let log = Arc::new(Mutex::new(Vec::new()));
    let _server = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("org.freedesktop.Notifications")
        .unwrap()
        .serve_at(
            "/org/freedesktop/Notifications",
            FakeServer {
                answer: "open".into(),
                log: log.clone(),
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();

    let w = World::new();
    let id = w.meeting();
    w.run(&["meeting", "assistant", "allow", "claude"]);
    let mut cmd = w.command(&[
        "meeting",
        "ask",
        "latest",
        "When <do> we ship?",
        "--notify",
        "--json",
    ]);
    cmd.env("DBUS_SESSION_BUS_ADDRESS", &bus.address);
    let o = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::task::spawn_blocking(move || cmd.output().unwrap()),
    )
    .await
    .expect("ask --notify returns once Open is clicked")
    .unwrap();
    assert!(o.status.success(), "{}", text(&o.stderr));
    let answer = json_lines(&o).pop().unwrap();

    let seen = log.lock().unwrap().clone();
    assert_eq!(
        seen[0],
        "notify Voxtype | Claude answered: When &lt;do&gt; we ship? | \
         Steve said we ship Friday [00:41]. | default,Open,open,Open"
    );
    assert!(seen.contains(&"close 7".to_string()), "{seen:?}");

    // The Open command runs detached; give it a moment.
    let opened = w.p("out/opened");
    let mut env = String::new();
    for _ in 0..50 {
        env = std::fs::read_to_string(&opened).unwrap_or_default();
        if env.lines().count() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        env.contains(&format!(
            "VOXTYPE_ASK_ID={}",
            answer["id"].as_str().unwrap()
        )),
        "{env}"
    );
    assert!(env.contains(&format!("VOXTYPE_MEETING_ID={id}")), "{env}");
}
