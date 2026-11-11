//! Meeting transcription mode
//!
//! Provides continuous meeting transcription with chunked processing,
//! speaker attribution, and export capabilities.
//!
//! Enables transcription of longer meetings (up to 3 hours) with
//! automatic chunking and speaker separation.
//!
//! # Architecture
//!
//! ```text
//! Mic + Loopback → ChunkProcessor → VAD → Transcription → Storage
//!                                           ↓
//!                                   Diarization (Phase 3)
//! ```
//!
//! # Phases
//!
//! - **Phase 1 (v0.5.0):** Basic meeting mode with chunked processing
//! - **Phase 2 (v0.5.1):** Dual audio + simple You/Remote attribution
//! - **Phase 3 (v0.5.2):** ML-based speaker diarization
//! - **Phase 4 (v0.6.0):** Remote server sync for corporate deployments
//! - **Phase 5 (v0.6.1):** AI summarization with action items

pub mod chunk;
pub mod data;
pub mod diarization;
pub mod events;
pub mod export;
pub mod state;
pub mod storage;
pub mod summary;

pub use chunk::{ChunkBuffer, ChunkConfig, ChunkProcessor, ProcessedChunk, VoiceActivityDetector};
pub use data::{
    ActionItem, AudioSource, MeetingData, MeetingId, MeetingMetadata, MeetingStatus,
    MeetingSummary, Transcript, TranscriptSegment,
};
pub use events::{DiarizationInfo, MeetingEventHub, MeetingHudState};
pub use export::{export_meeting, export_meeting_to_file, ExportFormat, ExportOptions};
pub use state::{ChunkState, MeetingState};
pub use storage::{MeetingStorage, StorageConfig, StorageError};

use crate::error::{MeetingError, Result};
use crate::output::post_process::PostProcessor;
use crate::transcribe::{self, Transcriber};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Meeting daemon configuration
#[derive(Debug, Clone)]
pub struct MeetingConfig {
    /// Enable meeting mode
    pub enabled: bool,
    /// Duration of each audio chunk in seconds
    pub chunk_duration_secs: u32,
    /// Storage configuration
    pub storage: StorageConfig,
    /// Whether to retain raw audio files
    pub retain_audio: bool,
    /// Maximum meeting duration in minutes (0 = unlimited)
    pub max_duration_mins: u32,
    /// RMS threshold for meeting chunk voice activity detection
    pub vad_threshold: f32,
    /// Diarization configuration (None = disabled)
    pub diarization: Option<diarization::DiarizationConfig>,
}

impl Default for MeetingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            chunk_duration_secs: 30,
            storage: StorageConfig::default(),
            retain_audio: false,
            max_duration_mins: 180,
            vad_threshold: 0.01,
            diarization: None,
        }
    }
}

/// Events from the meeting daemon
#[derive(Debug)]
pub enum MeetingEvent {
    /// Meeting started
    Started {
        meeting_id: MeetingId,
        title: Option<String>,
        started_at: chrono::DateTime<chrono::Utc>,
        diarization_enabled: bool,
        diarization_backend: Option<String>,
    },
    /// Chunk processed
    ChunkProcessed {
        chunk_id: u32,
        segments: Vec<TranscriptSegment>,
    },
    /// Segments already published under speaker `from` are now `to`, as
    /// when hybrid diarization numbers the remote side on hearing a second
    /// voice
    SpeakerRenamed { from: String, to: String },
    /// A segment already published changed speaker, as when the user moves
    /// a line to someone else with `meeting reassign`
    SegmentUpdated { segment: TranscriptSegment },
    /// Meeting paused
    Paused,
    /// Meeting resumed
    Resumed,
    /// Meeting stopped
    Stopped { meeting_id: MeetingId },
    /// Error occurred
    Error(String),
}

/// Meeting daemon for continuous transcription
pub struct MeetingDaemon {
    config: MeetingConfig,
    state: MeetingState,
    storage: MeetingStorage,
    current_meeting: Option<MeetingData>,
    transcriber: Option<Arc<dyn Transcriber>>,
    diarizer: Option<Box<dyn diarization::Diarizer>>,
    engine_name: String,
    event_tx: mpsc::Sender<MeetingEvent>,
    post_processor: Option<PostProcessor>,
    /// Previous chunk's post-processed text, tracked per audio source
    /// so mic and loopback contexts don't bleed into each other
    last_chunk_text: HashMap<AudioSource, String>,
    /// Cumulative audio duration consumed per source, in milliseconds.
    /// Used to compute per-source start offsets so mic and loopback
    /// timelines stay anchored to real wall-clock elapsed time instead
    /// of being pushed forward by the other source's segments.
    source_offsets: HashMap<AudioSource, u64>,
    /// ID for the meeting's next transcript segment, unique across chunks
    /// and tracks so consumers (the HUD, edits) can address a segment.
    next_segment_id: u32,
}

