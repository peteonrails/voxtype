//! State machine for voxtype daemon
//!
//! Defines the states for the push-to-talk workflow:
//! Idle → Recording → Transcribing → Outputting → Idle

use std::time::Instant;

/// Audio samples collected during recording (f32, mono, 16kHz)
pub type AudioBuffer = Vec<f32>;

/// Result from transcribing a single chunk during eager processing
#[derive(Debug, Clone)]
pub struct ChunkResult {
    /// Transcribed text from this chunk
    pub text: String,
    /// Which chunk this result corresponds to (0-indexed)
    pub chunk_index: usize,
}

/// Application state
#[derive(Debug, Clone)]
pub enum State {
    /// Waiting for hotkey press
    Idle,

    /// Hotkey held, recording audio
    Recording {
        /// When recording started
        started_at: Instant,
        /// Optional model override for this recording
        model_override: Option<String>,
    },

    /// Hotkey held, recording audio with eager chunk processing
    EagerRecording {
        /// When recording started
        started_at: Instant,
        /// Optional model override for this recording
        model_override: Option<String>,
        /// Accumulated audio samples during recording
        accumulated_audio: AudioBuffer,
        /// Number of chunks already sent for transcription
        chunks_sent: usize,
        /// Results received from completed chunk transcriptions
        chunk_results: Vec<ChunkResult>,
        /// Number of transcription tasks currently in flight
        tasks_in_flight: usize,
    },

    /// Hotkey released, transcribing audio
    Transcribing {
        /// Recorded audio samples
        audio: AudioBuffer,
    },

    /// Transcription complete, outputting text
    Outputting {
        /// Transcribed text
        text: String,
    },
}

impl State {
    /// Create a new idle state
    pub fn new() -> Self {
        State::Idle
    }

    /// Check if in idle state
    pub fn is_idle(&self) -> bool {
        matches!(self, State::Idle)
    }

    /// Check if in recording state (normal or eager)
    pub fn is_recording(&self) -> bool {
        matches!(self, State::Recording { .. } | State::EagerRecording { .. })
    }

    /// Check if in eager recording state specifically
    pub fn is_eager_recording(&self) -> bool {
        matches!(self, State::EagerRecording { .. })
    }

    /// Get recording duration if currently recording (normal or eager)
    pub fn recording_duration(&self) -> Option<std::time::Duration> {
        match self {
            State::Recording { started_at, .. } | State::EagerRecording { started_at, .. } => {
                Some(started_at.elapsed())
            }
            _ => None,
        }
    }

    /// Get the number of chunks sent for transcription (eager mode only)
    pub fn eager_chunks_sent(&self) -> Option<usize> {
        match self {
            State::EagerRecording { chunks_sent, .. } => Some(*chunks_sent),
            _ => None,
        }
    }

    /// Get the number of transcription tasks currently in flight (eager mode only)
    pub fn eager_tasks_in_flight(&self) -> Option<usize> {
        match self {
            State::EagerRecording {
                tasks_in_flight, ..
            } => Some(*tasks_in_flight),
            _ => None,
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Idle => write!(f, "Idle"),
            State::Recording { started_at, .. } => {
                write!(f, "Recording ({:.1}s)", started_at.elapsed().as_secs_f32())
            }
            State::EagerRecording {
                started_at,
                chunks_sent,
                tasks_in_flight,
                ..
            } => {
                write!(
                    f,
                    "Recording ({:.1}s, {} chunks, {} pending)",
                    started_at.elapsed().as_secs_f32(),
                    chunks_sent,
                    tasks_in_flight
                )
            }
            State::Transcribing { audio } => {
                let duration = audio.len() as f32 / 16000.0;
                write!(f, "Transcribing ({:.1}s of audio)", duration)
            }
            State::Outputting { text } => {
                // Use chars() to handle multi-byte UTF-8 characters
                let preview = if text.chars().count() > 20 {
                    format!("{}...", text.chars().take(20).collect::<String>())
                } else {
                    text.clone()
                };
                write!(f, "Outputting: {:?}", preview)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_state_is_idle() {
        let state = State::new();
        assert!(state.is_idle());
    }

    #[test]
    fn test_recording_state() {
        let state = State::Recording {
            started_at: Instant::now(),
            model_override: None,
        };
        assert!(state.is_recording());
        assert!(!state.is_idle());
        assert!(state.recording_duration().is_some());
    }

    #[test]
    fn test_idle_has_no_duration() {
        let state = State::Idle;
        assert!(state.recording_duration().is_none());
    }

    #[test]
    fn test_state_display() {
        let state = State::Idle;
        assert_eq!(format!("{}", state), "Idle");

        let state = State::Recording {
            started_at: Instant::now(),
            model_override: None,
        };
        assert!(format!("{}", state).starts_with("Recording"));
    }

    #[test]
    fn test_eager_recording_state() {
        let state = State::EagerRecording {
            started_at: Instant::now(),
            model_override: None,
            accumulated_audio: vec![],
            chunks_sent: 2,
            chunk_results: vec![],
            tasks_in_flight: 1,
        };
        assert!(state.is_recording());
        assert!(state.is_eager_recording());
        assert!(!state.is_idle());
        assert!(state.recording_duration().is_some());
        assert_eq!(state.eager_chunks_sent(), Some(2));
        assert_eq!(state.eager_tasks_in_flight(), Some(1));
    }

    #[test]
    fn test_regular_recording_not_eager() {
        let state = State::Recording {
            started_at: Instant::now(),
            model_override: None,
        };
        assert!(state.is_recording());
        assert!(!state.is_eager_recording());
        assert_eq!(state.eager_chunks_sent(), None);
        assert_eq!(state.eager_tasks_in_flight(), None);
    }

    #[test]
    fn test_eager_recording_display() {
        let state = State::EagerRecording {
            started_at: Instant::now(),
            model_override: None,
            accumulated_audio: vec![],
            chunks_sent: 3,
            chunk_results: vec![],
            tasks_in_flight: 2,
        };
        let display = format!("{}", state);
        assert!(display.contains("Recording"));
        assert!(display.contains("3 chunks"));
        assert!(display.contains("2 pending"));
    }
}
