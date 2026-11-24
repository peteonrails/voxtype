// Fidelity controls are separate from the unchanged ASR reference checks.
// Only the approved, pinned local model and committed WAVs are used.
const FIDELITY_MODEL: &str = "/opt/history-models/ggml-tiny.en.bin";

fn enter_real_fidelity_case(name: &str) -> bool {
    if std::env::var("VOXTYPE_HISTORY_REAL_MODEL_CHILD").as_deref() == Ok(name) {
        return true;
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("runtime")).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--ignored", "--nocapture"])
        .env_clear().env("HOME", root.path()).env("TMPDIR", root.path())
        .env("XDG_RUNTIME_DIR", root.path().join("runtime"))
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("RUST_MIN_STACK", "16777216")
        .env("VOXTYPE_HISTORY_REAL_MODEL_CHILD", name)
        .stdin(std::process::Stdio::null()).output().unwrap();
    print!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    assert!(result.status.success(), "Actual fidelity child failed: {}", result.status);
    false
}

fn fidelity_daemon(root: &Path) -> Daemon {
    let mut daemon = history_daemon(root);
    daemon.config.whisper.model = FIDELITY_MODEL.into();
    daemon.config.whisper.mode = Some(crate::config::WhisperMode::Local);
    daemon.config.whisper.language = crate::config::LanguageConfig::Single("en".into());
    daemon.config.whisper.threads = Some(2);
    daemon.config.whisper.on_demand_loading = true;
    daemon.config.whisper.gpu_isolation = false;
    daemon.config.whisper.gpu_wait_secs = 0;
    daemon.config.audio.device = "synthetic-prerecorded-tts".into();
    daemon.config.output.mode = crate::config::OutputMode::File;
    daemon.config.output.file_path = Some(root.join("transcript.txt"));
    daemon.config.output.fallback_to_clipboard = false;
    daemon.config.output.notification.on_recording_start = false;
    daemon.config.output.notification.on_recording_stop = false;
    daemon.config.output.notification.on_transcription = false;
    // These contracts pin what history records when a stream fails. With
    // recovery on, a failed stream with captured audio hands off to batch
    // transcription instead; the daemon's recovery tests cover that path.
    daemon.config.streaming_recovery = Some(crate::config::StreamingRecoveryConfig {
        enabled: false,
        ..Default::default()
    });
    daemon.config.output.notification.on_no_speech = false;
    // Deliberate ordinary postprocessing, not an alteration of ASR/ground truth.
    daemon.config.text.replacements.insert("world".into(), "HISTORYTRANSFORM".into());
    daemon.config.text.replacements.insert("words".into(), "HISTORYTRANSFORM".into());
    daemon.text_processor = crate::text::TextProcessor::new(&daemon.config.text);
    daemon
}

fn fidelity_samples(fixture: &str) -> Vec<f32> {
    let mut wav = hound::WavReader::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vad").join(fixture)).unwrap();
    assert_eq!(wav.spec().sample_rate, 16000);
    assert_eq!(wav.spec().channels, 1);
    wav.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect()
}

