// Included inside daemon::tests::history_contracts; no live daemon or devices.
use super::*;

fn enter_private_case(name: &str) -> bool {
    if std::env::var("VOXTYPE_HISTORY_CONTRACT_CHILD").as_deref() == Ok(name) {
        return true;
    }
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env_clear()
        .env("HOME", root.path())
        .env("TMPDIR", root.path())
        .env("XDG_RUNTIME_DIR", runtime)
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("RUST_MIN_STACK", "16777216")
        .env("VOXTYPE_HISTORY_CONTRACT_CHILD", name)
        .stdin(std::process::Stdio::null()).output().unwrap();
    assert!(result.status.success(), "{}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
    false
}

#[tokio::test]
async fn history_streaming_cleanup_preserves_original_engine_text() {
    const NAME: &str = "daemon::tests::history_contracts::history_streaming_cleanup_preserves_original_engine_text";
    if !enter_private_case(NAME) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = history_daemon(root.path());
    let mut state = State::Streaming {
        started_at: std::time::Instant::now(), model_override: None,
        partial_buffer: String::new(), finalized_text: String::new(),
        typed_chars: 0, file_output_path: None,
    };
    let (finished, receiver) = std::sync::mpsc::channel();
    daemon.history.lock().unwrap().test_finished = Some(finished);
    daemon.begin_osd_session(&state);
    let got = Arc::new(std::sync::Mutex::new(None));
    let chain: Vec<Box<dyn TextOutput>> = vec![Box::new(RecordingOutput { got: got.clone(), succeed: true })];
    let raw = "um synthetic invoice";
    let processed = daemon.text_processor.process(raw);
    assert_ne!(raw, processed, "The fixture must actually change engine wording");
    let mut session = StreamingSession::new().with_history_context(daemon.history_context());
    session.commit_segment(&chain, raw, Some((&daemon.text_processor, None)), None, None).await.unwrap();
    assert_eq!(got.lock().unwrap().as_deref(), Some(processed.as_str()));
    let mut session = Some(session);
    let mut chain = Some(chain);
    daemon.end_streaming(&mut state, &mut None, &mut None, &mut session, &mut chain).await;
    tokio::task::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)).unwrap()).await.unwrap();
    let entries = crate::history::HistoryStore::at(root.path().join("history")).all().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].raw_text, raw, "Streaming history must preserve original engine wording");
    assert_eq!(entries[0].text, processed);
    assert!(!entries[0].edits.is_empty(), "Streaming cleanup must have an automatic-edit diff");
}

async fn streaming_output_failure(name: &str, prior_delivery: bool) {
    if !enter_private_case(name) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = history_daemon(root.path());
    let mut state = State::Streaming {
        started_at: std::time::Instant::now(), model_override: None,
        partial_buffer: String::new(), finalized_text: String::new(),
        typed_chars: 0, file_output_path: None,
    };
    let (finished, receiver) = std::sync::mpsc::channel();
    daemon.history.lock().unwrap().test_finished = Some(finished);
    daemon.begin_osd_session(&state);
    let got = Arc::new(std::sync::Mutex::new(None));
    let success: Vec<Box<dyn TextOutput>> = vec![Box::new(RecordingOutput { got: got.clone(), succeed: true })];
    let failure: Vec<Box<dyn TextOutput>> = vec![Box::new(RecordingOutput { got, succeed: false })];
    let mut session = StreamingSession::new().with_history_context(daemon.history_context());
    if prior_delivery {
        session.commit_segment(&success, "synthetic prior ", None, None, None).await.unwrap();
    }
    assert!(session.type_partial_delta(&failure, "recoverable tail".into(), None, None).await.is_err());
    let mut session = Some(session);
    let mut chain = Some(failure);
    daemon.end_streaming(&mut state, &mut None, &mut None, &mut session, &mut chain).await;
    tokio::task::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)).unwrap()).await.unwrap();
    let entries = crate::history::HistoryStore::at(root.path().join("history")).all().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].outcome, crate::history::Outcome::OutputFailed, "Streaming output failure must not become delivered or empty");
    assert!(!entries[0].delivered);
    assert!(entries[0].error.as_ref().is_some_and(|error| !error.is_empty()));
    assert_eq!(entries[0].raw_text, if prior_delivery { "synthetic prior recoverable tail" } else { "recoverable tail" });
    assert_eq!(entries[0].text, if prior_delivery { "synthetic prior " } else { "" });
}

#[tokio::test]
async fn history_streaming_total_output_failure_is_not_empty() {
    streaming_output_failure("daemon::tests::history_contracts::history_streaming_total_output_failure_is_not_empty", false).await;
}

#[tokio::test]
async fn history_streaming_partial_output_failure_is_not_delivered() {
    streaming_output_failure("daemon::tests::history_contracts::history_streaming_partial_output_failure_is_not_delivered", true).await;
}

