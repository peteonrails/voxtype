// Real-model acceptance. Explicitly ignored unless run in the private NAS image
// carrying the pinned tiny.en model. The audio is committed TTS, never a device.
struct PrerecordedHistoryAudio {
    samples: Vec<f32>,
}

#[async_trait::async_trait]
impl AudioCapture for PrerecordedHistoryAudio {
    async fn start(&mut self) -> std::result::Result<tokio::sync::mpsc::Receiver<Vec<f32>>, crate::error::AudioError> {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender.send(self.samples.clone()).await.unwrap();
        Ok(receiver)
    }

    async fn stop(&mut self) -> std::result::Result<Vec<f32>, crate::error::AudioError> {
        Ok(std::mem::take(&mut self.samples))
    }

    async fn get_samples(&mut self) -> Vec<f32> {
        self.samples.clone()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires the pinned tiny.en model in the authorized offline NAS image"]
async fn history_real_whisper_batch_records_measured_metadata() {
    real_whisper_history_case("daemon::tests::history_contracts::history_real_whisper_batch_records_measured_metadata", "speech_hello.wav", "hello world", "real-history-whisper-batch").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires the pinned tiny.en model in the authorized offline NAS image"]
async fn history_real_whisper_long_batch_records_measured_metadata() {
    real_whisper_history_case("daemon::tests::history_contracts::history_real_whisper_long_batch_records_measured_metadata", "speech_long.wav", "this is a longer test of voice activity detection with multiple words and phrases", "real-history-whisper-long-batch").await;
}

async fn real_whisper_history_case(name: &'static str, fixture: &'static str, reference: &'static str, evidence_name: &'static str) {
    if std::env::var("VOXTYPE_HISTORY_REAL_MODEL_CHILD").as_deref() != Ok(name) {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--ignored", "--nocapture"])
            .env_clear()
            .env("HOME", root.path())
            .env("TMPDIR", root.path())
            .env("XDG_RUNTIME_DIR", runtime)
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("RUST_MIN_STACK", "16777216")
            .env("VOXTYPE_HISTORY_REAL_MODEL_CHILD", name)
            .stdin(std::process::Stdio::null())
            .output().unwrap();
        print!("{}", String::from_utf8_lossy(&result.stdout));
        eprint!("{}", String::from_utf8_lossy(&result.stderr));
        assert!(result.status.success(), "Real model child failed: {}", result.status);
        return;
    }
    const MODEL: &str = "/opt/history-models/ggml-tiny.en.bin";
    let model = std::fs::read(MODEL).unwrap();
    assert_eq!(model.len(), 77_704_715);
    assert_eq!(&model[..4], b"lmgg");
    let mut wav = hound::WavReader::open(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vad").join(fixture)).unwrap();
    assert_eq!(wav.spec().sample_rate, 16_000);
    assert_eq!(wav.spec().channels, 1);
    assert_eq!(wav.spec().bits_per_sample, 16);
    let samples: Vec<f32> = wav.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
    let expected_audio_ms = samples.len() as u64 * 1000 / 16000;
    let root = tempfile::tempdir().unwrap();
    let mut daemon = history_daemon(root.path());
    daemon.config.whisper.model = MODEL.into();
    daemon.config.whisper.mode = Some(crate::config::WhisperMode::Local);
    daemon.config.whisper.language = crate::config::LanguageConfig::Single("en".into());
    daemon.config.whisper.threads = Some(2);
    daemon.config.whisper.on_demand_loading = true;
    daemon.config.whisper.gpu_isolation = false;
    daemon.config.whisper.gpu_wait_secs = 0;
    daemon.config.whisper.streaming = false;
    daemon.config.audio.device = "synthetic-prerecorded-tts".into();
    daemon.config.output.mode = crate::config::OutputMode::File;
    let output = root.path().join("transcript.txt");
    daemon.config.output.file_path = Some(output.clone());
    daemon.config.output.fallback_to_clipboard = false;
    daemon.config.output.notification.on_recording_start = false;
    daemon.config.output.notification.on_recording_stop = false;
    daemon.config.output.notification.on_transcription = false;
    daemon.config.output.notification.on_no_speech = false;
    let mut state = State::Recording { started_at: std::time::Instant::now(), model_override: None };
    daemon.begin_osd_session(&state);
    daemon.prepare_for_recording(None);
    let mut audio: Option<Box<dyn AudioCapture>> = Some(Box::new(PrerecordedHistoryAudio { samples }));
    assert!(daemon.start_transcription_task(&mut state, &mut audio, None).await, "Real recording-stop path must reach model inference");
    let result = tokio::time::timeout(Duration::from_secs(120), daemon.transcription_task.take().unwrap()).await.unwrap();
    let raw = result.as_ref().unwrap().as_ref().unwrap().clone();
    let measured = daemon.active_transcriber.as_ref().unwrap().runtime_metadata();
    let detected = daemon.active_transcriber.as_ref().unwrap().last_detected_language();
    daemon.handle_transcription_result(&mut state, result).await;
    let entries = wait_for_entries(root.path(), 1).await;
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    let normalize = |text: &str| text.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let evidence = serde_json::json!({"kind":"ACTUAL_REAL_MODEL_SYNTHETIC_BATCH_RECORDER_NOT_LIVE_CAPTURE","fixture":fixture,"reference":reference,"reference_matches":normalize(&raw)==normalize(reference),"requested_model":MODEL,"requested_language":"en","language_source":"configured en, not automatic detection","measured":measured,"reported_language":detected,"entry":entry,"scope":"Actual daemon prepare_for_recording/start_transcription_task/handle_transcription_result + file delivery + background Recorder. Prerecorded AudioCapture fixture replaces devices. No production daemon or event-listener startup. Observations are saved BEFORE metadata assertions; consult actual producer exit."});
    std::fs::write(format!("/work/out/{evidence_name}.json"), serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("REAL_MODEL_HISTORY_EVIDENCE {evidence}");
    let dayfiles: Vec<_> = std::fs::read_dir(root.path().join("history")).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e=="jsonl")).collect();
    assert_eq!(dayfiles.len(), 1);
    let day = std::fs::read(&dayfiles[0]).unwrap();
    std::fs::write(format!("/work/out/{evidence_name}.jsonl"), &day).unwrap();
    assert_eq!(detected.as_deref(), Some("en"), "The configured language used by real inference must be retained; this is not automatic language-detection acceptance");
    assert_eq!(entry.raw_text, raw);
    assert_eq!(entry.text, std::fs::read_to_string(&output).unwrap().trim_end());
    assert_eq!(entry.model, MODEL);
    assert_eq!(entry.backend.as_deref(), Some("cpu"));
    assert_eq!(entry.model_load_ms, measured.model_load_ms);
    assert!(entry.model_load_ms.is_some());
    assert!(entry.transcribe_ms.is_some());
    assert_eq!(entry.language, detected);
    assert_eq!(entry.audio_ms, expected_audio_ms);
    assert_eq!(entry.engine, "whisper");
    assert_eq!(entry.outcome, crate::history::Outcome::Delivered);
    assert!(entry.delivered && !entry.streaming);
    assert_eq!(entry.input_device.as_deref(), Some("synthetic-prerecorded-tts"));
    assert_eq!(entry.output.as_ref().unwrap().driver.as_deref(), Some("file"));
    assert_eq!(std::str::from_utf8(&day).unwrap().lines().count(), 1);
    assert_eq!(serde_json::from_slice::<crate::history::HistoryEntry>(&day).unwrap(), *entry);
    println!("REAL_MODEL_HISTORY_METADATA_ASSERTIONS_PASSED_REFERENCE_CHECK_STILL_REQUIRED");
    assert_eq!(normalize(&raw), normalize(reference), "Committed TTS reference must actually be recognized: {reference}");
    println!("REAL_MODEL_HISTORY_ASSERTIONS_PASSED");
}

#[test]
#[ignore = "Requires the pinned tiny.en model in the authorized offline NAS image"]
fn history_real_whisper_metadata_lifecycle_never_reuses_backend_after_empty_input() {
    const MODEL: &str = "/opt/history-models/ggml-tiny.en.bin";
    let mut config = Config::default();
    config.whisper.model = MODEL.into();
    config.whisper.mode = Some(crate::config::WhisperMode::Local);
    config.whisper.language = crate::config::LanguageConfig::Single("en".into());
    config.whisper.threads = Some(2);
    config.whisper.gpu_wait_secs = 0;
    config.whisper.gpu_isolation = false;
    let engine = crate::transcribe::create_transcriber(&config).unwrap();
    let before = engine.runtime_metadata();
    assert_eq!(before.model.as_deref(), Some(MODEL));
    assert!(before.model_load_ms.is_some());
    assert!(before.backend.is_none(), "Loading a CPU context is not proof of a decode");
    let mut wav = hound::WavReader::open(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/vad/speech_hello.wav")).unwrap();
    assert_eq!(wav.spec().sample_rate, 16_000);
    let samples: Vec<f32> = wav.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
    let raw = engine.transcribe(&samples).unwrap();
    assert!(!raw.is_empty(), "This fixture checks actual inference metadata, not transcript accuracy");
    let batch = engine.runtime_metadata();
    assert_eq!(batch.backend.as_deref(), Some("cpu"));
    assert_eq!(batch.model_load_ms, before.model_load_ms);
    assert!(engine.transcribe(&[]).is_err());
    let rejected_batch = engine.runtime_metadata();
    assert!(rejected_batch.backend.is_none(), "An input rejection must not inherit a previous decode's backend");
    assert_eq!(rejected_batch.model, before.model);
    assert_eq!(rejected_batch.model_load_ms, before.model_load_ms);
    assert!(!engine.transcribe_timed(&samples).unwrap().is_empty());
    let timed = engine.runtime_metadata();
    assert_eq!(timed.backend.as_deref(), Some("cpu"));
    assert!(engine.transcribe_timed(&[]).is_err());
    let rejected_timed = engine.runtime_metadata();
    assert!(rejected_timed.backend.is_none());
    let evidence = serde_json::json!({"kind":"ACTUAL_REAL_MODEL_METADATA_LIFECYCLE_NOT_ACCURACY_OR_RECORDER_ACCEPTANCE","before_decode":before,"after_batch":batch,"after_rejected_batch":rejected_batch,"after_timed":timed,"after_rejected_timed":rejected_timed,"raw_observed_not_reference":raw});
    std::fs::write("/work/out/real-whisper-metadata-lifecycle.json", serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("REAL_MODEL_LIFECYCLE_ASSERTIONS_PASSED {evidence}");
}