impl MeetingDaemon {
    /// Create a new meeting daemon
    pub fn new(
        config: MeetingConfig,
        app_config: &crate::config::Config,
        event_tx: mpsc::Sender<MeetingEvent>,
    ) -> Result<Self> {
        let storage = MeetingStorage::open(config.storage.clone())
            .map_err(|e| MeetingError::Storage(e.to_string()))?;

        let meeting_app_config = app_config.with_meeting_mode_overrides();
        let transcriber: Arc<dyn Transcriber> =
            Arc::from(transcribe::create_transcriber(&meeting_app_config)?);
        let engine_name = format!("{:?}", meeting_app_config.engine).to_lowercase();

        let post_processor = app_config.output.active_post_process().map(|cfg| {
            tracing::info!(
                "Meeting post-processing enabled: command={:?}, timeout={}ms",
                cfg.command,
                cfg.timeout_ms
            );
            PostProcessor::new(cfg)
        });

        // Create diarizer if configured
        let diarizer = config.diarization.as_ref().and_then(|diar_config| {
            if diar_config.enabled {
                let d = diarization::create_diarizer(diar_config);
                tracing::info!("Meeting diarization enabled: {}", d.name());
                Some(d)
            } else {
                None
            }
        });

        Ok(Self {
            config,
            state: MeetingState::Idle,
            storage,
            current_meeting: None,
            transcriber: Some(transcriber),
            diarizer,
            engine_name,
            event_tx,
            post_processor,
            last_chunk_text: HashMap::new(),
            source_offsets: HashMap::new(),
            next_segment_id: 0,
        })
    }

    /// A daemon around a given transcriber, for tests that drive chunks
    /// without loading a model.
    #[cfg(test)]
    fn for_test(
        config: MeetingConfig,
        transcriber: Arc<dyn Transcriber>,
        diarizer: Option<Box<dyn diarization::Diarizer>>,
        event_tx: mpsc::Sender<MeetingEvent>,
    ) -> Self {
        Self {
            storage: MeetingStorage::open(config.storage.clone()).unwrap(),
            config,
            state: MeetingState::Idle,
            current_meeting: None,
            transcriber: Some(transcriber),
            diarizer,
            engine_name: "test".to_string(),
            event_tx,
            post_processor: None,
            last_chunk_text: HashMap::new(),
            source_offsets: HashMap::new(),
            next_segment_id: 0,
        }
    }

    /// Start a new meeting
    pub async fn start(&mut self, title: Option<String>) -> Result<MeetingId> {
        if !self.state.is_idle() {
            return Err(MeetingError::AlreadyInProgress.into());
        }

        // Create meeting
        let mut meeting = MeetingData::new(title);
        meeting.metadata.model = Some(self.engine_name.clone());

        // Create storage directory
        let storage_path = self
            .storage
            .create_meeting(&meeting.metadata)
            .map_err(|e| MeetingError::Storage(e.to_string()))?;
        meeting.metadata.storage_path = Some(storage_path);

        let meeting_id = meeting.metadata.id;
        let title = meeting.metadata.title.clone();
        let started_at = meeting.metadata.started_at;
        let (diarization_enabled, diarization_backend) = match &self.config.diarization {
            Some(cfg) => (cfg.enabled, Some(cfg.backend.clone())),
            None => (false, None),
        };
        self.current_meeting = Some(meeting);
        self.state = MeetingState::start();
        self.next_segment_id = 0;

        let _ = self
            .event_tx
            .send(MeetingEvent::Started {
                meeting_id,
                title,
                started_at,
                diarization_enabled,
                diarization_backend,
            })
            .await;
        tracing::info!("Meeting started: {}", meeting_id);

        Ok(meeting_id)
    }

    /// Pause the current meeting
    pub async fn pause(&mut self) -> Result<()> {
        if !self.state.is_active() {
            return Err(MeetingError::NotActive.into());
        }

        self.state = std::mem::take(&mut self.state).pause();
        let _ = self.event_tx.send(MeetingEvent::Paused).await;
        tracing::info!("Meeting paused");

        Ok(())
    }

