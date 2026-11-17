//! `voxtype meeting search` and `[meeting] my_name`: the index follows every
//! writer, old meetings are backfilled, and the name shows for "You" in
//! search, exports and transcript.md without ever being saved as a label.

use std::process::{Command, Output};

use voxtype::meeting::data::{
    AudioSource, MeetingMetadata, MeetingSummary, Transcript, TranscriptSegment,
};
use voxtype::meeting::search::{HitKind, SearchQuery};
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
        Self { dir }
    }

    fn path(&self, sub: &str) -> std::path::PathBuf {
        self.dir.path().join(sub)
    }

    fn storage_named(&self, my_name: Option<&str>) -> MeetingStorage {
        MeetingStorage::open(StorageConfig {
            storage_path: self.path("data/voxtype/meetings"),
            retain_audio: false,
            max_meetings: 0,
            my_name: my_name.map(str::to_string),
        })
        .unwrap()
    }

    fn storage(&self) -> MeetingStorage {
        self.storage_named(None)
    }

    fn write_config(&self, body: &str) {
        std::fs::write(self.path("config/voxtype/config.toml"), body).unwrap();
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

    /// A meeting with lines (id, key, source, text), saved through storage.
    fn meeting(&self, title: &str, lines: &[(u32, &str, AudioSource, &str)]) -> MeetingMetadata {
        let storage = self.storage();
        let metadata = MeetingMetadata::new(Some(title.to_string()));
        storage.create_meeting(&metadata).unwrap();
        let mut transcript = Transcript::new();
        for &(id, key, source, text) in lines {
            let start = id as u64 * 10_000;
            let mut seg = TranscriptSegment::new(id, start, start + 9_000, text.into(), 0);
            seg.source = source;
            seg.speaker_id = Some(key.to_string());
            transcript.add_segment(seg);
        }
        storage.save_transcript(&metadata.id, &transcript).unwrap();
        metadata
    }

    fn standup(&self) -> MeetingMetadata {
        self.meeting(
            "Weekly standup",
            &[
                (
                    0,
                    "You",
                    AudioSource::Microphone,
                    "The budget review moved to Friday.",
                ),
                (
                    1,
                    "Remote 1",
                    AudioSource::Loopback,
                    "Can we ship the deploy tool first?",
                ),
                (
                    2,
                    "Remote 1",
                    AudioSource::Loopback,
                    "Budget numbers are not final.",
                ),
                (3, "You", AudioSource::Microphone, "Agreed, deploy first."),
            ],
        )
    }
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        limit: 20,
        ..Default::default()
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
fn finds_lines_with_their_place_and_marks_the_match() {
    let s = Scratch::new();
    let m = s.standup();
    let hits = s.storage().search(&query("deploy")).unwrap();
    let lines: Vec<_> = hits.iter().map(|h| h.segment_id).collect();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(lines.contains(&Some(1)) && lines.contains(&Some(3)));
    let first = hits.iter().find(|h| h.segment_id == Some(1)).unwrap();
    assert_eq!(first.meeting_id, m.id.to_string());
    assert_eq!(first.title, "Weekly standup");
    assert_eq!(first.kind, HitKind::Segment);
    assert_eq!(first.start_ms, Some(10_000));
    assert_eq!(first.speaker.as_deref(), Some("Remote 1"));
    assert_eq!(first.snippet, "Can we ship the «deploy» tool first?");
    assert_eq!(first.snippet_plain, "Can we ship the deploy tool first?");
    assert_eq!(first.highlights, vec![[16, 22]]);
}

#[test]
fn every_word_must_match_and_case_and_accents_do_not_matter() {
    let s = Scratch::new();
    s.meeting(
        "Café sync",
        &[(
            0,
            "You",
            AudioSource::Microphone,
            "Résumé review at the café",
        )],
    );
    let storage = s.storage();
    // Every word in one line (or one title): the title lacks "resume".
    assert_eq!(storage.search(&query("RESUME cafe")).unwrap().len(), 1);
    assert_eq!(
        storage.search(&query("cafe")).unwrap().len(),
        2,
        "line and title"
    );
    assert!(storage.search(&query("resume budget")).unwrap().is_empty());
    assert_eq!(storage.search(&query("rev*")).unwrap().len(), 1);
    assert_eq!(
        storage.search(&query("\"review at the\"")).unwrap().len(),
        1
    );
    assert!(storage.search(&query("\"at review\"")).unwrap().is_empty());
}

#[test]
fn search_follows_labels_moves_edits_and_deletions() {
    let s = Scratch::new();
    let m = s.standup();
    let storage = s.storage();
    storage
        .set_speaker_label(&m.id, "Remote 1", "Dana")
        .unwrap();
    storage.reassign_segment(&m.id, 2, "You", true).unwrap();
    storage
        .edit_segment_text(&m.id, 3, "Agreed, release first.", true)
        .unwrap();
    storage.delete_segment(&m.id, 0, true).unwrap();

    let deploy = storage.search(&query("deploy")).unwrap();
    assert_eq!(
        deploy.len(),
        1,
        "edited line no longer says deploy: {deploy:?}"
    );
    assert_eq!(deploy[0].speaker.as_deref(), Some("Dana"));
    assert_eq!(
        storage.search(&query("release")).unwrap()[0].segment_id,
        Some(3)
    );
    let budget = storage.search(&query("budget")).unwrap();
    assert_eq!(budget.len(), 1, "deleted line is gone: {budget:?}");
    assert_eq!(budget[0].segment_id, Some(2));
    assert_eq!(budget[0].speaker.as_deref(), Some("You"), "moved to You");

    storage.restore_segment(&m.id, 0, true).unwrap();
    assert_eq!(storage.search(&query("budget")).unwrap().len(), 2);
}

#[test]
fn edits_while_recording_are_searchable_before_the_daemon_saves() {
    let s = Scratch::new();
    let m = s.standup();
    let storage = s.storage();
    // rewrite = false: the running daemon owns transcript.json.
    storage
        .edit_segment_text(&m.id, 1, "Can we ship the installer first?", false)
        .unwrap();
    assert!(storage.search(&query("installer")).unwrap().len() == 1);
    storage.delete_segment(&m.id, 3, false).unwrap();
    assert!(storage.search(&query("agreed")).unwrap().is_empty());
}

#[test]
fn speaker_and_since_filter_the_lines() {
    let s = Scratch::new();
    s.standup();
    let storage = s.storage();
    let mut q = query("deploy");
    q.speaker = Some("remote 1".into());
    let hits = storage.search(&q).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].segment_id, Some(1));

    let mut future = query("deploy");
    future.since = Some(chrono::Utc::now() + chrono::Duration::days(1));
    assert!(storage.search(&future).unwrap().is_empty());
    let mut past = query("deploy");
    past.since = Some(chrono::Utc::now() - chrono::Duration::days(1));
    assert_eq!(storage.search(&past).unwrap().len(), 2);
}

