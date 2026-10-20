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

/// A ready voice activity detector and the backend actually running it.
pub struct VadInit {
    pub detector: Box<dyn VoiceActivityDetector>,
    /// Never `Auto`: the backend `auto` resolved to, or the one it fell back to.
    pub backend: VadBackend,
}

/// The backend a config asks for, with `auto` resolved by engine: Whisper VAD
/// for the Whisper engine, Energy VAD for every other engine.
pub fn preferred_backend(config: &Config) -> VadBackend {
    match config.vad.backend {
        VadBackend::Auto => match config.engine {
            TranscriptionEngine::Whisper => VadBackend::Whisper,
            TranscriptionEngine::Parakeet
            | TranscriptionEngine::Moonshine
            | TranscriptionEngine::SenseVoice
            | TranscriptionEngine::Paraformer
            | TranscriptionEngine::Dolphin
            | TranscriptionEngine::Omnilingual
            | TranscriptionEngine::Cohere
            | TranscriptionEngine::Soniox
            | TranscriptionEngine::OpenVino => VadBackend::Energy,
        },
        explicit => explicit,
    }
}

/// Create a VAD instance based on configuration
///
/// Returns None if VAD is disabled, or Err if the configured backend cannot
/// start. With `backend = "auto"`, a Whisper VAD that cannot start (model
/// missing or unloadable) falls back to Energy VAD, which needs no model, so
/// silence is still filtered. An explicit `backend = "whisper"` never falls
/// back: the user chose that backend, and the caller reports the error.
pub fn create_vad(config: &Config) -> Result<Option<VadInit>, VadError> {
    if !config.vad.enabled {
        return Ok(None);
    }

    let init = match preferred_backend(config) {
        VadBackend::Whisper => match create_whisper_vad(&config.vad) {
            Ok(detector) => VadInit {
                detector,
                backend: VadBackend::Whisper,
            },
            Err(e) if config.vad.backend == VadBackend::Auto => {
                tracing::warn!(
                    "Whisper VAD unavailable, [vad] backend = \"auto\" falls back to Energy VAD: {}",
                    e
                );
                energy_vad(&config.vad)
            }
            Err(e) => return Err(e),
        },
        VadBackend::Energy | VadBackend::Auto => energy_vad(&config.vad),
    };

    Ok(Some(init))
}

fn energy_vad(config: &crate::config::VadConfig) -> VadInit {
    tracing::info!("Using Energy VAD backend");
    VadInit {
        detector: Box::new(EnergyVad::new(config)),
        backend: VadBackend::Energy,
    }
}

fn create_whisper_vad(
    config: &crate::config::VadConfig,
) -> Result<Box<dyn VoiceActivityDetector>, VadError> {
    let model_path = resolve_whisper_vad_model_path(config)?;
    tracing::info!("Using Whisper VAD backend with model {:?}", model_path);
    Ok(Box::new(WhisperVad::new(&model_path, config)?))
}

/// Notification title and body for a VAD that was enabled but could not
/// start: which model failed, what that means, and what fixes it.
pub fn unavailable_notice(
    config: &crate::config::VadConfig,
    error: &VadError,
) -> (&'static str, String) {
    const CONSEQUENCE: &str =
        "Silence and noise now go straight to transcription and can come out as made-up text.";
    let body = match (error, config.model.as_deref()) {
        (VadError::ModelNotFound(_), Some(model)) => format!(
            "The Silero VAD model set in [vad] model was not found: {}. {} \
             Fix the path, or remove [vad] model and run `voxtype setup vad`.",
            model, CONSEQUENCE
        ),
        (VadError::ModelNotFound(_), None) => format!(
            "The Silero VAD model is not installed ({}). {} \
             Run `voxtype setup vad`, or set [vad] backend = \"energy\" (no model needed).",
            default_whisper_vad_model_path().display(),
            CONSEQUENCE
        ),
        (_, Some(model)) => format!(
            "The Silero VAD model {} failed to load ({}). {} \
             Replace it with a valid ggml Silero model, or set [vad] backend = \"energy\".",
            model, error, CONSEQUENCE
        ),
        (_, None) => format!(
            "The Silero VAD model {} failed to load ({}). {} \
             Delete it and run `voxtype setup vad` to download it again, \
             or set [vad] backend = \"energy\".",
            default_whisper_vad_model_path().display(),
            error,
            CONSEQUENCE
        ),
    };
    ("Voxtype: voice activity detection is off", body)
}

fn default_whisper_vad_model_path() -> PathBuf {
    Config::models_dir().join(get_whisper_vad_model_filename())
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

    let model_path = default_whisper_vad_model_path();
    if model_path.exists() {
        Ok(model_path)
    } else {
        // ModelNotFound's message already names `voxtype setup vad`.
        Err(VadError::ModelNotFound(model_path.display().to_string()))
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

    fn whisper_vad_config(backend: VadBackend) -> Config {
        let mut config = Config::default();
        config.vad.enabled = true;
        config.vad.backend = backend;
        config.engine = TranscriptionEngine::Whisper;
        config.vad.model = Some("/nonexistent/voxtype-test/ggml-silero-vad.bin".to_string());
        config
    }

    #[test]
    fn test_preferred_backend_resolves_auto_by_engine() {
        let mut config = Config::default();
        config.vad.backend = VadBackend::Auto;
        config.engine = TranscriptionEngine::Whisper;
        assert_eq!(preferred_backend(&config), VadBackend::Whisper);
        config.engine = TranscriptionEngine::Parakeet;
        assert_eq!(preferred_backend(&config), VadBackend::Energy);
        config.vad.backend = VadBackend::Whisper;
        assert_eq!(preferred_backend(&config), VadBackend::Whisper);
    }

    #[test]
    fn test_auto_falls_back_to_energy_when_whisper_vad_model_missing() {
        let config = whisper_vad_config(VadBackend::Auto);
        let init = create_vad(&config).unwrap().unwrap();
        assert_eq!(init.backend, VadBackend::Energy);
    }

    #[test]
    fn test_explicit_whisper_backend_does_not_fall_back() {
        let config = whisper_vad_config(VadBackend::Whisper);
        assert!(matches!(
            create_vad(&config),
            Err(VadError::ModelNotFound(_))
        ));
    }

    #[test]
    fn test_unavailable_notice_default_model_names_setup_command() {
        let config = crate::config::VadConfig::default();
        let error = VadError::ModelNotFound("x".to_string());
        let (title, body) = unavailable_notice(&config, &error);
        assert!(title.contains("voice activity detection is off"));
        assert!(body.contains("ggml-silero-vad.bin"));
        assert!(body.contains("`voxtype setup vad`"));
        assert!(body.contains("backend = \"energy\""));
    }

    #[test]
    fn test_unavailable_notice_custom_model_names_the_path() {
        let config = whisper_vad_config(VadBackend::Whisper).vad;
        let error = VadError::ModelNotFound("x".to_string());
        let (_, body) = unavailable_notice(&config, &error);
        assert!(body.contains("/nonexistent/voxtype-test/ggml-silero-vad.bin"));
        assert!(body.contains("[vad] model"));

        let error = VadError::InitFailed("bad magic".to_string());
        let (_, body) = unavailable_notice(&config, &error);
        assert!(body.contains("failed to load"));
        assert!(body.contains("bad magic"));
    }

    #[test]
    fn test_whisper_vad_model_url() {
        let url = get_whisper_vad_model_url();
        assert!(url.contains("huggingface"));
        assert!(url.contains("silero"));
    }
}
