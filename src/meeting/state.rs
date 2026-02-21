//! State machine for meeting transcription mode
//!
//! Defines the states for continuous meeting recording:
//! Idle -> Active -> Paused -> Active -> Finalizing -> Idle

use std::time::Instant;

/// State of an individual audio chunk being processed
#[derive(Debug, Clone)]
pub enum ChunkState {
    /// Recording audio for this chunk
    Recording {
        /// When this chunk started recording
        started_at: Instant,
    },
    /// Processing this chunk (transcribing)
    Processing {
        /// Sequential ID of this chunk
        chunk_id: u32,
    },
}

impl ChunkState {
    /// Check if this chunk is currently recording
    pub fn is_recording(&self) -> bool {
        matches!(self, ChunkState::Recording { .. })
    }

    /// Get the recording duration if currently recording
    pub fn recording_duration(&self) -> Option<std::time::Duration> {
        match self {
            ChunkState::Recording { started_at } => Some(started_at.elapsed()),
            _ => None,
        }
    }
}

/// Meeting transcription state
#[derive(Debug, Clone)]
#[derive(Default)]
pub enum MeetingState {
    /// No meeting in progress
    #[default]
    Idle,

    /// Meeting is actively recording
    Active {
        /// When the meeting started
        started_at: Instant,
        /// Current chunk being processed
        current_chunk: ChunkState,
        /// Number of chunks processed so far
        chunks_processed: u32,
    },

    /// Meeting is temporarily paused
    Paused {
        /// When the meeting started
        started_at: Instant,
        /// When the meeting was paused
        paused_at: Instant,
        /// Number of chunks processed before pause
        chunks_processed: u32,
    },

    /// Meeting has ended, finalizing (processing last chunk, saving)
    Finalizing {
        /// When the meeting started
        started_at: Instant,
        /// When the meeting was stopped
        ended_at: Instant,
        /// Total chunks processed
        total_chunks: u32,
    },
}


impl MeetingState {
    /// Create a new idle state
    pub fn new() -> Self {
        MeetingState::Idle
    }

    /// Check if in idle state
    pub fn is_idle(&self) -> bool {
        matches!(self, MeetingState::Idle)
    }

    /// Check if actively recording
    pub fn is_active(&self) -> bool {
        matches!(self, MeetingState::Active { .. })
    }

    /// Check if paused
    pub fn is_paused(&self) -> bool {
        matches!(self, MeetingState::Paused { .. })
    }

    /// Check if finalizing
    pub fn is_finalizing(&self) -> bool {
        matches!(self, MeetingState::Finalizing { .. })
    }

    /// Get meeting duration (including paused time)
    pub fn meeting_duration(&self) -> Option<std::time::Duration> {
        match self {
            MeetingState::Idle => None,
            MeetingState::Active { started_at, .. } => Some(started_at.elapsed()),
            MeetingState::Paused { started_at, .. } => Some(started_at.elapsed()),
            MeetingState::Finalizing {
                started_at,
                ended_at,
                ..
            } => Some(ended_at.duration_since(*started_at)),
        }
    }

    /// Alias for meeting_duration - time elapsed since meeting started
    pub fn elapsed(&self) -> Option<std::time::Duration> {
        self.meeting_duration()
    }

    /// Get number of chunks processed
    pub fn chunks_processed(&self) -> u32 {
        match self {
            MeetingState::Idle => 0,
            MeetingState::Active {
                chunks_processed, ..
            } => *chunks_processed,
            MeetingState::Paused {
                chunks_processed, ..
            } => *chunks_processed,
            MeetingState::Finalizing { total_chunks, .. } => *total_chunks,
        }
    }

    /// Start a new meeting
    pub fn start() -> Self {
        let now = Instant::now();
        MeetingState::Active {
            started_at: now,
            current_chunk: ChunkState::Recording { started_at: now },
            chunks_processed: 0,
        }
    }

    /// Pause the current meeting (only valid from Active state)
    pub fn pause(self) -> Self {
        match self {
            MeetingState::Active {
                started_at,
                chunks_processed,
                ..
            } => MeetingState::Paused {
                started_at,
                paused_at: Instant::now(),
                chunks_processed,
            },
            other => other, // No-op for other states
        }
    }

    /// Resume a paused meeting (only valid from Paused state)
    pub fn resume(self) -> Self {
        match self {
            MeetingState::Paused {
                started_at,
                chunks_processed,
                ..
            } => MeetingState::Active {
                started_at,
                current_chunk: ChunkState::Recording {
                    started_at: Instant::now(),
                },
                chunks_processed,
            },
            other => other, // No-op for other states
        }
    }

    /// Stop the meeting and begin finalization (valid from Active or Paused)
    pub fn stop(self) -> Self {
        match self {
            MeetingState::Active {
                started_at,
                chunks_processed,
                ..
            } => MeetingState::Finalizing {
                started_at,
                ended_at: Instant::now(),
                total_chunks: chunks_processed,
            },
            MeetingState::Paused {
                started_at,
                chunks_processed,
                ..
            } => MeetingState::Finalizing {
                started_at,
                ended_at: Instant::now(),
                total_chunks: chunks_processed,
            },
            other => other, // No-op for idle/finalizing
        }
    }

