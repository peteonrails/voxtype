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
pub mod export;
pub mod state;
pub mod storage;
pub mod summary;

pub use chunk::{ChunkBuffer, ChunkConfig, ChunkProcessor, ProcessedChunk, VoiceActivityDetector};
pub use data::{
    ActionItem, AudioSource, MeetingData, MeetingId, MeetingMetadata, MeetingStatus,
    MeetingSummary, Transcript, TranscriptSegment,
};
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
            diarization: None,
        }
    }
}

/// Events from the meeting daemon
#[derive(Debug)]
pub enum MeetingEvent {
    /// Meeting started
    Started { meeting_id: MeetingId },
    /// Chunk processed
    ChunkProcessed {
        chunk_id: u32,
        segments: Vec<TranscriptSegment>,
    },
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

        let transcriber: Arc<dyn Transcriber> =
            Arc::from(transcribe::create_transcriber(app_config)?);
        let engine_name = format!("{:?}", app_config.engine).to_lowercase();

        let post_processor = app_config.output.post_process.as_ref().map(|cfg| {
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
        })
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
        self.current_meeting = Some(meeting);
        self.state = MeetingState::start();

        let _ = self
            .event_tx
            .send(MeetingEvent::Started { meeting_id })
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
        if !self.state.is_active() {
            return Ok(None);
        }

        let Some(ref transcriber) = self.transcriber else {
            return Err(MeetingError::TranscriberNotInitialized.into());
        };

        let chunk_id = self.state.chunks_processed();
        let chunk_config = ChunkConfig {
            chunk_duration_secs: self.config.chunk_duration_secs,
            ..Default::default()
        };

        // Start offset is tracked per source: each source has its own wall-clock
        // timeline. Deriving this from transcript.duration_ms() would conflate
        // mic and loopback, pushing every new chunk past the other source's end
        // and roughly doubling apparent meeting length on dual-track captures.
        let start_offset_ms = *self.source_offsets.entry(source).or_insert(0);

        let mut processor = ChunkProcessor::new(chunk_config, transcriber.clone());
        let mut buffer = processor.new_buffer(chunk_id, source, start_offset_ms);
        buffer.add_samples(&samples);

        let mut result = processor
            .process_chunk(buffer)
            .map_err(crate::error::VoxtypeError::Transcribe)?;

        // Advance the per-source offset by the actual audio duration consumed,
        // regardless of whether VAD found speech in this chunk.
        if let Some(offset) = self.source_offsets.get_mut(&source) {
            *offset += result.audio_duration_ms;
        }

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
                self.last_chunk_text
                    .insert(source, last_seg.text.clone());
            }
        }

        // Run diarization on the transcribed segments
        if let Some(ref diarizer) = self.diarizer {
            if !result.segments.is_empty() {
                let diarized = diarizer.diarize(&samples, source, &result.segments);
                for (seg, diar) in result.segments.iter_mut().zip(diarized.iter()) {
                    seg.speaker_id = Some(diar.speaker.display_name());
                    seg.confidence = Some(diar.confidence);
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

        // Send event
        let _ = self
            .event_tx
            .send(MeetingEvent::ChunkProcessed {
                chunk_id,
                segments: result.segments.clone(),
            })
            .await;

        Ok(Some(result.segments))
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

    #[test]
    fn test_meeting_config_default() {
        let config = MeetingConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.chunk_duration_secs, 30);
        assert_eq!(config.max_duration_mins, 180);
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