#[test]
fn titles_notes_and_summaries_are_searched_and_kept_current() {
    let s = Scratch::new();
    let m = s.standup();
    let storage = s.storage();
    let title = storage.search(&query("weekly")).unwrap();
    assert_eq!(title.len(), 1);
    assert_eq!(title[0].kind, HitKind::Title);
    assert_eq!(title[0].segment_id, None);

    storage
        .set_notes(&m.id, "Follow up with procurement.\n")
        .unwrap();
    assert_eq!(
        storage.search(&query("procurement")).unwrap()[0].kind,
        HitKind::Notes
    );
    storage.set_notes(&m.id, "Nothing to add.\n").unwrap();
    assert!(
        storage.search(&query("procurement")).unwrap().is_empty(),
        "old notes gone"
    );

    let summary = MeetingSummary {
        summary: "Team agreed on the roadmap.".into(),
        key_points: vec!["Quarterly targets".into()],
        action_items: vec![],
        decisions: vec![],
        chapters: vec![],
        generated_at: chrono::Utc::now(),
        model: None,
    };
    storage.save_summary(&m.id, &summary).unwrap();
    assert_eq!(
        storage.search(&query("quarterly")).unwrap()[0].kind,
        HitKind::Summary
    );

    let mut renamed = storage.get_meeting(&m.id).unwrap().unwrap();
    renamed.title = Some("Planning".into());
    storage.update_meeting(&renamed).unwrap();
    assert!(storage.search(&query("weekly")).unwrap().is_empty());
    assert_eq!(storage.search(&query("planning")).unwrap().len(), 1);

    storage.delete_meeting(&m.id).unwrap();
    assert!(storage.search(&query("planning")).unwrap().is_empty());
}

