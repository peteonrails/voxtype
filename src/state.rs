//! State machine for voxtype daemon
//!
//! Defines the states for the push-to-talk workflow:
//! Idle → Recording → Transcribing → Outputting → Idle

use std::time::Instant;

/// Audio samples collected during recording (f32, mono, 16kHz)
pub type AudioBuffer = Vec<f32>;

/// Application state
#[derive(Debug)]
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
        session: Box<crate::transcribe::streaming::StreamingSession>,
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
            State::EagerRecording { started_at, .. } => {
                write!(
                    f,
                    "Recording ({:.1}s, eager)",
                    started_at.elapsed().as_secs_f32()
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
            session: Box::new(crate::transcribe::streaming::StreamingSession::new(
                crate::transcribe::streaming::StreamingConfig::default(),
            )),
        };
        assert!(state.is_recording());
        assert!(state.is_eager_recording());
        assert!(!state.is_idle());
        assert!(state.recording_duration().is_some());
    }

    #[test]
    fn test_regular_recording_not_eager() {
        let state = State::Recording {
            started_at: Instant::now(),
            model_override: None,
        };
        assert!(state.is_recording());
        assert!(!state.is_eager_recording());
    }

    #[test]
    fn test_eager_recording_display() {
        let state = State::EagerRecording {
            started_at: Instant::now(),
            model_override: None,
            session: Box::new(crate::transcribe::streaming::StreamingSession::new(
                crate::transcribe::streaming::StreamingConfig::default(),
            )),
        };
        let display = format!("{}", state);
        assert!(display.contains("Recording"));
        assert!(display.contains("eager"));
    }
}
