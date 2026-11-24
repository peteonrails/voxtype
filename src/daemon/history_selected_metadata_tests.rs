// Controlled selection/persistence contracts. No ASR/provider/model/device calls.
struct SelectedMetadataFixture { fail: bool, empty: bool, attempted: std::sync::atomic::AtomicBool }
impl Transcriber for SelectedMetadataFixture {
    fn transcribe(&self, _: &[f32]) -> std::result::Result<String, crate::error::TranscribeError> {
        self.attempted.store(true, std::sync::atomic::Ordering::Relaxed);
        if self.fail { Err(crate::error::TranscribeError::InferenceFailed("CONTROLLED_SELECTED_ERROR_NOT_ASR".into())) }
        else if self.empty { Ok(String::new()) }
        else { Ok("Controlled selected transcript".into()) }
    }
    fn runtime_metadata(&self) -> crate::transcribe::RuntimeMetadata {
        crate::transcribe::RuntimeMetadata { model: Some("measured-selected-not-configured.bin".into()), backend: self.attempted.load(std::sync::atomic::Ordering::Relaxed).then(|| "controlled-measured-backend".into()), model_load_ms: Some(29) }
    }
    fn last_detected_language(&self) -> Option<String> { self.attempted.load(std::sync::atomic::Ordering::Relaxed).then(|| "de".into()) }
}

