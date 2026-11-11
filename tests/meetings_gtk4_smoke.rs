//! The GTK4 meetings window, headless: GTK's Broadway backend draws into a
//! private `gtk4-broadwayd`, a fake meeting.sock feeds it a live meeting,
//! and the window reports what it shows as JSON before quitting
//! (`VOXTYPE_MEETINGS_GTK4_SMOKE`). Skips when gtk4-broadwayd is missing.

#![cfg(all(unix, feature = "meetings-gtk4"))]

use std::io::Write;
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::time::Duration;

fn seg(id: u32, secs: u64, who: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "start_ms": secs * 1000, "end_ms": secs * 1000 + 900, "text": text,
        "source": "loopback", "speaker_id": who, "chunk_id": 0
    })
}

struct Broadway(std::process::Child);

impl Drop for Broadway {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_window(runtime: &std::path::Path, display: u32) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_voxtype-meetings-gtk4"))
        .env("GDK_BACKEND", "broadway")
        .env("BROADWAY_DISPLAY", format!(":{display}"))
        .env("XDG_RUNTIME_DIR", runtime)
        .env("HOME", runtime)
        .env("XDG_DATA_HOME", runtime.join("data"))
        .env("XDG_CONFIG_HOME", runtime.join("config"))
        .env("VOXTYPE_MEETINGS_GTK4_SMOKE", "3")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .rfind(|l| l.starts_with('{'))
        .unwrap_or_else(|| panic!("no report: {stdout}"));
    serde_json::from_str(line).unwrap()
}

fn broadway(runtime: &std::path::Path, display: u32) -> Option<Broadway> {
    let child = Command::new("gtk4-broadwayd")
        .arg(format!(":{display}"))
        .env("XDG_RUNTIME_DIR", runtime)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    std::thread::sleep(Duration::from_millis(500));
    Some(Broadway(child))
}

#[test]
fn the_window_shows_a_live_meeting() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = dir.path();
    let Some(_bw) = broadway(runtime, 41) else {
        eprintln!("skipping: gtk4-broadwayd is not available");
        return;
    };
    let sock_dir = runtime.join("voxtype");
    std::fs::create_dir_all(&sock_dir).unwrap();
    let listener = UnixListener::bind(sock_dir.join("meeting.sock")).unwrap();
    std::thread::spawn(move || {
        let hello = serde_json::json!({
            "event": "hello", "status": "recording", "meeting_id": "m1", "title": "Standup",
            "started_at": "2026-11-11T14:00:00Z", "chunk_count": 1,
            "segments": [seg(0, 1, "You", "Good morning"), seg(1, 4, "Remote 1", "We ship <Friday> & celebrate")]
        });
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { break };
            let _ = writeln!(conn, "{hello}");
            std::thread::sleep(Duration::from_millis(600));
            let more = serde_json::json!({"event": "segments", "chunk_id": 1,
                "segments": [seg(2, 9, "Remote 2", "Agreed")]});
            let _ = writeln!(conn, "{more}");
            std::thread::sleep(Duration::from_secs(10));
        }
    });

    let report = run_window(runtime, 41);
    assert_eq!(report["screen"], "meeting", "{report}");
    assert_eq!(report["title"], "Standup", "{report}");
    assert_eq!(report["state"], "● Recording", "{report}");
    assert_eq!(report["connected"], true);
    assert_eq!(report["pause_visible"], true);
    assert_eq!(
        report["rows"],
        serde_json::json!([
            "[00:01] You: Good morning",
            "[00:04] Remote 1: We ship <Friday> & celebrate",
            "[00:09] Remote 2: Agreed"
        ]),
        "{report}"
    );
    assert_eq!(
        report["chips"],
        serde_json::json!(["You", "Remote 1", "Remote 2"]),
        "{report}"
    );
}

#[test]
fn without_a_daemon_the_window_lists_meetings() {
    let dir = tempfile::tempdir().unwrap();
    let Some(_bw) = broadway(dir.path(), 42) else {
        eprintln!("skipping: gtk4-broadwayd is not available");
        return;
    };
    let report = run_window(dir.path(), 42);
    assert_eq!(report["screen"], "list", "{report}");
    assert_eq!(report["connected"], false);
    assert_eq!(report["pause_visible"], false);
    assert_eq!(report["list_rows"], 1, "the empty-list message: {report}");
}
