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

pub mod assistant;
pub mod chunk;
pub mod data;
pub mod detect;
pub mod diarization;
pub mod echo;
pub mod events;
pub mod export;
pub mod hook;
pub mod import;
pub mod manifest;
pub mod meeting_recorder;
pub mod notes;
pub mod notice;
pub mod playback;
pub mod repeat;
pub mod retained_audio;
pub mod retranscribe;
pub mod start;
pub mod state;
pub mod status_notice;
pub mod storage;
pub mod summary;
pub mod title_labels;
pub mod view_presence;

pub use chunk::{ChunkBuffer, ChunkConfig, ChunkProcessor, ProcessedChunk, VoiceActivityDetector};
pub use data::{
    ActionItem, AudioSource, MeetingData, MeetingId, MeetingMetadata, MeetingStatus,
    MeetingSummary, SegmentAnnotation, Transcript, TranscriptSegment,
};
pub use events::{DiarizationInfo, MeetingEventHub, MeetingHudState};
pub use export::{export_meeting, export_meeting_to_file, ExportFormat, ExportOptions};
pub use state::{ChunkState, MeetingState};
pub use storage::{MeetingListEntry, MeetingStorage, StorageConfig, StorageError};

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
    /// The short-line echo rules (`[meeting.audio] echo_fuzzy`)
    pub echo_rules: echo::EchoRules,
    /// Name the speakers of a two-person meeting from its title
    /// (`meeting.auto_label_from_title`)
    pub auto_label_from_title: bool,
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
            echo_rules: echo::EchoRules::default(),
            auto_label_from_title: true,
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
    /// A segment already published was deleted with `meeting delete-line`
    SegmentDeleted { segment_id: u32 },
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
    /// The engine's model, for meeting.json
    model_name: Option<String>,
    /// Writes the meeting's audio into its folder when `retain_audio` is on
    audio: Option<retained_audio::AudioRetainer>,
    /// `post_meeting_command`, run once a stopped meeting's folder is final
    post_meeting: hook::PostMeetingSettings,
    /// Microphone lines the audio showed to be your own voice. They are
    /// judged once, with their chunk's audio, and kept out of the short-echo
    /// rules on every later dedup pass, which has no audio for them.
    own_voice_ids: std::collections::HashSet<u32>,
    /// Assistant settings for the summary written when a meeting stops
    assistant: Option<assistant::AssistantSettings>,
    /// The user, for naming speakers from a meeting's title; `None` when
    /// that is off or the user's name is unknown
    me: Option<title_labels::Me>,
    /// The names the running meeting's title gives its two sides
    title_labels: Option<title_labels::TitleLabels>,
}

