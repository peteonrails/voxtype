//! Privacy regressions use only task-owned synthetic filesystem fixtures.
use std::os::unix::fs::symlink;
use std::os::unix::fs::PermissionsExt;
use voxtype::history::{HistoryEntry, HistoryStore};

#[test]
fn append_rejects_a_symlinked_day_without_touching_its_target() {
    let t = tempfile::tempdir().unwrap();
    let store = HistoryStore::open(t.path().join("history")).unwrap();
    let entry = HistoryEntry::new(chrono::Local::now());
    let sentinel = t.path().join("sentinel");
    std::fs::write(&sentinel, "must not change").unwrap();
    let day = store.dir().join(format!("{}.jsonl", entry.day().unwrap()));
    symlink(&sentinel, day).unwrap();
    assert!(
        store.append(&entry).is_err(),
        "history must reject a symlinked day"
    );
    assert_eq!(
        std::fs::read_to_string(sentinel).unwrap(),
        "must not change"
    );
}

#[test]
fn a_shared_index_tightens_existing_database_and_directory_permissions() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("meetings");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let db = dir.join("index.db");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch("CREATE TABLE fixture_meeting (id TEXT);")
        .unwrap();
    drop(connection);
    std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
    let index = voxtype::history::HistoryIndex::open(&db).unwrap();
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&db).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(index
        .connection()
        .prepare("SELECT * FROM fixture_meeting")
        .is_ok());
}

#[test]
fn a_symlinked_history_directory_is_rejected_without_chmodding_its_target() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
    let link = t.path().join("history");
    symlink(&outside, &link).unwrap();
    assert!(HistoryStore::open(&link).is_err());
    assert!(
        HistoryStore::at(&link).recent(1).is_err(),
        "read-only callers must reject the same symlink"
    );
    assert_eq!(
        std::fs::metadata(outside).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[test]
fn rewriting_uses_an_exclusive_random_temp_instead_of_a_preplanted_name() {
    let t = tempfile::tempdir().unwrap();
    let store = HistoryStore::open(t.path().join("history")).unwrap();
    let entry = HistoryEntry::new(chrono::Local::now());
    store.append(&entry).unwrap();
    let sentinel = t.path().join("sentinel");
    std::fs::write(&sentinel, "must not change").unwrap();
    let predictable = store.dir().join(format!(
        ".tmp-{}.jsonl-{}",
        entry.day().unwrap(),
        std::process::id()
    ));
    symlink(&sentinel, predictable).unwrap();
    store
        .update(&entry.id, |e| e.corrected_text = Some("corrected".into()))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(sentinel).unwrap(),
        "must not change"
    );
    assert_eq!(
        store
            .find(&entry.id)
            .unwrap()
            .unwrap()
            .corrected_text
            .as_deref(),
        Some("corrected")
    );
}

#[test]
fn adding_one_entry_cannot_hide_an_unsynchronized_append_or_deletion() {
    use voxtype::history::{HistoryIndex, HistoryQuery};
    let t = tempfile::tempdir().unwrap();
    let store = HistoryStore::open(t.path().join("history")).unwrap();
    let index = HistoryIndex::open(&t.path().join("meetings/index.db")).unwrap();
    let mut a = HistoryEntry::new(chrono::Local::now());
    a.text = "uniquedeletedtoken".into();
    store.append(&a).unwrap();
    index.sync(&store).unwrap();
    let mut b = HistoryEntry::new(chrono::Local::now());
    b.text = "uniquemissedtoken".into();
    store.append(&b).unwrap(); // Simulates a missed best-effort index update.
    store.delete(&a.id).unwrap();
    let mut c = HistoryEntry::new(chrono::Local::now());
    c.text = "newestentrytoken".into();
    store.append(&c).unwrap();
    index.add(&store, &c).unwrap();
    index.sync(&store).unwrap();
    let query = |text: &str| HistoryQuery {
        text: text.into(),
        limit: 20,
        since: None,
    };
    assert_eq!(
        index
            .search(&store, &query("uniquemissedtoken"))
            .unwrap()
            .len(),
        1
    );
    assert!(index
        .search(&store, &query("uniquedeletedtoken"))
        .unwrap()
        .is_empty());
    assert_eq!(index.count().unwrap(), 2);
}

