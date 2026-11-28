// Real Whisper inference behind a transparent task barrier, not a provider mock.
struct ObservedEagerWhisper {
    base: Arc<dyn Transcriber>,
    calls: std::sync::Mutex<Vec<(Vec<f32>, String)>>,
    started: std::sync::mpsc::Sender<()>,
    gate: (std::sync::Mutex<bool>, std::sync::Condvar),
}
impl Transcriber for ObservedEagerWhisper {
    fn transcribe(&self, samples: &[f32]) -> std::result::Result<String, crate::error::TranscribeError> {
        let _ = self.started.send(());
        let (lock, changed) = &self.gate;
        let mut released = lock.lock().unwrap();
        while !*released { released = changed.wait(released).unwrap(); }
        drop(released);
        let text = self.base.transcribe(samples)?;
        self.calls.lock().unwrap().push((samples.to_vec(), text.clone()));
        Ok(text)
    }
    fn runtime_metadata(&self) -> crate::transcribe::RuntimeMetadata { self.base.runtime_metadata() }
    fn last_detected_language(&self) -> Option<String> { self.base.last_detected_language() }
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore = "Requires approved pinned NAS model"]
async fn history_real_whisper_inflight_eager_chunks_preserve_selected_fidelity() {
    const NAME: &str = "daemon::tests::history_contracts::history_real_whisper_inflight_eager_chunks_preserve_selected_fidelity";
    if !enter_real_fidelity_case(NAME) { return; }
    let root = tempfile::tempdir().unwrap();
    let mut daemon = fidelity_daemon(root.path());
    daemon.config.whisper.eager_processing = true;
    daemon.config.whisper.eager_chunk_secs = 2.0;
    let samples = fidelity_samples("speech_long.wav");
    let base: Arc<dyn Transcriber> = crate::transcribe::create_transcriber(&daemon.config).unwrap().into();
    let (started, receiver) = std::sync::mpsc::channel();
    let observed = Arc::new(ObservedEagerWhisper { base, calls: Default::default(), started, gate: (std::sync::Mutex::new(false), std::sync::Condvar::new()) });
    let selected: Arc<dyn Transcriber> = observed.clone();
    let mut state = State::EagerRecording { started_at: std::time::Instant::now(), model_override: None, accumulated_audio: samples.clone(), chunks_sent: 0, chunk_results: Vec::new(), tasks_in_flight: 0 };
    daemon.begin_osd_session(&state);
    let (mut sent, mut in_flight) = (0,0);
    let spawned = daemon.process_eager_chunks(&samples, &mut sent, &mut in_flight, &selected);
    assert!(spawned > 0);
    tokio::task::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(10)).unwrap()).await.unwrap();
    assert!(daemon.eager_chunk_tasks.iter().all(|(_,task)| !task.is_finished()), "Barrier must establish actual in-flight tasks before stop");
    if let State::EagerRecording { chunks_sent, tasks_in_flight, .. } = &mut state { *chunks_sent = sent; *tasks_in_flight = in_flight; }
    *observed.gate.0.lock().unwrap() = true;
    observed.gate.1.notify_all();
    let mut eager_selected = Some(selected);
    tokio::time::timeout(Duration::from_secs(120), daemon.stop_active_recording(&mut state, &mut None, &mut None, &mut eager_selected)).await.unwrap();
    assert!(daemon.eager_chunk_tasks.is_empty());
    assert!(eager_selected.is_none());
    let entries = wait_for_entries(root.path(),1).await;
    let entry = &entries[0];
    let cfg = EagerConfig::from_whisper_config(&daemon.config.whisper);
    let boundaries = eager::chunk_boundaries(&samples,&cfg);
    let calls = observed.calls.lock().unwrap();
    let mut expected = Vec::new();
    let mut lengths = Vec::new();
    for i in 0..sent {
        let (start,end) = eager::chunk_range(&boundaries,i).unwrap();
        let (_,raw) = calls.iter().find(|(audio,_)| audio.as_slice()==&samples[start..end]).expect("Every chunk must use the real engine on its exact original samples");
        expected.push(ChunkResult { text: raw.clone(), chunk_index:i });
        lengths.push(end-start);
    }
    let tail_start = eager::tail_start(&boundaries,sent);
    if samples.len()-tail_start>=4800 {
        let (_,raw)=calls.iter().find(|(audio,_)| audio.as_slice()==&samples[tail_start..]).expect("Tail must use the same selected real engine");
        expected.push(ChunkResult { text:raw.clone(),chunk_index:sent });lengths.push(samples.len()-tail_start);
    }
    assert_eq!(calls.len(),expected.len());
    let raw=eager::combine_chunk_results(expected);
    let processed=daemon.text_processor.process(&raw);
    let measured=observed.runtime_metadata();
    let evidence=serde_json::json!({"kind":"ACTUAL_REAL_WHISPER_EAGER_INFLIGHT_CHUNKS_NOT_REFERENCE_ACCURACY","spawned":spawned,"chunk_and_tail_samples":lengths,"actual_chunk_texts":calls.iter().map(|(_,text)|text).collect::<Vec<_>>(),"canonical_combined_raw":raw,"processed":processed,"measured":measured,"entry":entry,"scope":"Existing process_eager_chunks + real selected engine behind test-only task barrier + stop_active_recording waits real chunks/tail + common completion/file/Recorder. No live capture/full daemon loop or reference-accuracy claim."});
    write_evidence("real-eager-inflight.json",serde_json::to_vec_pretty(&evidence).unwrap());
    println!("REAL_EAGER_INFLIGHT_OBSERVATION {evidence}");
    assert_eq!(entries.len(),1);
    assert_eq!(entry.raw_text,raw);
    assert_eq!(entry.text,processed);
    assert_eq!(std::fs::read_to_string(root.path().join("transcript.txt")).unwrap().trim_end(),processed);
    assert_eq!(entry.audio_ms,samples.len() as u64/16);
    assert_eq!(entry.model,FIDELITY_MODEL);
    assert_eq!(entry.backend.as_deref(),Some("cpu"));
    assert_eq!(entry.model_load_ms,measured.model_load_ms);
    assert!(entry.transcribe_ms.is_some());
    assert_eq!(entry.outcome,crate::history::Outcome::Delivered);
    let store=crate::history::HistoryStore::at(root.path().join("history"));
    assert_eq!(store.find(&entry.id).unwrap().as_ref(),Some(entry));
    let day=std::fs::read_dir(root.path().join("history")).unwrap().map(|e|e.unwrap().path()).find(|p|p.extension().is_some_and(|e|e=="jsonl")).unwrap();
    let day=std::fs::read(day).unwrap();
    assert_eq!(serde_json::from_slice::<crate::history::HistoryEntry>(&day).unwrap(),*entry);
    write_evidence("real-eager-inflight.jsonl",day);
    println!("REAL_EAGER_INFLIGHT_ASSERTIONS_PASSED");
}
