//! `voxtype meeting title`, through the real binary, against scratch XDG
//! directories: a finished meeting is retitled in storage; a running one
//! also through the daemon, which tells meeting.sock.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use voxtype::meeting::data::{AudioSource, MeetingMetadata, Transcript, TranscriptSegment};
use voxtype::meeting::{MeetingStorage, StorageConfig};

struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["config/voxtype", "data/voxtype", "runtime/voxtype", "state"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        std::fs::write(
            dir.path().join("config/voxtype/config.toml"),
            "[hotkey]\nenabled = false\n\n[osd]\nenabled = false\n\n\
             [whisper]\non_demand_loading = true\n\n[meeting]\nenabled = true\n\n\
             [meeting.summary]\nauto = false\n",
        )
        .unwrap();
        Self { dir }
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.dir.path().join(sub)
    }

    fn storage(&self) -> MeetingStorage {
        MeetingStorage::open(StorageConfig {
            storage_path: self.path("data/voxtype/meetings"),
            retain_audio: false,
            max_meetings: 0,
            my_name: None,
        })
        .unwrap()
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_voxtype"));
        cmd.env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("XDG_CACHE_HOME", self.path("cache"))
            .env("XDG_RUNTIME_DIR", self.path("runtime"))
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env("RUST_LOG", "warn")
            .args(args)
            .stdin(Stdio::null());
        cmd
    }

    fn voxtype(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn meeting(&self, title: &str) -> (String, PathBuf) {
        let storage = self.storage();
        let metadata = MeetingMetadata::new(Some(title.to_string()));
        let folder = storage.create_meeting(&metadata).unwrap();
        let mut transcript = Transcript::new();
        for (id, speaker, source) in [
            (0, "You", AudioSource::Microphone),
            (1, "Remote", AudioSource::Loopback),
        ] {
            let mut seg = TranscriptSegment::new(id, id as u64 * 5_000, 4_000, "Hi.".into(), 0);
            seg.source = source;
            seg.speaker_id = Some(speaker.to_string());
            transcript.add_segment(seg);
        }
        storage.save_transcript(&metadata.id, &transcript).unwrap();
        (metadata.id.to_string(), folder)
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

#[test]
fn a_finished_meeting_is_retitled_everywhere_but_its_folder() {
    let s = Scratch::new();
    let (id, folder) = s.meeting("Zoom call");

    let out = json(&s.voxtype(&["meeting", "title", &id[..8], "  Q4   planning ", "--json"]));
    assert_eq!(out["meeting_id"], id);
    assert_eq!(out["title"], "Q4 planning");
    assert_eq!(out["previous_title"], "Zoom call");
    assert_eq!(out["live"], false);
    assert!(out["labelled"].as_array().unwrap().is_empty());

    let show = json(&s.voxtype(&["meeting", "show", &id, "--json"]));
    assert_eq!(show["metadata"]["title"], "Q4 planning");
    let list = String::from_utf8(s.voxtype(&["meeting", "list"]).stdout).unwrap();
    assert!(list.contains("Q4 planning"), "{list}");
    assert!(std::fs::read_to_string(folder.join("transcript.md"))
        .unwrap()
        .starts_with("# Q4 planning"));
    assert!(folder.exists(), "the folder keeps its name");

    let empty = s.voxtype(&["meeting", "title", &id, "   "]);
    assert!(!empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("can't be empty"));
    let show = json(&s.voxtype(&["meeting", "show", &id, "--json"]));
    assert_eq!(show["metadata"]["title"], "Q4 planning");
}

#[test]
fn a_running_meeting_is_retitled_through_the_daemon() {
    let s = Scratch::new();
    let (id, folder) = s.meeting("Zoom call");
    let md_before = std::fs::read_to_string(folder.join("transcript.md")).unwrap();

    let _daemon = Daemon(
        s.command(&["daemon"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let socket = s.path("runtime/voxtype/meeting.sock");
    let started = Instant::now();
    let stream = loop {
        if let Ok(stream) = UnixStream::connect(&socket) {
            break stream;
        }
        assert!(started.elapsed() < Duration::from_secs(30), "no daemon");
        std::thread::sleep(Duration::from_millis(50));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut lines = BufReader::new(stream);

    // The daemon is recording this meeting, as far as the CLI can tell.
    std::fs::write(
        s.path("runtime/voxtype/meeting_state"),
        format!("recording\n{id}\n"),
    )
    .unwrap();
    let out = json(&s.voxtype(&["meeting", "title", &id, "Pete <> Chad", "--json"]));
    assert_eq!(out["live"], true);
    // Stored at once; the transcript is the daemon's to rewrite.
    let show = json(&s.voxtype(&["meeting", "show", &id, "--json"]));
    assert_eq!(show["metadata"]["title"], "Pete <> Chad");
    assert_eq!(
        std::fs::read_to_string(folder.join("transcript.md")).unwrap(),
        md_before
    );

    let deadline = Instant::now() + Duration::from_secs(15);
    let event = loop {
        assert!(Instant::now() < deadline, "no title_changed event");
        let mut line = String::new();
        lines.read_line(&mut line).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v["event"] == "title_changed" {
            break v;
        }
    };
    assert_eq!(
        event,
        serde_json::json!({"event": "title_changed", "meeting_id": id, "title": "Pete <> Chad"})
    );
    assert!(
        !s.path("runtime/voxtype/meeting_title").exists(),
        "consumed"
    );
}

/// Stops the daemon however the test ends.
struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