    /// Resume a paused meeting
    pub async fn resume(&mut self) -> Result<()> {
        if !self.state.is_paused() {
            return Err(MeetingError::NotPaused.into());
        }

        self.state = std::mem::take(&mut self.state).resume();
        let _ = self.event_tx.send(MeetingEvent::Resumed).await;
        tracing::info!("Meeting resumed");

        Ok(())
    }

    /// Stop the current meeting
    pub async fn stop(&mut self) -> Result<MeetingId> {
        if self.state.is_idle() {
            return Err(MeetingError::NotInProgress.into());
        }

        self.state = std::mem::take(&mut self.state).stop();
        self.last_chunk_text.clear();
        self.source_offsets.clear();

        // Finalize meeting
        if let Some(ref mut meeting) = self.current_meeting {
            meeting.complete();
            meeting.metadata.chunk_count = meeting.transcript.total_chunks;

            // Save transcript
            self.storage
                .save_transcript(&meeting.metadata.id, &meeting.transcript)
                .map_err(|e| MeetingError::Storage(e.to_string()))?;

            // Update metadata
            self.storage
                .update_meeting(&meeting.metadata)
                .map_err(|e| MeetingError::Storage(e.to_string()))?;
        }

        let meeting_id = self
            .current_meeting
            .as_ref()
            .map(|m| m.metadata.id)
            .unwrap_or_default();

        let _ = self
            .event_tx
            .send(MeetingEvent::Stopped { meeting_id })
            .await;
        tracing::info!("Meeting stopped: {}", meeting_id);

        // Clean up
        self.state = std::mem::take(&mut self.state).finalize();
        self.current_meeting = None;

        Ok(meeting_id)
    }

    /// Get current meeting state
    pub fn state(&self) -> &MeetingState {
        &self.state
    }

    /// Get current meeting ID if one is active
    pub fn current_meeting_id(&self) -> Option<MeetingId> {
        self.current_meeting.as_ref().map(|m| m.metadata.id)
    }

    /// Align all per-source timestamp offsets to the furthest-advanced source.
    ///
    /// Call this after dispatching the per-iteration chunks for all sources so
    /// any source that received a short or empty chunk (e.g. loopback monitor
    /// lagging at startup) catches up to wall-clock before the next iteration.
    pub fn sync_source_offsets(&mut self) {
        let max_offset = self.source_offsets.values().copied().max().unwrap_or(0);
        for offset in self.source_offsets.values_mut() {
            *offset = max_offset;
        }
    }

    /// Get mutable access to current meeting data (for dedup, etc.)
    pub fn current_meeting_mut(&mut self) -> Option<&mut MeetingData> {
        self.current_meeting.as_mut()
    }

    /// Process a chunk of audio
    pub async fn process_chunk(
        &mut self,
        samples: Vec<f32>,
    ) -> Result<Option<Vec<TranscriptSegment>>> {
        self.process_chunk_with_source(samples, AudioSource::Microphone)
            .await
    }

    /// Process a chunk of audio with a specific source label
    pub async fn process_chunk_with_source(
        &mut self,
        samples: Vec<f32>,
        source: AudioSource,
    ) -> Result<Option<Vec<TranscriptSegment>>> {
        let Some((chunk_id, segments)) = self.transcribe_track(samples, source).await? else {
            return Ok(None);
        };
        self.persist_transcript();
        self.emit_chunk(chunk_id, segments.clone()).await;
        Ok(Some(segments))
    }

    /// Process one chunk of each track covering the same stretch of time.
    ///
    /// The microphone picks up the remote side through the speakers, so
    /// once both tracks are transcribed, phrases on the microphone track
    /// that the loopback track also holds are stripped as echo. Only then
    /// are the chunks published, so a live consumer never sees the echo as
    /// something you said. A track that fails to transcribe is logged and
    /// the other still processed.
    pub async fn process_pair(&mut self, mic: Vec<f32>, loopback: Vec<f32>) {
        let mut done = Vec::new();
        for (samples, source) in [
            (mic, AudioSource::Microphone),
            (loopback, AudioSource::Loopback),
        ] {
            if samples.is_empty() {
                continue;
            }
            match self.transcribe_track(samples, source).await {
                Ok(Some(chunk)) => done.push(chunk),
                Ok(None) => {}
                Err(e) => tracing::error!("Error processing {} chunk: {}", source, e),
            }
        }

        // Keep both timelines on wall-clock even if one track came up short
        // or was skipped this round (PR #330).
        self.sync_source_offsets();

        let had_loopback = done
            .iter()
            .flat_map(|(_, segments)| segments)
            .any(|s| s.source == AudioSource::Loopback);
        if had_loopback {
            if let Some(ref mut meeting) = self.current_meeting {
                let removed = meeting.transcript.dedup_bleed_through();
                if removed > 0 {
                    tracing::info!("Removed {} bleed-through word(s) via dedup", removed);
                }
                // Publish what is left of each segment after the dedup.
                let kept = &meeting.transcript.segments;
                for (_, segments) in &mut done {
                    segments.retain_mut(|seg| match kept.iter().find(|k| k.id == seg.id) {
                        Some(k) => {
                            seg.text = k.text.clone();
                            true
                        }
                        None => false,
                    });
                }
            }
        }

        self.persist_transcript();
        for (chunk_id, segments) in done {
            self.emit_chunk(chunk_id, segments).await;
        }
    }