#[test]
fn appending_after_an_unterminated_line_keeps_the_next_dictation_readable() {
    for partial in [false, true] {
        let t = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(t.path().join("history")).unwrap();
        let old = HistoryEntry::new(chrono::Local::now());
        let path = store.dir().join(format!("{}.jsonl", old.day().unwrap()));
        let mut bytes = serde_json::to_string(&old).unwrap();
        if partial {
            bytes.push_str("\n{\"interrupted\":");
        }
        std::fs::write(path, bytes).unwrap();
        let next = HistoryEntry::new(chrono::Local::now());
        store.append(&next).unwrap();
        let entries = store.all().unwrap();
        assert!(entries.iter().any(|e| e.id == next.id));
        assert!(entries.iter().any(|e| e.id == old.id));
    }
}

#[test]
fn opening_a_store_does_not_remove_a_writers_active_temporary_file() {
    use std::os::fd::AsRawFd;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("history");
    let store = HistoryStore::open(&dir).unwrap();
    store
        .append(&HistoryEntry::new(chrono::Local::now()))
        .unwrap();
    let lock = std::fs::File::open(dir.join(".lock")).unwrap();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    let active = dir.join(".tmp-active-writer");
    std::fs::write(&active, "in-flight atomic rewrite").unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = HistoryStore::open(&dir);
        done_tx.send(result.is_ok()).unwrap();
    });
    started_rx.recv().unwrap();
    let early = done_rx.recv_timeout(std::time::Duration::from_millis(150));
    let preserved = active.exists();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) }, 0);
    thread.join().unwrap();
    assert!(
        early.is_err(),
        "store cleanup must serialize with an active writer"
    );
    assert!(
        preserved,
        "an active temporary file must not be unlinked by another open"
    );
}

#[test]
fn a_corrupt_day_cannot_keep_a_deleted_days_terms_in_the_index() {
    use voxtype::history::HistoryIndex;
    let t = tempfile::tempdir().unwrap();
    let store = HistoryStore::open(t.path().join("history")).unwrap();
    let index = HistoryIndex::open(&t.path().join("meetings/index.db")).unwrap();
    let mut old = HistoryEntry::new(chrono::Local::now() - chrono::Duration::days(10));
    old.text = "deletedolduniquetoken".into();
    let mut current = HistoryEntry::new(chrono::Local::now());
    current.text = "corruptdayuniquetoken".into();
    store.append(&old).unwrap();
    store.append(&current).unwrap();
    index.sync(&store).unwrap();
    store.delete(&old.id).unwrap();
    std::fs::write(
        store
            .dir()
            .join(format!("{}.jsonl", current.day().unwrap())),
        [0xff, 0xfe],
    )
    .unwrap();
    assert!(
        index.sync(&store).is_err(),
        "the corrupt day must be reported honestly"
    );
    assert_eq!(
        index.count().unwrap(),
        0,
        "missing and unreadable days must not retain old terms"
    );
}

