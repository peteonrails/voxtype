//! `voxtype meeting reassign`, `notes` and `annotate` from the command line,
//! against scratch XDG data and runtime directories. After a meeting has
//! stopped they rewrite the transcript; while one is recording they leave the
//! file to the daemon and drop a trigger for it.

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
            my_name: None,
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
    assert_eq!(segment["original_speaker"], "Remote 1");
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
        std::fs::read_to_string(s.path("runtime/voxtype/meeting_edit")).unwrap(),
        "2"
    );
    // Readers see the move even before the daemon applies it.
    let show: serde_json::Value = serde_json::from_str(&stdout(
        &s.voxtype(&["meeting", "show", "latest", "--json"]),
    ))
    .unwrap();
    assert_eq!(show["transcript"]["segments"][2]["speaker_label"], "Casey");
}

#[test]
fn notes_print_append_set_and_json() {
    let s = Scratch::new();
    let id = s.meeting();

    assert_eq!(stdout(&s.voxtype(&["meeting", "notes", "latest"])), "");
    let out = stdout(&s.voxtype(&["meeting", "notes", "latest", "--append", "Ask about budget"]));
    assert!(out.contains("Notes saved"), "{out}");
    stdout(&s.voxtype(&["meeting", "notes", &id[..8], "--append", "Follow up Friday"]));
    assert_eq!(
        stdout(&s.voxtype(&["meeting", "notes", "latest"])),
        "Ask about budget\nFollow up Friday\n"
    );

    let file = s.path("new-notes.md");
    std::fs::write(&file, "# Replaced\n").unwrap();
    stdout(&s.voxtype(&[
        "meeting",
        "notes",
        "latest",
        "--set-file",
        file.to_str().unwrap(),
    ]));
    let json: serde_json::Value = serde_json::from_str(&stdout(
        &s.voxtype(&["meeting", "notes", "latest", "--json"]),
    ))
    .unwrap();
    assert_eq!(json["meeting_id"], id);
    assert_eq!(json["notes"], "# Replaced\n");
    assert!(json["updated_at"].as_str().unwrap().contains('T'));

    // stdin
    let mut child = Command::new(env!("CARGO_BIN_EXE_voxtype"))
        .env("XDG_CONFIG_HOME", s.path("config"))
        .env("XDG_DATA_HOME", s.path("data"))
        .env("XDG_RUNTIME_DIR", s.path("runtime"))
        .args(["meeting", "notes", "latest", "--set-file", "-", "--json"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"From stdin\n")
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&child.wait_with_output().unwrap())).unwrap();
    assert_eq!(json["notes"], "From stdin\n");

    // Exports put them on top.
    let export = stdout(&s.voxtype(&["meeting", "export", "latest", "--format", "markdown"]));
    assert!(export.contains("## Notes\n\nFrom stdin\n"), "{export}");
    assert!(!s.path("runtime/voxtype/meeting_notes").exists());
}

#[test]
fn notes_without_a_meeting_fail() {
    let s = Scratch::new();
    assert!(!s.voxtype(&["meeting", "notes", "latest"]).status.success());
}