#[test]
fn meetings_from_before_search_are_backfilled() {
    let s = Scratch::new();
    let m = s.standup();
    s.storage()
        .set_notes(&m.id, "Remember the vendor call.")
        .unwrap();
    // Forget the index, as an index.db from before search would look.
    let db = rusqlite::Connection::open(s.path("data/voxtype/meetings/index.db")).unwrap();
    db.execute_batch("DROP TABLE meeting_search; DROP TABLE meeting_search_state;")
        .unwrap();
    drop(db);

    let storage = s.storage();
    assert_eq!(storage.backfill_search_index().unwrap(), 1);
    assert_eq!(storage.backfill_search_index().unwrap(), 0, "only once");
    assert_eq!(storage.search(&query("vendor")).unwrap().len(), 1);
    assert_eq!(storage.search(&query("deploy")).unwrap().len(), 2);
}

#[test]
fn an_empty_query_says_what_to_do() {
    let s = Scratch::new();
    let err = s.storage().search(&query("   ")).unwrap_err().to_string();
    assert!(err.contains("voxtype meeting search"), "{err}");
}

#[test]
fn my_name_shows_for_you_and_is_never_saved() {
    let s = Scratch::new();
    let m = s.standup();
    let named = s.storage_named(Some("Pete"));

    let hits = named.search(&query("agreed")).unwrap();
    assert_eq!(hits[0].speaker.as_deref(), Some("Pete"));
    let mut by_name = query("deploy");
    by_name.speaker = Some("pete".into());
    let mine = named.search(&by_name).unwrap();
    assert_eq!(mine.len(), 1, "my_name finds You lines: {mine:?}");
    assert_eq!(mine[0].segment_id, Some(3));

    let shown = named.load_transcript(&m.id).unwrap();
    assert_eq!(shown.segments[0].speaker_display(), "Pete");
    assert_eq!(shown.segments[0].speaker_key(), "You", "still keyed You");

    // A save through the named storage writes the name only into the
    // markdown, never into transcript.json or the labels.
    let all = named.load_transcript(&m.id).unwrap();
    named.save_transcript(&m.id, &all).unwrap();
    let dir = named.get_meeting_path(&m.id).unwrap();
    let json = std::fs::read_to_string(dir.join("transcript.json")).unwrap();
    assert!(!json.contains("Pete"), "{json}");
    let md = std::fs::read_to_string(dir.join("transcript.md")).unwrap();
    assert!(md.contains("Pete"), "{md}");
    assert!(
        !s.storage()
            .get_speaker_labels(&m.id)
            .unwrap()
            .values()
            .any(|l| l == "Pete"),
        "my_name never becomes a label"
    );
    assert_eq!(
        s.storage().load_transcript(&m.id).unwrap().segments[0].speaker_display(),
        "You",
        "unnamed storage still shows You"
    );
}

#[test]
fn a_label_set_by_hand_wins_over_my_name() {
    let s = Scratch::new();
    let m = s.standup();
    let named = s.storage_named(Some("Pete"));
    named.set_speaker_label(&m.id, "You", "Peter J").unwrap();
    let shown = named.load_transcript(&m.id).unwrap();
    assert_eq!(shown.segments[0].speaker_display(), "Peter J");
}