#[test]
fn correcting_a_future_schema_is_rejected_without_stripping_nested_fields() {
    let t = tempfile::tempdir().unwrap();
    let store = HistoryStore::open(t.path().join("history")).unwrap();
    let entry = HistoryEntry::new(chrono::Local::now());
    let mut value = serde_json::to_value(&entry).unwrap();
    value["schema"] = serde_json::json!(2);
    value["output"] = serde_json::json!({"mode":"type","future_field":"retain me"});
    value["edits"] = serde_json::json!([{"from":"a","to":"b","future_field":42}]);
    let path = store.dir().join(format!("{}.jsonl", entry.day().unwrap()));
    let bytes = format!("{}\n", serde_json::to_string(&value).unwrap());
    std::fs::write(&path, &bytes).unwrap();
    assert!(store
        .update(&entry.id, |e| e.corrected_text = Some("correction".into()))
        .is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
}

#[test]
fn an_intentional_empty_correction_does_not_restore_unwanted_text() {
    let mut entry = HistoryEntry::new(chrono::Local::now());
    entry.raw_text = "unwanted recognition".into();
    entry.text = "Unwanted recognition.".into();
    assert_eq!(entry.best_text(), "Unwanted recognition.");
    entry.corrected_text = Some(String::new());
    assert_eq!(entry.best_text(), "");
    entry.corrected_text = None;
    assert_eq!(entry.best_text(), "Unwanted recognition.");
}

#[test]
fn deletion_reports_a_blocked_wal_checkpoint_and_retries_after_the_reader_closes() {
    use voxtype::history::HistoryIndex;
    let t = tempfile::tempdir().unwrap();
    let store = HistoryStore::open(t.path().join("history")).unwrap();
    let db = t.path().join("meetings/index.db");
    let index = HistoryIndex::open(&db).unwrap();
    index
        .connection()
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    index
        .connection()
        .busy_timeout(std::time::Duration::ZERO)
        .unwrap();
    let secret = "syntheticwaldeletionuniquetoken";
    let mut entry = HistoryEntry::new(chrono::Local::now());
    entry.text = secret.into();
    store.append(&entry).unwrap();
    index.sync(&store).unwrap();
    let reader = rusqlite::Connection::open(&db).unwrap();
    reader.execute_batch("BEGIN;").unwrap();
    let _: i64 = reader
        .query_row("SELECT COUNT(*) FROM history_search_rows", [], |r| r.get(0))
        .unwrap();
    store.delete(&entry.id).unwrap();
    let deletion = index.sync(&store);
    let contains_secret = || {
        [
            db.clone(),
            std::path::PathBuf::from(format!("{}-wal", db.display())),
        ]
        .iter()
        .any(|path| {
            std::fs::read(path)
                .unwrap_or_default()
                .windows(secret.len())
                .any(|w| w == secret.as_bytes())
        })
    };
    assert!(
        deletion.is_err(),
        "a busy reader prevents secure WAL cleanup; retained bytes = {}",
        contains_secret()
    );
    assert!(deletion.unwrap_err().to_string().contains("reader"));
    assert!(
        contains_secret(),
        "the negative control must really pin historical content"
    );
    drop(reader);
    index.sync(&store).unwrap();
    assert!(
        !contains_secret(),
        "retry must scrub database and WAL, not just remove search mappings"
    );
}

/// The Privacy note under `[history]` says where dictated words are stored
/// (history, recordings kept after a failed transcription, undelivered
/// text) and that the log is not one of those places: lengths at the default
/// level, the words only at debug. The note and the code are checked
/// together so a change to either one sends the author to the other.
#[test]
fn the_privacy_note_says_where_dictated_words_are_kept_and_what_the_log_holds() {
    let root = env!("CARGO_MANIFEST_DIR");
    let docs = std::fs::read_to_string(format!("{root}/docs/CONFIGURATION.md")).unwrap();
    let history = &docs[docs.find("\n## [history]").expect("the [history] section")..];
    let note = &history[history.find("\n### Privacy").expect("its Privacy note")..];
    let note = &note[..note[1..].find("\n## ").map_or(note.len(), |end| end + 1)];
    let note = note.split_whitespace().collect::<Vec<_>>().join(" ");

    for said in [
        // The three places the words are kept, each with its setting.
        "History is one of three places",
        "`[audio] keep_failed_recordings`",
        "`[output] keep_undelivered_text`",
        // The log is not a fourth.
        "voxtype does not log what you say",
        "journalctl --user -u voxtype",
        "Transcribed 42 chars (whisper engine)",
        "logged only at debug level",
        "the journal keeps every dictation whatever the history settings say",
    ] {
        assert!(
            note.contains(said),
            "the Privacy note must say {said:?}:\n{note}"
        );
    }
    for stale in [
        "History is not the only place your words are written",
        "logs each transcription",
        "never written to the log",
    ] {
        assert!(
            !note.contains(stale),
            "the Privacy note still says {stale:?}"
        );
    }

    // What the note promises is what the daemon does.
    let daemon = std::fs::read_to_string(format!("{root}/src/daemon.rs")).unwrap();
    assert!(
        daemon.contains(r#""Transcribed {} chars ({} engine)""#)
            && daemon.contains(r#"tracing::debug!("Transcribed: {:?}", text)"#)
            && !daemon.contains(r#"tracing::info!("Transcribed: {:?}", text)"#),
        "the daemon's transcription log lines changed: rewrite the Privacy note in \
         docs/CONFIGURATION.md to match, then update this test"
    );
}

#[derive(Clone)]
struct LogSink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The promise the Privacy note makes, for the history code itself: storing,
/// correcting, searching, offering a replacement rule, deleting and pruning
/// dictations logs none of their words at info, the level the daemon and
/// its systemd unit run at, on the paths that work and on the ones that
/// warn. The words are in the day file, which is where the user chose to
/// keep them, and nowhere in the log.
///
/// Runs in a process of its own so its subscriber is the process-wide one
/// and sees every thread. The daemon's side, a dictation recorded while it
/// is logged, is `daemon::tests::history_keeps_the_words_and_the_log_at_info_does_not`,
/// and `tests/log_privacy.rs` scans the source of every log call.
#[test]
fn the_history_code_logs_none_of_the_dictated_words_at_the_default_level() {
    use voxtype::history::{suggest_rule, HistoryIndex, HistoryQuery};
    const NAME: &str = "the_history_code_logs_none_of_the_dictated_words_at_the_default_level";
    const CHILD: &str = "VOXTYPE_HISTORY_LOG_PRIVACY_CHILD";
    if std::env::var(CHILD).as_deref() != Ok(NAME) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture"])
            .env(CHILD, NAME)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        // "1 passed": a name that matches no test also exits successfully.
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{NAME} in its own process:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = LogSink(lines.clone());
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_writer(move || sink.clone())
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .finish(),
    )
    .unwrap();

    let t = tempfile::tempdir().unwrap();
    let store = HistoryStore::open(t.path().join("history")).unwrap();
    let index = HistoryIndex::open(&t.path().join("meetings/index.db")).unwrap();

    // A dictation as the engine heard it, as it was typed, and as the user
    // corrected it afterwards.
    let heard = "um ask the cone ogres about the quarry invoice";
    let typed = "Ask the cone ogres about the quarry invoice.";
    let corrected = "Ask the CODEOWNERS about the harbour invoice.";
    let mut entry = HistoryEntry::new(chrono::Local::now());
    entry.raw_text = heard.into();
    entry.text = typed.into();
    entry.edits = voxtype::history::edits_between(heard, typed);
    entry.window_title = Some("Quarry invoice draft".into());
    store.append(&entry).unwrap();
    index.add(&store, &entry).unwrap();
    store
        .update(&entry.id, |e| e.corrected_text = Some(corrected.into()))
        .unwrap();
    index.sync(&store).unwrap();
    let query = HistoryQuery {
        text: "quarry".into(),
        limit: 20,
        since: None,
    };
    assert_eq!(index.search(&store, &query).unwrap().len(), 1);
    assert!(
        suggest_rule(
            "ask the cone ogres about it",
            "ask the CODEOWNERS about it",
            &Default::default()
        )
        .is_some(),
        "the correction must be one a rule is offered for"
    );
    assert_eq!(store.recent(5).unwrap()[0].best_text(), corrected);

    // An older day, for retention and compression to work on.
    let mut old = HistoryEntry::new(chrono::Local::now() - chrono::Duration::days(40));
    old.raw_text = "the lighthouse keeper owes us nothing".into();
    old.text = old.raw_text.clone();
    store.append(&old).unwrap();
    let today = chrono::Local::now().date_naive();
    store.compress_old(today).unwrap();
    assert_eq!(store.prune(30, today).unwrap().len(), 1);

    // The paths that warn. A line that is not an entry, holding words, is
    // counted and not quoted; an append refused for a symlinked day names
    // the file, not the dictation.
    let day = store.dir().join(format!("{}.jsonl", entry.day().unwrap()));
    let mut bytes = std::fs::read(&day).unwrap();
    assert!(
        String::from_utf8_lossy(&bytes).contains("cone ogres"),
        "the words are kept in the day file"
    );
    bytes.extend_from_slice(b"{\"raw_text\": \"the quarry gate code is four one\n");
    std::fs::write(&day, bytes).unwrap();
    assert_eq!(store.all().unwrap().len(), 1);
    index.sync(&store).unwrap();
    assert!(store.delete(&entry.id).unwrap().is_some());
    index.sync(&store).unwrap();
    let mut refused = HistoryEntry::new(chrono::Local::now() + chrono::Duration::days(1));
    refused.raw_text = "the harbour master knows about the quarry".into();
    refused.text = refused.raw_text.clone();
    let sentinel = t.path().join("sentinel");
    std::fs::write(&sentinel, "").unwrap();
    symlink(
        &sentinel,
        store
            .dir()
            .join(format!("{}.jsonl", refused.day().unwrap())),
    )
    .unwrap();
    assert!(store.append(&refused).is_err());
    let _ = store.all();
    let _ = index.sync(&store);
    let _ = store.clear(None);

    let logged = String::from_utf8_lossy(&lines.lock().unwrap()).into_owned();
    assert!(
        logged.contains("Skipped 1 unreadable line(s)"),
        "the capture is not seeing what the history code logs:\n{logged}"
    );
    let lowered = logged.to_lowercase();
    for words in [
        "cone ogres",
        "codeowners",
        "quarry",
        "harbour",
        "invoice",
        "lighthouse",
        "gate code",
    ] {
        assert!(
            !lowered.contains(words),
            "dictated text ({words:?}) in the log at info:\n{logged}"
        );
    }
}
