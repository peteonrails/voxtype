//! The GTK4 meetings window, headless: GTK's Broadway backend draws into a
//! private `gtk4-broadwayd`, a fake meeting.sock feeds it a live meeting,
//! and the window reports what it shows as JSON before quitting
//! (`VOXTYPE_MEETINGS_GTK4_SMOKE`). Skips when gtk4-broadwayd is missing.
//! Set `VOXTYPE_MEETINGS_GTK4_SHOT_DIR` to also save each window as a PNG.

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
    run_window_with(runtime, display, &[])
}

fn run_window_with(runtime: &std::path::Path, display: u32, args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_voxtype-meetings-gtk4"))
        .args(args)
        .env("GDK_BACKEND", "broadway")
        .env("BROADWAY_DISPLAY", format!(":{display}"))
        .env("XDG_RUNTIME_DIR", runtime)
        .env("HOME", runtime)
        .env("XDG_DATA_HOME", runtime.join("data"))
        .env("XDG_CONFIG_HOME", runtime.join("config"))
        .env("VOXTYPE_MEETINGS_GTK4_SMOKE", "3")
        .envs(std::env::var_os("VOXTYPE_MEETINGS_GTK4_SHOT_DIR").map(|d| {
            let name = format!("meetings-gtk4-{display}.png");
            (
                "VOXTYPE_MEETINGS_GTK4_SHOT",
                std::path::PathBuf::from(d).join(name),
            )
        }))
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
    eprintln!("{}", String::from_utf8_lossy(&out.stderr));
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
            let mut moved = seg(3, 12, "Remote 1", "Moved and fixed");
            moved["original_speaker"] = "Remote 2".into();
            moved["original_text"] = "moved and fixt".into();
            let more = serde_json::json!({"event": "segments", "chunk_id": 1,
                "segments": [seg(2, 9, "Remote 2", "Agreed"), seg(4, 15, "You", "Scratch that")]});
            let _ = writeln!(conn, "{more}");
            let _ = writeln!(
                conn,
                "{}",
                serde_json::json!({"event": "segment_updated", "segment": moved})
            );
            let _ = writeln!(
                conn,
                r#"{{"event":"segment_deleted","meeting_id":"m1","segment_id":4}}"#
            );
            // Levels come ten times a second while recording.
            for i in 0..100 {
                let level = serde_json::json!({"event": "levels", "meeting_id": "m1",
                    "t_ms": i * 100, "mic": -20.0, "mic_peak": -10.0,
                    "loopback": -45.0, "loopback_peak": -40.0});
                if writeln!(conn, "{level}").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
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
            "[00:09] Remote 2: Agreed",
            "[00:12] Remote 1 (was Remote 2): Moved and fixed (edited)"
        ]),
        "the deleted line is gone and the moved one is marked: {report}"
    );
    assert_eq!(
        report["chips"],
        serde_json::json!(["You", "Remote 1", "Remote 2"]),
        "{report}"
    );
    assert_eq!(report["chip_buttons"], true, "chips rename: {report}");
    assert_eq!(report["read_only"], false, "{report}");
    assert_eq!(report["move_visible"], true, "{report}");
    assert_eq!(report["notes_editable"], true, "{report}");
    assert_eq!(report["levels_visible"], true, "{report}");
}

#[test]
fn a_past_meeting_opens_read_only_with_its_summary() {
    use voxtype::meeting::data::{AudioSource, MeetingMetadata, MeetingSummary, Transcript};
    use voxtype::meeting::{MeetingStorage, StorageConfig, TranscriptSegment};

    let dir = tempfile::tempdir().unwrap();
    let runtime = dir.path();
    let Some(_bw) = broadway(runtime, 43) else {
        eprintln!("skipping: gtk4-broadwayd is not available");
        return;
    };
    // Where the window looks: $XDG_DATA_HOME/voxtype/meetings.
    let storage = MeetingStorage::open(StorageConfig {
        storage_path: runtime.join("data/voxtype/meetings"),
        ..StorageConfig::default()
    })
    .unwrap();
    let mut meta = MeetingMetadata::new(Some("Retro".into()));
    meta.duration_secs = Some(125);
    let folder = storage.create_meeting(&meta).unwrap();
    let mut transcript = Transcript::new();
    for (id, secs, who, text) in [
        (0, 1, AudioSource::Microphone, "Kept line"),
        (1, 5, AudioSource::Loopback, "Hidden line"),
        (2, 9, AudioSource::Loopback, "Last line"),
    ] {
        let mut s = TranscriptSegment::new(id, secs * 1000, secs * 1000 + 900, text.into(), 0);
        s.source = who;
        transcript.add_segment(s);
    }
    storage.save_transcript(&meta.id, &transcript).unwrap();
    // As `meeting delete-line` would.
    storage.delete_segment(&meta.id, 1, true).unwrap();
    let summary: MeetingSummary = serde_json::from_value(serde_json::json!({
        "summary": "Short version.", "generated_at": "2026-11-11T15:00:00Z", "model": "claude"
    }))
    .unwrap();
    storage.save_summary(&meta.id, &summary).unwrap();
    std::fs::write(
        folder.join("summary.md"),
        "# Retro\n\n## Decisions\n- Keep **Friday** releases\n",
    )
    .unwrap();

    // No daemon: the window opens the stored meeting it was asked for.
    let report = run_window_with(runtime, 43, &["--meeting", &meta.id.to_string()]);
    assert_eq!(report["screen"], "meeting", "{report}");
    assert_eq!(report["title"], "Retro", "{report}");
    assert_eq!(report["state"], "Ended", "{report}");
    assert_eq!(
        report["rows"],
        serde_json::json!(["[00:01] You: Kept line", "[00:09] Remote: Last line"]),
        "{report}"
    );
    assert_eq!(report["read_only"], true, "{report}");
    assert_eq!(report["move_visible"], false, "{report}");
    assert_eq!(report["notes_editable"], false, "{report}");
    assert_eq!(
        report["chip_buttons"], false,
        "chips are names, not buttons: {report}"
    );
    assert_eq!(report["pause_visible"], false, "{report}");
    let shown = report["summary"].as_str().unwrap();
    assert!(
        shown.starts_with("Retro\n\nDecisions\n• Keep Friday releases"),
        "summary.md, rendered: {shown:?}"
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
