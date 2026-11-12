//! `voxtype meeting share`, through the real binary, against scratch XDG
//! directories: the preview the meeting view shows, a saved file, and a
//! share command.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use voxtype::meeting::data::{
    ActionItem, AudioSource, MeetingMetadata, MeetingSummary, Transcript, TranscriptSegment,
};
use voxtype::meeting::{MeetingStorage, StorageConfig};

struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["config/voxtype", "data/voxtype", "runtime", "state", "out"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        let s = Self { dir };
        std::fs::write(
            s.path("config/voxtype/config.toml"),
            format!(
                "[meeting]\nenabled = true\nmy_name = \"Pete\"\n\n\
                 [meeting.share]\nfolder = \"{}\"\n",
                s.path("out").display()
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
            my_name: None,
        })
        .unwrap()
    }

    /// A finished meeting with a summary, notes and three lines, one of
    /// them deleted.
    fn meeting(&self) -> String {
        let storage = self.storage();
        let mut metadata = MeetingMetadata::new(Some("Release sync".into()));
        metadata.duration_secs = Some(30 * 60);
        storage.create_meeting(&metadata).unwrap();
        let mut transcript = Transcript::new();
        for (id, speaker, source, text) in [
            (0, "You", AudioSource::Microphone, "Is the build green?"),
            (1, "Remote", AudioSource::Loopback, "It is."),
            (2, "Remote", AudioSource::Loopback, "Sorry, wrong window."),
        ] {
            let start = id as u64 * 5_000;
            let mut seg = TranscriptSegment::new(id, start, start + 4_000, text.into(), 0);
            seg.source = source;
            seg.speaker_id = Some(speaker.to_string());
            transcript.add_segment(seg);
        }
        storage.save_transcript(&metadata.id, &transcript).unwrap();
        storage
            .set_speaker_label(&metadata.id, "Remote", "Dana")
            .unwrap();
        storage.delete_segment(&metadata.id, 2, true).unwrap();
        storage
            .set_notes(&metadata.id, "Check the changelog.\n")
            .unwrap();
        storage
            .save_summary(
                &metadata.id,
                &MeetingSummary {
                    summary: "The build is green and we ship Friday.".into(),
                    key_points: vec![],
                    action_items: vec![ActionItem {
                        description: "Tag the release".into(),
                        assignee: Some("Dana".into()),
                        due_date: None,
                        completed: false,
                    }],
                    decisions: vec!["Ship Friday".into()],
                    chapters: vec![],
                    generated_at: chrono::Utc::now(),
                    model: None,
                },
            )
            .unwrap();
        metadata.id.to_string()
    }

    fn voxtype(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_voxtype"))
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("XDG_RUNTIME_DIR", self.path("runtime"))
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env("RUST_LOG", "warn")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

fn ok(output: &Output) -> String {
    assert!(
        output.status.success(),
        "voxtype failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn the_preview_shows_exactly_what_would_leave() {
    let s = Scratch::new();
    let id = s.meeting();
    let preview: serde_json::Value = serde_json::from_str(&ok(&s.voxtype(&[
        "meeting",
        "share",
        &id,
        "--transcript",
        "--preview",
        "--json",
    ])))
    .unwrap();
    assert_eq!(preview["meeting_id"], id);
    assert_eq!(preview["title"], "Release sync");
    assert_eq!(preview["format"], "markdown");
    assert_eq!(preview["transcript"], true);
    assert_eq!(preview["transcript_lines"], 2);
    assert_eq!(
        preview["sections"],
        serde_json::json!([
            "overview",
            "decisions",
            "action_items",
            "notes",
            "transcript"
        ])
    );
    assert_eq!(preview["available"]["file"], true);
    assert_eq!(preview["available"]["command"], false);
    assert!(preview["why"]["command"]
        .as_str()
        .unwrap()
        .contains("meeting.share.command"));
    assert_eq!(preview["folder"], s.path("out").display().to_string());
    let body = preview["body"].as_str().unwrap();
    assert!(body.contains("**Attendees:** Pete, Dana"), "{body}");
    assert!(body.contains("- [ ] Tag the release (Dana)"));
    assert!(body.contains("**[00:05] Dana:** It is."));
    assert!(!body.contains("wrong window"), "deleted lines never leave");
    assert_eq!(preview["chars"], body.chars().count());

    // Email previews the text its draft carries.
    let email: serde_json::Value = serde_json::from_str(&ok(&s.voxtype(&[
        "meeting",
        "share",
        &id,
        "--to",
        "email",
        "--preview",
        "--json",
    ])))
    .unwrap();
    assert_eq!(email["format"], "text");
    assert!(email["subject"]
        .as_str()
        .unwrap()
        .starts_with("Release sync ("));
}

#[test]
fn file_and_command_shares_send_the_rendered_notes() {
    let s = Scratch::new();
    let id = s.meeting();
    let saved: serde_json::Value = serde_json::from_str(&ok(&s.voxtype(&[
        "meeting", "share", "latest", "--to", "file", "--format", "txt", "--json",
    ])))
    .unwrap();
    let path = PathBuf::from(saved["path"].as_str().unwrap());
    assert!(path.starts_with(s.path("out")), "{path:?}");
    assert_eq!(path.extension().unwrap(), "txt");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("Release sync\n============\n"), "{text}");
    assert!(!text.contains("It is."), "no transcript unless asked");

    let sink = s.path("sink");
    let out = ok(&s.voxtype(&[
        "meeting",
        "share",
        &id,
        "--to",
        "command",
        "--command",
        &format!("cat > {}; echo posted", sink.display()),
    ]));
    assert!(out.contains("sent to the share command"), "{out}");
    assert!(out.contains("posted"));
    assert!(std::fs::read_to_string(&sink)
        .unwrap()
        .starts_with("# Release sync\n"));

    // No command configured or given: a clear error, nothing run.
    let none = s.voxtype(&["meeting", "share", &id, "--to", "command"]);
    assert!(!none.status.success());
    assert!(String::from_utf8_lossy(&none.stderr).contains("meeting.share.command"));
}