async fn streaming_backend_failure(name: &str, prior_delivery: bool) {
    if !enter_private_case(name) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = history_daemon(root.path());
    let mut state = State::Streaming {
        started_at: std::time::Instant::now(), model_override: None,
        partial_buffer: String::new(), finalized_text: String::new(),
        typed_chars: 0, file_output_path: None,
    };
    let (finished, receiver) = std::sync::mpsc::channel();
    daemon.history.lock().unwrap().test_finished = Some(finished);
    daemon.begin_osd_session(&state);
    let chain: Vec<Box<dyn TextOutput>> = vec![Box::new(RecordingOutput {
        got: Arc::new(std::sync::Mutex::new(None)), succeed: true,
    })];
    let mut session = StreamingSession::new().with_history_context(daemon.history_context());
    if prior_delivery { session.commit_segment(&chain, "synthetic prior", None, None, None).await.unwrap(); }
    let mut session = Some(session);
    let mut chain = Some(chain);
    let error = crate::error::TranscribeError::InferenceFailed("synthetic backend stopped".into());
    daemon.end_streaming_with_error(error, &mut state, &mut None, &mut None, &mut session, &mut chain).await;
    tokio::task::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)).unwrap()).await.unwrap();
    let entries = crate::history::HistoryStore::at(root.path().join("history")).all().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].outcome, crate::history::Outcome::TranscriptionFailed, "Streaming backend error must not become delivered or empty");
    assert!(!entries[0].delivered);
    assert!(entries[0].error.as_ref().is_some_and(|error| error.contains("synthetic backend stopped")));
    assert_eq!(entries[0].raw_text, if prior_delivery { "synthetic prior" } else { "" });
    assert_eq!(entries[0].text, entries[0].raw_text);
}

#[tokio::test]
async fn history_streaming_total_backend_failure_is_not_empty() {
    streaming_backend_failure("daemon::tests::history_contracts::history_streaming_total_backend_failure_is_not_empty", false).await;
}

#[tokio::test]
async fn history_streaming_partial_backend_failure_is_not_delivered() {
    streaming_backend_failure("daemon::tests::history_contracts::history_streaming_partial_backend_failure_is_not_delivered", true).await;
}

struct PrivateStreamCapture {
    writes: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl TextOutput for PrivateStreamCapture {
    async fn output(&self, text: &str) -> std::result::Result<(), crate::error::OutputError> {
        self.writes.lock().unwrap().push(text.to_owned());
        Ok(())
    }
    async fn is_available(&self) -> bool { true }
    fn name(&self) -> &'static str { "private stream capture" }
}

async fn streaming_delivered_unfinished_tail(name: &str, prior_final: bool) {
    if !enter_private_case(name) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = history_daemon(root.path());
    let mut state = State::Streaming {
        started_at: std::time::Instant::now(), model_override: None,
        partial_buffer: String::new(), finalized_text: String::new(),
        typed_chars: 0, file_output_path: None,
    };
    let (finished, receiver) = std::sync::mpsc::channel();
    daemon.history.lock().unwrap().test_finished = Some(finished);
    daemon.begin_osd_session(&state);
    let writes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let chain: Vec<Box<dyn TextOutput>> = vec![Box::new(PrivateStreamCapture { writes: writes.clone() })];
    let mut session = StreamingSession::new().with_history_context(daemon.history_context());
    let prefix = if prior_final { "Final prefix " } else { "" };
    if prior_final { session.commit_segment(&chain, prefix, None, None, None).await.unwrap(); }
    let tail = "um unfinished";
    session.type_partial_delta(&chain, tail.to_owned(), None, None).await.unwrap();
    let actually_delivered = writes.lock().unwrap().concat();
    assert_eq!(actually_delivered, format!("{prefix}{tail}"));
    let writes_before_end = writes.lock().unwrap().clone();
    let mut session = Some(session);
    let mut chain = Some(chain);
    daemon.end_streaming(&mut state, &mut None, &mut None, &mut session, &mut chain).await;
    tokio::task::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)).unwrap()).await.unwrap();
    assert_eq!(*writes.lock().unwrap(), writes_before_end, "Ending must not retype or clean up already delivered partial text");
    let entries = crate::history::HistoryStore::at(root.path().join("history")).all().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text, actually_delivered, "History must include the successfully delivered unfinished partial");
    assert_eq!(entries[0].raw_text, format!("{prefix}{tail}"));
    assert_eq!(entries[0].best_text(), actually_delivered, "Recovery must not truncate the delivered tail");
    assert_eq!(entries[0].outcome, crate::history::Outcome::Delivered);
    assert!(entries[0].delivered);
    assert!(entries[0].edits.is_empty(), "An unfinished partial was not removed from the actual output");
}

#[tokio::test]
async fn history_streaming_final_then_partial_end_preserves_delivered_tail() {
    streaming_delivered_unfinished_tail("daemon::tests::history_contracts::history_streaming_final_then_partial_end_preserves_delivered_tail", true).await;
}

#[tokio::test]
async fn history_streaming_partial_only_end_is_delivered_not_empty() {
    streaming_delivered_unfinished_tail("daemon::tests::history_contracts::history_streaming_partial_only_end_is_delivered_not_empty", false).await;
}