async fn assert_fidelity(root: &Path, daemon: &Daemon, raw: &str, tag: &str, streaming: bool) {
    let entries = wait_for_entries(root, 1).await;
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    let processed = daemon.text_processor.process(raw);
    assert_ne!(processed, raw, "The control must exercise real postprocessing");
    let output = std::fs::read_to_string(root.join("transcript.txt")).unwrap();
    let store = crate::history::HistoryStore::at(root.join("history"));
    let index = crate::history::index::HistoryIndex::open(&root.join("meetings/index.db")).unwrap();
    let hits = index.search(&store, &crate::history::index::HistoryQuery { text: "HISTORYTRANSFORM".into(), limit: 10, ..Default::default() }).unwrap();
    let day = store.days().unwrap();
    let observation = serde_json::json!({"kind":"ACTUAL_REAL_MODEL_HISTORY_FIDELITY_NOT_REFERENCE_ACCURACY","route":tag,"engine_output":raw,"processed":processed,"actual_output":output,"entry":entry,"index_hits":hits,"dayfile_count":day.len(),"scope":"Approved tiny.en and same committed WAV. Batch/eager compare independent direct-engine output; streaming observes actual backend events through real startup/tap, existing StreamingSession methods and canonical end_streaming, not the full daemon event loop or live capture. Original reference tests unchanged."});
    write_evidence(format!("{tag}.json"), serde_json::to_vec_pretty(&observation).unwrap());
    println!("REAL_FIDELITY_OBSERVATION {observation}");
    assert_eq!(entry.raw_text, raw);
    assert_eq!(entry.text, processed);
    assert_eq!(output.trim_end(), processed);
    assert!(!entry.edits.is_empty());
    assert_eq!(entry.outcome, crate::history::Outcome::Delivered);
    assert!(entry.delivered);
    assert_eq!(entry.streaming, streaming);
    let audio = fidelity_samples(if tag.ends_with("short") { "speech_hello.wav" } else { "speech_long.wav" });
    assert_eq!(entry.audio_ms, audio.len() as u64 / 16);
    assert!(entry.transcribe_ms.is_some());
    assert_eq!(entry.input_device.as_deref(), Some("synthetic-prerecorded-tts"));
    assert_eq!(entry.model, FIDELITY_MODEL);
    assert_eq!(entry.backend.as_deref(), Some("cpu"));
    assert!(entry.model_load_ms.is_some());
    assert_eq!(entry.language.as_deref(), Some("en"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, entry.id);
    assert_eq!(store.find(&entry.id).unwrap().as_ref(), Some(entry));
    // Authoritative day JSON must retain the full entry, not just index hits.
    let files: Vec<_> = std::fs::read_dir(root.join("history")).unwrap().map(|p| p.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "jsonl")).collect();
    assert_eq!(files.len(), 1);
    let jsonl = std::fs::read(&files[0]).unwrap();
    assert_eq!(serde_json::from_slice::<crate::history::HistoryEntry>(&jsonl).unwrap(), *entry);
    write_evidence(format!("{tag}.jsonl"), jsonl);
    println!("REAL_FIDELITY_ASSERTIONS_PASSED {tag}");
}

async fn real_batch_or_eager_fidelity(name: &str, fixture: &str, eager: bool) {
    if !enter_real_fidelity_case(name) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = fidelity_daemon(root.path());
    let samples = fidelity_samples(fixture);
    let direct = crate::transcribe::create_transcriber(&daemon.config).unwrap();
    let raw = direct.transcribe(&samples).unwrap();
    assert!(!raw.is_empty());
    println!("INDEPENDENT_DIRECT_ENGINE_CONTROL model={FIDELITY_MODEL} fixture={fixture} raw={raw:?}");
    let mut state = if eager { State::EagerRecording { started_at: std::time::Instant::now(), model_override: None, accumulated_audio: Vec::new(), chunks_sent: 0, chunk_results: Vec::new(), tasks_in_flight: 0 } } else { State::Recording { started_at: std::time::Instant::now(), model_override: None } };
    daemon.begin_osd_session(&state);
    daemon.prepare_for_recording(None);
    let mut capture: Option<Box<dyn AudioCapture>> = Some(Box::new(PrerecordedHistoryAudio { samples }));
    if eager {
        daemon.stop_active_recording(&mut state, &mut capture, &mut None, &mut None).await;
    } else {
        assert!(daemon.start_transcription_task(&mut state, &mut capture, None).await);
        let result = tokio::time::timeout(Duration::from_secs(120), daemon.transcription_task.take().unwrap()).await.unwrap();
        daemon.handle_transcription_result(&mut state, result).await;
    }
    assert_fidelity(root.path(), &daemon, &raw, name.rsplit("::").next().unwrap(), false).await;
}