impl MeetingDaemon {
    /// Create a new meeting daemon.
    ///
    /// `meeting_app_config` is the configuration the meeting transcribes
    /// with ([`start::meeting_transcription_config`]). `resident` is
    /// dictation's loaded transcriber when it serves that configuration
    /// ([`start::shares_dictation_transcriber`]); without one the meeting
    /// loads its own. Transcribers serialize their own calls, so a shared one
    /// takes dictation and meeting chunks in turn.
    pub fn new(
        config: MeetingConfig,
        meeting_app_config: &crate::config::Config,
        event_tx: mpsc::Sender<MeetingEvent>,
        resident: Option<Arc<dyn Transcriber>>,
    ) -> Result<Self> {
        let storage = MeetingStorage::open(config.storage.clone())
            .map_err(|e| MeetingError::Storage(e.to_string()))?;

        let transcriber: Arc<dyn Transcriber> = match resident {
            Some(transcriber) => {
                tracing::info!("Meeting transcribes with dictation's loaded model");
                transcriber
            }
            None => {
                let start = std::time::Instant::now();
                let transcriber = Arc::from(transcribe::create_transcriber(meeting_app_config)?);
                tracing::info!(
                    "Meeting model loaded in {:.2}s",
                    start.elapsed().as_secs_f32()
                );
                transcriber
            }
        };
        let engine_name = format!("{:?}", meeting_app_config.engine).to_lowercase();
        let model_name = Some(meeting_app_config.model_name().to_string());
        let app_config = meeting_app_config;

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

        // No setting names the user yet: the account's full name does.
        let me = config
            .auto_label_from_title
            .then(|| title_labels::Me::resolve(None))
            .flatten();

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
            model_name,
            audio: None,
            post_meeting: hook::PostMeetingSettings::from_config(app_config),
            own_voice_ids: Default::default(),
            assistant: Some(assistant::AssistantSettings::from_config(app_config)),
            me,
            title_labels: None,
        })
    }

    /// A daemon around a given transcriber, for tests that drive chunks
    /// without loading a model.
    #[cfg(test)]
    pub(crate) fn for_test(
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
            model_name: Some("test-model".to_string()),
            audio: None,
            post_meeting: hook::PostMeetingSettings::default(),
            own_voice_ids: Default::default(),
            assistant: None,
            me: None,
            title_labels: None,
        }
    }

    /// Start a new meeting
    pub async fn start(&mut self, title: Option<String>, url: Option<String>) -> Result<MeetingId> {
        self.start_at(title, url, None).await
    }

    /// Start a new meeting dated `started_at` instead of now, as an
    /// imported recording is.
    pub async fn start_at(
        &mut self,
        title: Option<String>,
        url: Option<String>,
        started_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<MeetingId> {
        if !self.state.is_idle() {
            return Err(MeetingError::AlreadyInProgress.into());
        }

        // Create meeting
        let mut meeting = MeetingData::new(title);
        meeting.metadata.model = Some(self.engine_name.clone());
        meeting.metadata.url = url;
        if let Some(started_at) = started_at {
            meeting.metadata.started_at = started_at;
        }

        // Create storage directory
        let storage_path = self
            .storage
            .create_meeting(&meeting.metadata)
            .map_err(|e| MeetingError::Storage(e.to_string()))?;
        meeting.metadata.storage_path = Some(storage_path.clone());

        let meeting_id = meeting.metadata.id;
        let title = meeting.metadata.title.clone();
        let started_at = meeting.metadata.started_at;
        self.title_labels = self.me.as_ref().and_then(|me| {
            let labels = title_labels::labels_for(title.as_deref()?, me)?;
            tracing::info!(
                "Naming speakers from the title: You as {:?}, every remote voice as {:?}",
                labels.you,
                labels.other
            );
            Some(labels)
        });
        let (diarization_enabled, diarization_backend) = match &self.config.diarization {
            Some(cfg) => (cfg.enabled, Some(cfg.backend.clone())),
            None => (false, None),
        };

        let context = manifest::MeetingContext {
            engine: Some(self.engine_name.clone()),
            model: self.model_name.clone(),
            diarization: diarization_backend.clone().filter(|_| diarization_enabled),
            voxtype_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            echo_cancel: None,
        };
        if let Err(e) = self.storage.set_context(&meeting_id, &context) {
            tracing::warn!("Failed to record the meeting's engine and model: {}", e);
        }

        self.audio = None;
        if self.config.retain_audio {
            match retained_audio::AudioRetainer::start(
                &storage_path,
                retained_audio::Encoder::detect(),
            ) {
                Ok(audio) => {
                    self.audio = Some(audio);
                    meeting.metadata.audio_retained = true;
                    if let Err(e) = self.storage.update_meeting(&meeting.metadata) {
                        tracing::warn!("Failed to mark the meeting's audio as kept: {}", e);
                    }
                }
                Err(e) => tracing::error!("Failed to start keeping meeting audio: {}", e),
            }
        }
        self.current_meeting = Some(meeting);
        self.state = MeetingState::start();
        self.next_segment_id = 0;
        self.own_voice_ids.clear();

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

        // Close the audio files first, so the manifest written below sees
        // their final sizes.
        if let Some(audio) = self.audio.take() {
            audio.finish();
        }

        // Finalize meeting
        if let Some(ref mut meeting) = self.current_meeting {
            meeting.complete();
            meeting.metadata.chunk_count = meeting.transcript.total_chunks;
            if let Some(ref dir) = meeting.metadata.storage_path {
                meeting.metadata.audio_retained = !retained_audio::retained_files(dir).is_empty();
            }

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

    /// What runs once a stopped meeting's folder is final: the manifest
    /// rewrite and `post_meeting_command`. The daemon spawns it after the
    /// auto-summary, off its event loop.
    pub fn post_meeting(&self, meeting_id: MeetingId) -> hook::PostMeeting {
        hook::PostMeeting {
            storage: self.config.storage.clone(),
            meeting_id,
            settings: self.post_meeting.clone(),
        }
    }

    /// The summary to run for a meeting that just stopped, unless
    /// `[meeting.summary] auto = false`. Whether it runs when auto is unset
    /// (consent given or not) is decided by the job itself, off the event
    /// loop, since that means asking `omarchy-default-agent`.
    pub fn auto_summary(&self, meeting_id: MeetingId) -> Option<assistant::summarize::AutoSummary> {
        let settings = self.assistant.clone()?;
        if settings.summary_auto == Some(false) {
            return None;
        }
        Some(assistant::summarize::AutoSummary {
            settings,
            storage: self.config.storage.clone(),
            meeting_id,
        })
    }

    /// Record the echo handling the running meeting uses (`aec`, `gtcrn`
    /// or `off`), for meeting.json and `meeting status --json`.
    pub fn record_echo_cancel(&self, path: &str) {
        let Some(id) = self.current_meeting_id() else {
            return;
        };
        let result = self.storage.get_context(&id).and_then(|context| {
            self.storage.set_context(
                &id,
                &manifest::MeetingContext {
                    echo_cancel: Some(path.to_string()),
                    ..context
                },
            )
        });
        if let Err(e) = result {
            tracing::warn!("Failed to record the meeting's echo handling: {}", e);
        }
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
        // Kept as recorded, before VAD skips anything, so the files hold
        // the whole meeting on the transcript's timeline. Paused audio is
        // dropped like it is from the transcript.
        if self.state.is_active() {
            if let Some(ref mut audio) = self.audio {
                audio.push(mic.clone(), loopback.clone());
            }
        }

        // Measure both tracks before they are handed off, so the short-echo
        // rules can tell your voice from the remote's echo by level.
        let probe = (!mic.is_empty() && !loopback.is_empty()).then(|| {
            let offset = |s| self.source_offsets.get(&s).copied().unwrap_or(0);
            echo::EchoProbe::new(
                &mic,
                offset(AudioSource::Microphone),
                &loopback,
                offset(AudioSource::Loopback),
                ChunkConfig::default().sample_rate,
                self.config.vad_threshold,
            )
        });
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
        // Judge this chunk's microphone lines by their audio, once, while
        // it is at hand: your own voice, or nothing but the remote's echo.
        let rules = self.config.echo_rules;
        let mut echo_ids = std::collections::HashSet::new();
        // Lines the audio measured and found no voice of yours in.
        let mut quiet_ids = std::collections::HashSet::new();
        // Loopback lines with no speech on the loopback under them.
        let mut silent_remote_ids = std::collections::HashSet::new();
        if let Some(probe) = &probe {
            for seg in done.iter().flat_map(|(_, segments)| segments) {
                if seg.source == AudioSource::Loopback
                    && !probe.remote_speech(seg.start_ms, seg.end_ms)
                {
                    silent_remote_ids.insert(seg.id);
                }
                if seg.source != AudioSource::Microphone {
                    continue;
                }
                if probe.own_voice(seg.start_ms, seg.end_ms) {
                    self.own_voice_ids.insert(seg.id);
                    continue;
                }
                quiet_ids.insert(seg.id);
                if rules.guard && probe.echo_only(seg.start_ms, seg.end_ms) {
                    echo_ids.insert(seg.id);
                }
            }
        }
        if !rules.guard {
            quiet_ids.clear();
            silent_remote_ids.clear();
        }
        if had_loopback || !echo_ids.is_empty() || !quiet_ids.is_empty() {
            let own = &self.own_voice_ids;
            if let Some(ref mut meeting) = self.current_meeting {
                if !silent_remote_ids.is_empty() {
                    let dropped = meeting
                        .transcript
                        .drop_remote_no_speech_lines(&silent_remote_ids);
                    if !dropped.is_empty() {
                        tracing::info!(
                            "Echo guard: dropped {} no-speech phrase(s) on the loopback with no remote speech under them",
                            dropped.len()
                        );
                    }
                }
                // A no-speech phrase is no one's line: dropped before an
                // echo line could move it to the remote speaker.
                if !quiet_ids.is_empty() {
                    let dropped = meeting.transcript.drop_no_speech_lines(&quiet_ids);
                    if !dropped.is_empty() {
                        tracing::info!(
                            "Echo guard: dropped {} no-speech phrase(s) said over the remote",
                            dropped.len()
                        );
                        for id in dropped {
                            echo_ids.remove(&id);
                        }
                    }
                }
                if !quiet_ids.is_empty() {
                    let dropped = meeting.transcript.drop_opening_echoes(&quiet_ids);
                    if !dropped.is_empty() {
                        tracing::info!(
                            "Echo guard: dropped {} short microphone line(s) that open with the remote's words",
                            dropped.len()
                        );
                        for id in dropped {
                            echo_ids.remove(&id);
                        }
                    }
                }
                if !echo_ids.is_empty() {
                    let settled = meeting.transcript.resolve_echo_lines(&echo_ids);
                    if !settled.dropped.is_empty() || !settled.moved.is_empty() {
                        tracing::info!(
                            "Echo guard: dropped {} microphone line(s) the loopback already has, moved {} to the remote speaker",
                            settled.dropped.len(),
                            settled.moved.len()
                        );
                    }
                }
                if had_loopback {
                    let removed = meeting
                        .transcript
                        .dedup_bleed_through_with(&rules, &|s| own.contains(&s.id));
                    if removed > 0 {
                        tracing::info!("Removed {} bleed-through word(s) via dedup", removed);
                    }
                }
                // Publish what is left of each segment: text the dedup
                // trimmed, and the speaker of a line moved to the remote.
                let kept = &meeting.transcript.segments;
                for (_, segments) in &mut done {
                    segments.retain_mut(|seg| match kept.iter().find(|k| k.id == seg.id) {
                        Some(k) => {
                            seg.text = k.text.clone();
                            seg.speaker_id = k.speaker_id.clone();
                            seg.speaker_label = k.speaker_label.clone();
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

    /// Re-read what the user changed line by line (`meeting reassign`,
    /// `annotate`, `edit`, `delete-line`), apply it to the running meeting,
    /// save, and publish each line that changed: `SegmentDeleted` for a line
    /// just deleted, `SegmentUpdated` for any other change, a restored line
    /// included. Returns how many lines changed.
    pub async fn apply_segment_edits(&mut self) -> usize {
        let Some(ref mut meeting) = self.current_meeting else {
            return 0;
        };
        let id = meeting.metadata.id;
        let read = || -> std::result::Result<_, StorageError> {
            Ok((
                self.storage.get_speaker_overrides(&id)?,
                self.storage.get_speaker_labels(&id)?,
                self.storage.get_annotations(&id)?,
                self.storage.get_text_edits(&id)?,
                self.storage.get_deletions(&id)?,
            ))
        };
        let (overrides, labels, annotations, edits, deletions) = match read() {
            Ok(all) => all,
            Err(e) => {
                tracing::warn!("Failed to read transcript edits: {}", e);
                return 0;
            }
        };

        let before = meeting.transcript.segments.clone();
        meeting.transcript.apply_speaker_overrides(&overrides);
        // Names given with `meeting label` since a line was sent: without
        // this, lines from before the name keep the bare ID in the saved
        // transcript and in the hello backlog a reopened view starts from.
        meeting.transcript.apply_speaker_labels(&labels);
        meeting.transcript.apply_annotations(&annotations);
        meeting.transcript.apply_text_edits(&edits);
        meeting.transcript.apply_deletions(&deletions);

        let mut events = Vec::new();
        for (now, was) in meeting.transcript.segments.iter().zip(&before) {
            if now.deleted {
                if !was.deleted {
                    events.push(MeetingEvent::SegmentDeleted { segment_id: now.id });
                }
                continue;
            }
            let changed = was.deleted
                || now.speaker_label != was.speaker_label
                || now.speaker_id != was.speaker_id
                || now.original_speaker != was.original_speaker
                || now.annotation != was.annotation
                || now.text != was.text;
            if changed {
                // Names the line now carries are applied to this copy alone.
                let mut segment = now.clone();
                segment.speaker_label = labels.get(&segment.speaker_key()).cloned();
                events.push(MeetingEvent::SegmentUpdated { segment });
            }
        }

        if events.is_empty() {
            return 0;
        }
        self.persist_transcript();
        let count = events.len();
        for event in events {
            let _ = self.event_tx.send(event).await;
        }
        tracing::info!("{} transcript line(s) changed by hand", count);
        count
    }

    /// Name the speakers the title names (see [`title_labels`]) among
    /// the meeting so far and `new` segments, leaving any speaker that has a
    /// name alone. A speaker heard before but only named now has its earlier
    /// lines updated on the live feed.
    async fn label_from_title(&mut self, new: &[TranscriptSegment]) {
        let (Some(labels), Some(meeting)) = (&self.title_labels, &self.current_meeting) else {
            return;
        };
        let id = meeting.metadata.id;
        let pairs =
            title_labels::assignments(labels, meeting.transcript.segments.iter().chain(new));
        let mut earlier = false;
        for (key, label) in pairs {
            match self.storage.label_speaker_if_unlabeled(&id, &key, &label) {
                Ok(true) => {
                    tracing::info!("Named {} as {} from the meeting title", key, label);
                    earlier |= meeting
                        .transcript
                        .segments
                        .iter()
                        .any(|s| s.speaker_key() == key);
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("Failed to name {} from the title: {}", key, e),
            }
        }
        if earlier {
            self.apply_segment_edits().await;
        }
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

        // A decoder stuck repeating a phrase, which no one says three
        // times running: keep one instance.
        for segment in &mut result.segments {
            if let Some(collapsed) = repeat::collapse(&segment.text) {
                tracing::info!(
                    "Collapsed a repeated phrase in a {} line: {:?} (was {} words)",
                    source,
                    collapsed,
                    segment.text.split_whitespace().count()
                );
                segment.text = collapsed;
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
            let renames = diarizer.take_speaker_renames();
            let merges = diarizer.merge_similar_speakers();
            for (from, to) in renames {
                self.rename_speaker(&from, &to).await;
            }
            // Speakers found to be one voice become one, labels included.
            for (from, to) in merges {
                let (from, to) = (from.display_name(), to.display_name());
                // This chunk's own segments are not in the transcript yet.
                for seg in &mut result.segments {
                    if seg.speaker_id.as_deref() == Some(from.as_str()) {
                        seg.speaker_id = Some(to.clone());
                    }
                }
                self.rename_speaker(&from, &to).await;
            }
        }

        self.label_from_title(&result.segments).await;

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

/// List meetings with each one's summary flag and speaker count, from the
/// index (see [`MeetingStorage::list_meeting_entries`])
pub fn list_meeting_entries(
    config: &MeetingConfig,
    limit: Option<u32>,
) -> std::result::Result<Vec<storage::MeetingListEntry>, StorageError> {
    let storage = MeetingStorage::open(config.storage.clone())?;
    storage.list_meeting_entries(limit)
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
        harness_with(|_| {}).await
    }

    async fn harness_with(tweak: impl FnOnce(&mut MeetingConfig)) -> Harness {
        let dir = tempfile::TempDir::new().unwrap();
        let mut config = MeetingConfig {
            enabled: true,
            chunk_duration_secs: 30,
            storage: StorageConfig {
                storage_path: dir.path().to_path_buf(),
                retain_audio: false,
                max_meetings: 0,
            },
            ..Default::default()
        };
        tweak(&mut config);
        let (tx, events) = mpsc::channel(64);
        let mut daemon = MeetingDaemon::for_test(
            config.clone(),
            Arc::new(CountingTranscriber(AtomicU32::new(0))),
            Some(Box::new(diarization::simple::SimpleDiarizer::new())),
            tx,
        );
        daemon.start(Some("Test".to_string()), None).await.unwrap();
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

        fn folder(&self) -> std::path::PathBuf {
            self.saved().metadata.storage_path.unwrap()
        }

        fn manifest(&self) -> manifest::Manifest {
            manifest::Manifest::read(&self.folder()).unwrap()
        }
    }

    /// Seconds of audio in a retained track, decoded the way a player would.
    fn audio_secs(path: &std::path::Path) -> f32 {
        try_audio_secs(path).unwrap_or_else(|| panic!("{} does not decode", path.display()))
    }

    fn try_audio_secs(path: &std::path::Path) -> Option<f32> {
        if path.extension().is_some_and(|e| e == "wav") {
            let reader = hound::WavReader::open(path).ok()?;
            return Some(reader.duration() as f32 / 16000.0);
        }
        let retained_audio::Encoder::Ffmpeg(ffmpeg) = retained_audio::Encoder::detect() else {
            panic!("an Opus file without ffmpeg");
        };
        let out = std::process::Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-f", "s16le", "-ac", "1", "-ar", "16000", "pipe:1"])
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| out.stdout.len() as f32 / 2.0 / 16000.0)
    }

    fn file_names(m: &manifest::Manifest) -> Vec<&str> {
        m.files.iter().map(|f| f.name.as_str()).collect()
    }

    #[tokio::test]
    async fn retain_audio_off_writes_no_audio() {
        let mut h = harness().await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        h.daemon.stop().await.unwrap();
        assert!(retained_audio::retained_files(&h.folder()).is_empty());
        assert!(!h.saved().metadata.audio_retained);
        let m = h.manifest();
        assert!(!m.audio_retained);
        assert_eq!(file_names(&m), ["transcript.json", "transcript.md"]);
    }

    #[tokio::test]
    async fn retain_audio_keeps_both_tracks_and_a_mix() {
        let mut h = harness_with(|c| c.retain_audio = true).await;
        let ext = retained_audio::Encoder::detect().format().extension();
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        // Paused audio is dropped, as it is from the transcript.
        h.daemon.pause().await.unwrap();
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        h.daemon.resume().await.unwrap();
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        h.daemon.stop().await.unwrap();

        let dir = h.folder();
        for stem in ["mic", "loopback", "mixed"] {
            let secs = audio_secs(&dir.join(format!("{stem}.{ext}")));
            assert!((secs - 60.0).abs() < 0.1, "{stem}: {secs}s");
        }
        assert!(h.saved().metadata.audio_retained);
        let m = h.manifest();
        assert!(m.audio_retained);
        let audio: Vec<(&str, &str)> = m
            .files
            .iter()
            .filter(|f| f.kind.starts_with("audio_"))
            .map(|f| (f.name.as_str(), f.kind.as_str()))
            .collect();
        assert_eq!(
            audio,
            [
                (format!("mic.{ext}").as_str(), "audio_mic"),
                (format!("loopback.{ext}").as_str(), "audio_loopback"),
                (format!("mixed.{ext}").as_str(), "audio_mixed"),
            ]
        );
        for f in &m.files {
            assert_eq!(f.bytes, std::fs::metadata(dir.join(&f.name)).unwrap().len());
        }
    }

    #[tokio::test]
    async fn manifest_describes_the_meeting() {
        let mut h = harness().await;
        let m = h.manifest();
        assert_eq!(m.status, "active", "written when the meeting starts");
        assert!(m.ended_at.is_none());

        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        let id = h.daemon.current_meeting_id().unwrap();
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        cli.set_speaker_label(&id, "You", "Pete").unwrap();
        h.daemon.stop().await.unwrap();

        let m = h.manifest();
        assert_eq!(m.manifest_version, 1);
        assert_eq!(m.id, id.to_string());
        assert_eq!(m.title.as_deref(), Some("Test"));
        assert_eq!(m.url, None);
        assert_eq!(m.status, "completed");
        assert!(m.ended_at.is_some() && m.duration_secs.is_some());
        assert_eq!(m.engine.as_deref(), Some("test"));
        assert_eq!(m.model.as_deref(), Some("test-model"));
        assert_eq!(m.diarization, None, "the harness config has none");
        let speakers: Vec<(&str, &str)> = m
            .speakers
            .iter()
            .map(|s| (s.id.as_str(), s.name.as_str()))
            .collect();
        assert_eq!(speakers, [("Remote", "Remote"), ("You", "Pete")]);
        assert_eq!(m.voxtype_version, env!("CARGO_PKG_VERSION"));

        // transcript.md sits beside it and carries the same names.
        let md = std::fs::read_to_string(h.folder().join("transcript.md")).unwrap();
        assert!(md.contains("Pete"), "{md}");

        // Human-readable: pretty JSON, keys in a fixed order.
        let raw = std::fs::read_to_string(h.folder().join("meeting.json")).unwrap();
        assert!(raw.starts_with("{\n  \"manifest_version\": 1,\n  \"id\""));
        assert!(raw.ends_with("}\n"));
    }

    /// Engine and model come from the meeting's own transcription config,
    /// with `meeting start --model` applied, and the URL from `--url`.
    #[tokio::test]
    async fn manifest_names_what_the_meeting_was_started_with() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = MeetingConfig {
            enabled: true,
            storage: StorageConfig {
                storage_path: dir.path().to_path_buf(),
                retain_audio: false,
                max_meetings: 0,
            },
            ..Default::default()
        };
        let app = crate::config::Config::default();
        let options = crate::config::MeetingStartOptions {
            model: Some("small.en".to_string()),
            ..Default::default()
        };
        assert_ne!(app.model_name(), "small.en");
        let effective = app.with_meeting_mode_overrides(&options).unwrap();
        let (tx, _events) = mpsc::channel(8);
        let resident: Arc<dyn Transcriber> = Arc::new(CountingTranscriber(AtomicU32::new(0)));
        let mut daemon =
            MeetingDaemon::new(config.clone(), &effective, tx, Some(resident)).unwrap();
        let url = "https://meet.example.com/abc-defg-hij";
        let id = daemon
            .start(Some("Call".to_string()), Some(url.to_string()))
            .await
            .unwrap();
        daemon.stop().await.unwrap();

        let storage = MeetingStorage::open(config.storage).unwrap();
        let m = manifest::Manifest::read(&storage.get_meeting_path(&id).unwrap()).unwrap();
        assert_eq!(m.engine.as_deref(), Some("whisper"));
        assert_eq!(m.model.as_deref(), Some("small.en"));
        assert_eq!(m.url.as_deref(), Some(url));
    }

    /// Labels and deletions after the meeting reach meeting.json: a speaker
    /// whose only line was deleted is gone from it, while transcript.json
    /// keeps the line as a tombstone.
    #[tokio::test]
    async fn manifest_follows_labels_and_deleted_lines() {
        let mut h = harness().await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        let id = h.daemon.stop().await.unwrap();
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let mine = h
            .saved()
            .transcript
            .segments
            .iter()
            .find(|s| s.source == AudioSource::Microphone)
            .unwrap()
            .id;

        cli.set_speaker_label(&id, "Remote", "Dana").unwrap();
        cli.delete_segment(&id, mine, true).unwrap();

        let m = h.manifest();
        let speakers: Vec<(&str, &str)> = m
            .speakers
            .iter()
            .map(|s| (s.id.as_str(), s.name.as_str()))
            .collect();
        assert_eq!(speakers, [("Remote", "Dana")]);
        let raw: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(h.folder().join("transcript.json")).unwrap(),
        )
        .unwrap();
        let tombstone = raw["segments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == mine)
            .unwrap();
        assert_eq!(tombstone["deleted"], true);
        let listed = m
            .files
            .iter()
            .find(|f| f.name == "transcript.json")
            .unwrap();
        assert_eq!(
            listed.bytes,
            std::fs::metadata(h.folder().join("transcript.json"))
                .unwrap()
                .len()
        );
    }

    #[tokio::test]
    async fn manifest_follows_notes_written_after_the_meeting() {
        let mut h = harness().await;
        h.pair(&[(1.0, 3.0)], &[]).await;
        let id = h.daemon.stop().await.unwrap();
        assert!(!file_names(&h.manifest()).contains(&"notes.md"));

        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        cli.set_notes(&id, "Ship it.\n").unwrap();
        let m = h.manifest();
        let notes = m.files.iter().find(|f| f.name == "notes.md").unwrap();
        assert_eq!((notes.kind.as_str(), notes.bytes), ("notes", 9));
        assert_eq!(
            file_names(&m),
            ["transcript.json", "transcript.md", "notes.md"]
        );

        // A summary written later (by the meeting assistant) appears on the
        // post-meeting rewrite.
        std::fs::write(h.folder().join("summary.md"), "# Summary\n").unwrap();
        h.daemon.post_meeting(id).run().await;
        assert!(file_names(&h.manifest()).contains(&"summary.md"));
    }

    #[tokio::test]
    async fn a_crashed_meeting_is_completed_in_its_manifest_on_restart() {
        let mut h = harness_with(|c| c.retain_audio = true).await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        let folder = h.folder();
        // The daemon goes away without stopping the meeting.
        drop(h.daemon);
        assert_eq!(manifest::Manifest::read(&folder).unwrap().status, "active");

        // Whatever reached the disk plays; the writer thread drains its
        // queue on its own once the daemon is gone.
        let ext = retained_audio::Encoder::detect().format().extension();
        let mic = folder.join(format!("mic.{ext}"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while try_audio_secs(&mic).unwrap_or(0.0) < 25.0 {
            assert!(std::time::Instant::now() < deadline, "audio never landed");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        // The next daemon start.
        let storage = MeetingStorage::open(h.config.storage.clone()).unwrap();
        assert_eq!(storage.complete_stale_meetings().unwrap(), 1);
        let m = manifest::Manifest::read(&folder).unwrap();
        assert_eq!(m.status, "completed");
        assert!(m.audio_retained);
        assert!(m.files.iter().any(|f| f.kind == "audio_mic"));
    }

    fn with_command(h: &mut Harness, command: &str, timeout: std::time::Duration) {
        h.daemon.post_meeting = hook::PostMeetingSettings {
            command: Some(command.to_string()),
            timeout,
        };
    }

    #[tokio::test]
    async fn post_meeting_command_gets_the_meeting() {
        let mut h = harness().await;
        with_command(
            &mut h,
            r#"test -f "$VOXTYPE_MEETING_MANIFEST" && printf '%s\n%s\n%s\n%s\n' \
                "$VOXTYPE_MEETING_ID" "$VOXTYPE_MEETING_DIR" \
                "$VOXTYPE_MEETING_MANIFEST" "$PWD" > hook.out"#,
            std::time::Duration::from_secs(10),
        );
        h.pair(&[(1.0, 3.0)], &[]).await;
        let id = h.daemon.stop().await.unwrap();
        let outcome = h.daemon.post_meeting(id).run().await;
        assert_eq!(outcome, Some(hook::HookOutcome::Succeeded));

        let dir = h.folder();
        let out = std::fs::read_to_string(dir.join("hook.out")).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], id.to_string());
        assert_eq!(lines[1], dir.to_str().unwrap());
        assert_eq!(lines[2], dir.join("meeting.json").to_str().unwrap());
        assert_eq!(
            lines[3],
            dir.to_str().unwrap(),
            "runs in the meeting folder"
        );
    }

    #[tokio::test]
    async fn no_post_meeting_command_still_rewrites_the_manifest() {
        let mut h = harness().await;
        let id = h.daemon.stop().await.unwrap();
        std::fs::remove_file(h.folder().join("meeting.json")).unwrap();
        assert_eq!(h.daemon.post_meeting(id).run().await, None);
        assert_eq!(h.manifest().status, "completed");
    }

    #[tokio::test]
    async fn a_slow_post_meeting_command_is_killed_with_its_children() {
        let mut h = harness().await;
        with_command(
            &mut h,
            "sleep 30 & echo $! > child.pid; wait",
            std::time::Duration::from_millis(500),
        );
        let id = h.daemon.stop().await.unwrap();
        let started = std::time::Instant::now();
        let outcome = h.daemon.post_meeting(id).run().await;
        assert_eq!(outcome, Some(hook::HookOutcome::TimedOut));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));

        let pid: i32 = std::fs::read_to_string(h.folder().join("child.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // Killed; reaped by init or a zombie until then, but never running.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let running = state
            .rsplit(')')
            .next()
            .and_then(|rest| rest.split_whitespace().next())
            .is_some_and(|s| s != "Z");
        assert!(!running, "sleep {pid} outlived its timeout: {state}");
    }

    /// A command that exits but leaves something running with its output
    /// open finishes when it exits, not when the leftover does.
    #[tokio::test]
    async fn a_post_meeting_command_that_leaves_a_child_running_still_finishes() {
        let mut h = harness().await;
        with_command(
            &mut h,
            "sleep 30 & echo $! > child.pid; echo started",
            std::time::Duration::from_millis(800),
        );
        let id = h.daemon.stop().await.unwrap();
        let started = std::time::Instant::now();
        let outcome = h.daemon.post_meeting(id).run().await;
        assert_eq!(outcome, Some(hook::HookOutcome::Succeeded));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let pid = std::fs::read_to_string(h.folder().join("child.pid")).unwrap();
        let _ = std::process::Command::new("kill").arg(pid.trim()).status();
    }

    #[tokio::test]
    async fn a_failing_post_meeting_command_leaves_the_daemon_working() {
        let mut h = harness().await;
        with_command(
            &mut h,
            "echo nope >&2; exit 3",
            std::time::Duration::from_secs(10),
        );
        h.pair(&[(1.0, 3.0)], &[]).await;
        let id = h.daemon.stop().await.unwrap();
        // Spawned the way the daemon does it: stop has already returned.
        let job = tokio::spawn(h.daemon.post_meeting(id).run());
        h.daemon
            .start(Some("Next".to_string()), None)
            .await
            .unwrap();
        h.pair(&[(1.0, 3.0)], &[]).await;
        assert_eq!(job.await.unwrap(), Some(hook::HookOutcome::Failed(Some(3))));
        let next = h.daemon.stop().await.unwrap();
        let saved = MeetingStorage::open(h.config.storage.clone())
            .unwrap()
            .load_meeting_data(&next)
            .unwrap();
        assert_eq!(saved.metadata.title.as_deref(), Some("Next"));
        assert_eq!(saved.transcript.segments.len(), 1);
    }

    #[tokio::test]
    async fn a_missing_post_meeting_program_is_only_logged() {
        let mut h = harness().await;
        with_command(
            &mut h,
            "/nonexistent/voxtype-hook",
            std::time::Duration::from_secs(10),
        );
        let id = h.daemon.stop().await.unwrap();
        assert_eq!(
            h.daemon.post_meeting(id).run().await,
            Some(hook::HookOutcome::Failed(Some(127)))
        );
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

    /// One line of the demo meeting: the carrier tone that stands for it in
    /// the rebuilt audio, and what the fake engine hears when it finds it.
    struct DemoLine {
        freq: f32,
        text: Option<String>,
    }

    /// The 1.2.1 demo's release sync, rebuilt at 16 kHz from the per-frame
    /// levels in tests/fixtures/meeting_echo/release-sync.json. Every line
    /// gets a carrier of its own, and so does the echo of each remote line
    /// on the microphone, standing in for the engine mishearing it.
    struct ReleaseSync {
        mic: Vec<f32>,
        loopback: Vec<f32>,
        lines: Vec<DemoLine>,
        echoes: Vec<String>,
    }

    impl ReleaseSync {
        const FRAME: usize = 480;

        /// `replies` are short lines of yours added on top: (start in
        /// seconds, text), voiced with the level of the demo's "Good.".
        fn load(replies: &[(f32, &str)]) -> Self {
            let replies: Vec<Reply> = replies
                .iter()
                .map(|&(at, text)| Reply {
                    at,
                    text,
                    from_ms: 18150,
                    frames: 15,
                    gain: 1.0,
                })
                .collect();
            Self::load_in(&Room::demo(), &replies)
        }

        /// The demo meeting played in `room`, with `replies` of yours on top.
        fn load_in(room: &Room, replies: &[Reply]) -> Self {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/meeting_echo/release-sync.json");
            let fixture: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            let rms = |key: &str| -> Vec<f32> {
                fixture[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap() as f32)
                    .collect()
            };
            let (own_rms, remote_rms) = (rms("own_rms"), rms("remote_rms"));
            let frames = own_rms.len();
            let gain = room
                .gain
                .unwrap_or(fixture["bleed_gain"].as_f64().unwrap() as f32);
            let delay = (room
                .delay_ms
                .unwrap_or(fixture["bleed_delay_ms"].as_u64().unwrap())
                / 30) as usize;

            let mut own = vec![0.0f32; frames * Self::FRAME];
            let mut loopback = vec![0.0f32; frames * Self::FRAME];
            let mut bleed = vec![0.0f32; frames * Self::FRAME];
            let mut lines = Vec::new();
            let mut echoes = Vec::new();
            // A frame at its measured level on a carrier.
            let voice = |track: &mut [f32], frame: usize, level: f32, freq: f32| {
                for n in frame * Self::FRAME..((frame + 1) * Self::FRAME).min(track.len()) {
                    let t = n as f32 / 16000.0;
                    track[n] += std::f32::consts::SQRT_2
                        * level
                        * (2.0 * std::f32::consts::PI * freq * t).sin();
                }
            };
            let (mut mics, mut remotes) = (0, 0);
            for u in fixture["utterances"].as_array().unwrap() {
                let first = (u["start_ms"].as_u64().unwrap() / 30) as usize;
                let last = ((u["end_ms"].as_u64().unwrap() / 30) as usize).min(frames);
                let text = u["text"].as_str().unwrap().to_string();
                if u["track"] == "mic" {
                    let freq = 400.0 + 200.0 * mics as f32;
                    mics += 1;
                    for (f, &level) in own_rms.iter().enumerate().take(last).skip(first) {
                        voice(&mut own, f, level, freq);
                    }
                    lines.push(DemoLine {
                        freq,
                        text: Some(text),
                    });
                } else {
                    let freq = 3000.0 + 200.0 * remotes as f32;
                    let echo_freq = 5000.0 + 200.0 * remotes as f32;
                    remotes += 1;
                    for (f, &level) in remote_rms.iter().enumerate().take(last).skip(first) {
                        voice(&mut loopback, f, level, freq);
                        // The room: the direct sound, then reflections
                        // dying away, summing to `gain` in steady state.
                        for (k, w) in room.taps().into_iter().enumerate() {
                            if f + delay + k < frames {
                                voice(&mut bleed, f + delay + k, gain * w * level, echo_freq);
                            }
                        }
                    }
                    let heard = match &room.heard {
                        Some(heard) => heard(remotes - 1, &text),
                        None => u["heard_on_mic"].as_str().map(str::to_string),
                    };
                    echoes.extend(heard.clone());
                    lines.push(DemoLine {
                        freq,
                        text: Some(text),
                    });
                    lines.push(DemoLine {
                        freq: echo_freq,
                        text: heard,
                    });
                }
            }
            for reply in replies {
                let freq = 400.0 + 200.0 * mics as f32;
                mics += 1;
                let at = (reply.at / 0.03) as usize;
                let from = (reply.from_ms / 30) as usize;
                for i in 0..reply.frames {
                    voice(&mut own, at + i, reply.gain * own_rms[from + i], freq);
                }
                lines.push(DemoLine {
                    freq,
                    text: Some(reply.text.to_string()),
                });
            }
            let mic = own.iter().zip(&bleed).map(|(o, b)| o + b).collect();
            Self {
                mic,
                loopback,
                lines,
                echoes,
            }
        }

        /// Feed it in 30 s chunks and return the saved transcript as
        /// "Speaker: text" lines, and what was published live.
        async fn run(self, h: &mut Harness) -> (Vec<String>, Vec<String>) {
            h.daemon.transcriber = Some(Arc::new(DemoEngine {
                lines: self.lines,
                said: std::sync::Mutex::new(std::collections::HashSet::new()),
            }));
            let chunk = 30 * 16000;
            for start in (0..self.mic.len()).step_by(chunk) {
                let end = (start + chunk).min(self.mic.len());
                h.daemon
                    .process_pair(
                        self.mic[start..end].to_vec(),
                        self.loopback[start..end].to_vec(),
                    )
                    .await;
            }
            let said = |segs: &[TranscriptSegment]| {
                let mut segs = segs.to_vec();
                segs.sort_by_key(|s| s.start_ms);
                segs.iter()
                    .map(|s| format!("{}: {}", s.speaker_display(), s.text))
                    .collect::<Vec<_>>()
            };
            let mut live = Vec::new();
            while let Ok(event) = h.events.try_recv() {
                if let MeetingEvent::ChunkProcessed { segments, .. } = event {
                    live.extend(segments);
                }
            }
            (said(&h.saved().transcript.segments), said(&live))
        }
    }

    /// A line of yours added to the demo: said at `at` seconds, with the
    /// level of the demo's own voice from `from_ms` for `frames` frames,
    /// times `gain`.
    #[derive(Clone, Copy)]
    struct Reply<'a> {
        at: f32,
        text: &'a str,
        from_ms: u64,
        frames: usize,
        gain: f32,
    }

    /// How the remote side reaches the microphone.
    #[derive(Default)]
    struct Room {
        /// Echo level relative to the loopback; the demo's 0.12 if unset.
        gain: Option<f32>,
        /// Echo delay; the demo's 30 ms if unset.
        delay_ms: Option<u64>,
        /// Reflections dying away after the direct sound, as speakers in a
        /// room give, instead of one clean copy.
        reverb: bool,
        /// What the engine hears in the echo of remote line `n` (its text
        /// given); the demo's own fragments if unset.
        #[allow(clippy::type_complexity)]
        heard: Option<Box<dyn Fn(usize, &str) -> Option<String>>>,
    }

    impl Room {
        fn demo() -> Self {
            Self::default()
        }

        /// Weights of the direct sound and its reflections, one per 30 ms
        /// frame, adding up to 1.
        fn taps(&self) -> Vec<f32> {
            if !self.reverb {
                return vec![1.0];
            }
            let raw: Vec<f32> = (0..8).map(|k| 0.55f32.powi(k)).collect();
            let sum: f32 = raw.iter().sum();
            raw.iter().map(|w| w / sum).collect()
        }
    }

    /// Hears the demo's carriers: each line once, timed by the frames where
    /// its carrier is the loudest.
    struct DemoEngine {
        lines: Vec<DemoLine>,
        said: std::sync::Mutex<std::collections::HashSet<usize>>,
    }

    impl DemoEngine {
        fn loudest(&self, frame: &[f32]) -> Option<usize> {
            let n = frame.len() as f32;
            let amplitude = |freq: f32| {
                let w = 2.0 * std::f32::consts::PI * freq / 16000.0;
                let (re, im) = frame
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |(re, im), (i, s)| {
                        (re + s * (w * i as f32).cos(), im - s * (w * i as f32).sin())
                    });
                2.0 * (re * re + im * im).sqrt() / n
            };
            self.lines
                .iter()
                .map(|l| amplitude(l.freq))
                .enumerate()
                .filter(|&(_, a)| a > 0.003)
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        }
    }

    impl Transcriber for DemoEngine {
        fn short_clips_are_cheap(&self) -> bool {
            true
        }

        fn transcribe(&self, samples: &[f32]) -> std::result::Result<String, TranscribeError> {
            Ok(self
                .transcribe_timed(samples)?
                .into_iter()
                .map(|s| s.text)
                .collect::<Vec<_>>()
                .join(" "))
        }

        fn transcribe_timed(
            &self,
            samples: &[f32],
        ) -> std::result::Result<Vec<crate::transcribe::TimedSegment>, TranscribeError> {
            // Runs of frames per carrier, bridging pauses shorter than the
            // utterance gap; a line is timed by its longest run.
            let mut runs: Vec<(usize, usize, usize)> = Vec::new();
            for (f, frame) in samples.chunks(ReleaseSync::FRAME).enumerate() {
                let Some(line) = self.loudest(frame) else {
                    continue;
                };
                match runs.last_mut() {
                    Some((l, _, last)) if *l == line && f - *last <= 23 => *last = f,
                    _ => runs.push((line, f, f)),
                }
            }
            let mut said = self.said.lock().unwrap();
            let mut out = Vec::new();
            for line in runs
                .iter()
                .map(|r| r.0)
                .collect::<std::collections::BTreeSet<_>>()
            {
                let Some(text) = &self.lines[line].text else {
                    continue;
                };
                let (_, first, last) = runs
                    .iter()
                    .filter(|r| r.0 == line)
                    .max_by_key(|r| r.2 - r.1)
                    .copied()
                    .unwrap();
                if said.insert(line) {
                    out.push(crate::transcribe::TimedSegment {
                        text: text.clone(),
                        start_secs: first as f32 * 0.03,
                        end_secs: (last + 1) as f32 * 0.03,
                    });
                }
            }
            out.sort_by(|a, b| a.start_secs.total_cmp(&b.start_secs));
            Ok(out)
        }
    }

    /// The demo's own lines, in order, with `replies` slotted in by time.
    fn demo_lines(extra: &[(&str, usize)]) -> Vec<String> {
        let mut lines = vec![
            "You: Okay, let's start the release sync.",
            "You: The build for 1.2.1 passed on every variant last night.",
            "Remote: Great! I tested the Amarky package this morning, and the settings panel opened without any problems.",
            "You: Good. Are there any blockers left before we tag it?",
            "Remote: Only one. The restart banner stays visible after the demon restarts. I think we can ship with that as a known issue.",
            "You: Agreed. Let's put it in the release notes and fix it in the next patch.",
            "Remote: I'll draft the notes today and send them to you by five.",
            "You: Perfect, then we tag tomorrow morning at 10. Thanks, Dana.",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        for &(line, at) in extra.iter().rev() {
            lines.insert(at, line.to_string());
        }
        lines
    }

    /// The echo fragments the demo left under "You" ("Omarky problem.",
    /// "after the daemon restarts.", "5.") are misheard, so the exact rules
    /// keep them; the fuzzy rule drops them.
    #[tokio::test]
    async fn misheard_echo_from_the_demo_is_dropped() {
        let mut h = harness().await;
        let demo = ReleaseSync::load(&[]);
        let echoes = demo.echoes.clone();
        assert_eq!(echoes.len(), 3);
        let (saved, live) = demo.run(&mut h).await;
        assert_eq!(saved, demo_lines(&[]), "{saved:#?}");
        assert_eq!(live, saved);

        let mut h = harness().await;
        h.daemon.config.echo_rules.guard = false;
        let (saved, _) = ReleaseSync::load(&[]).run(&mut h).await;
        for echo in &echoes {
            assert!(
                saved.contains(&format!("You: {echo}")),
                "without the fuzzy rule {echo:?} stays: {saved:#?}"
            );
        }
    }

    /// Short replies of yours that repeat the remote's words, one said over
    /// the remote ("Only one?" while Dana says "Only one.") and one just
    /// after it, are kept: the microphone is louder than the echo explains.
    #[tokio::test]
    async fn real_short_replies_over_the_remote_are_kept() {
        let mut h = harness().await;
        let replies = [(16.8, "Problems?"), (23.1, "Only one?")];
        let (saved, live) = ReleaseSync::load(&replies).run(&mut h).await;
        let expected = demo_lines(&[("You: Problems?", 3), ("You: Only one?", 5)]);
        assert_eq!(saved, expected, "{saved:#?}");
        assert_eq!(live, saved);
    }

    /// The demo meeting played through speakers instead of headphones, like
    /// Pete's video in a 1.3.0 meeting: the whole remote side reaches the
    /// microphone at 0.3 of the loopback's level, 60 ms late, with the
    /// room's reflections, and the engine transcribes it at length under
    /// You, misheard enough that no four words in a row match the loopback.
    /// The third remote line's echo comes out as words the loopback never
    /// said. On top, two lines of yours over the remote: a long one at the
    /// demo's own speaking level and a short "Only one?".
    fn speakers_room() -> Room {
        Room {
            gain: Some(0.3),
            delay_ms: Some(60),
            reverb: true,
            heard: Some(Box::new(|n, _| {
                Some(
                    [
                        "great i tasted the omarky package this mourning and the setting panel open without any problem",
                        "only won the restore banner stay visible after the daemon restart i thing we can shit with that as a known issue",
                        "mm hmm yeah yeah okay sure thing",
                    ][n]
                        .to_string(),
                )
            })),
        }
    }

    const LONG_REPLY: Reply<'static> = Reply {
        at: 12.0,
        text: "We should check the Windows build too.",
        from_ms: 31890,
        frames: 100,
        gain: 1.0,
    };
    /// Said over the loudest part of the remote's line. At the demo's own
    /// level it is quieter than the echo there, which no engine (and not
    /// the fake one) would make out; half again as loud, as a voice a foot
    /// from the microphone is against speakers across the desk.
    const SHORT_REPLY: Reply<'static> = Reply {
        at: 23.1,
        text: "Only one?",
        from_ms: 18150,
        frames: 15,
        gain: 1.5,
    };

    #[tokio::test]
    async fn speaker_echo_of_any_length_leaves_your_lines() {
        let mut h = harness().await;
        let (saved, live) = ReleaseSync::load_in(&speakers_room(), &[LONG_REPLY, SHORT_REPLY])
            .run(&mut h)
            .await;
        // The two long echoes are gone: the loopback has the same speech.
        // The one the loopback has no words for moves to the remote side.
        // Both of your lines over the remote stay yours.
        let mut expected = demo_lines(&[
            ("You: We should check the Windows build too.", 3),
            ("You: Only one?", 5),
        ]);
        expected.insert(9, "Remote: mm hmm yeah yeah okay sure thing".to_string());
        assert_eq!(saved, expected, "{saved:#?}");
        assert_eq!(live, saved);
    }

    #[tokio::test]
    async fn without_the_guard_speaker_echo_stays_under_you() {
        let mut h = harness().await;
        h.daemon.config.echo_rules.guard = false;
        let (saved, _) = ReleaseSync::load_in(&speakers_room(), &[LONG_REPLY, SHORT_REPLY])
            .run(&mut h)
            .await;
        // The old phrase rule trims the one run of words that matches the
        // loopback; the rest stays yours.
        for echo in [
            "You: great i tasted the omarky package this mourning and the setting panel open without any problem",
            "You: only won the restore banner stay visible after the daemon restart i thing we can shit",
            "You: mm hmm yeah yeah okay sure thing",
            "You: We should check the Windows build too.",
            "You: Only one?",
        ] {
            assert!(saved.contains(&echo.to_string()), "{echo:?} missing: {saved:#?}");
        }
    }

    /// The demo meeting through speakers with the WebRTC canceller on, as in
    /// Pete's 1.3.0 hardware test: what is left of the remote on the
    /// microphone is a faint residual (0.03 of the loopback, 60 ms late)
    /// that the engine makes no words of.
    fn cancelled_room() -> Room {
        Room {
            gain: Some(0.03),
            delay_ms: Some(60),
            reverb: true,
            heard: Some(Box::new(|_, _| None)),
        }
    }

    /// A "Thank you." on the microphone while the remote talks, at
    /// `gain` times the demo's own "Good.".
    fn thank_you(at: f32, gain: f32) -> Reply<'static> {
        Reply {
            at,
            text: "Thank you.",
            from_ms: 18150,
            frames: 12,
            gain,
        }
    }

    /// Pete's test had two "Thank you." lines under You, 3 s apart, while
    /// he was silent and the remote talked: the engine's no-speech phrase,
    /// made from the canceller's faint residual. Here they come from sound
    /// at a twentieth of the demo's voice, below the meeting's VAD level,
    /// and the guard drops them.
    #[tokio::test]
    async fn a_no_speech_phrase_over_the_remote_is_dropped() {
        let faint = [thank_you(25.0, 0.05), thank_you(28.0, 0.05)];
        let mut h = harness().await;
        let (saved, live) = ReleaseSync::load_in(&cancelled_room(), &faint)
            .run(&mut h)
            .await;
        assert_eq!(saved, demo_lines(&[]), "{saved:#?}");
        assert_eq!(live, saved);

        let mut h = harness().await;
        h.daemon.config.echo_rules.guard = false;
        let (saved, _) = ReleaseSync::load_in(&cancelled_room(), &faint)
            .run(&mut h)
            .await;
        let kept = saved.iter().filter(|l| *l == "You: Thank you.").count();
        assert_eq!(kept, 2, "without the guard both stay: {saved:#?}");
    }

    /// A "Thank you." you really say over the remote is your voice by its
    /// level, and stays, the same words as the dropped phrase above.
    #[tokio::test]
    async fn a_thank_you_you_say_over_the_remote_stays() {
        let mut h = harness().await;
        let (saved, live) = ReleaseSync::load_in(&cancelled_room(), &[thank_you(25.0, 1.0)])
            .run(&mut h)
            .await;
        assert_eq!(saved, demo_lines(&[("You: Thank you.", 5)]), "{saved:#?}");
        assert_eq!(live, saved);
    }

    impl ReleaseSync {
        /// A line on the loopback, `text`, at `at` seconds for `frames`
        /// frames at a steady `level`, with no echo of it on the mic (the
        /// canceller took it out).
        fn with_remote_line(mut self, at: f32, text: &str, frames: usize, level: f32) -> Self {
            let freq = 4100.0 + 200.0 * self.lines.len() as f32;
            let first = (at / 0.03) as usize * Self::FRAME;
            for n in first..(first + frames * Self::FRAME).min(self.loopback.len()) {
                let t = n as f32 / 16000.0;
                self.loopback[n] += std::f32::consts::SQRT_2
                    * level
                    * (2.0 * std::f32::consts::PI * freq * t).sin();
            }
            self.lines.push(DemoLine {
                freq,
                text: Some(text.to_string()),
            });
            self
        }
    }

    /// Pete's Zoom call had a "Thank you." on the far side at 00:56, as the
    /// call was joined, with nobody speaking: the engine's no-speech phrase
    /// for near-silence or a tone on the loopback. Here it comes from sound
    /// just under the meeting's VAD level, between two remote lines, and
    /// the guard drops it; so does a "you" the same way.
    #[tokio::test]
    async fn a_no_speech_phrase_on_a_silent_loopback_is_dropped() {
        let load = || {
            ReleaseSync::load_in(&cancelled_room(), &[])
                .with_remote_line(18.0, "Thank you.", 20, 0.008)
                .with_remote_line(20.5, "you", 10, 0.008)
        };
        let mut h = harness().await;
        let (saved, live) = load().run(&mut h).await;
        assert_eq!(saved, demo_lines(&[]), "{saved:#?}");
        assert_eq!(live, saved);

        let mut h = harness().await;
        h.daemon.config.echo_rules.guard = false;
        let (saved, _) = load().run(&mut h).await;
        assert!(
            saved.contains(&"Remote: Thank you.".to_string()),
            "{saved:#?}"
        );
        assert!(saved.contains(&"Remote: you".to_string()), "{saved:#?}");
    }

    /// A "Thank you." the far side really says, at speaking level, stays.
    #[tokio::test]
    async fn a_thank_you_the_remote_says_stays() {
        let mut h = harness().await;
        let (saved, live) = ReleaseSync::load_in(&cancelled_room(), &[])
            .with_remote_line(18.0, "Thank you.", 20, 0.08)
            .run(&mut h)
            .await;
        let mut expected = demo_lines(&[]);
        expected.insert(3, "Remote: Thank you.".to_string());
        assert_eq!(saved, expected, "{saved:#?}");
        assert_eq!(live, saved);
    }

    /// Pete's call with Chad: the faint echo of Chad's line sent the engine
    /// into a loop on the mic, "I don't know what you're talking about."
    /// six times, sharing only "I don't know" with Chad's line. Here the
    /// echo of Dana's second line (".. I think we can ship with that ..")
    /// comes out as a seven-word phrase five times over, too quiet to read
    /// as echo by level. The loop collapses to one instance, and that opens
    /// with three of Dana's words in a row, so it goes.
    fn looping_room() -> Room {
        Room {
            gain: Some(0.05),
            delay_ms: Some(60),
            reverb: false,
            heard: Some(Box::new(|n, _| {
                (n == 1).then(|| {
                    "I think we should not do that. "
                        .repeat(5)
                        .trim()
                        .to_string()
                })
            })),
        }
    }

    #[tokio::test]
    async fn a_decoder_loop_on_faint_echo_collapses_and_goes() {
        let mut h = harness().await;
        let (saved, live) = ReleaseSync::load_in(&looping_room(), &[]).run(&mut h).await;
        assert_eq!(saved, demo_lines(&[]), "{saved:#?}");
        assert_eq!(live, saved);

        // Without the guard the line stays, collapsed all the same.
        let mut h = harness().await;
        h.daemon.config.echo_rules.guard = false;
        let (saved, _) = ReleaseSync::load_in(&looping_room(), &[]).run(&mut h).await;
        assert!(
            saved.contains(&"You: I think we should not do that.".to_string()),
            "{saved:#?}"
        );
    }

    /// Your own words over the remote stay yours even when they open with
    /// the remote's ("I think we should wait." while Dana says "I think we
    /// can ship"), and a phrase said twice is left as said.
    #[tokio::test]
    async fn your_lines_that_open_like_the_remote_or_repeat_twice_stay() {
        let mut h = harness().await;
        let replies = [
            Reply {
                at: 26.0,
                text: "I think we should wait.",
                from_ms: 18150,
                frames: 15,
                gain: 1.5,
            },
            thank_you(13.0, 1.0),
        ];
        let mut replies = replies.to_vec();
        replies[1].text = "Ship it now. Ship it now.";
        let (saved, live) = ReleaseSync::load_in(&cancelled_room(), &replies)
            .run(&mut h)
            .await;
        let expected = demo_lines(&[
            ("You: Ship it now. Ship it now.", 3),
            ("You: I think we should wait.", 5),
        ]);
        assert_eq!(saved, expected, "{saved:#?}");
        assert_eq!(live, saved);
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

    /// Voices 0, 1, 1 for three loopback chunks, then decides after the
    /// third that voice 1 was voice 0 all along.
    struct SplitThenMerge(AtomicU32);

    impl diarization::Diarizer for SplitThenMerge {
        fn diarize(
            &self,
            _samples: &[f32],
            _source: AudioSource,
            _chunk_start_ms: u64,
            segments: &[TranscriptSegment],
        ) -> Vec<diarization::DiarizedSegment> {
            let call = self.0.fetch_add(1, Ordering::SeqCst);
            segments
                .iter()
                .map(|seg| diarization::DiarizedSegment {
                    speaker: diarization::SpeakerId::Auto(call.min(1)),
                    start_ms: seg.start_ms,
                    end_ms: seg.end_ms,
                    text: seg.text.clone(),
                    confidence: 0.8,
                })
                .collect()
        }

        fn merge_similar_speakers(&self) -> Vec<(diarization::SpeakerId, diarization::SpeakerId)> {
            if self.0.load(Ordering::SeqCst) == 3 {
                vec![(
                    diarization::SpeakerId::Auto(1),
                    diarization::SpeakerId::Auto(0),
                )]
            } else {
                Vec::new()
            }
        }

        fn name(&self) -> &'static str {
            "split-then-merge"
        }
    }

    /// One voice split in two (Pete's Chris meeting) is merged back: earlier
    /// lines and the chunk that triggered it both end up on the survivor,
    /// and the name Pete gave the survivor holds for all of them.
    #[tokio::test]
    async fn a_merged_speaker_takes_over_earlier_lines_and_keeps_the_survivors_name() {
        let mut h = harness().await;
        h.daemon.diarizer = Some(Box::new(diarization::hybrid::HybridDiarizer::new(
            Box::new(SplitThenMerge(AtomicU32::new(0))),
        )));
        h.pair(&[], &[(5.0, 7.0)]).await;
        h.pair(&[], &[(5.0, 7.0)]).await;
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        cli.set_speaker_label(&id, "Remote 1", "Chris").unwrap();
        cli.set_speaker_label(&id, "Remote 2", "Chris?").unwrap();
        h.pair(&[], &[(5.0, 7.0)]).await;

        let mut events = Vec::new();
        while let Ok(event) = h.events.try_recv() {
            match event {
                MeetingEvent::SpeakerRenamed { from, to } => {
                    events.push(format!("rename {from} -> {to}"))
                }
                MeetingEvent::ChunkProcessed { segments, .. } => {
                    events.extend(segments.iter().map(|s| s.speaker_key()))
                }
                _ => {}
            }
        }
        assert_eq!(
            events.last().map(String::as_str),
            Some("Remote 1"),
            "{events:?}"
        );
        assert!(
            events.contains(&"rename Remote 2 -> Remote 1".to_string()),
            "{events:?}"
        );

        h.daemon.stop().await.unwrap();
        let speakers: Vec<(String, String)> = h
            .saved()
            .transcript
            .segments
            .iter()
            .map(|s| (s.speaker_key(), s.speaker_display()))
            .collect();
        assert_eq!(
            speakers,
            vec![("Remote 1".to_string(), "Chris".to_string()); 3]
        );
    }

    /// A meeting titled `title`, hybrid-diarized with a new remote voice
    /// every chunk, run by a user named Pete Jackson.
    async fn titled_harness(title: &str) -> Harness {
        let dir = tempfile::TempDir::new().unwrap();
        let config = MeetingConfig {
            enabled: true,
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
            Some(Box::new(diarization::hybrid::HybridDiarizer::new(
                Box::new(NewVoicePerChunk(AtomicU32::new(0))),
            ))),
            tx,
        );
        daemon.me = Some(title_labels::Me::named(&["Pete Jackson"]));
        daemon.start(Some(title.to_string()), None).await.unwrap();
        Harness {
            daemon,
            config,
            events,
            _dir: dir,
        }
    }

    /// Each live line as "speaker key: name shown".
    fn live_names(events: &mut mpsc::Receiver<MeetingEvent>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(event) = events.try_recv() {
            match event {
                MeetingEvent::ChunkProcessed { segments, .. } => out.extend(
                    segments
                        .iter()
                        .map(|s| format!("{}: {}", s.speaker_key(), s.speaker_display())),
                ),
                MeetingEvent::SegmentUpdated { segment } => out.push(format!(
                    "updated {}: {}",
                    segment.speaker_key(),
                    segment.speaker_display()
                )),
                _ => {}
            }
        }
        out
    }

    /// In a 1:1 every remote voice is the other person, so a voice the
    /// diarizer first hears mid-meeting is named on arrival.
    #[tokio::test]
    async fn a_one_to_one_title_names_every_remote_voice_as_it_appears() {
        let mut h = titled_harness("Chris <> Pete weekly").await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        h.pair(&[], &[(5.0, 7.0)]).await;

        assert_eq!(
            live_names(&mut h.events),
            [
                "You: Pete Jackson",
                "Remote: Chris",
                "You: Pete Jackson",
                "Remote 2: Chris",
                "Remote 3: Chris",
            ]
        );
        h.daemon.stop().await.unwrap();
        let saved: Vec<(String, String)> = h
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
            saved,
            pairs(&[
                ("You", "Pete Jackson"),
                ("Remote 1", "Chris"),
                ("You", "Pete Jackson"),
                ("Remote 2", "Chris"),
                ("Remote 3", "Chris"),
            ])
        );
    }

    /// A name given by hand stays, on the speaker it was given to, while
    /// voices heard later still get the title's name.
    #[tokio::test]
    async fn a_hand_label_is_never_replaced_by_the_title() {
        let mut h = titled_harness("Pete / Chris").await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        // The second voice really is someone else, and the user says so.
        cli.set_speaker_label(&id, "Remote", "Sam").unwrap();
        cli.set_speaker_label(&id, "You", "PJ").unwrap();
        h.daemon.apply_segment_edits().await;

        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        h.daemon.stop().await.unwrap();
        let names: Vec<String> = h
            .saved()
            .transcript
            .segments
            .iter()
            .map(|s| s.speaker_display())
            .collect();
        assert_eq!(names, ["PJ", "Sam", "PJ", "Chris"]);
    }

    /// A title that is not two people, or that does not name the user,
    /// leaves every speaker as the diarizer named it.
    #[tokio::test]
    async fn other_titles_name_nobody() {
        for title in ["Q4 planning", "Andrew <> Chris", "Pete, Andrew, Sam sync"] {
            let mut h = titled_harness(title).await;
            h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
            assert_eq!(live_names(&mut h.events), ["You: You", "Remote: Remote"]);
        }
    }

    /// Off, a qualifying title changes nothing.
    #[tokio::test]
    async fn turned_off_the_title_names_nobody() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = MeetingConfig {
            storage: StorageConfig {
                storage_path: dir.path().to_path_buf(),
                retain_audio: false,
                max_meetings: 0,
            },
            auto_label_from_title: false,
            ..Default::default()
        };
        title_labels::set_account_name_for_test(Some("Pete Jackson"));
        let (tx, _events) = mpsc::channel(64);
        let mut app = crate::config::Config::default();
        app.whisper.model = "/nonexistent/ggml-model.bin".into();
        // Constructing the real daemon resolves the user only when on.
        let off = MeetingDaemon::new(
            config.clone(),
            &app,
            tx.clone(),
            Some(Arc::new(CountingTranscriber(AtomicU32::new(0)))),
        )
        .unwrap();
        assert!(off.me.is_none());
        let on = MeetingDaemon::new(
            MeetingConfig {
                auto_label_from_title: true,
                ..config
            },
            &app,
            tx,
            Some(Arc::new(CountingTranscriber(AtomicU32::new(0)))),
        )
        .unwrap();
        assert_eq!(on.me, Some(title_labels::Me::named(&["Pete Jackson"])));
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
        assert_eq!(h.daemon.apply_segment_edits().await, 1);
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
        assert_eq!(line.original_speaker.as_deref(), Some("Remote 1"));

        // --reset after stop gives it back to the renamed diarizer speaker.
        let back = cli.reset_segment_speaker(&id, 1, true).unwrap();
        assert_eq!(back.speaker_key(), "Remote 1");
    }

    /// `meeting label` mid-meeting: lines sent before the name get it too,
    /// published as updates so every view (and the hello backlog a reopened
    /// view starts from) shows it, even for a speaker who never speaks again.
    #[tokio::test]
    async fn a_label_given_mid_meeting_names_the_lines_already_sent() {
        let mut h = harness().await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        drain(&mut h.events);

        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        cli.set_speaker_label(&id, "Remote", "Chris").unwrap();
        assert_eq!(h.daemon.apply_segment_edits().await, 1);
        assert_eq!(drain(&mut h.events), vec!["updated 1 Chris (Remote)"]);
        // Nothing left to name: a second pass publishes nothing.
        assert_eq!(h.daemon.apply_segment_edits().await, 0);

        // Lines after the name carry it as they are sent.
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        h.daemon.stop().await.unwrap();
        let saved = h.saved();
        let remote: Vec<_> = saved
            .transcript
            .segments
            .iter()
            .filter(|s| s.speaker_key() == "Remote")
            .map(|s| s.speaker_label.as_deref())
            .collect();
        assert_eq!(remote, vec![Some("Chris"), Some("Chris")]);
    }

    /// A client offers "Restore original" from `original_speaker`; after a
    /// reset the published segment must not carry it.
    #[tokio::test]
    async fn a_reset_mid_meeting_publishes_the_line_without_original_speaker() {
        let mut h = harness().await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        cli.reassign_segment(&id, 1, "Casey", false).unwrap();
        h.daemon.apply_segment_edits().await;
        cli.reset_segment_speaker(&id, 1, false).unwrap();
        h.daemon.apply_segment_edits().await;

        let updates: Vec<TranscriptSegment> = std::iter::from_fn(|| h.events.try_recv().ok())
            .filter_map(|e| match e {
                MeetingEvent::SegmentUpdated { segment } => Some(segment),
                _ => None,
            })
            .collect();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].original_speaker.as_deref(), Some("Remote"));
        let json = serde_json::to_value(&updates[1]).unwrap();
        assert_eq!(json["speaker_id"], "Remote");
        assert!(json.get("original_speaker").is_none(), "{json}");
    }

    #[tokio::test]
    async fn an_annotation_mid_meeting_is_published_and_kept() {
        let mut h = harness().await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        drain(&mut h.events);
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        cli.annotate_segment(&id, 1, "check this", true, false)
            .unwrap();
        assert_eq!(h.daemon.apply_segment_edits().await, 1);

        let updated: Vec<TranscriptSegment> = std::iter::from_fn(|| h.events.try_recv().ok())
            .filter_map(|e| match e {
                MeetingEvent::SegmentUpdated { segment } => Some(segment),
                _ => None,
            })
            .collect();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].id, 1);
        assert_eq!(
            updated[0].annotation,
            Some(SegmentAnnotation {
                text: "check this".to_string(),
                highlight: true
            })
        );

        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        h.daemon.stop().await.unwrap();
        assert!(h.saved().transcript.segments[1].annotation.is_some());
    }

    /// `meeting edit` and `meeting delete-line` mid-meeting: the daemon
    /// publishes them, and the next chunk and stop keep them.
    #[tokio::test]
    async fn edits_and_deletions_mid_meeting_are_published_and_kept() {
        let mut h = harness().await;
        h.pair(&[(1.0, 3.0), (10.0, 12.0)], &[(5.0, 7.0)]).await;
        drain(&mut h.events);
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        cli.edit_segment_text(&id, 0, "Corrected", false).unwrap();
        cli.delete_segment(&id, 2, false).unwrap();
        assert_eq!(h.daemon.apply_segment_edits().await, 2);

        let events: Vec<MeetingEvent> = std::iter::from_fn(|| h.events.try_recv().ok()).collect();
        assert_eq!(events.len(), 2);
        match &events[0] {
            MeetingEvent::SegmentUpdated { segment } => {
                assert_eq!(segment.id, 0);
                assert_eq!(segment.text, "Corrected");
                assert!(segment.original_text.is_some());
            }
            other => panic!("expected SegmentUpdated, got {other:?}"),
        }
        assert!(matches!(
            events[1],
            MeetingEvent::SegmentDeleted { segment_id: 2 }
        ));

        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        h.daemon.stop().await.unwrap();
        let saved = h.saved();
        assert_eq!(saved.transcript.segments[0].text, "Corrected");
        assert!(saved.transcript.segments.iter().all(|s| s.id != 2));
        assert_eq!(saved.transcript.segments.len(), 4);

        // --restore after stop brings it back.
        let back = cli.restore_segment(&id, 2, true).unwrap();
        assert!(!back.deleted);
        assert_eq!(h.saved().transcript.segments.len(), 5);
    }

    /// A restored line comes back to a live subscriber as segment_updated.
    #[tokio::test]
    async fn a_line_restored_mid_meeting_is_published_as_updated() {
        let mut h = harness().await;
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        cli.delete_segment(&id, 1, false).unwrap();
        h.daemon.apply_segment_edits().await;
        drain(&mut h.events);
        cli.restore_segment(&id, 1, false).unwrap();
        assert_eq!(h.daemon.apply_segment_edits().await, 1);
        match h.events.try_recv().unwrap() {
            MeetingEvent::SegmentUpdated { segment } => {
                assert_eq!(segment.id, 1);
                assert!(!segment.deleted);
            }
            other => panic!("expected SegmentUpdated, got {other:?}"),
        }
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
        h.daemon.apply_segment_edits().await;

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

    /// What the daemon's dictation path does with a loaded model: transcribe
    /// on another thread while the meeting is mid-chunk.
    struct Shared {
        calls: AtomicU32,
        busy: std::sync::Mutex<()>,
    }

    impl Transcriber for Shared {
        fn short_clips_are_cheap(&self) -> bool {
            true
        }

        fn transcribe(&self, samples: &[f32]) -> std::result::Result<String, TranscribeError> {
            // Like the GGUF worker and the ONNX engines: one call at a time.
            let _turn = self.busy.lock().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(if samples.iter().any(|s| s.abs() > 0.05) {
                "words".to_string()
            } else {
                String::new()
            })
        }
    }

    fn meeting_config(dir: &tempfile::TempDir) -> MeetingConfig {
        MeetingConfig {
            enabled: true,
            storage: StorageConfig {
                storage_path: dir.path().to_path_buf(),
                retain_audio: false,
                max_meetings: 0,
            },
            ..Default::default()
        }
    }

    /// A4: a meeting handed dictation's transcriber uses that instance
    /// rather than loading another, and dictation keeps working on it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_resident_transcriber_serves_the_meeting_and_dictation_together() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = Arc::new(Shared {
            calls: AtomicU32::new(0),
            busy: std::sync::Mutex::new(()),
        });
        let resident: Arc<dyn Transcriber> = shared.clone();
        let (tx, mut events) = mpsc::channel(64);
        // An app config whose engine cannot load here: proves no second load.
        let mut app = crate::config::Config::default();
        app.whisper.model = "/nonexistent/ggml-model.bin".into();
        let mut daemon =
            MeetingDaemon::new(meeting_config(&dir), &app, tx, Some(resident.clone())).unwrap();
        assert!(Arc::ptr_eq(daemon.transcriber.as_ref().unwrap(), &resident));
        daemon
            .start(
                Some("Shared".into()),
                Some("https://meet.example.com/x".into()),
            )
            .await
            .unwrap();

        // Dictation transcribes on its own thread while the meeting does.
        let dictation = {
            let resident = resident.clone();
            std::thread::spawn(move || {
                (0..20)
                    .map(|_| resident.transcribe(&[0.5; 1600]).unwrap())
                    .filter(|t| t == "words")
                    .count()
            })
        };
        for _ in 0..3 {
            daemon
                .process_pair(
                    track(30.0, &[(1.0, 3.0), (10.0, 12.0)]),
                    track(30.0, &[(5.0, 7.0)]),
                )
                .await;
        }
        assert_eq!(dictation.join().unwrap(), 20);
        daemon.stop().await.unwrap();

        let mut segments = 0;
        while let Ok(event) = events.try_recv() {
            if let MeetingEvent::ChunkProcessed { segments: s, .. } = event {
                segments += s.len();
            }
        }
        assert_eq!(segments, 9);
        assert_eq!(shared.calls.load(Ordering::SeqCst), 20 + 9);

        let saved = get_meeting(&meeting_config(&dir), "latest").unwrap();
        assert_eq!(
            saved.metadata.url.as_deref(),
            Some("https://meet.example.com/x")
        );
    }

    /// Without a resident transcriber the meeting loads its own: here the
    /// load fails, which is how this test sees it was attempted.
    #[tokio::test]
    async fn without_a_resident_transcriber_the_meeting_loads_its_own() {
        let dir = tempfile::TempDir::new().unwrap();
        let (tx, _events) = mpsc::channel(64);
        let mut app = crate::config::Config::default();
        app.whisper.model = "/nonexistent/ggml-model.bin".into();
        app.whisper.on_demand_loading = false;
        assert!(MeetingDaemon::new(meeting_config(&dir), &app, tx, None).is_err());
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

    use assistant::summarize::SummaryEvent;

    /// Stop the harness meeting and run its auto-summary, returning the
    /// events it emitted.
    async fn stop_and_summarize(h: &mut Harness) -> (bool, Vec<SummaryEvent>) {
        let id = h.daemon.stop().await.unwrap();
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = events.clone();
        let ran = match h.daemon.auto_summary(id) {
            Some(job) => job.run(move |e| sink.lock().unwrap().push(e)).await,
            None => false,
        };
        let events = events.lock().unwrap().clone();
        (ran, events)
    }

    const SUMMARY_REPLY: &str = r#"{"overview": "A short test meeting.", "decisions": [],
        "action_items": [{"description": "Follow up", "owner": "Dana"}],
        "chapters": [{"start": "00:01", "title": "Start"}]}"#;

    #[tokio::test]
    async fn auto_summary_reports_summarizing_then_ready() {
        let sb = assistant::testing::Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&[], SUMMARY_REPLY);
        let mut h = harness().await;
        h.daemon.assistant = Some(sb.settings());
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;
        let cli = MeetingStorage::open(h.config.storage.clone()).unwrap();
        let id = cli.resolve_meeting_id("latest").unwrap();
        cli.set_speaker_label(&id, "Remote", "Dana").unwrap();

        let (ran, events) = stop_and_summarize(&mut h).await;
        assert!(ran);
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(
            matches!(&events[0], SummaryEvent::Summarizing { meeting_id } if *meeting_id == id.to_string())
        );
        let SummaryEvent::Ready(outcome) = &events[1] else {
            panic!("{events:?}")
        };
        assert_eq!(outcome.agent, "claude");
        assert_eq!(
            outcome.summary.action_items[0].assignee.as_deref(),
            Some("Dana")
        );
        let md = std::fs::read_to_string(outcome.path.as_ref().unwrap()).unwrap();
        assert!(md.contains("A short test meeting."));
        assert!(
            sb.claude_stdin(0).contains("Dana: u1"),
            "{}",
            sb.claude_stdin(0)
        );

        let saved = h.saved();
        assert_eq!(saved.metadata.status, MeetingStatus::Completed);
        assert_eq!(saved.metadata.summary.unwrap().chapters[0].title, "Start");
    }

    #[tokio::test]
    async fn a_failing_agent_still_stops_cleanly() {
        let sb = assistant::testing::Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        std::fs::write(sb.out().join("claude.fail"), "").unwrap();
        let mut h = harness().await;
        h.daemon.assistant = Some(sb.settings());
        h.pair(&[(1.0, 3.0)], &[(5.0, 7.0)]).await;

        let (ran, events) = stop_and_summarize(&mut h).await;
        assert!(ran);
        assert!(matches!(events[0], SummaryEvent::Summarizing { .. }));
        match &events[1] {
            SummaryEvent::Failed {
                code,
                message,
                backend,
                ..
            } => {
                assert_eq!(*code, "agent_failed");
                assert!(message.contains("Not logged in"), "{message}");
                assert_eq!(backend.as_deref(), Some("Claude"));
            }
            other => panic!("{other:?}"),
        }
        assert!(h.daemon.state().is_idle());
        let saved = h.saved();
        assert_eq!(saved.transcript.segments.len(), 2);
        assert_eq!(saved.metadata.status, MeetingStatus::Completed);
        assert!(saved.metadata.summary.is_none());
        assert!(!saved
            .metadata
            .storage_path
            .unwrap()
            .join(assistant::summarize::SUMMARY_FILE)
            .exists());
    }

    #[tokio::test]
    async fn auto_summary_waits_for_consent_and_honors_auto_false() {
        let sb = assistant::testing::Sandbox::new();
        sb.fake_claude();
        sb.claude_replies(&[], SUMMARY_REPLY);
        let mut h = harness().await;
        h.daemon.assistant = Some(sb.settings());
        h.pair(&[(1.0, 3.0)], &[]).await;
        let (ran, events) = stop_and_summarize(&mut h).await;
        assert!(!ran, "no consent, auto unset: off");
        assert!(events.is_empty());
        assert_eq!(sb.claude_calls(), 0);

        let mut h = harness().await;
        sb.allow("claude");
        let mut settings = sb.settings();
        settings.summary_auto = Some(false);
        h.daemon.assistant = Some(settings);
        let id = h.daemon.stop().await.unwrap();
        assert!(h.daemon.auto_summary(id).is_none());
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