    /// Rename a speaker in the segments already transcribed, carry over any
    /// name `meeting label` gave it, and tell the daemon so the HUD's
    /// backlog follows.
    async fn rename_speaker(&mut self, from: &str, to: &str) {
        let Some(ref mut meeting) = self.current_meeting else {
            return;
        };
        let renamed = meeting.transcript.rename_speaker(from, to);
        if renamed == 0 {
            return;
        }
        tracing::info!(
            "Renamed speaker {} to {} in {} segment(s)",
            from,
            to,
            renamed
        );
        if let Err(e) = self
            .storage
            .rename_speaker_key(&meeting.metadata.id, from, to)
        {
            tracing::warn!(
                "Failed to carry speaker label from {} to {}: {}",
                from,
                to,
                e
            );
        }
        let _ = self
            .event_tx
            .send(MeetingEvent::SpeakerRenamed {
                from: from.to_string(),
                to: to.to_string(),
            })
            .await;
    }

    /// Re-read the speaker overrides `meeting reassign` stored, move the
    /// affected lines in the running meeting, save, and publish each line
    /// that changed. Returns how many did.
    pub async fn apply_speaker_overrides(&mut self) -> usize {
        let Some(ref mut meeting) = self.current_meeting else {
            return 0;
        };
        let id = meeting.metadata.id;
        let (overrides, labels) = match (
            self.storage.get_speaker_overrides(&id),
            self.storage.get_speaker_labels(&id),
        ) {
            (Ok(overrides), Ok(labels)) => (overrides, labels),
            (Err(e), _) | (_, Err(e)) => {
                tracing::warn!("Failed to read speaker overrides: {}", e);
                return 0;
            }
        };

        let before = meeting.transcript.segments.clone();
        meeting.transcript.apply_speaker_overrides(&overrides);
        // Only the moved lines are published; names they now carry are
        // applied to those copies alone.
        let changed: Vec<TranscriptSegment> = meeting
            .transcript
            .segments
            .iter()
            .zip(&before)
            .filter(|(now, was)| {
                now.speaker_id != was.speaker_id || now.reassigned_from != was.reassigned_from
            })
            .map(|(now, _)| {
                let mut segment = now.clone();
                segment.speaker_label = labels.get(&segment.speaker_key()).cloned();
                segment
            })
            .collect();

        if changed.is_empty() {
            return 0;
        }
        self.persist_transcript();
        for segment in &changed {
            tracing::info!(
                "Segment {} now belongs to {}",
                segment.id,
                segment.speaker_display()
            );
            let _ = self
                .event_tx
                .send(MeetingEvent::SegmentUpdated {
                    segment: segment.clone(),
                })
                .await;
        }
        changed.len()
    }

    /// Persist after every chunk, not just on stop(): a crash mid-meeting
    /// would otherwise lose the whole transcript, and `meeting show --json`
    /// would report nothing while a meeting is in progress. One small JSON
    /// write per chunk is negligible next to the transcription.
    fn persist_transcript(&self) {
        if let Some(ref meeting) = self.current_meeting {
            if let Err(e) = self
                .storage
                .save_transcript(&meeting.metadata.id, &meeting.transcript)
            {
                tracing::warn!("Failed to persist transcript incrementally: {}", e);
            }
        }
    }

    async fn emit_chunk(&self, chunk_id: u32, segments: Vec<TranscriptSegment>) {
        let _ = self
            .event_tx
            .send(MeetingEvent::ChunkProcessed { chunk_id, segments })
            .await;
    }

