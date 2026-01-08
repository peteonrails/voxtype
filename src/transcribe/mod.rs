//! Speech-to-text transcription module
//!
//! Provides local transcription using whisper.cpp via the whisper-rs crate,
//! or optionally NVIDIA Parakeet via ONNX Runtime (when `parakeet` feature is enabled).

pub mod whisper;

#[cfg(feature = "parakeet")]
pub mod parakeet;

use crate::config::{Config, TranscriptionBackend};
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
        TranscriptionBackend::Whisper => {
            Ok(Box::new(whisper::WhisperTranscriber::new(&config.whisper)?))
        }
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

/// Factory function to create Whisper transcriber specifically
/// (for backwards compatibility with code that calls this directly)
pub fn create_whisper_transcriber(
    config: &crate::config::WhisperConfig,
) -> Result<Box<dyn Transcriber>, TranscribeError> {
    Ok(Box::new(whisper::WhisperTranscriber::new(config)?))
}