struct StreamingFidelityAudio {
    samples: Vec<f32>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    /// Told once the samples are on their way to the streaming session, for
    /// a test that stops the recording without waiting for an event first.
    /// Stopping aborts the daemon's tap (`cut_streaming_audio`), and audio
    /// the tap has not passed on by then never reaches the session.
    delivered: Option<tokio::sync::oneshot::Sender<()>>,
}
#[async_trait::async_trait]
impl AudioCapture for StreamingFidelityAudio {
    async fn start(&mut self) -> std::result::Result<tokio::sync::mpsc::Receiver<Vec<f32>>, crate::error::AudioError> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let (stop, wait) = tokio::sync::oneshot::channel();
        self.stop = Some(stop);
        let samples = self.samples.clone();
        let delivered = self.delivered.take();
        tokio::spawn(async move {
            tx.send(samples).await.unwrap();
            if let Some(delivered) = delivered {
                // The channel holds one chunk, and the tap takes a chunk,
                // passes it to the session's queue, then comes back for the
                // next. So the first empty chunk is accepted when the tap has
                // taken the samples, and the second when it has taken the
                // first, which it does only after passing the samples on.
                // Empty chunks add no audio. The session reads its queue in
                // order, so it has the samples before it sees the end.
                for _ in 0..2 { tx.send(Vec::new()).await.unwrap(); }
                let _ = delivered.send(());
            }
            let _ = wait.await;
            drop(tx);
        });
        Ok(rx)
    }
    async fn stop(&mut self) -> std::result::Result<Vec<f32>, crate::error::AudioError> { self.stop.take(); Ok(std::mem::take(&mut self.samples)) }
    async fn get_samples(&mut self) -> Vec<f32> { self.samples.clone() }
}

async fn real_stream_fidelity(name: &str, partials: bool) {
    if !enter_real_fidelity_case(name) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = fidelity_daemon(root.path());
    daemon.config.whisper.streaming = true;
    daemon.config.streaming = Some(crate::config::StreamingConfig { type_partials: partials, ..Default::default() });
    let base: Arc<dyn Transcriber> = crate::transcribe::create_transcriber(&daemon.config).unwrap().into();
    assert!(base.as_streaming().is_some(), "Actual factory must provide the supported adapter");
    daemon.transcriber_preloaded = Some(base.clone());
    daemon.history_test_capture = Some(Box::new(StreamingFidelityAudio { samples: fidelity_samples("speech_long.wav"), stop: None, delivered: None }));
    let mut state = State::Idle;
    let (mut capture, mut handle, mut session, mut chain) = (None, None, None, None);
    assert!(daemon.try_start_streaming(&mut state, &mut capture, &mut handle, &mut session, &mut chain, None, false).await);
    let mut observed = Vec::new();
    // Production file-event routing uses these exact existing session methods.
    // Stop after a real Partial; EOF then drains the actual backend's final tail.
    let mut stopped = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(60), handle.as_mut().unwrap().events.recv()).await.unwrap().unwrap();
        match event {
            StreamingEvent::Partial { text, .. } => {
                observed.push(serde_json::json!({"partial":text}));
                assert!(partials, "The real backend must emit the selected event kind");
                session.as_mut().unwrap().observe_partial_delta(&text);
                if !stopped { daemon.stop_active_recording(&mut state, &mut capture, &mut chain, &mut None).await; stopped = true; }
            }
            StreamingEvent::Final { text, .. } => {
                observed.push(serde_json::json!({"final":text}));
                session.as_mut().unwrap().commit_segment_silent(&text);
                if !stopped { daemon.stop_active_recording(&mut state, &mut capture, &mut chain, &mut None).await; stopped = true; }
            }
            StreamingEvent::Replace { backspace, text, .. } => { observed.push(serde_json::json!({"backspace":backspace,"replacement":text})); session.as_mut().unwrap().replace_and_commit_silent(backspace, &text); }
            StreamingEvent::RevisePartial { backspace, text, .. } => { observed.push(serde_json::json!({"backspace":backspace,"revision":text})); session.as_mut().unwrap().revise_partial_silent(backspace, &text); }
            StreamingEvent::Ended => break,
            StreamingEvent::Error(error) => panic!("Actual streaming backend error: {error}"),
        }
    }
    session.as_mut().unwrap().finalize_pending_partial();
    let raw = session.as_ref().unwrap().finalized_text().to_owned();
    let tag = name.rsplit("::").next().unwrap();
    write_evidence(format!("{tag}-events.json"), serde_json::to_vec_pretty(&observed).unwrap());
    assert!(!observed.is_empty());
    daemon.end_streaming(&mut state, &mut capture, &mut handle, &mut session, &mut chain, StreamingEndReason::Completed).await;
    assert_fidelity(root.path(), &daemon, &raw, tag, true).await;
}