    /// Transcribe, post-process, diarize and label one chunk of one track
    /// and add it to the transcript. Returns the chunk ID and its segments,
    /// or `None` when no meeting is recording.
    async fn transcribe_track(
        &mut self,
        samples: Vec<f32>,
        source: AudioSource,
    ) -> Result<Option<(u32, Vec<TranscriptSegment>)>> {
        if !self.state.is_active() {
            return Ok(None);
        }

        let Some(ref transcriber) = self.transcriber else {
            return Err(MeetingError::TranscriberNotInitialized.into());
        };

        let chunk_id = self.state.chunks_processed();
        let chunk_config = ChunkConfig {
            chunk_duration_secs: self.config.chunk_duration_secs,
            vad_threshold: self.config.vad_threshold,
            ..Default::default()
        };

        // Start offset is tracked per source: each source has its own wall-clock
        // timeline. Deriving this from transcript.duration_ms() would conflate
        // mic and loopback, pushing every new chunk past the other source's end
        // and roughly doubling apparent meeting length on dual-track captures.
        let start_offset_ms = *self.source_offsets.entry(source).or_insert(0);

        // Advance the per-source offset up front, based on the input sample
        // count, so the offset tracks wall-clock even when transcription errors
        // or VAD skips this chunk (the caller has already drained these samples
        // from its buffer, so the time has elapsed regardless).
        let audio_duration_ms =
            (samples.len() as f64 / chunk_config.sample_rate as f64 * 1000.0) as u64;
        if let Some(offset) = self.source_offsets.get_mut(&source) {
            *offset += audio_duration_ms;
        }

        let mut processor = ChunkProcessor::new(chunk_config, transcriber.clone())
            .with_first_segment_id(self.next_segment_id);
        let mut buffer = processor.new_buffer(chunk_id, source, start_offset_ms);
        buffer.add_samples(&samples);

        let mut result = processor
            .process_chunk(buffer)
            .map_err(crate::error::VoxtypeError::Transcribe)?;
        self.next_segment_id = processor.next_segment_id();

        // Post-process segment text if configured
        if let Some(ref post_processor) = self.post_processor {
            let context = self.last_chunk_text.get(&source).cloned();
            for segment in &mut result.segments {
                if !segment.text.is_empty() {
                    segment.text = post_processor
                        .process_with_context(&segment.text, context.as_deref())
                        .await;
                }
            }
            // Update context for next chunk (per source), using the last non-empty
            // segment to avoid losing useful context when a chunk ends with silence
            if let Some(last_seg) = result.segments.iter().rfind(|s| !s.text.is_empty()) {
                self.last_chunk_text.insert(source, last_seg.text.clone());
            }
        }

        // Run diarization on the transcribed segments
        if let Some(ref diarizer) = self.diarizer {
            if !result.segments.is_empty() {
                let diarized =
                    diarizer.diarize(&samples, source, start_offset_ms, &result.segments);
                for (seg, diar) in result.segments.iter_mut().zip(diarized.iter()) {
                    seg.speaker_id = Some(diar.speaker.display_name());
                    seg.confidence = Some(diar.confidence);
                }
            }
            for (from, to) in diarizer.take_speaker_renames() {
                self.rename_speaker(&from, &to).await;
            }
        }

        // Names given with `meeting label` mid-meeting, so the live feed
        // shows them too. A failed lookup only costs the names.
        if let Some(ref meeting) = self.current_meeting {
            if let Ok(labels) = self.storage.get_speaker_labels(&meeting.metadata.id) {
                for segment in &mut result.segments {
                    if let Some(label) = labels.get(&segment.speaker_key()) {
                        segment.speaker_label = Some(label.clone());
                    }
                }
            }
        }

        // Add segments to transcript
        if let Some(ref mut meeting) = self.current_meeting {
            for segment in &result.segments {
                meeting.transcript.add_segment(segment.clone());
            }
            meeting.transcript.total_chunks = chunk_id + 1;
        }

        // Advance state
        self.state = std::mem::take(&mut self.state).next_chunk();

        Ok(Some((chunk_id, result.segments)))
    }

    /// Get storage access
    pub fn storage(&self) -> &MeetingStorage {
        &self.storage
    }
}

/// List meetings from storage
pub fn list_meetings(
    config: &MeetingConfig,
    limit: Option<u32>,
) -> std::result::Result<Vec<MeetingMetadata>, StorageError> {
    let storage = MeetingStorage::open(config.storage.clone())?;
    storage.list_meetings(limit)
}

/// Get a meeting by ID (or "latest")
pub fn get_meeting(
    config: &MeetingConfig,
    id_str: &str,
) -> std::result::Result<MeetingData, StorageError> {
    let storage = MeetingStorage::open(config.storage.clone())?;
    let id = storage.resolve_meeting_id(id_str)?;
    storage.load_meeting_data(&id)
}

