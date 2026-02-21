//! Speech-to-text transcription module
//!
//! Provides transcription via:
//! - Local whisper.cpp inference (whisper-rs crate)
//! - Remote OpenAI-compatible Whisper API (whisper.cpp server, OpenAI, etc.)
//! - CLI subprocess using whisper-cli (fallback for glibc 2.42+ compatibility)
//! - Subprocess isolation for GPU memory release
//! - Optionally NVIDIA Parakeet via ONNX Runtime (when `parakeet` feature is enabled)
//! - Optionally Moonshine via ONNX Runtime (when `moonshine` feature is enabled)
//! - Optionally SenseVoice via ONNX Runtime (when `sensevoice` feature is enabled)
//! - Optionally Paraformer via ONNX Runtime (when `paraformer` feature is enabled)
//! - Optionally Dolphin via ONNX Runtime (when `dolphin` feature is enabled)
//! - Optionally Omnilingual via ONNX Runtime (when `omnilingual` feature is enabled)

pub mod cli;
pub mod remote;
pub mod subprocess;
pub mod whisper;
pub mod worker;

/// Shared log-mel filterbank feature extraction for ONNX-based ASR engines
#[cfg(any(
    feature = "sensevoice",
    feature = "paraformer",
    feature = "dolphin",
    feature = "omnilingual",
))]
pub mod fbank;

/// Shared CTC greedy decoder for CTC-based ASR engines
#[cfg(any(
    feature = "sensevoice",
    feature = "paraformer",
    feature = "dolphin",
    feature = "omnilingual",
))]
pub mod ctc;

#[cfg(feature = "parakeet")]
pub mod parakeet;

#[cfg(feature = "moonshine")]
pub mod moonshine;

#[cfg(feature = "sensevoice")]
pub mod sensevoice;

#[cfg(feature = "paraformer")]
pub mod paraformer;

#[cfg(feature = "dolphin")]
pub mod dolphin;

#[cfg(feature = "omnilingual")]
pub mod omnilingual;

use crate::config::{Config, TranscriptionEngine, WhisperConfig, WhisperMode};
use crate::error::TranscribeError;
use crate::setup::gpu;

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

/// Factory function to create transcriber based on configured engine
pub fn create_transcriber(config: &Config) -> Result<Box<dyn Transcriber>, TranscribeError> {
    match config.engine {
        TranscriptionEngine::Whisper => create_whisper_transcriber(&config.whisper),
        #[cfg(feature = "parakeet")]
        TranscriptionEngine::Parakeet => {
            let parakeet_config = config.parakeet.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Parakeet engine selected but [parakeet] config section is missing".to_string(),
                )
            })?;
            Ok(Box::new(parakeet::ParakeetTranscriber::new(
                parakeet_config,
            )?))
        }
        #[cfg(not(feature = "parakeet"))]
        TranscriptionEngine::Parakeet => Err(TranscribeError::InitFailed(
            "Parakeet engine requested but voxtype was not compiled with --features parakeet"
                .to_string(),
        )),
        #[cfg(feature = "moonshine")]
        TranscriptionEngine::Moonshine => {
            let moonshine_config = config.moonshine.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Moonshine engine selected but [moonshine] config section is missing"
                        .to_string(),
                )
            })?;
            Ok(Box::new(moonshine::MoonshineTranscriber::new(
                moonshine_config,
            )?))
        }
        #[cfg(not(feature = "moonshine"))]
        TranscriptionEngine::Moonshine => Err(TranscribeError::InitFailed(
            "Moonshine engine requested but voxtype was not compiled with --features moonshine"
                .to_string(),
        )),
        #[cfg(feature = "sensevoice")]
        TranscriptionEngine::SenseVoice => {
            let sensevoice_config = config.sensevoice.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "SenseVoice engine selected but [sensevoice] config section is missing"
                        .to_string(),
                )
            })?;
            Ok(Box::new(sensevoice::SenseVoiceTranscriber::new(
                sensevoice_config,
            )?))
        }
        #[cfg(not(feature = "sensevoice"))]
        TranscriptionEngine::SenseVoice => Err(TranscribeError::InitFailed(
            "SenseVoice engine requested but voxtype was not compiled with --features sensevoice"
                .to_string(),
        )),
        #[cfg(feature = "paraformer")]
        TranscriptionEngine::Paraformer => {
            let cfg = config.paraformer.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Paraformer engine selected but [paraformer] config section is missing"
                        .to_string(),
                )
            })?;
            Ok(Box::new(paraformer::ParaformerTranscriber::new(cfg)?))
        }
        #[cfg(not(feature = "paraformer"))]
        TranscriptionEngine::Paraformer => Err(TranscribeError::InitFailed(
            "Paraformer engine requested but voxtype was not compiled with --features paraformer"
                .to_string(),
        )),
        #[cfg(feature = "dolphin")]
        TranscriptionEngine::Dolphin => {
            let cfg = config.dolphin.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Dolphin engine selected but [dolphin] config section is missing".to_string(),
                )
            })?;
            Ok(Box::new(dolphin::DolphinTranscriber::new(cfg)?))
        }
        #[cfg(not(feature = "dolphin"))]
        TranscriptionEngine::Dolphin => Err(TranscribeError::InitFailed(
            "Dolphin engine requested but voxtype was not compiled with --features dolphin"
                .to_string(),
        )),
        #[cfg(feature = "omnilingual")]
        TranscriptionEngine::Omnilingual => {
            let cfg = config.omnilingual.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Omnilingual engine selected but [omnilingual] config section is missing"
                        .to_string(),
                )
            })?;
            Ok(Box::new(omnilingual::OmnilingualTranscriber::new(cfg)?))
        }
        #[cfg(not(feature = "omnilingual"))]
        TranscriptionEngine::Omnilingual => Err(TranscribeError::InitFailed(
            "Omnilingual engine requested but voxtype was not compiled with --features omnilingual"
                .to_string(),
        )),
    }
}

/// Factory function to create Whisper transcriber (local or remote)
pub fn create_whisper_transcriber(
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
    // Apply GPU selection from VOXTYPE_VULKAN_DEVICE environment variable
    // This sets VK_LOADER_DRIVERS_SELECT to filter Vulkan drivers
    if let Some(vendor) = gpu::apply_gpu_selection() {
        tracing::info!(
            "GPU selection: {} (via VOXTYPE_VULKAN_DEVICE)",
            vendor.display_name()
        );
    }

    match config.effective_mode() {
        WhisperMode::Local => {
            if config.gpu_isolation {
                tracing::info!(
                    "Using subprocess-isolated whisper transcription (gpu_isolation=true)"
                );
                Ok(Box::new(subprocess::SubprocessTranscriber::new(
                    config,
                    config_path,
                )?))
            } else {
                tracing::info!("Using local whisper transcription mode");
                Ok(Box::new(whisper::WhisperTranscriber::new(config)?))
            }
        }
        WhisperMode::Remote => {
            tracing::info!("Using remote whisper transcription mode");
            Ok(Box::new(remote::RemoteTranscriber::new(config)?))
        }
        WhisperMode::Cli => {
            tracing::info!("Using whisper-cli subprocess backend");
            Ok(Box::new(cli::CliTranscriber::new(config)?))
        }
    }
}