#[tokio::test(flavor="multi_thread", worker_threads=2)]
#[ignore = "Requires approved pinned NAS model"]
async fn history_real_fidelity_batch_short() { real_batch_or_eager_fidelity("daemon::tests::history_contracts::history_real_fidelity_batch_short", "speech_hello.wav", false).await; }
#[tokio::test(flavor="multi_thread", worker_threads=2)]
#[ignore = "Requires approved pinned NAS model"]
async fn history_real_fidelity_batch_long() { real_batch_or_eager_fidelity("daemon::tests::history_contracts::history_real_fidelity_batch_long", "speech_long.wav", false).await; }
#[tokio::test(flavor="multi_thread", worker_threads=2)]
#[ignore = "Requires approved pinned NAS model"]
async fn history_real_fidelity_eager_short() { real_batch_or_eager_fidelity("daemon::tests::history_contracts::history_real_fidelity_eager_short", "speech_hello.wav", true).await; }
#[tokio::test(flavor="multi_thread", worker_threads=2)]
#[ignore = "Requires approved pinned NAS model"]
async fn history_real_fidelity_eager_long() { real_batch_or_eager_fidelity("daemon::tests::history_contracts::history_real_fidelity_eager_long", "speech_long.wav", true).await; }
#[tokio::test(flavor="multi_thread", worker_threads=2)]
#[ignore = "Requires approved pinned NAS model"]
async fn history_real_fidelity_stream_partial() { real_stream_fidelity("daemon::tests::history_contracts::history_real_fidelity_stream_partial", true).await; }

#[tokio::test(flavor="multi_thread", worker_threads=2)]
#[ignore = "Requires approved pinned NAS model"]
async fn history_real_fidelity_stream_final() { real_stream_fidelity("daemon::tests::history_contracts::history_real_fidelity_stream_final", false).await; }