/// Export a meeting
pub fn export_meeting_by_id(
    config: &MeetingConfig,
    id_str: &str,
    format: ExportFormat,
    options: &ExportOptions,
) -> std::result::Result<String, StorageError> {
    let meeting = get_meeting(config, id_str)?;
    export_meeting(&meeting, format, options)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::TranscribeError;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Numbers each utterance it is given: "u0", "u1", ...
    struct CountingTranscriber(AtomicU32);

    impl Transcriber for CountingTranscriber {
        fn short_clips_are_cheap(&self) -> bool {
            true
        }

        fn transcribe(&self, samples: &[f32]) -> std::result::Result<String, TranscribeError> {
            if samples.iter().all(|s| s.abs() < 0.05) {
                return Ok(String::new());
            }
            Ok(format!("u{}", self.0.fetch_add(1, Ordering::SeqCst)))
        }
    }

    /// `secs` of audio with a 440 Hz tone over each (start, end) span.
    fn track(secs: f32, spans: &[(f32, f32)]) -> Vec<f32> {
        let mut samples = vec![0.0; (secs * 16000.0) as usize];
        for &(start, end) in spans {
            let span = (start * 16000.0) as usize..(end * 16000.0) as usize;
            for (i, s) in samples[span.clone()].iter_mut().enumerate() {
                let t = (span.start + i) as f32 / 16000.0;
                *s = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            }
        }
        samples
    }

    struct Harness {
        daemon: MeetingDaemon,
        config: MeetingConfig,
        events: mpsc::Receiver<MeetingEvent>,
        _dir: tempfile::TempDir,
    }

    async fn harness() -> Harness {
        let dir = tempfile::TempDir::new().unwrap();
        let config = MeetingConfig {
            enabled: true,
            chunk_duration_secs: 30,
            storage: StorageConfig {
                storage_path: dir.path().to_path_buf(),
                retain_audio: false,
                max_meetings: 0,
            },
            ..Default::default()
        };
        let (tx, events) = mpsc::channel(64);
        let mut daemon = MeetingDaemon::for_test(
            config.clone(),
            Arc::new(CountingTranscriber(AtomicU32::new(0))),
            Some(Box::new(diarization::simple::SimpleDiarizer::new())),
            tx,
        );
        daemon.start(Some("Test".to_string())).await.unwrap();
        Harness {
            daemon,
            config,
            events,
            _dir: dir,
        }
    }

    impl Harness {
        /// One chunk of each track, the way the daemon feeds them.
        async fn pair(&mut self, mic: &[(f32, f32)], loopback: &[(f32, f32)]) {
            self.daemon
                .process_pair(track(30.0, mic), track(30.0, loopback))
                .await;
        }

        fn saved(&self) -> MeetingData {
            get_meeting(&self.config, "latest").unwrap()
        }
    }

    #[tokio::test]
    async fn transcript_on_disk_grows_with_each_chunk() {
        let mut h = harness().await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        assert_eq!(h.saved().transcript.segments.len(), 2);
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        assert_eq!(h.saved().transcript.segments.len(), 4);

        let mut chunk_events = 0;
        while let Ok(event) = h.events.try_recv() {
            if let MeetingEvent::ChunkProcessed { segments, .. } = event {
                assert_eq!(segments.len(), 1);
                chunk_events += 1;
            }
        }
        assert_eq!(chunk_events, 4);
    }

    #[tokio::test]
    async fn segments_interleave_and_ids_are_unique() {
        let mut h = harness().await;
        h.pair(&[(2.0, 4.0), (12.0, 14.0)], &[(6.0, 9.0), (20.0, 22.0)])
            .await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;

        let saved = h.saved();
        let order: Vec<(String, u64)> = saved
            .transcript
            .segments
            .iter()
            .map(|s| (s.speaker_display(), s.start_ms / 1000))
            .collect();
        assert_eq!(
            order,
            vec![
                ("You".to_string(), 1),
                ("Remote".to_string(), 5),
                ("You".to_string(), 11),
                ("Remote".to_string(), 19),
                ("You".to_string(), 30),
                ("Remote".to_string(), 34),
            ]
        );

        let mut ids: Vec<u32> = saved.transcript.segments.iter().map(|s| s.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(
            ids.len(),
            6,
            "segment IDs repeat: {:?}",
            saved.transcript.segments
        );
    }

    /// Says one line per loudness: the remote voice at 0.5, ours at 0.3.
    struct TwoVoices;

    impl Transcriber for TwoVoices {
        fn short_clips_are_cheap(&self) -> bool {
            true
        }

        fn transcribe(&self, samples: &[f32]) -> std::result::Result<String, TranscribeError> {
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            Ok(if peak > 0.4 {
                "we should ship the release on friday".to_string()
            } else if peak > 0.2 {
                "agreed".to_string()
            } else {
                String::new()
            })
        }
    }

    /// The remote sentence reaches the microphone through the speakers.
    /// It must not be published, live or on disk, as something You said.
    #[tokio::test]
    async fn echo_on_the_microphone_never_reaches_the_live_feed() {
        let mut h = harness().await;
        h.daemon.transcriber = Some(Arc::new(TwoVoices));

        // The echo, as loud as the remote, plus our own quieter "agreed".
        let mut mic = track(30.0, &[(5.0, 8.0)]);
        for (m, ours) in mic.iter_mut().zip(track(30.0, &[(10.0, 11.0)])) {
            *m += ours * 0.6;
        }
        let loopback = track(30.0, &[(5.0, 8.0)]);
        h.daemon.process_pair(mic, loopback).await;

        let mut live = Vec::new();
        while let Ok(event) = h.events.try_recv() {
            if let MeetingEvent::ChunkProcessed { segments, .. } = event {
                live.extend(segments);
            }
        }
        let said = |segs: &[TranscriptSegment]| {
            segs.iter()
                .map(|s| format!("{}: {}", s.speaker_display(), s.text))
                .collect::<Vec<_>>()
        };
        let expected = vec![
            "Remote: we should ship the release on friday".to_string(),
            "You: agreed".to_string(),
        ];
        let mut live_said = said(&live);
        live_said.sort();
        assert_eq!(live_said, expected);
        assert_eq!(said(&h.saved().transcript.segments), expected);
    }

    /// Clusters each loopback chunk as the next new voice: Auto(0), then
    /// Auto(1), standing in for ECAPA hearing a second remote speaker.
    struct NewVoicePerChunk(AtomicU32);

    impl diarization::Diarizer for NewVoicePerChunk {
        fn diarize(
            &self,
            _samples: &[f32],
            _source: AudioSource,
            _chunk_start_ms: u64,
            segments: &[TranscriptSegment],
        ) -> Vec<diarization::DiarizedSegment> {
            let voice = self.0.fetch_add(1, Ordering::SeqCst);
            segments
                .iter()
                .map(|seg| diarization::DiarizedSegment {
                    speaker: diarization::SpeakerId::Auto(voice),
                    start_ms: seg.start_ms,
                    end_ms: seg.end_ms,
                    text: seg.text.clone(),
                    confidence: 0.8,
                })
                .collect()
        }

        fn name(&self) -> &'static str {
            "new-voice-per-chunk"
        }
    }

    /// Hybrid diarization calls the one remote voice "Remote". When a
    /// second voice speaks, earlier segments become "Remote 1", on disk
    /// and on the live feed, and a name given to "Remote" moves with them.
    #[tokio::test]
    async fn a_second_remote_voice_renames_the_first_everywhere() {
        let mut h = harness().await;
        h.daemon.diarizer = Some(Box::new(diarization::hybrid::HybridDiarizer::new(
            Box::new(NewVoicePerChunk(AtomicU32::new(0))),
        )));

        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        cli.set_speaker_label(&id, "Remote", "Dana").unwrap();

        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;

        let mut events = Vec::new();
        while let Ok(event) = h.events.try_recv() {
            match event {
                MeetingEvent::SpeakerRenamed { from, to } => {
                    events.push(format!("rename {from} -> {to}"))
                }
                MeetingEvent::ChunkProcessed { segments, .. } => {
                    for s in segments {
                        events.push(s.speaker_key());
                    }
                }
                _ => {}
            }
        }
        // The rename reaches the live feed before the chunk that caused it.
        assert_eq!(
            events,
            vec![
                "You",
                "Remote",
                "rename Remote -> Remote 1",
                "You",
                "Remote 2"
            ]
        );

        cli.set_speaker_label(&id, "Remote 2", "Sam").unwrap();
        h.daemon.stop().await.unwrap();
        let speakers: Vec<(String, String)> = h
            .saved()
            .transcript
            .segments
            .iter()
            .map(|s| (s.speaker_key(), s.speaker_display()))
            .collect();
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        assert_eq!(
            speakers,
            pairs(&[
                ("You", "You"),
                ("Remote 1", "Dana"),
                ("You", "You"),
                ("Remote 2", "Sam"),
            ])
        );
    }

    fn drain(events: &mut mpsc::Receiver<MeetingEvent>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(event) = events.try_recv() {
            match event {
                MeetingEvent::SpeakerRenamed { from, to } => {
                    out.push(format!("rename {from} -> {to}"))
                }
                MeetingEvent::SegmentUpdated { segment } => out.push(format!(
                    "updated {} {} ({})",
                    segment.id,
                    segment.speaker_display(),
                    segment.speaker_key()
                )),
                MeetingEvent::ChunkProcessed { segments, .. } => out.extend(
                    segments
                        .iter()
                        .map(|s| format!("{} {}", s.id, s.speaker_key())),
                ),
                _ => {}
            }
        }
        out
    }

    /// `meeting reassign` mid-meeting: the daemon moves the line, publishes
    /// it, and neither the next chunk nor hybrid's rename moves it back.
    #[tokio::test]
    async fn a_line_reassigned_mid_meeting_stays_put() {
        let mut h = harness().await;
        h.daemon.diarizer = Some(Box::new(diarization::hybrid::HybridDiarizer::new(
            Box::new(NewVoicePerChunk(AtomicU32::new(0))),
        )));
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        drain(&mut h.events);

        // What the CLI does while the meeting records: store, don't rewrite.
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        cli.reassign_segment(&id, 1, "Casey", false).unwrap();
        assert_eq!(h.daemon.apply_speaker_overrides().await, 1);
        assert_eq!(drain(&mut h.events), vec!["updated 1 Casey (Person 1)"]);

        // A second remote voice renames Remote; the moved line stays Casey.
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        assert_eq!(
            drain(&mut h.events),
            vec!["rename Remote -> Remote 1", "2 You", "3 Remote 2"]
        );
        h.daemon.stop().await.unwrap();

        let saved = h.saved();
        let line = &saved.transcript.segments[1];
        assert_eq!(line.id, 1);
        assert_eq!(line.speaker_display(), "Casey");
        assert_eq!(line.reassigned_from.as_deref(), Some("Remote 1"));

        // --reset after stop gives it back to the renamed diarizer speaker.
        let back = cli.reset_segment_speaker(&id, 1, true).unwrap();
        assert_eq!(back.speaker_key(), "Remote 1");
    }

    #[tokio::test]
    async fn a_line_moved_onto_a_speaker_follows_its_rename() {
        let mut h = harness().await;
        h.daemon.diarizer = Some(Box::new(diarization::hybrid::HybridDiarizer::new(
            Box::new(NewVoicePerChunk(AtomicU32::new(0))),
        )));
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        // The mic line was really the remote voice.
        cli.reassign_segment(&id, 0, "Remote", false).unwrap();
        h.daemon.apply_speaker_overrides().await;

        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        h.daemon.stop().await.unwrap();

        let keys: Vec<String> = h
            .saved()
            .transcript
            .segments
            .iter()
            .map(|s| s.speaker_key())
            .collect();
        assert_eq!(keys, vec!["Remote 1", "Remote 1", "You", "Remote 2"]);
    }

    #[tokio::test]
    async fn label_given_mid_meeting_survives_stop() {
        let mut h = harness().await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;

        // What `voxtype meeting label latest Remote Dana` does, from its own process.
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        cli.set_speaker_label(&id, "Remote", "Dana").unwrap();

        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        let mut live = Vec::new();
        while let Ok(event) = h.events.try_recv() {
            if let MeetingEvent::ChunkProcessed { segments, .. } = event {
                live.extend(segments);
            }
        }
        h.daemon.stop().await.unwrap();

        let speakers: Vec<String> = h
            .saved()
            .transcript
            .segments
            .iter()
            .map(|s| s.speaker_display())
            .collect();
        assert_eq!(speakers, vec!["You", "Dana", "You", "Dana"]);
        // Segments streamed after the label carry it too.
        assert_eq!(live.last().unwrap().speaker_display(), "Dana");
    }

    #[test]
    fn test_meeting_config_default() {
        let config = MeetingConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.chunk_duration_secs, 30);
        assert_eq!(config.max_duration_mins, 180);
        assert_eq!(config.vad_threshold, 0.01);
    }

    #[test]
    fn test_meeting_state_transitions() {
        let state = MeetingState::Idle;
        assert!(state.is_idle());

        let state = MeetingState::start();
        assert!(state.is_active());

        let state = state.pause();
        assert!(state.is_paused());

        let state = state.resume();
        assert!(state.is_active());

        let state = state.stop();
        assert!(state.is_finalizing());

        let state = state.finalize();
        assert!(state.is_idle());
    }
}
