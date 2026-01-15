//! Speech-to-text transcription module
//!
//! Provides transcription via:
//! - Local whisper.cpp inference (whisper-rs crate)
//! - Remote OpenAI-compatible Whisper API (whisper.cpp server, OpenAI, etc.)
//! - Subprocess isolation for GPU memory release

pub mod remote;
pub mod subprocess;
pub mod whisper;
pub mod worker;

use crate::config::{WhisperBackend, WhisperConfig};
use crate::error::TranscribeError;

/// Trait for speech-to-text implementations
pub trait Transcriber: Send + Sync {
    /// Transcribe audio samples to text
    /// Input: f32 samples, mono, 16kHz
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError>;

    /// Prepare for transcription (optional, called when recording starts)
    ///
    /// For subprocess-based transcribers, this spawns the worker process
    /// and begins loading the model while the user is still speaking.
    /// This hides model loading latency behind recording time.
    ///
    /// Default implementation does nothing (for transcribers that don't
    /// benefit from preparation, like those with preloaded models).
    fn prepare(&self) {
        // Default: no-op
    }
}

/// Factory function to create transcriber based on configured backend
pub fn create_transcriber(
    config: &WhisperConfig,
) -> Result<Box<dyn Transcriber>, TranscribeError> {
    create_transcriber_with_config_path(config, None)
}

/// Factory function to create transcriber with optional config path
/// The config path is passed to subprocess transcriber for isolated GPU execution
pub fn create_transcriber_with_config_path(
    config: &WhisperConfig,
    config_path: Option<std::path::PathBuf>,
) -> Result<Box<dyn Transcriber>, TranscribeError> {
    match config.backend {
        WhisperBackend::Local => {
            if config.gpu_isolation {
                tracing::info!("Using subprocess-isolated whisper transcription (gpu_isolation=true)");
                Ok(Box::new(subprocess::SubprocessTranscriber::new(
                    config,
                    config_path,
                )?))
            } else {
                tracing::info!("Using local whisper transcription backend");
                Ok(Box::new(whisper::WhisperTranscriber::new(config)?))
            }
        }
        WhisperBackend::Remote => {
            tracing::info!("Using remote whisper transcription backend");
            Ok(Box::new(remote::RemoteTranscriber::new(config)?))
        }
    }
}
