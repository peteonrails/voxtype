//! Voice Activity Detection module
//!
//! Provides VAD to filter silence-only recordings before transcription,
//! preventing Whisper hallucinations when processing silence.
//!
//! Two backends are available:
//! - **Energy VAD**: Simple RMS-based detection, no model needed, fast
//! - **Whisper VAD**: Silero model via whisper-rs, more accurate, requires model download

mod energy;
mod whisper_vad;

use crate::config::{Config, TranscriptionEngine, VadBackend};
use crate::error::VadError;
use std::path::PathBuf;

pub use energy::EnergyVad;
pub use whisper_vad::WhisperVad;

/// Result of voice activity detection
#[derive(Debug, Clone)]
pub struct VadResult {
    /// Whether speech was detected in the audio
    pub has_speech: bool,
    /// Estimated duration of speech in seconds
    pub speech_duration_secs: f32,
    /// Ratio of speech to total audio duration (0.0 - 1.0)
    pub speech_ratio: f32,
    /// RMS energy level of the audio (for debugging)
    pub rms_energy: f32,
}

/// Trait for voice activity detection implementations
pub trait VoiceActivityDetector: Send + Sync {
    /// Detect speech in audio samples
    ///
    /// # Arguments
    /// * `samples` - Audio samples at 16kHz mono (f32, normalized to -1.0 to 1.0)
    ///
    /// # Returns
    /// * `Ok(VadResult)` - Detection result with speech metrics
    /// * `Err(VadError)` - If detection fails
    fn detect(&self, samples: &[f32]) -> Result<VadResult, VadError>;
}

/// Create a VAD instance based on configuration
///
/// Returns None if VAD is disabled, or Err if initialization fails
pub fn create_vad(config: &Config) -> Result<Option<Box<dyn VoiceActivityDetector>>, VadError> {
    if !config.vad.enabled {
        return Ok(None);
    }

    // Determine which backend to use
    let backend = match config.vad.backend {
        VadBackend::Auto => {
            // Auto-select: Whisper VAD for Whisper engine, Energy for Parakeet
            match config.engine {
                TranscriptionEngine::Whisper => VadBackend::Whisper,
                TranscriptionEngine::Parakeet => VadBackend::Energy,
            }
        }
        explicit => explicit,
    };

    let vad: Box<dyn VoiceActivityDetector> = match backend {
        VadBackend::Energy | VadBackend::Auto => {
            tracing::info!("Using Energy VAD backend");
            Box::new(EnergyVad::new(&config.vad))
        }
        VadBackend::Whisper => {
            let model_path = resolve_whisper_vad_model_path(&config.vad)?;
            tracing::info!("Using Whisper VAD backend with model {:?}", model_path);
            Box::new(WhisperVad::new(&model_path, &config.vad)?)
        }
    };

    Ok(Some(vad))
}

/// Resolve the path to the Whisper VAD model
fn resolve_whisper_vad_model_path(config: &crate::config::VadConfig) -> Result<PathBuf, VadError> {
    // If model path is explicitly configured, use it
    if let Some(ref model) = config.model {
        let path = PathBuf::from(model);
        if path.exists() {
            return Ok(path);
        }
        return Err(VadError::ModelNotFound(model.clone()));
    }

    // Use default model location
    let models_dir = Config::models_dir();
    let model_path = models_dir.join("ggml-silero-vad.bin");

    if model_path.exists() {
        Ok(model_path)
    } else {
        Err(VadError::ModelNotFound(format!(
            "{}. Download with: voxtype setup vad",
            model_path.display()
        )))
    }
}

/// Get the download URL for the Whisper VAD model
pub fn get_whisper_vad_model_url() -> &'static str {
    "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin"
}

/// Get the default VAD model filename
pub fn get_whisper_vad_model_filename() -> &'static str {
    "ggml-silero-vad.bin"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vad_result_defaults() {
        let result = VadResult {
            has_speech: false,
            speech_duration_secs: 0.0,
            speech_ratio: 0.0,
            rms_energy: 0.0,
        };
        assert!(!result.has_speech);
        assert_eq!(result.speech_duration_secs, 0.0);
        assert_eq!(result.speech_ratio, 0.0);
    }

    #[test]
    fn test_create_vad_disabled() {
        let config = Config::default();
        // VAD is disabled by default
        assert!(!config.vad.enabled);
        let vad = create_vad(&config).unwrap();
        assert!(vad.is_none());
    }

    #[test]
    fn test_create_vad_energy_backend() {
        let mut config = Config::default();
        config.vad.enabled = true;
        config.vad.backend = VadBackend::Energy;
        let vad = create_vad(&config).unwrap();
        assert!(vad.is_some());
    }

    #[test]
    fn test_create_vad_auto_parakeet_uses_energy() {
        let mut config = Config::default();
        config.vad.enabled = true;
        config.vad.backend = VadBackend::Auto;
        config.engine = TranscriptionEngine::Parakeet;
        // Auto + Parakeet = Energy (no model needed)
        let vad = create_vad(&config).unwrap();
        assert!(vad.is_some());
    }

    #[test]
    fn test_whisper_vad_model_url() {
        let url = get_whisper_vad_model_url();
        assert!(url.contains("huggingface"));
        assert!(url.contains("silero"));
    }
}