#[tokio::test(flavor="multi_thread", worker_threads=2)]
#[ignore = "Requires approved pinned NAS model"]
async fn history_real_stream_cancel_loaded_model_stays_unknown_before_decode() {
    let name = "daemon::tests::history_contracts::history_real_stream_cancel_loaded_model_stays_unknown_before_decode";
    if !enter_real_fidelity_case(name) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = fidelity_daemon(root.path());
    daemon.config.whisper.streaming = true;
    let base: Arc<dyn Transcriber> = crate::transcribe::create_transcriber(&daemon.config).unwrap().into();
    let loaded = base.runtime_metadata();
    assert!(loaded.backend.is_none());
    // A new stream with no decode must not inherit the previous batch call.
    assert!(!base.transcribe(&fidelity_samples("speech_hello.wav")).unwrap().is_empty());
    assert_eq!(base.runtime_metadata().backend.as_deref(), Some("cpu"));
    daemon.transcriber_preloaded = Some(base.clone());
    daemon.history_test_capture = Some(Box::new(StreamingFidelityAudio { samples: Vec::new(), stop: None, delivered: None }));
    let mut state = State::Idle;
    let (mut capture, mut handle, mut session, mut chain) = (None, None, None, None);
    assert!(daemon.try_start_streaming(&mut state, &mut capture, &mut handle, &mut session, &mut chain, None, false).await);
    daemon.cancel_requested_session(&mut state, &mut capture, &mut handle, &mut session, &mut chain, &mut None).await;
    let entries = wait_for_entries(root.path(), 1).await;
    let evidence = serde_json::json!({"kind":"ACTUAL_LOADED_MODEL_STREAM_CANCEL_WITHOUT_DECODE_NOT_ASR_SUCCESS","loaded":loaded,"after":base.runtime_metadata(),"entries":entries});
    write_evidence("real-stream-cancel.json", serde_json::to_vec_pretty(&evidence).unwrap());
    println!("REAL_STREAM_CANCEL_OBSERVATION {evidence}");
    assert!(base.runtime_metadata().backend.is_none(), "A stream cancelled without decode must not reuse the preceding batch backend");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].outcome, crate::history::Outcome::Cancelled);
    assert!(!entries[0].delivered);
    assert!(entries[0].raw_text.is_empty());
    assert_eq!(entries[0].model, FIDELITY_MODEL);
    assert_eq!(entries[0].model_load_ms, loaded.model_load_ms);
    assert!(entries[0].backend.is_none());
    assert!(!root.path().join("transcript.txt").exists());
    assert!(daemon.active_transcriber.is_none());
}

#[tokio::test(flavor="multi_thread", worker_threads=2)]
async fn history_controlled_stream_error_keeps_unmeasured_metadata_unknown() {
    const NAME: &str = "daemon::tests::history_contracts::history_controlled_stream_error_keeps_unmeasured_metadata_unknown";
    if !enter_private_case(NAME) { return; }
    struct UnmeasuredFailure;
    impl Transcriber for UnmeasuredFailure {
        fn transcribe(&self, _: &[f32]) -> std::result::Result<String, crate::error::TranscribeError> {
            Err(crate::error::TranscribeError::InferenceFailed("CONTROLLED_PROTOCOL_ERROR_NOT_ASR".into()))
        }
    }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = fidelity_daemon(root.path());
    let adapter = crate::transcribe::SlidingWindowStreamingTranscriber::new(Arc::new(UnmeasuredFailure), crate::transcribe::SlidingWindowConfig { interval_s: 0.02, min_audio_s: 0.0, min_speech_rms: 0.0, ..Default::default() });
    assert_eq!(adapter.runtime_metadata(), crate::transcribe::RuntimeMetadata::default());
    daemon.transcriber_preloaded = Some(Arc::new(adapter));
    daemon.history_test_capture = Some(Box::new(StreamingFidelityAudio { samples: vec![0.2;16000], stop: None, delivered: None }));
    let mut state = State::Idle;
    let (mut capture, mut handle, mut session, mut chain) = (None, None, None, None);
    assert!(daemon.try_start_streaming(&mut state, &mut capture, &mut handle, &mut session, &mut chain, None, false).await);
    let event = tokio::time::timeout(Duration::from_secs(10), handle.as_mut().unwrap().events.recv()).await.unwrap().unwrap();
    let StreamingEvent::Error(error) = event else { panic!("Expected controlled adapter error, got {event:?}"); };
    daemon.end_streaming_with_error(error, &mut state, &mut capture, &mut handle, &mut session, &mut chain).await;
    let entries = wait_for_entries(root.path(), 1).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].outcome, crate::history::Outcome::TranscriptionFailed);
    assert!(!entries[0].delivered);
    assert!(entries[0].backend.is_none());
    assert!(entries[0].model_load_ms.is_none());
    assert!(entries[0].raw_text.is_empty());
    assert!(daemon.active_transcriber.is_none());
}
