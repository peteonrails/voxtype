//! Voice Activity Detection (VAD) module
//!
//! Provides voice activity detection to filter silence-only recordings before transcription.
//! This prevents Whisper hallucinations when processing silence.
//!
//! Two backends are supported:
//! - Whisper VAD: Uses the built-in VAD from whisper-rs (for Whisper engine)
//! - Silero VAD: Uses the voice_activity_detector crate (for Parakeet engine)

mod whisper_vad;

#[cfg(feature = "parakeet")]
mod silero_vad;

use crate::config::{Config, TranscriptionEngine, VadConfig};
use crate::error::VadError;
use std::path::PathBuf;

/// Result of voice activity detection
#[derive(Debug, Clone)]
pub struct VadResult {
    /// Whether speech was detected in the audio
    pub has_speech: bool,

    /// Total duration of detected speech in seconds
    pub speech_duration_secs: f32,

    /// Ratio of speech to total audio duration (0.0 - 1.0)
    pub speech_ratio: f32,
}

/// Trait for voice activity detection implementations
pub trait VoiceActivityDetector: Send + Sync {
    /// Detect voice activity in audio samples
    ///
    /// # Arguments
    /// * `samples` - Audio samples at 16kHz mono (f32 normalized to [-1.0, 1.0])
    ///
    /// # Returns
    /// * `VadResult` containing speech detection results
    fn detect(&self, samples: &[f32]) -> Result<VadResult, VadError>;
}

/// Create a VAD instance based on configuration and transcription engine
pub fn create_vad(config: &Config) -> Result<Option<Box<dyn VoiceActivityDetector>>, VadError> {
    if !config.vad.enabled {
        return Ok(None);
    }

    let vad: Box<dyn VoiceActivityDetector> = match config.engine {
        TranscriptionEngine::Whisper => {
            let model_path = resolve_whisper_vad_model_path(&config.vad)?;
            Box::new(whisper_vad::WhisperVad::new(&model_path, &config.vad)?)
        }
        #[cfg(feature = "parakeet")]
        TranscriptionEngine::Parakeet => {
            let model_path = resolve_silero_vad_model_path(&config.vad)?;
            Box::new(silero_vad::SileroVad::new(&model_path, &config.vad)?)
        }
        #[cfg(not(feature = "parakeet"))]
        TranscriptionEngine::Parakeet => {
            return Err(VadError::InitFailed(
                "Parakeet VAD requires the 'parakeet' feature".to_string(),
            ));
        }
    };

    Ok(Some(vad))
}

/// Resolve the path to the Whisper VAD model
fn resolve_whisper_vad_model_path(config: &VadConfig) -> Result<PathBuf, VadError> {
    // If model path is explicitly configured, use it
    if let Some(ref model) = config.model {
        let path = PathBuf::from(model);
        if path.exists() {
            return Ok(path);
        }
        return Err(VadError::ModelNotFound(model.clone()));
    }

    // Use default model location
    let models_dir = crate::config::Config::models_dir();
    let model_path = models_dir.join("ggml-silero-vad.bin");

    if model_path.exists() {
        Ok(model_path)
    } else {
        Err(VadError::ModelNotFound(model_path.display().to_string()))
    }
}

/// Resolve the path to the Silero VAD ONNX model
/// Note: voice_activity_detector uses a bundled model, so this returns a dummy path
#[cfg(feature = "parakeet")]
fn resolve_silero_vad_model_path(_config: &VadConfig) -> Result<PathBuf, VadError> {
    // voice_activity_detector crate uses a bundled Silero model
    // No external model file is needed
    Ok(PathBuf::from("bundled"))
}

/// Get the default VAD model filename for the given transcription engine
pub fn get_default_model_filename(engine: TranscriptionEngine) -> &'static str {
    match engine {
        TranscriptionEngine::Whisper => "ggml-silero-vad.bin",
        TranscriptionEngine::Parakeet => "silero_vad.onnx",
    }
}

/// Get the download URL for the VAD model
pub fn get_vad_model_url(engine: TranscriptionEngine) -> &'static str {
    match engine {
        TranscriptionEngine::Whisper => {
            "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin"
        }
        TranscriptionEngine::Parakeet => {
            "https://github.com/snakers4/silero-vad/raw/master/files/silero_vad.onnx"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vad_result_default() {
        let result = VadResult {
            has_speech: false,
            speech_duration_secs: 0.0,
            speech_ratio: 0.0,
        };
        assert!(!result.has_speech);
        assert_eq!(result.speech_duration_secs, 0.0);
        assert_eq!(result.speech_ratio, 0.0);
    }

    #[test]
    fn test_get_default_model_filename() {
        assert_eq!(
            get_default_model_filename(TranscriptionEngine::Whisper),
            "ggml-silero-vad.bin"
        );
        assert_eq!(
            get_default_model_filename(TranscriptionEngine::Parakeet),
            "silero_vad.onnx"
        );
    }

    #[test]
    fn test_vad_disabled_returns_none() {
        let config = Config::default();
        // VAD is disabled by default
        assert!(!config.vad.enabled);
        let vad = create_vad(&config).unwrap();
        assert!(vad.is_none());
    }
}
