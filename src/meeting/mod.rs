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
use crate::transcribe::{self, Transcriber};
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
}

impl Default for MeetingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            chunk_duration_secs: 30,
            storage: StorageConfig::default(),
            retain_audio: false,
            max_duration_mins: 180,
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
    engine_name: String,
    event_tx: mpsc::Sender<MeetingEvent>,
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

        Ok(Self {
            config,
            state: MeetingState::Idle,
            storage,
            current_meeting: None,
            transcriber: Some(transcriber),
            engine_name,
            event_tx,
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

    /// Process a chunk of audio
    pub async fn process_chunk(
        &mut self,
        samples: Vec<f32>,
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

        // Calculate start offset
        let start_offset_ms = if let Some(ref meeting) = self.current_meeting {
            meeting.transcript.duration_ms()
        } else {
            0
        };

        let mut processor = ChunkProcessor::new(chunk_config, transcriber.clone());
        let mut buffer = processor.new_buffer(chunk_id, AudioSource::Microphone, start_offset_ms);
        buffer.add_samples(&samples);

        let result = processor
            .process_chunk(buffer)
            .map_err(crate::error::VoxtypeError::Transcribe)?;

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
