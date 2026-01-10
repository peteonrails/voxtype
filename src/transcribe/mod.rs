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
pub fn create_transcriber(config: &WhisperConfig) -> Result<Box<dyn Transcriber>, TranscribeError> {
    tracing::info!(
        "Creating transcriber: backend={:?}, model={}, retry_model={:?}",
        config.backend,
        config.model,
        config.retry_model
    );
    
    match config.backend {
        WhisperBackend::Local => {
            if let Some(ref retry_model) = config.retry_model {
                tracing::info!(
                    "Hybrid mode enabled: using hybrid transcription with primary={} and retry={}",
                    config.model,
                    retry_model
                );
                Ok(Box::new(whisper::HybridTranscriber::new(config)?))
            } else {
                tracing::info!(
                    "Single model mode: using local whisper transcription backend with model={}",
                    config.model
                );
                Ok(Box::new(whisper::WhisperTranscriber::new(config)?))
            }
        }
        WhisperBackend::Remote => {
            tracing::info!("Using remote whisper transcription backend");
            Ok(Box::new(remote::RemoteTranscriber::new(config)?))
        }
    }
}
