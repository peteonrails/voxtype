//! `voxtype meeting recover` and the daemon's `recoverable` event, through
//! the real binary, against scratch XDG directories.
//!
//! The fixture is a meeting folder as a killed daemon leaves it: the
//! meeting still `active` in the index, its transcript saved after the last
//! chunk, nothing finished. The next daemon start must announce it on
//! meeting.sock; `recover` must list, finish and discard such meetings.

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
        let s = Self { dir };
        // No agent is ever asked for a summary; the post-meeting command
        // leaves a mark to show it ran.
        std::fs::write(
            s.path("config/voxtype/config.toml"),
            format!(
                "[hotkey]\nenabled = false\n\n[osd]\nenabled = false\n\n\
                 [whisper]\non_demand_loading = true\n\n\
                 [meeting]\nenabled = true\npost_meeting_command = \"touch {}\"\n\n\
                 [meeting.summary]\nauto = false\n",
                s.path("hook-ran").display()
            ),
        )
        .unwrap();
        s
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.dir.path().join(sub)
    }

    fn storage(&self) -> MeetingStorage {
        MeetingStorage::open(StorageConfig {
            storage_path: self.path("data/voxtype/meetings"),
            retain_audio: false,
            max_meetings: 0,
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

    /// A meeting as a daemon killed mid-meeting leaves it: active, with
    /// the transcript of the chunks it got through.
    fn killed_meeting(&self, title: &str) -> String {
        let storage = self.storage();
        let mut metadata = MeetingMetadata::new(Some(title.to_string()));
        metadata.started_at = chrono::Utc::now() - chrono::Duration::minutes(5);
        storage.create_meeting(&metadata).unwrap();
        let mut transcript = Transcript::new();
        for (id, speaker, source, text) in [
            (
                0,
                "You",
                AudioSource::Microphone,
                "Let's go through the release.",
            ),
            (1, "Remote 1", AudioSource::Loopback, "The build is green."),
        ] {
            let start = id as u64 * 5_000;
            let mut seg = TranscriptSegment::new(id, start, start + 4_000, text.into(), 0);
            seg.source = source;
            seg.speaker_id = Some(speaker.to_string());
            transcript.add_segment(seg);
        }
        transcript.total_chunks = 1;
        storage.save_transcript(&metadata.id, &transcript).unwrap();
        metadata.id.to_string()
    }

    fn daemon(&self) -> Daemon {
        let child = self
            .command(&["daemon"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let socket = self.path("runtime/voxtype/meeting.sock");
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
        Daemon {
            child,
            lines: BufReader::new(stream),
        }
    }
}

struct Daemon {
    child: Child,
    lines: BufReader<UnixStream>,
}

impl Daemon {
    /// The next meeting.sock event named `event`.
    fn next(&mut self, event: &str) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            assert!(Instant::now() < deadline, "no {event} event");
            let mut line = String::new();
            match self.lines.read_line(&mut line) {
                Ok(0) => panic!("meeting.sock closed before a {event} event"),
                Ok(_) => {
                    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
                    if v["event"] == event {
                        return v;
                    }
                }
                Err(e) => panic!("reading meeting.sock: {e}"),
            }
        }
    }

    /// SIGKILL, as a crash would end it.
    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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
fn the_next_daemon_announces_a_killed_meeting_and_recover_finishes_it() {
    let s = Scratch::new();
    let id = s.killed_meeting("Release sync");

    // Without a daemon, recover already sees it.
    let listed = json(&s.voxtype(&["meeting", "recover", "--json"]));
    assert_eq!(listed["meetings"][0]["id"], id);

    let mut daemon = s.daemon();
    // hello first, then the recovery card's list.
    let hello = daemon.next("hello");
    assert_eq!(hello["status"], "idle");
    let event = daemon.next("recoverable");
    let meetings = event["meetings"].as_array().unwrap();
    assert_eq!(meetings.len(), 1);
    let m = &meetings[0];
    assert_eq!(m["id"], id);
    assert_eq!(m["title"], "Release sync");
    assert_eq!(m["lines"], 2);
    assert_eq!(m["has_audio"], false);
    assert_eq!(m["last_chunk_ms"], 9_000);
    assert_eq!(m["untranscribed_chunks"], 0);
    assert!(m["started_at"].is_string() && m["stopped_at"].is_string());
    assert!(m["duration_secs"].as_u64().unwrap() >= 240, "{m}");
    let show = json(&s.voxtype(&["meeting", "show", &id, "--json"]));
    assert_eq!(show["metadata"]["status"], "interrupted");

    // A panel that connects later gets the list right after hello.
    let mut late = s.daemon_socket();
    assert_eq!(read_event(&mut late)["event"], "hello");
    assert_eq!(read_event(&mut late)["event"], "recoverable");

    let finished = json(&s.voxtype(&["meeting", "recover", "--finish", &id[..8], "--json"]));
    // The metadata, as `meeting show --json` has it, and what was done.
    assert_eq!(finished["id"], id);
    assert_eq!(finished["status"], "completed");
    assert!(finished["ended_at"].is_string());
    assert_eq!(finished["recovered"]["transcribed_chunks"], 0);
    assert_eq!(finished["recovered"]["summary"]["state"], "skipped");
    assert_eq!(finished["recovered"]["post_meeting_command"], "succeeded");
    assert!(s.path("hook-ran").exists());

    // The card goes away: the daemon announces an empty list.
    let event = daemon.next("recoverable");
    assert_eq!(event["meetings"], serde_json::json!([]));
    let show = json(&s.voxtype(&["meeting", "show", &id, "--json"]));
    assert_eq!(show["metadata"]["status"], "completed");
    assert!(show["metadata"]["ended_at"].is_string());
    assert!(
        json(&s.voxtype(&["meeting", "recover", "--json"]))["meetings"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // A finished meeting is not recoverable again.
    let again = s.voxtype(&["meeting", "recover", "--discard", &id]);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("not interrupted"));
    daemon.kill();
}

#[test]
fn a_meeting_left_by_a_killed_daemon_can_be_discarded() {
    let s = Scratch::new();
    // The first daemon dies by SIGKILL with the meeting still active.
    let first = s.daemon();
    let id = s.killed_meeting("Standup");
    first.kill();

    let mut daemon = s.daemon();
    let event = daemon.next("recoverable");
    assert_eq!(event["meetings"][0]["id"], id);
    let folder = s
        .storage()
        .get_meeting_path(&s.storage().resolve_meeting_id(&id).unwrap())
        .unwrap();
    assert!(folder.exists());

    let out = json(&s.voxtype(&["meeting", "recover", "--discard", &id, "--json"]));
    assert_eq!(out["discarded"], id);
    assert!(!folder.exists());
    assert_eq!(
        daemon.next("recoverable")["meetings"],
        serde_json::json!([])
    );
    let gone = s.voxtype(&["meeting", "show", &id]);
    assert!(!gone.status.success());
}

#[test]
fn with_nothing_to_recover_the_daemon_says_nothing() {
    let s = Scratch::new();
    let mut daemon = s.daemon();
    assert_eq!(daemon.next("hello")["status"], "idle");
    // The next line, if any, is not a recovery card.
    daemon
        .lines
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let mut line = String::new();
    let _ = daemon.lines.read_line(&mut line);
    assert!(!line.contains("recoverable"), "{line}");
    let out = s.voxtype(&["meeting", "recover"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("No meetings to recover."));
}

impl Scratch {
    fn daemon_socket(&self) -> BufReader<UnixStream> {
        let stream = UnixStream::connect(self.path("runtime/voxtype/meeting.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        BufReader::new(stream)
    }
}

fn read_event(lines: &mut BufReader<UnixStream>) -> serde_json::Value {
    let mut line = String::new();
    lines.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