async fn selected_metadata_case(name: &str, eager: bool, fail: bool, cancel: bool, empty: bool) {
    if !enter_private_case(name) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = fidelity_daemon(root.path());
    let selected: Arc<dyn Transcriber> = Arc::new(SelectedMetadataFixture { fail, empty, attempted: std::sync::atomic::AtomicBool::new(false) });
    if eager {
        let mut state = State::EagerRecording { started_at: std::time::Instant::now(), model_override: None, accumulated_audio: vec![0.2;16000], chunks_sent: 0, chunk_results: Vec::new(), tasks_in_flight: 0 };
        daemon.begin_osd_session(&state);
        let text = daemon.finish_eager_recording(&mut state, selected.clone()).await;
        if let Some(text) = text {
            state = State::Transcribing { audio: Vec::new() };
            daemon.handle_transcription_result(&mut state, Ok(Ok(text))).await;
        } else {
            // Same empty/error completion path as stop_active_recording.
            daemon.publish_osd_outcome(osd::session::OutcomeKind::Empty, 0);
            daemon.reset_to_idle(&mut state).await;
        }
    } else {
        let adapter = crate::transcribe::SlidingWindowStreamingTranscriber::new(selected.clone(), crate::transcribe::SlidingWindowConfig { interval_s: 0.02, min_audio_s: 0.0, min_speech_rms: 0.0, ..Default::default() });
        daemon.transcriber_preloaded = Some(Arc::new(adapter));
        daemon.history_test_capture = Some(Box::new(StreamingFidelityAudio { samples: if cancel { Vec::new() } else { vec![0.2;16000] }, stop: None, delivered: None }));
        let mut state = State::Idle;
        let (mut capture, mut handle, mut session, mut chain) = (None, None, None, None);
        assert!(daemon.try_start_streaming(&mut state, &mut capture, &mut handle, &mut session, &mut chain, None, false).await);
        if cancel {
            daemon.cancel_requested_session(&mut state, &mut capture, &mut handle, &mut session, &mut chain, &mut None).await;
        } else {
            let mut stopped = false;
            loop {
                let event = tokio::time::timeout(Duration::from_secs(10), handle.as_mut().unwrap().events.recv()).await.unwrap().unwrap();
                match event {
                    StreamingEvent::Partial { text, .. } => { session.as_mut().unwrap().observe_partial_delta(&text); if !stopped { daemon.stop_active_recording(&mut state, &mut capture, &mut chain, &mut None).await; stopped = true; } }
                    StreamingEvent::Final { text, .. } => session.as_mut().unwrap().commit_segment_silent(&text),
                    StreamingEvent::Replace { .. } | StreamingEvent::RevisePartial { .. } => panic!("Controlled fixture does not revise"),
                    StreamingEvent::Error(error) => { assert!(fail); daemon.end_streaming_with_error(error, &mut state, &mut capture, &mut handle, &mut session, &mut chain).await; break; }
                    StreamingEvent::Ended => { daemon.end_streaming(&mut state, &mut capture, &mut handle, &mut session, &mut chain, StreamingEndReason::Completed).await; break; }
                }
            }
        }
    }
    let entries = wait_for_entries(root.path(), 1).await;
    let entry = &entries[0];
    let observation = serde_json::json!({"kind":"CONTROLLED_SELECTED_METADATA_PERSISTENCE_NOT_ASR","eager":eager,"fail":fail,"cancel":cancel,"empty":empty,"configured_model":daemon.config.whisper.model,"configured_language":"en","reported":selected.runtime_metadata(),"reported_language":selected.last_detected_language(),"entry":entry});
    write_evidence(format!("{}.json",name.rsplit("::").next().unwrap()),serde_json::to_vec_pretty(&observation).unwrap());
    println!("SELECTED_METADATA_PERSISTENCE {observation}");
    assert_eq!(entries.len(),1);
    if fail { assert_eq!(entry.outcome, crate::history::Outcome::TranscriptionFailed, "A controlled failed inference must not be recorded as empty or delivered"); }
    assert_ne!(entry.model, daemon.config.whisper.model, "Measured selection must replace the configured fallback");
    assert_eq!(entry.model,"measured-selected-not-configured.bin");
    if cancel { assert!(entry.backend.is_none()); }
    else {
        assert_eq!(entry.language.as_deref(),Some("de"));
        assert_eq!(entry.backend.as_deref(),Some("controlled-measured-backend"));
    }
    assert_eq!(entry.model_load_ms,Some(29));
    if cancel { assert_eq!(entry.outcome,crate::history::Outcome::Cancelled); assert!(!entry.delivered); }
    else if fail && !eager { assert_eq!(entry.outcome,crate::history::Outcome::TranscriptionFailed); assert!(!entry.delivered); }
    else if !empty && !fail { assert_eq!(entry.outcome,crate::history::Outcome::Delivered); assert_eq!(entry.raw_text,"Controlled selected transcript"); }
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_selected_metadata_stream_completed() { selected_metadata_case("daemon::tests::history_contracts::history_selected_metadata_stream_completed",false,false,false,false).await; }
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_selected_metadata_stream_error() { selected_metadata_case("daemon::tests::history_contracts::history_selected_metadata_stream_error",false,true,false,false).await; }
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_selected_metadata_stream_cancelled() { selected_metadata_case("daemon::tests::history_contracts::history_selected_metadata_stream_cancelled",false,false,true,false).await; }
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_selected_metadata_eager_completed() { selected_metadata_case("daemon::tests::history_contracts::history_selected_metadata_eager_completed",true,false,false,false).await; }
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_selected_metadata_eager_empty() { selected_metadata_case("daemon::tests::history_contracts::history_selected_metadata_eager_empty",true,false,false,true).await; }
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_selected_metadata_eager_error() { selected_metadata_case("daemon::tests::history_contracts::history_selected_metadata_eager_error",true,true,false,false).await; }

async fn eager_chunk_error_contract(name: &str, poll: bool) {
    if !enter_private_case(name) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = fidelity_daemon(root.path());
    let selected: Arc<dyn Transcriber> = Arc::new(SelectedMetadataFixture { fail: true, empty: false, attempted: std::sync::atomic::AtomicBool::new(false) });
    let mut state = State::EagerRecording { started_at: std::time::Instant::now(), model_override: None, accumulated_audio: vec![0.2;16000], chunks_sent:1, chunk_results:Vec::new(), tasks_in_flight:1 };
    daemon.begin_osd_session(&state);
    daemon.spawn_chunk_transcription(0,vec![0.2;16000],selected.clone());
    if poll {
        tokio::time::timeout(Duration::from_secs(10),async { while !daemon.eager_chunk_tasks[0].1.is_finished() { tokio::time::sleep(Duration::from_millis(1)).await; } }).await.unwrap();
        let completed = daemon.poll_chunk_tasks().await;
        if let State::EagerRecording { chunk_results, .. } = &mut state { chunk_results.extend(completed); }
    }
    daemon.stop_active_recording(&mut state, &mut None, &mut None, &mut Some(selected)).await;
    let entries=wait_for_entries(root.path(),1).await;
    assert_eq!(entries.len(),1);
    assert_eq!(entries[0].outcome,crate::history::Outcome::TranscriptionFailed);
    assert!(!entries[0].delivered);
    assert!(entries[0].error.as_deref().unwrap().contains("CONTROLLED_SELECTED_ERROR_NOT_ASR"));
    assert_eq!(entries[0].model,"measured-selected-not-configured.bin");
    assert_eq!(entries[0].model_load_ms,Some(29));
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_selected_metadata_eager_polled_chunk_error() { eager_chunk_error_contract("daemon::tests::history_contracts::history_selected_metadata_eager_polled_chunk_error",true).await; }
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_selected_metadata_eager_waited_chunk_error() { eager_chunk_error_contract("daemon::tests::history_contracts::history_selected_metadata_eager_waited_chunk_error",false).await; }

async fn no_decode_eager_authority(name: &str, samples: usize) {
    if !enter_private_case(name) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = fidelity_daemon(root.path());
    let selected: Arc<dyn Transcriber> = Arc::new(SelectedMetadataFixture { fail:false, empty:false, attempted:std::sync::atomic::AtomicBool::new(false) });
    selected.transcribe(&[0.2;4800]).unwrap();
    assert!(selected.runtime_metadata().backend.is_some());
    let mut state = State::EagerRecording { started_at:std::time::Instant::now(), model_override:None, accumulated_audio:vec![0.2;samples], chunks_sent:0, chunk_results:Vec::new(), tasks_in_flight:0 };
    daemon.begin_osd_session(&state);
    daemon.stop_active_recording(&mut state, &mut None, &mut None, &mut Some(selected)).await;
    let entries=wait_for_entries(root.path(),1).await;
    println!("NO_DECODE_EAGER_PERSISTED {}",serde_json::to_string(&entries).unwrap());
    assert_eq!(entries.len(),1);
    assert_eq!(entries[0].outcome,crate::history::Outcome::Empty);
    assert_eq!(entries[0].audio_ms,samples as u64/16);
    assert!(entries[0].backend.is_none(),"No current decode must not inherit a preceding backend");
    assert!(entries[0].language.is_none(),"No current decode must not inherit a preceding language");
    assert_eq!(entries[0].model,"measured-selected-not-configured.bin");
    assert_eq!(entries[0].model_load_ms,Some(29));
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_authority_eager_zero_no_decode() { no_decode_eager_authority("daemon::tests::history_contracts::history_authority_eager_zero_no_decode",0).await; }
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_authority_eager_subthreshold_no_decode() { no_decode_eager_authority("daemon::tests::history_contracts::history_authority_eager_subthreshold_no_decode",4799).await; }

struct EofFailureEngine { fail_after: usize, calls: std::sync::atomic::AtomicUsize }
impl Transcriber for EofFailureEngine {
    fn transcribe(&self, _: &[f32]) -> std::result::Result<String,crate::error::TranscribeError> {
        let n=self.calls.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
        if n>=self.fail_after { Err(crate::error::TranscribeError::InferenceFailed("CONTROLLED_EOF_FAILURE".into())) }
        else { Ok("Earlier controlled partial".into()) }
    }
}
async fn eof_failure_persistence(name: &str, partial: bool) {
    if !enter_private_case(name) { return; }
    let root=tempfile::tempdir().unwrap(); let mut daemon=fidelity_daemon(root.path());
    let engine=Arc::new(EofFailureEngine { fail_after:if partial {2} else {0}, calls:std::sync::atomic::AtomicUsize::new(0) });
    daemon.transcriber_preloaded=Some(Arc::new(crate::transcribe::SlidingWindowStreamingTranscriber::new(engine.clone(),crate::transcribe::SlidingWindowConfig { interval_s:if partial {0.2} else {60.0}, partial_min_words:1, min_audio_s:0.0,min_speech_rms:0.0,..Default::default() })));
    let (delivered,audio_delivered)=tokio::sync::oneshot::channel();
    daemon.history_test_capture=Some(Box::new(StreamingFidelityAudio { samples:vec![0.2;16000],stop:None,delivered:(!partial).then_some(delivered) }));
    let mut state=State::Idle; let (mut capture,mut handle,mut session,mut chain)=(None,None,None,None);
    assert!(daemon.try_start_streaming(&mut state,&mut capture,&mut handle,&mut session,&mut chain,None,false).await);
    if !partial {
        // Stop only once the audio is on its way to the session. A stop that
        // beats it there ends the session with nothing to flush: no engine
        // call, no error, and correctly so, but not the case under test.
        // The wait is on the fixture's own signal; the limit only keeps a
        // broken fixture from hanging the run.
        tokio::time::timeout(Duration::from_secs(60),audio_delivered).await.expect("the fixture audio never reached the streaming session").unwrap();
        daemon.stop_active_recording(&mut state, &mut capture, &mut chain, &mut None).await;
    }
    let mut saw_partial=false; let mut errors=0;
    loop {
        let event=tokio::time::timeout(Duration::from_secs(10),handle.as_mut().unwrap().events.recv()).await.unwrap().unwrap();
        match event {
            StreamingEvent::Partial { text,.. } => { saw_partial=true; session.as_mut().unwrap().observe_partial_delta(&text); daemon.stop_active_recording(&mut state, &mut capture, &mut chain, &mut None).await; }
            StreamingEvent::Final {text,..} => session.as_mut().unwrap().commit_segment_silent(&text),
            StreamingEvent::Error(e) => { errors+=1; daemon.end_streaming_with_error(e,&mut state,&mut capture,&mut handle,&mut session,&mut chain).await; break; }
            StreamingEvent::Ended => { daemon.end_streaming(&mut state, &mut capture, &mut handle, &mut session, &mut chain, StreamingEndReason::Completed).await; break; }
            StreamingEvent::Replace {..} | StreamingEvent::RevisePartial {..} => panic!("fixture does not revise"),
        }
    }
    let entries=wait_for_entries(root.path(),1).await;
    println!("EOF_FAILURE_PERSISTED calls={} errors={errors} {}",engine.calls.load(std::sync::atomic::Ordering::SeqCst),serde_json::to_string(&entries).unwrap());
    assert_eq!(entries.len(),1); assert_eq!(saw_partial,partial); assert_eq!(errors,1);
    // With no partials the only engine call is the flush at the end.
    if !partial { assert_eq!(engine.calls.load(std::sync::atomic::Ordering::SeqCst),1); }
    assert_eq!(entries[0].outcome,crate::history::Outcome::TranscriptionFailed);
    assert!(!entries[0].delivered);
    assert!(entries[0].error.as_deref().unwrap().contains("CONTROLLED_EOF_FAILURE"));
    if partial { assert!(entries[0].raw_text.contains("Earlier controlled partial")); }
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_authority_eof_failure_before_partial() { eof_failure_persistence("daemon::tests::history_contracts::history_authority_eof_failure_before_partial",false).await; }
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_authority_eof_failure_after_partial() { eof_failure_persistence("daemon::tests::history_contracts::history_authority_eof_failure_after_partial",true).await; }

struct LoopIdentityEngine { id:&'static str,calls:std::sync::atomic::AtomicUsize, gate:std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>> }
impl Transcriber for LoopIdentityEngine {
    fn transcribe(&self,_:&[f32])->std::result::Result<String,crate::error::TranscribeError> {
        self.calls.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
        if let Some(wait)=self.gate.lock().unwrap().take() { wait.recv_timeout(Duration::from_secs(10)).unwrap(); }
        Ok("Selected identity transcript".into())
    }
    fn runtime_metadata(&self)->crate::transcribe::RuntimeMetadata { crate::transcribe::RuntimeMetadata { model:Some(self.id.into()), backend:Some(self.id.into()),model_load_ms:Some(17) } }
}
async fn real_loop_stop_identity(name:&str,route:&str) {
    if !enter_private_case(name) {return;}
    tokio::task::LocalSet::new().run_until(async {
    let root=tempfile::tempdir().unwrap(); let mut daemon=fidelity_daemon(root.path());
    daemon.config.hotkey.enabled=false; daemon.config.meeting.enabled=false; daemon.config.tray.enabled=false; daemon.config.osd.enabled=false;
    daemon.config.hotkey.mode=if route=="toggle" {ActivationMode::Toggle} else {ActivationMode::PushToTalk};
    daemon.config.audio.max_duration_secs=if route=="timeout" {1} else {60};
    let (release,wait)=std::sync::mpsc::channel();
    let selected=Arc::new(LoopIdentityEngine {id:"SELECTED_CHUNK_INSTANCE",calls:std::sync::atomic::AtomicUsize::new(0),gate:std::sync::Mutex::new(Some(wait))});
    let replacement=Arc::new(LoopIdentityEngine {id:"WRONG_REACQUIRED_INSTANCE",calls:std::sync::atomic::AtomicUsize::new(0),gate:std::sync::Mutex::new(None)});
    let initial=State::EagerRecording { started_at:std::time::Instant::now()-Duration::from_secs(if route=="timeout" {2} else {0}),model_override:None,accumulated_audio:vec![0.2;32000],chunks_sent:1,chunk_results:Vec::new(),tasks_in_flight:1 };
    let (events,rx)=tokio::sync::mpsc::channel(2);
    daemon.history_test_eager_loop=Some((initial,selected.clone(),replacement.clone(),rx));
    daemon.history_test_capture=Some(Box::new(PrerecordedHistoryAudio {samples:Vec::new()}));
    let task=tokio::task::spawn_local(async move {daemon.run().await});
    tokio::time::timeout(Duration::from_secs(10),async {while selected.calls.load(std::sync::atomic::Ordering::SeqCst)==0 {tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
    match route { "release"=>events.send(HotkeyEvent::Released).await.unwrap(), "toggle"=>events.send(HotkeyEvent::Pressed {model_override:None,profile_override:None}).await.unwrap(),"timeout"=>{},_=>panic!("unknown route") }
    tokio::time::sleep(Duration::from_millis(120)).await;
    release.send(()).unwrap();
    let entries=wait_for_entries(root.path(),1).await;
    task.abort(); let _=task.await;
    println!("ACTUAL_SELECT_LOOP_ROUTE {route} selected_calls={} replacement_calls={} {}",selected.calls.load(std::sync::atomic::Ordering::SeqCst),replacement.calls.load(std::sync::atomic::Ordering::SeqCst),serde_json::to_string(&entries).unwrap());
    assert_eq!(entries.len(),1); assert_eq!(entries[0].outcome,crate::history::Outcome::Delivered);
    assert_eq!(replacement.calls.load(std::sync::atomic::Ordering::SeqCst),0,"Stop caller must not replace the selected in-flight chunk instance");
    assert_eq!(entries[0].model,"SELECTED_CHUNK_INSTANCE");
    assert!(selected.calls.load(std::sync::atomic::Ordering::SeqCst)>=2);
    }).await;
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_authority_loop_release_identity() {real_loop_stop_identity("daemon::tests::history_contracts::history_authority_loop_release_identity","release").await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_authority_loop_toggle_identity() {real_loop_stop_identity("daemon::tests::history_contracts::history_authority_loop_toggle_identity","toggle").await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_authority_loop_timeout_identity() {real_loop_stop_identity("daemon::tests::history_contracts::history_authority_loop_timeout_identity","timeout").await;}


// Two-session ownership controls: actual cancel/start/stop loop and private
// capture. Identifiable protocol engines, not provider/GPU/model inference.
struct LifecycleAudio {
    samples: Vec<f32>,
    started: Arc<std::sync::atomic::AtomicBool>,
}
#[async_trait::async_trait]
impl AudioCapture for LifecycleAudio {
    async fn start(&mut self) -> std::result::Result<tokio::sync::mpsc::Receiver<Vec<f32>>, crate::error::AudioError> {
        self.started.store(true, std::sync::atomic::Ordering::Release);
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }
    async fn stop(&mut self) -> std::result::Result<Vec<f32>, crate::error::AudioError> { Ok(std::mem::take(&mut self.samples)) }
    async fn get_samples(&mut self) -> Vec<f32> { std::mem::take(&mut self.samples) }
}
struct LifecycleEngine {
    id: &'static str,
    language: &'static str,
    load: u64,
    fail: bool,
    calls: std::sync::atomic::AtomicUsize,
    completed: std::sync::atomic::AtomicBool,
    gate: std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}
impl Transcriber for LifecycleEngine {
    fn transcribe(&self, audio: &[f32]) -> std::result::Result<String, crate::error::TranscribeError> {
        assert!(!audio.is_empty());
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(wait) = self.gate.lock().unwrap().take() { wait.recv_timeout(Duration::from_secs(30)).unwrap(); }
        self.completed.store(true, std::sync::atomic::Ordering::Release);
        if self.fail { Err(crate::error::TranscribeError::InferenceFailed("LATE_A_ERROR".into())) }
        else { Ok(format!("{} transcript", self.id)) }
    }
    fn runtime_metadata(&self) -> crate::transcribe::RuntimeMetadata {
        crate::transcribe::RuntimeMetadata { model: Some(self.id.into()), backend: self.completed.load(std::sync::atomic::Ordering::Acquire).then(||self.id.into()), model_load_ms:Some(self.load) }
    }
    fn last_detected_language(&self) -> Option<String> { self.completed.load(std::sync::atomic::Ordering::Acquire).then(||self.language.into()) }
}
async fn lifecycle_entries(root: &Path, count: usize, timeout: Duration) -> Option<Vec<crate::history::HistoryEntry>> {
    tokio::time::timeout(timeout, async {
        loop {
            let entries=crate::history::HistoryStore::at(root.join("history")).all().unwrap();
            if entries.len()>=count { break entries; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.ok()
}
async fn cancel_then_next_selected(name:&str, cancel:&str, work:&str, stop:&str, signal_start:bool, named:bool) {
    if !enter_private_case(name) {return;}
    tokio::task::LocalSet::new().run_until(async {
        use std::sync::atomic::{AtomicBool,AtomicUsize,Ordering};
        let root=tempfile::tempdir().unwrap(); let mut daemon=fidelity_daemon(root.path());
        daemon.config.hotkey.enabled=false;daemon.config.meeting.enabled=false;daemon.config.tray.enabled=false;daemon.config.osd.enabled=false;
        daemon.config.whisper.eager_processing=true; daemon.config.whisper.eager_chunk_secs=10.0;
        daemon.config.hotkey.mode=if stop=="toggle" {ActivationMode::Toggle} else {ActivationMode::PushToTalk};
        daemon.config.audio.max_duration_secs=if stop=="timeout" {1} else {60};
        let gated=work!="completed"; let (release,wait)=std::sync::mpsc::channel();
        let a=Arc::new(LifecycleEngine {id:"CURRENT_A",language:"de",load:31,fail:work=="gated-error",calls:AtomicUsize::new(0),completed:AtomicBool::new(false),gate:std::sync::Mutex::new(gated.then_some(wait))});
        let default_b=Arc::new(LifecycleEngine {id:"NEXT_DEFAULT_B",language:"fr",load:47,fail:false,calls:AtomicUsize::new(0),completed:AtomicBool::new(false),gate:std::sync::Mutex::new(None)});
        let b=if named { Arc::new(LifecycleEngine {id:"NEXT_NAMED_B",language:"fr",load:47,fail:false,calls:AtomicUsize::new(0),completed:AtomicBool::new(false),gate:std::sync::Mutex::new(None)}) } else {default_b.clone()};
        if named {
            assert!(!signal_start);
            daemon.config.engine=crate::config::TranscriptionEngine::Soniox;
            daemon.config.whisper.mode=Some(crate::config::WhisperMode::Remote);
            assert!(matches!(crate::model_catalog::override_route(&daemon.config,"tiny.en"),Ok(crate::model_catalog::OverrideRoute::Separate {..})));
            let expected:Arc<dyn Transcriber>=b.clone();
            daemon.override_cache=Some(("tiny.en".into(),expected.clone()));
            let ready=daemon.ready_override("tiny.en").await.expect("named B ready");
            assert!(Arc::ptr_eq(&ready,&expected));
            let selected=daemon.get_transcriber_for_recording(Some("tiny.en")).await.expect("named B selected");
            assert!(Arc::ptr_eq(&selected,&expected));
            daemon.override_cache=Some(("tiny.en".into(),expected));
            // Keep ordinary private startup on the on-demand Whisper path;
            // existing loop injection selects the controlled protocol engine.
            daemon.config.engine=crate::config::TranscriptionEngine::Whisper;
        }
        let a_flag=Arc::new(AtomicBool::new(false)); let started_b=Arc::new(AtomicBool::new(false));
        let initial=State::EagerRecording {started_at:std::time::Instant::now(),model_override:None,accumulated_audio:vec![0.2;16000],chunks_sent:1,chunk_results:Vec::new(),tasks_in_flight:1};
        let (events,rx)=tokio::sync::mpsc::channel(4);
        daemon.history_test_eager_loop=Some((initial,a.clone(),default_b.clone(),rx));
        daemon.history_test_capture=Some(Box::new(PrerecordedHistoryAudio {samples:Vec::new()}));
        daemon.history_test_next_capture=Some(Box::new(LifecycleAudio {samples:vec![0.2;8000],started:started_b.clone()}));
        daemon.history_test_completion_observer=Some(a_flag.clone());
        let task=tokio::task::spawn_local(async move {daemon.run().await});
        tokio::time::timeout(Duration::from_secs(5),async {while a.calls.load(Ordering::SeqCst)==0 || (!gated && !a_flag.load(Ordering::Acquire)) {tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
        if cancel=="hotkey" {events.send(HotkeyEvent::Cancel).await.unwrap();}
        else {std::fs::write(Config::runtime_dir().join("cancel"),trigger_stamp()).unwrap();}
        let cancelled=lifecycle_entries(root.path(),1,Duration::from_secs(5)).await.unwrap();
        assert_eq!(cancelled.len(),1);
        if signal_start {assert_eq!(unsafe {libc::kill(libc::getpid(),libc::SIGUSR1)},0);}
        else {events.send(HotkeyEvent::Pressed {model_override:named.then(||"tiny.en".into()),profile_override:None}).await.unwrap();}
        tokio::time::timeout(Duration::from_secs(5),async {while !started_b.load(Ordering::Acquire) {tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
        // B is deliberately a tail-only recording. Its real capture returns
        // 8000 samples, below the controlled ten-second eager chunk boundary.
        match stop {
            "release"=>events.send(HotkeyEvent::Released).await.unwrap(),
            "toggle"=>events.send(HotkeyEvent::Pressed {model_override:None,profile_override:None}).await.unwrap(),
            "external"=>assert_eq!(unsafe {libc::kill(libc::getpid(),libc::SIGUSR2)},0),
            "timeout"=>{}, _=>panic!("unknown stop"),
        }
        let before_a=lifecycle_entries(root.path(),2,Duration::from_secs(if stop=="timeout" {3} else {1})).await;
        let b_finished_before_a=before_a.is_some();
        if gated {assert!(b_finished_before_a,"B must persist before releasing cancelled A");release.send(()).unwrap();}
        let entries=lifecycle_entries(root.path(),2,Duration::from_secs(5)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5),async {
            while !a_flag.load(Ordering::Acquire) {tokio::task::yield_now().await;}
        }).await.expect("old A must reach its post-terminal-result completion witness");
        assert!(a.completed.load(Ordering::Acquire));
        tokio::task::yield_now().await;
        let final_entries=crate::history::HistoryStore::at(root.path().join("history")).all().unwrap();
        task.abort();let _=task.await;
        println!("CANCEL_NEXT_SESSION {}",serde_json::json!({"cancel":cancel,"A_work":work,"B_stop":stop,"signal_start":signal_start,"named_B":named,"A_calls":a.calls.load(Ordering::SeqCst),"B_calls":b.calls.load(Ordering::SeqCst),"old_A_completion":a_flag.load(Ordering::Acquire),"B_finished_before_releasing_A":b_finished_before_a,"entries":entries,"final_entries":final_entries}));
        let cancelled=entries.iter().find(|e|e.outcome==crate::history::Outcome::Cancelled).unwrap();
        let next=entries.iter().find(|e|e.id!=cancelled.id).unwrap();
        let mut errors=Vec::new();
        if entries.len()!=2 || final_entries.len()!=2 {errors.push("not exactly one entry per session");}
        if cancelled.model!="CURRENT_A" || cancelled.model_load_ms!=Some(31) {errors.push("cancelled A lost known selected loaded facts");}
        if gated && (cancelled.backend.is_some() || cancelled.language.is_some()) {errors.push("A no-completed-decode cancellation inherited decode facts");}
        if !gated && cancelled.backend.as_deref()!=Some("CURRENT_A") {errors.push("completed A cancellation lost current decode facts");}
        if next.model!=b.id || next.model_load_ms!=Some(47) {errors.push("B persisted stale A selection");}
        if next.backend.as_deref()!=Some(b.id) || next.language.as_deref()!=Some("fr") {errors.push("B decode metadata not its own");}
        if next.audio_ms!=500 || next.raw_text!=format!("{} transcript",b.id) {errors.push("B tail/text not its own measured capture");}
        if next.outcome!=crate::history::Outcome::Delivered || !next.delivered {errors.push("B terminal outcome contaminated by A");}
        if a.calls.load(Ordering::SeqCst)!=1 || b.calls.load(Ordering::SeqCst)!=1 {errors.push("tail did not execute exactly on B");}
        if gated && !b_finished_before_a {errors.push("B depended on cancelled A completion");}
        assert!(a_flag.load(Ordering::Acquire),"late A result was actually completed");
        assert_eq!(serde_json::to_value(&entries).unwrap(),serde_json::to_value(&final_entries).unwrap(),"late A completion changed terminal history");
        assert_eq!(next.text,format!("{} transcript",b.id));
        assert_eq!(std::fs::read_to_string(root.path().join("transcript.txt")).unwrap(),format!("{}\n",next.text));
        if named {assert_eq!(default_b.calls.load(Ordering::SeqCst),0,"named routing fell back to default");}
        assert!(errors.is_empty(),"CANCEL_NEXT_OWNERSHIP_FAILURES {errors:?}");
    }).await;
}
async fn cancel_then_next_default(name:&str,cancel:&str,work:&str,stop:&str,signal_start:bool) {
    cancel_then_next_selected(name,cancel,work,stop,signal_start,false).await;
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_hotkey_completed_release(){cancel_then_next_default("daemon::tests::history_contracts::history_cancel_next_hotkey_completed_release","hotkey","completed","release",false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_hotkey_gated_release(){cancel_then_next_default("daemon::tests::history_contracts::history_cancel_next_hotkey_gated_release","hotkey","gated","release",false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_hotkey_gated_toggle(){cancel_then_next_default("daemon::tests::history_contracts::history_cancel_next_hotkey_gated_toggle","hotkey","gated","toggle",false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_hotkey_gated_timeout(){cancel_then_next_default("daemon::tests::history_contracts::history_cancel_next_hotkey_gated_timeout","hotkey","gated","timeout",false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_hotkey_gated_external(){cancel_then_next_default("daemon::tests::history_contracts::history_cancel_next_hotkey_gated_external","hotkey","gated","external",false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_hotkey_late_error(){cancel_then_next_default("daemon::tests::history_contracts::history_cancel_next_hotkey_late_error","hotkey","gated-error","release",false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_hotkey_external_start(){cancel_then_next_default("daemon::tests::history_contracts::history_cancel_next_hotkey_external_start","hotkey","gated","external",true).await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_file_completed_control(){cancel_then_next_default("daemon::tests::history_contracts::history_cancel_next_file_completed_control","file","completed","release",false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_file_gated_control(){cancel_then_next_default("daemon::tests::history_contracts::history_cancel_next_file_gated_control","file","gated","release",false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_file_late_error_control(){cancel_then_next_default("daemon::tests::history_contracts::history_cancel_next_file_late_error_control","file","gated-error","release",false).await;}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_named_hotkey_late_success(){cancel_then_next_selected("daemon::tests::history_contracts::history_cancel_next_named_hotkey_late_success","hotkey","gated","release",false,true).await;}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_named_hotkey_late_error(){cancel_then_next_selected("daemon::tests::history_contracts::history_cancel_next_named_hotkey_late_error","hotkey","gated-error","release",false,true).await;}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_named_file_late_success(){cancel_then_next_selected("daemon::tests::history_contracts::history_cancel_next_named_file_late_success","file","gated","release",false,true).await;}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn history_cancel_next_named_file_late_error(){cancel_then_next_selected("daemon::tests::history_contracts::history_cancel_next_named_file_late_error","file","gated-error","release",false,true).await;}
