//! Speech-to-text transcription module
//!
//! Provides transcription via:
//! - Local whisper.cpp inference (whisper-rs crate)
//! - Remote OpenAI-compatible Whisper API (whisper.cpp server, OpenAI, etc.)

pub mod remote;
pub mod whisper;

use crate::config::{WhisperBackend, WhisperConfig};
use crate::error::TranscribeError;

/// Trait for speech-to-text implementations
pub trait Transcriber: Send + Sync {
    /// Transcribe audio samples to text
    /// Input: f32 samples, mono, 16kHz
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError>;
}

/// Factory function to create transcriber based on configured backend
pub fn create_transcriber(
    config: &WhisperConfig,
) -> Result<Box<dyn Transcriber>, TranscribeError> {
    match config.backend {
        WhisperBackend::Local => {
            tracing::info!("Using local whisper transcription backend");
            Ok(Box::new(whisper::WhisperTranscriber::new(config)?))
        }
        WhisperBackend::Remote => {
            tracing::info!("Using remote whisper transcription backend");
            Ok(Box::new(remote::RemoteTranscriber::new(config)?))
        }
    }
}