struct PrivateCursorCapture {
    text: Arc<std::sync::Mutex<String>>,
    fail_next: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl TextOutput for PrivateCursorCapture {
    async fn output(&self, text: &str) -> std::result::Result<(), crate::error::OutputError> {
        let mut cursor = self.text.lock().unwrap();
        for count in StreamingSession::take_test_backspaces() {
            for _ in 0..count { cursor.pop(); }
        }
        if self.fail_next.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(crate::error::OutputError::AllMethodsFailed);
        }
        cursor.push_str(text);
        Ok(())
    }
    async fn is_available(&self) -> bool { true }
    fn name(&self) -> &'static str { "private cursor capture" }
}

async fn streaming_revised_tail(name: &str, output_failure: bool, backend_failure: bool) {
    if !enter_private_case(name) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = history_daemon(root.path());
    let mut state = State::Streaming {
        started_at: std::time::Instant::now(), model_override: None,
        partial_buffer: String::new(), finalized_text: String::new(),
        typed_chars: 0, file_output_path: None,
    };
    let (finished, receiver) = std::sync::mpsc::channel();
    daemon.history.lock().unwrap().test_finished = Some(finished);
    daemon.begin_osd_session(&state);
    let text = Arc::new(std::sync::Mutex::new(String::new()));
    let fail_next = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let chain: Vec<Box<dyn TextOutput>> = vec![Box::new(PrivateCursorCapture { text: text.clone(), fail_next: fail_next.clone() })];
    let mut session = StreamingSession::new().with_history_context(daemon.history_context());
    assert!(StreamingSession::take_test_backspaces().is_empty());
    let raw_final = "um synthetic invoice";
    let processed = daemon.text_processor.process(raw_final);
    assert_ne!(processed, raw_final);
    session.commit_segment(&chain, raw_final, Some((&daemon.text_processor, None)), None, None).await.unwrap();
    session.type_partial_delta(&chain, "誤り".into(), None, None).await.unwrap();
    fail_next.store(output_failure, std::sync::atomic::Ordering::SeqCst);
    let revised = session.replace_and_commit(&chain, 2, "正解 ", None, None, None).await;
    assert_eq!(revised.is_err(), output_failure);
    let mut expected_raw = format!("{raw_final}正解 ");
    if !output_failure {
        session.type_partial_delta(&chain, "um unfinished".into(), None, None).await.unwrap();
        expected_raw.push_str("um unfinished");
    }
    let actually_delivered = text.lock().unwrap().clone();
    assert_eq!(actually_delivered, if output_failure { processed } else { format!("{processed}正解 um unfinished") });
    let mut session = Some(session);
    let mut chain = Some(chain);
    if backend_failure {
        let error = crate::error::TranscribeError::InferenceFailed("synthetic backend failure after revised partial".into());
        daemon.end_streaming_with_error(error, &mut state, &mut None, &mut None, &mut session, &mut chain).await;
    } else {
        daemon.end_streaming(&mut state, &mut None, &mut None, &mut session, &mut chain).await;
    }
    tokio::task::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)).unwrap()).await.unwrap();
    assert_eq!(*text.lock().unwrap(), actually_delivered, "Completion must not send another output or erase prior delivery");
    assert!(StreamingSession::take_test_backspaces().is_empty(), "Completion must not emit more backspaces");
    let entries = crate::history::HistoryStore::at(root.path().join("history")).all().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.text, actually_delivered);
    assert_eq!(entry.raw_text, expected_raw);
    assert!(entry.corrected_text.is_none());
    assert!(!entry.edits.is_empty(), "Original cleanup remains separate from actual revised output");
    let outcome = if output_failure { crate::history::Outcome::OutputFailed } else if backend_failure { crate::history::Outcome::TranscriptionFailed } else { crate::history::Outcome::Delivered };
    assert_eq!(entry.outcome, outcome);
    assert_eq!(entry.delivered, !output_failure && !backend_failure);
    let recovered = crate::history::HistoryStore::at(root.path().join("history")).find(&entry.id).unwrap().unwrap();
    assert_eq!(recovered.best_text(), actually_delivered);
}

#[tokio::test]
async fn history_streaming_revision_then_unfinished_tail_matches_private_cursor() {
    streaming_revised_tail("daemon::tests::history_contracts::history_streaming_revision_then_unfinished_tail_matches_private_cursor", false, false).await;
}

#[tokio::test]
async fn history_streaming_failed_revision_does_not_resurrect_erased_partial() {
    streaming_revised_tail("daemon::tests::history_contracts::history_streaming_failed_revision_does_not_resurrect_erased_partial", true, false).await;
}

#[tokio::test]
async fn history_streaming_revised_unfinished_tail_backend_error_stays_failed() {
    streaming_revised_tail("daemon::tests::history_contracts::history_streaming_revised_unfinished_tail_backend_error_stays_failed", false, true).await;
}

include!("history_model_contract_tests.rs");
include!("history_fidelity_contract_tests.rs");
include!("history_selected_metadata_tests.rs");
include!("history_eager_model_contract_tests.rs");
