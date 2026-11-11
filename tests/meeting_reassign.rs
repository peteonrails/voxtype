//! `voxtype meeting reassign` from the command line, against a scratch
//! XDG data and runtime directory: after a meeting has stopped it rewrites the
//! transcript, while one is recording it leaves the file to the daemon and
//! drops a trigger for it.

use std::process::{Command, Output};

use voxtype::meeting::data::{AudioSource, MeetingMetadata, Transcript, TranscriptSegment};
use voxtype::meeting::{MeetingStorage, StorageConfig};

struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["config/voxtype", "data/voxtype", "runtime/voxtype"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        Self { dir }
    }

    fn path(&self, sub: &str) -> std::path::PathBuf {
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

    fn voxtype(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_voxtype"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("XDG_RUNTIME_DIR", self.path("runtime"))
            .args(args)
            .output()
            .unwrap()
    }

    /// You, then two lines clustered as Remote 1 ("Primagen"), then Remote 2.
    fn meeting(&self) -> String {
        let storage = self.storage();
        let metadata = MeetingMetadata::new(Some("Reassign".to_string()));
        storage.create_meeting(&metadata).unwrap();
        let mut transcript = Transcript::new();
        for (id, speaker, source, text) in [
            (0, "You", AudioSource::Microphone, "Shall we start?"),
            (1, "Remote 1", AudioSource::Loopback, "Yes, let's go."),
            (
                2,
                "Remote 1",
                AudioSource::Loopback,
                "I disagree with that.",
            ),
            (3, "Remote 2", AudioSource::Loopback, "Me too."),
        ] {
            let start = id as u64 * 5_000;
            let mut seg = TranscriptSegment::new(id, start, start + 4_000, text.into(), 0);
            seg.source = source;
            seg.speaker_id = Some(speaker.to_string());
            transcript.add_segment(seg);
        }
        storage.save_transcript(&metadata.id, &transcript).unwrap();
        storage
            .set_speaker_label(&metadata.id, "Remote 1", "Primagen")
            .unwrap();
        metadata.id.to_string()
    }

    fn transcript_file(&self, id: &str) -> std::path::PathBuf {
        let storage = self.storage();
        let id = storage.resolve_meeting_id(id).unwrap();
        storage
            .get_meeting_path(&id)
            .unwrap()
            .join("transcript.json")
    }
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "voxtype failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn reassign_after_stop_shows_in_the_export() {
    let s = Scratch::new();
    let id = s.meeting();

    let out = stdout(&s.voxtype(&["meeting", "reassign", &id[..8], "2", "Casey", "--json"]));
    let segment: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(segment["id"], 2);
    assert_eq!(segment["speaker_id"], "Person 1");
    assert_eq!(segment["speaker_label"], "Casey");
    assert_eq!(segment["reassigned_from"], "Remote 1");
    assert_eq!(segment["text"], "I disagree with that.");

    let export = stdout(&s.voxtype(&[
        "meeting",
        "export",
        "latest",
        "--format",
        "text",
        "--speakers",
    ]));
    let casey = export.find("Casey:").expect("export names Casey");
    let line = export.find("I disagree with that.").unwrap();
    assert!(casey < line, "{export}");
    assert!(export.find("Primagen:").unwrap() < export.find("Yes, let's go.").unwrap());

    // `show --json` carries the move and every segment's id.
    let show: serde_json::Value = serde_json::from_str(&stdout(
        &s.voxtype(&["meeting", "show", "latest", "--json"]),
    ))
    .unwrap();
    let ids: Vec<u64> = show["transcript"]["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, vec![0, 1, 2, 3]);
    assert_eq!(show["transcript"]["segments"][2]["speaker_label"], "Casey");

    // --reset gives it back.
    let out = stdout(&s.voxtype(&["meeting", "reassign", "latest", "2", "--reset"]));
    assert!(out.contains("back with Primagen"), "{out}");
    let export = stdout(&s.voxtype(&[
        "meeting",
        "export",
        "latest",
        "--format",
        "text",
        "--speakers",
    ]));
    assert!(!export.contains("Casey"), "{export}");
}

#[test]
fn reassign_matches_existing_speakers_like_label_does() {
    let s = Scratch::new();
    s.meeting();
    let out = stdout(&s.voxtype(&["meeting", "reassign", "latest", "1", "remote 2", "--json"]));
    let segment: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(segment["speaker_id"], "Remote 2");

    let out = stdout(&s.voxtype(&["meeting", "reassign", "latest", "3", "primagen", "--json"]));
    let segment: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(segment["speaker_id"], "Remote 1");
    assert_eq!(segment["speaker_label"], "Primagen");
}

#[test]
fn reassign_rejects_an_unknown_segment_with_the_valid_range() {
    let s = Scratch::new();
    s.meeting();
    let output = s.voxtype(&["meeting", "reassign", "latest", "99", "Casey"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Segment 99 is not in this meeting. Valid segment ids: 0 to 3."),
        "{stderr}"
    );
}

#[test]
fn reassign_needs_a_speaker_or_reset() {
    let s = Scratch::new();
    s.meeting();
    assert!(!s
        .voxtype(&["meeting", "reassign", "latest", "1"])
        .status
        .success());
    assert!(!s
        .voxtype(&["meeting", "reassign", "latest", "1", "Casey", "--reset"])
        .status
        .success());
}

/// While the meeting records, the CLI stores the move and pokes the daemon;
/// rewriting transcript.json itself could clobber a chunk the daemon saved.
#[test]
fn reassign_during_a_meeting_leaves_the_file_to_the_daemon() {
    let s = Scratch::new();
    let id = s.meeting();
    std::fs::write(
        s.path("runtime/voxtype/meeting_state"),
        format!("recording\n{id}\n"),
    )
    .unwrap();
    let before = std::fs::read_to_string(s.transcript_file(&id)).unwrap();

    let out = stdout(&s.voxtype(&["meeting", "reassign", "latest", "2", "Casey", "--json"]));
    let segment: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(segment["speaker_label"], "Casey");

    assert_eq!(
        std::fs::read_to_string(s.transcript_file(&id)).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read_to_string(s.path("runtime/voxtype/meeting_reassign")).unwrap(),
        "2"
    );
    // Readers see the move even before the daemon applies it.
    let show: serde_json::Value = serde_json::from_str(&stdout(
        &s.voxtype(&["meeting", "show", "latest", "--json"]),
    ))
    .unwrap();
    assert_eq!(show["transcript"]["segments"][2]["speaker_label"], "Casey");
}