#[test]
fn my_name_can_be_used_where_a_speaker_is_named() {
    let s = Scratch::new();
    let m = s.standup();
    let named = s.storage_named(Some("Pete"));
    let seg = named.reassign_segment(&m.id, 1, "pete", true).unwrap();
    assert_eq!(seg.speaker_key(), "You", "no new speaker called pete");
    assert_eq!(seg.speaker_display(), "Pete");
    assert_eq!(
        named
            .set_speaker_label(&m.id, "Pete", "Pete Jackson")
            .unwrap(),
        vec!["You".to_string()]
    );
}

#[test]
fn cli_search_prints_text_and_json() {
    let s = Scratch::new();
    s.write_config("[meeting]\nenabled = true\nmy_name = \"Pete\"\n");
    let m = s.standup();
    let text = stdout(&s.voxtype(&["meeting", "search", "deploy"]));
    assert!(text.contains("Weekly standup"), "{text}");
    assert!(
        text.contains("00:10  Remote 1:  Can we ship the «deploy» tool first?")
            || text.contains("0:10  Remote 1:  Can we ship the «deploy» tool first?"),
        "{text}"
    );
    assert!(text.contains("Pete:  Agreed, «deploy» first."), "{text}");

    let json: serde_json::Value = serde_json::from_str(&stdout(
        &s.voxtype(&["meeting", "search", "deploy", "--json"]),
    ))
    .unwrap();
    assert_eq!(json["query"], "deploy");
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    let r = &results[0];
    for key in [
        "meeting_id",
        "title",
        "date",
        "kind",
        "segment_id",
        "start_ms",
        "speaker",
        "snippet",
        "snippet_plain",
        "highlights",
        "rank",
    ] {
        assert!(r.get(key).is_some(), "missing {key}: {r}");
    }
    assert_eq!(r["meeting_id"], m.id.to_string());

    let none = stdout(&s.voxtype(&["meeting", "search", "zebra"]));
    assert!(none.contains("No meetings mention \"zebra\"."), "{none}");
    let bad = s.voxtype(&["meeting", "search", "x", "--since", "yesterday"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("2026-11-01"));
}

#[test]
fn cli_my_name_reports_the_setting() {
    let s = Scratch::new();
    s.write_config("[meeting]\nmy_name = \"Pete\"\n");
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&s.voxtype(&["meeting", "my-name", "--json"]))).unwrap();
    assert_eq!(json["my_name"], "Pete");
    assert!(json["suggestion"].is_null(), "no suggestion once set");
    s.write_config("[meeting]\n");
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&s.voxtype(&["meeting", "my-name", "--json"]))).unwrap();
    assert!(json["my_name"].is_null());
    assert!(json.get("suggestion").is_some());
}

#[test]
fn exports_and_meeting_data_use_my_name() {
    let s = Scratch::new();
    s.write_config("[meeting]\nenabled = true\nmy_name = \"Pete\"\n");
    let m = s.standup();
    let md = stdout(&s.voxtype(&[
        "meeting",
        "export",
        &m.id.to_string(),
        "--format",
        "markdown",
        "--speakers",
    ]));
    assert!(md.contains("Pete"), "{md}");
    assert!(!md.contains("**You**"), "{md}");
    let txt = stdout(&s.voxtype(&[
        "meeting",
        "export",
        "latest",
        "--format",
        "text",
        "--speakers",
    ]));
    assert!(txt.contains("Pete"), "{txt}");

    // What summaries and the assistant read: load_meeting_data.
    let data = s
        .storage_named(Some("Pete"))
        .load_meeting_data(&m.id)
        .unwrap();
    assert_eq!(
        data.transcript.speakers(),
        vec!["Pete".to_string(), "Remote 1".to_string()]
    );
}