#[test]
fn annotate_shows_on_the_segment_and_in_the_export() {
    let s = Scratch::new();
    s.meeting();

    let out = stdout(&s.voxtype(&[
        "meeting",
        "annotate",
        "latest",
        "2",
        "Check this claim",
        "--highlight",
        "--json",
    ]));
    let segment: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(segment["id"], 2);
    assert_eq!(
        segment["annotation"],
        serde_json::json!({"text": "Check this claim", "highlight": true})
    );

    let show: serde_json::Value = serde_json::from_str(&stdout(
        &s.voxtype(&["meeting", "show", "latest", "--json"]),
    ))
    .unwrap();
    assert_eq!(
        show["transcript"]["segments"][2]["annotation"]["text"],
        "Check this claim"
    );
    assert!(show["transcript"]["segments"][1]
        .get("annotation")
        .is_none());

    let export = stdout(&s.voxtype(&["meeting", "export", "latest", "--format", "markdown"]));
    assert!(
        export.contains("I disagree with that.\n\n> **Highlight: Check this claim**\n"),
        "{export}"
    );

    let out = stdout(&s.voxtype(&["meeting", "annotate", "latest", "2", "--remove", "--json"]));
    let segment: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(segment.get("annotation").is_none());

    assert!(!s
        .voxtype(&["meeting", "annotate", "latest", "2"])
        .status
        .success());
    assert!(!s
        .voxtype(&["meeting", "annotate", "latest", "2", "x", "--remove"])
        .status
        .success());
    let output = s.voxtype(&["meeting", "annotate", "latest", "42", "x"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Valid segment ids: 0 to 3."));
}

/// During a meeting, notes and annotations are stored and the daemon is
/// poked; transcript.json stays the daemon's to write.
#[test]
fn notes_and_annotations_during_a_meeting_poke_the_daemon() {
    let s = Scratch::new();
    let id = s.meeting();
    std::fs::write(
        s.path("runtime/voxtype/meeting_state"),
        format!("recording\n{id}\n"),
    )
    .unwrap();
    let before = std::fs::read_to_string(s.transcript_file(&id)).unwrap();

    stdout(&s.voxtype(&["meeting", "notes", "latest", "--append", "live note"]));
    assert!(s.path("runtime/voxtype/meeting_notes").exists());

    stdout(&s.voxtype(&["meeting", "annotate", "latest", "1", "live annotation"]));
    assert_eq!(
        std::fs::read_to_string(s.path("runtime/voxtype/meeting_edit")).unwrap(),
        "1"
    );
    assert_eq!(
        std::fs::read_to_string(s.transcript_file(&id)).unwrap(),
        before
    );

    let show: serde_json::Value = serde_json::from_str(&stdout(
        &s.voxtype(&["meeting", "show", "latest", "--json"]),
    ))
    .unwrap();
    assert_eq!(show["notes"], "live note\n");
    assert_eq!(
        show["transcript"]["segments"][1]["annotation"]["text"],
        "live annotation"
    );
}

impl Scratch {
    fn enable_meetings(&self) {
        std::fs::write(
            self.path("config/voxtype/config.toml"),
            "[meeting]\nenabled = true\n",
        )
        .unwrap();
    }

    fn start_error(&self, args: &[&str]) -> String {
        let mut all = vec!["meeting", "start"];
        all.extend_from_slice(args);
        let output = self.voxtype(&all);
        assert!(!output.status.success(), "start {args:?} should be refused");
        String::from_utf8_lossy(&output.stderr).into_owned()
    }
}

/// A5: a meeting that could not transcribe is refused before it starts,
/// with the fix, even before the daemon is consulted.
#[test]
fn meeting_start_refuses_engines_and_models_it_cannot_use() {
    let s = Scratch::new();
    s.enable_meetings();

    let err = s.start_error(&["--engine", "nope"]);
    assert!(
        err.contains("Unknown engine 'nope'. Engines: whisper,"),
        "{err}"
    );

    let err = s.start_error(&["--model", "no-such-model"]);
    assert!(err.contains("Unknown model 'no-such-model'"), "{err}");

    let err = s.start_error(&["--engine", "parakeet", "--model", "base.en"]);
    assert!(
        err.contains("Model 'base.en' is a whisper model, not a parakeet model"),
        "{err}"
    );

    // Nothing is downloaded in the scratch data dir.
    let err = s.start_error(&["--model", "large-v3-turbo"]);
    assert!(
        err.contains(
            "not installed. Download it with: voxtype setup --download --model large-v3-turbo"
        ),
        "{err}"
    );

    // base.en is English-only.
    let err = s.start_error(&["--language", "de"]);
    assert!(err.contains("Language 'de' is not available"), "{err}");

    // A default build has no Omnilingual engine; the hint says how to get it.
    if !cfg!(feature = "omnilingual") {
        let err = s.start_error(&["--engine", "omnilingual"]);
        assert!(
            err.contains(
                "This voxtype binary was built without the omnilingual engine. To use it,"
            ),
            "{err}"
        );
    }

    assert!(!s.path("runtime/voxtype/meeting_start").exists());
    assert!(!s.path("runtime/voxtype/meeting_start_options").exists());
}

#[test]
fn the_meeting_link_shows_in_show_and_the_export() {
    let s = Scratch::new();
    let storage = s.storage();
    let mut metadata = MeetingMetadata::new(Some("Linked".to_string()));
    metadata.url = Some("https://meet.example.com/abc-defg-hij".to_string());
    storage.create_meeting(&metadata).unwrap();

    let show: serde_json::Value = serde_json::from_str(&stdout(
        &s.voxtype(&["meeting", "show", "latest", "--json"]),
    ))
    .unwrap();
    assert_eq!(
        show["metadata"]["url"],
        "https://meet.example.com/abc-defg-hij"
    );
    let text = stdout(&s.voxtype(&["meeting", "show", "latest"]));
    assert!(
        text.contains("Link:     https://meet.example.com/abc-defg-hij"),
        "{text}"
    );

    let md = stdout(&s.voxtype(&["meeting", "export", "latest", "--metadata"]));
    assert!(
        md.contains("- **Link:** https://meet.example.com/abc-defg-hij"),
        "{md}"
    );
    let json: serde_json::Value = serde_json::from_str(&stdout(
        &s.voxtype(&["meeting", "export", "latest", "--format", "json"]),
    ))
    .unwrap();
    assert_eq!(
        json["metadata"]["url"],
        "https://meet.example.com/abc-defg-hij"
    );
}

#[test]
fn edit_corrects_a_line_and_reset_restores_it() {
    let s = Scratch::new();
    s.meeting();

    let out = stdout(&s.voxtype(&[
        "meeting",
        "edit",
        "latest",
        "2",
        "I agree with that.",
        "--json",
    ]));
    let segment: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(segment["id"], 2);
    assert_eq!(segment["text"], "I agree with that.");
    assert_eq!(segment["original_text"], "I disagree with that.");

    let export = stdout(&s.voxtype(&["meeting", "export", "latest", "--format", "text"]));
    assert!(export.contains("I agree with that."), "{export}");
    assert!(!export.contains("I disagree"), "{export}");

    let out = stdout(&s.voxtype(&["meeting", "edit", "latest", "2", "--reset", "--json"]));
    let segment: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(segment["text"], "I disagree with that.");
    assert!(segment.get("original_text").is_none());

    assert!(!s
        .voxtype(&["meeting", "edit", "latest", "2"])
        .status
        .success());
    let output = s.voxtype(&["meeting", "edit", "latest", "2", "  "]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("use `meeting delete-line`"));
}

#[test]
fn delete_line_hides_a_line_from_show_and_exports_until_restored() {
    let s = Scratch::new();
    let id = s.meeting();

    let out = stdout(&s.voxtype(&["meeting", "delete-line", "latest", "2", "--json"]));
    let segment: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(segment["id"], 2);
    assert_eq!(segment["deleted"], true);

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
    assert_eq!(ids, vec![0, 1, 3]);
    for format in ["markdown", "text", "json", "srt", "vtt"] {
        let export = stdout(&s.voxtype(&["meeting", "export", "latest", "--format", format]));
        assert!(!export.contains("I disagree"), "{format}: {export}");
    }
    let md =
        std::fs::read_to_string(s.transcript_file(&id).with_file_name("transcript.md")).unwrap();
    assert!(!md.contains("I disagree"), "{md}");
    // Kept in the file for --restore.
    assert!(std::fs::read_to_string(s.transcript_file(&id))
        .unwrap()
        .contains("I disagree"));

    let out = stdout(&s.voxtype(&[
        "meeting",
        "delete-line",
        "latest",
        "2",
        "--restore",
        "--json",
    ]));
    let segment: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(segment.get("deleted").is_none());
    let export = stdout(&s.voxtype(&["meeting", "export", "latest", "--format", "text"]));
    assert!(export.contains("I disagree with that."), "{export}");
}

#[test]
fn edit_and_delete_line_during_a_meeting_poke_the_daemon() {
    let s = Scratch::new();
    let id = s.meeting();
    std::fs::write(
        s.path("runtime/voxtype/meeting_state"),
        format!("recording\n{id}\n"),
    )
    .unwrap();
    let before = std::fs::read_to_string(s.transcript_file(&id)).unwrap();

    stdout(&s.voxtype(&["meeting", "edit", "latest", "1", "Yes, let's begin."]));
    assert_eq!(
        std::fs::read_to_string(s.path("runtime/voxtype/meeting_edit")).unwrap(),
        "1"
    );
    stdout(&s.voxtype(&["meeting", "delete-line", "latest", "3"]));
    assert_eq!(
        std::fs::read_to_string(s.transcript_file(&id)).unwrap(),
        before
    );

    let show: serde_json::Value = serde_json::from_str(&stdout(
        &s.voxtype(&["meeting", "show", "latest", "--json"]),
    ))
    .unwrap();
    let segments = show["transcript"]["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[1]["text"], "Yes, let's begin.");
}
