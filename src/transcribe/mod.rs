//! Speech-to-text transcription module
//!
//! Provides transcription via:
//! - Local whisper.cpp inference (whisper-rs crate)
//! - Remote OpenAI-compatible Whisper API (whisper.cpp server, OpenAI, etc.)
//! - Optionally NVIDIA Parakeet via ONNX Runtime (when `parakeet` feature is enabled)

pub mod remote;
pub mod whisper;

#[cfg(feature = "parakeet")]
pub mod parakeet;

use crate::config::{Config, TranscriptionBackend, WhisperBackend, WhisperConfig};
use crate::error::TranscribeError;

/// Trait for speech-to-text implementations
pub trait Transcriber: Send + Sync {
    /// Transcribe audio samples to text
    /// Input: f32 samples, mono, 16kHz
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError>;
}

/// Factory function to create transcriber based on configured backend
pub fn create_transcriber(config: &Config) -> Result<Box<dyn Transcriber>, TranscribeError> {
    match config.backend {
        TranscriptionBackend::Whisper => create_whisper_transcriber(&config.whisper),
        #[cfg(feature = "parakeet")]
        TranscriptionBackend::Parakeet => {
            let parakeet_config = config.parakeet.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Parakeet backend selected but [parakeet] config section is missing".to_string(),
                )
            })?;
            Ok(Box::new(parakeet::ParakeetTranscriber::new(parakeet_config)?))
        }
        #[cfg(not(feature = "parakeet"))]
        TranscriptionBackend::Parakeet => Err(TranscribeError::InitFailed(
            "Parakeet backend requested but voxtype was not compiled with --features parakeet"
                .to_string(),
        )),
    }
}

/// Factory function to create Whisper transcriber (local or remote)
pub fn create_whisper_transcriber(
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