    /// Complete finalization and return to idle
    pub fn finalize(self) -> Self {
        match self {
            MeetingState::Finalizing { .. } => MeetingState::Idle,
            other => other, // No-op for other states
        }
    }

    /// Advance to the next chunk (only valid in Active state)
    pub fn next_chunk(self) -> Self {
        match self {
            MeetingState::Active {
                started_at,
                chunks_processed,
                ..
            } => MeetingState::Active {
                started_at,
                current_chunk: ChunkState::Recording {
                    started_at: Instant::now(),
                },
                chunks_processed: chunks_processed + 1,
            },
            other => other,
        }
    }

    /// Mark current chunk as processing
    pub fn processing_chunk(self, chunk_id: u32) -> Self {
        match self {
            MeetingState::Active {
                started_at,
                chunks_processed,
                ..
            } => MeetingState::Active {
                started_at,
                current_chunk: ChunkState::Processing { chunk_id },
                chunks_processed,
            },
            other => other,
        }
    }
}

impl std::fmt::Display for MeetingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeetingState::Idle => write!(f, "Idle"),
            MeetingState::Active {
                started_at,
                chunks_processed,
                ..
            } => {
                write!(
                    f,
                    "Active ({:.0}m, {} chunks)",
                    started_at.elapsed().as_secs_f32() / 60.0,
                    chunks_processed
                )
            }
            MeetingState::Paused {
                paused_at,
                chunks_processed,
                ..
            } => {
                write!(
                    f,
                    "Paused ({:.0}s ago, {} chunks)",
                    paused_at.elapsed().as_secs_f32(),
                    chunks_processed
                )
            }
            MeetingState::Finalizing { total_chunks, .. } => {
                write!(f, "Finalizing ({} chunks)", total_chunks)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_new_state_is_idle() {
        let state = MeetingState::new();
        assert!(state.is_idle());
    }

    #[test]
    fn test_start_meeting() {
        let state = MeetingState::start();
        assert!(state.is_active());
        assert_eq!(state.chunks_processed(), 0);
    }

    #[test]
    fn test_pause_resume() {
        let state = MeetingState::start();
        let paused = state.pause();
        assert!(paused.is_paused());

        let resumed = paused.resume();
        assert!(resumed.is_active());
    }

    #[test]
    fn test_stop_meeting() {
        let state = MeetingState::start();
        let stopped = state.stop();
        assert!(stopped.is_finalizing());
    }

    #[test]
    fn test_finalize_meeting() {
        let state = MeetingState::start();
        let stopped = state.stop();
        let finalized = stopped.finalize();
        assert!(finalized.is_idle());
    }

    #[test]
    fn test_next_chunk() {
        let state = MeetingState::start();
        assert_eq!(state.chunks_processed(), 0);

        let state = state.next_chunk();
        assert_eq!(state.chunks_processed(), 1);

        let state = state.next_chunk();
        assert_eq!(state.chunks_processed(), 2);
    }

    #[test]
    fn test_meeting_duration() {
        let state = MeetingState::start();
        std::thread::sleep(Duration::from_millis(10));
        let duration = state.meeting_duration().unwrap();
        assert!(duration >= Duration::from_millis(10));
    }

    #[test]
    fn test_idle_has_no_duration() {
        let state = MeetingState::Idle;
        assert!(state.meeting_duration().is_none());
    }

    #[test]
    fn test_stop_from_paused() {
        let state = MeetingState::start();
        let state = state.next_chunk().next_chunk();
        let paused = state.pause();
        assert!(paused.is_paused());
        assert_eq!(paused.chunks_processed(), 2);

        let stopped = paused.stop();
        assert!(stopped.is_finalizing());
        assert_eq!(stopped.chunks_processed(), 2);
    }

    #[test]
    fn test_processing_chunk() {
        let state = MeetingState::start();
        let state = state.processing_chunk(0);
        assert!(state.is_active());
        if let MeetingState::Active { current_chunk, .. } = &state {
            assert!(!current_chunk.is_recording());
        } else {
            panic!("Expected Active state");
        }
    }

    #[test]
    fn test_pause_idle_is_noop() {
        let state = MeetingState::Idle;
        let state = state.pause();
        assert!(state.is_idle());
    }

    #[test]
    fn test_resume_idle_is_noop() {
        let state = MeetingState::Idle;
        let state = state.resume();
        assert!(state.is_idle());
    }

    #[test]
    fn test_stop_idle_is_noop() {
        let state = MeetingState::Idle;
        let state = state.stop();
        assert!(state.is_idle());
    }

    #[test]
    fn test_finalize_active_is_noop() {
        let state = MeetingState::start();
        let state = state.finalize();
        assert!(state.is_active());
    }

    #[test]
    fn test_next_chunk_paused_is_noop() {
        let state = MeetingState::start().pause();
        let state = state.next_chunk();
        assert!(state.is_paused());
    }

    #[test]
    fn test_display_trait() {
        let state = MeetingState::Idle;
        assert_eq!(format!("{}", state), "Idle");

        let state = MeetingState::start();
        let display = format!("{}", state);
        assert!(display.starts_with("Active"));
        assert!(display.contains("0 chunks"));
    }

    #[test]
    fn test_chunks_processed_in_paused() {
        let state = MeetingState::start().next_chunk().next_chunk().next_chunk();
        assert_eq!(state.chunks_processed(), 3);
        let paused = state.pause();
        assert_eq!(paused.chunks_processed(), 3);
    }

    #[test]
    fn test_meeting_duration_active() {
        let state = MeetingState::start();
        assert!(state.meeting_duration().is_some());
    }

    #[test]
    fn test_chunk_state_recording_duration() {
        let chunk = ChunkState::Recording {
            started_at: Instant::now(),
        };
        assert!(chunk.is_recording());
        assert!(chunk.recording_duration().is_some());
    }

    #[test]
    fn test_chunk_state_processing_no_duration() {
        let chunk = ChunkState::Processing { chunk_id: 5 };
        assert!(!chunk.is_recording());
        assert!(chunk.recording_duration().is_none());
    }

    #[test]
    fn test_default_is_idle() {
        let state = MeetingState::default();
        assert!(state.is_idle());
    }

    #[test]
    fn test_resume_active_is_noop() {
        let state = MeetingState::start();
        assert!(state.is_active());
        let state = state.resume();
        assert!(state.is_active());
    }

    #[test]
    fn test_pause_finalizing_is_noop() {
        let state = MeetingState::start().stop();
        assert!(state.is_finalizing());
        let state = state.pause();
        assert!(state.is_finalizing());
    }

    #[test]
    fn test_resume_finalizing_is_noop() {
        let state = MeetingState::start().stop();
        assert!(state.is_finalizing());
        let state = state.resume();
        assert!(state.is_finalizing());
    }

    #[test]
    fn test_stop_finalizing_is_noop() {
        let state = MeetingState::start().stop();
        assert!(state.is_finalizing());
        let state = state.stop();
        assert!(state.is_finalizing());
    }

    #[test]
    fn test_finalize_idle_is_noop() {
        let state = MeetingState::Idle;
        let state = state.finalize();
        assert!(state.is_idle());
    }

    #[test]
    fn test_finalize_paused_is_noop() {
        let state = MeetingState::start().pause();
        assert!(state.is_paused());
        let state = state.finalize();
        assert!(state.is_paused());
    }

    #[test]
    fn test_next_chunk_idle_is_noop() {
        let state = MeetingState::Idle;
        let state = state.next_chunk();
        assert!(state.is_idle());
    }

    #[test]
    fn test_next_chunk_finalizing_is_noop() {
        let state = MeetingState::start().stop();
        let chunks_before = state.chunks_processed();
        let state = state.next_chunk();
        assert!(state.is_finalizing());
        assert_eq!(state.chunks_processed(), chunks_before);
    }

    #[test]
    fn test_processing_chunk_idle_is_noop() {
        let state = MeetingState::Idle;
        let state = state.processing_chunk(0);
        assert!(state.is_idle());
    }

    #[test]
    fn test_processing_chunk_paused_is_noop() {
        let state = MeetingState::start().pause();
        let state = state.processing_chunk(0);
        assert!(state.is_paused());
    }

    #[test]
    fn test_full_lifecycle_with_chunks() {
        let state = MeetingState::start();
        assert!(state.is_active());
        assert_eq!(state.chunks_processed(), 0);

        let state = state.next_chunk().next_chunk().next_chunk();
        assert_eq!(state.chunks_processed(), 3);

        let state = state.pause();
        assert_eq!(state.chunks_processed(), 3);

        let state = state.resume();
        assert_eq!(state.chunks_processed(), 3);

        let state = state.next_chunk();
        assert_eq!(state.chunks_processed(), 4);

        let state = state.stop();
        assert!(state.is_finalizing());
        assert_eq!(state.chunks_processed(), 4);

        let state = state.finalize();
        assert!(state.is_idle());
        assert_eq!(state.chunks_processed(), 0);
    }

    #[test]
    fn test_elapsed_alias() {
        let state = MeetingState::Idle;
        assert!(state.elapsed().is_none());

        let state = MeetingState::start();
        assert!(state.elapsed().is_some());
    }

    #[test]
    fn test_display_paused() {
        let state = MeetingState::start().pause();
        let display = format!("{}", state);
        assert!(display.starts_with("Paused"));
    }

    #[test]
    fn test_display_finalizing() {
        let state = MeetingState::start().next_chunk().next_chunk().stop();
        let display = format!("{}", state);
        assert!(display.contains("Finalizing"));
        assert!(display.contains("2 chunks"));
    }
}
