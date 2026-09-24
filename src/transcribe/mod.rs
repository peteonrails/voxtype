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
//! - Optionally OpenVINO Whisper for Intel NPU/CPU/GPU (when `openvino-whisper` feature is enabled)

pub mod cli;
#[cfg(feature = "parakeet")]
pub mod parakeet_streaming;
pub mod remote;
pub mod sliding_window;
pub mod soniox;
pub mod streaming;
pub mod subprocess;
pub mod whisper;
pub mod worker;

pub use sliding_window::{SlidingWindowConfig, SlidingWindowStreamingTranscriber};
pub use streaming::{SegmentId, StreamHandle, StreamingEvent, StreamingTranscriber};

/// Shared log-mel filterbank feature extraction for ONNX-based ASR engines
#[cfg(any(
    feature = "sensevoice",
    feature = "paraformer",
    feature = "dolphin",
    feature = "omnilingual",
    feature = "cohere",
))]
pub mod fbank;

/// Shared GPU execution-provider registration for ONNX-based engines.
#[cfg(feature = "onnx-common")]
pub mod onnx_ep;

/// Shared CTC greedy decoder for CTC-based ASR engines
#[cfg(any(
    feature = "sensevoice",
    feature = "paraformer",
    feature = "dolphin",
    feature = "omnilingual",
    feature = "cohere",
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

/// Cohere Transcribe backend (ONNX-based encoder-decoder ASR).
#[cfg(feature = "cohere")]
pub mod cohere;

/// Cohere-specific log-mel feature extractor (NeMo conventions, 128 mels).
#[cfg(feature = "cohere")]
pub mod cohere_fbank;

#[cfg(feature = "openvino-whisper")]
pub mod openvino_whisper;

#[cfg(feature = "openvino-whisper")]
use crate::config::OpenVinoConfig;
use crate::config::{Config, StreamingConfig, TranscriptionEngine, WhisperConfig, WhisperMode};
use crate::error::TranscribeError;
use crate::setup::gpu;
use std::sync::Arc;

/// A timed segment from transcription (word or sentence level)
#[derive(Debug, Clone)]
pub struct TimedSegment {
    pub text: String,
    /// Start time in seconds relative to the audio input
    pub start_secs: f32,
    /// End time in seconds relative to the audio input
    pub end_secs: f32,
}

/// Trait for speech-to-text implementations
pub trait Transcriber: Send + Sync {
    /// Transcribe audio samples to text
    /// Input: f32 samples, mono, 16kHz
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError>;

    /// Transcribe with word-level timestamps.
    /// Default implementation falls back to transcribe() with a single segment.
    fn transcribe_timed(&self, samples: &[f32]) -> Result<Vec<TimedSegment>, TranscribeError> {
        let text = self.transcribe(samples)?;
        let duration = samples.len() as f32 / 16000.0;
        if text.is_empty() {
            Ok(vec![])
        } else {
            Ok(vec![TimedSegment {
                text,
                start_secs: 0.0,
                end_secs: duration,
            }])
        }
    }

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

    /// Streaming-capable view of this transcriber, if it supports streaming.
    ///
    /// Returns `None` by default. Streaming-capable backends override this to
    /// return `Some(self)` (or some other implementor of [`StreamingTranscriber`]).
    /// The daemon consults this when `[transcribe] streaming = true` is set in
    /// config to decide between batch and streaming pipelines.
    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        None
    }

    /// Two-letter language code detected (or selected) for the most recent
    /// transcription, if the backend tracks it.
    ///
    /// This is used by output methods that benefit from a layout hint
    /// (notably [`crate::output::eitype::EitypeOutput`] and
    /// [`crate::output::dotool::DotoolOutput`]). It is set by backends with
    /// language auto-detection or explicit single-language mode; backends
    /// without language awareness return `None`.
    ///
    /// The default implementation returns `None`. Backends override this when
    /// they track the language used for the previous call to
    /// [`Self::transcribe`].
    fn last_detected_language(&self) -> Option<String> {
        None
    }
}

/// Factory function to create transcriber based on configured engine
pub fn create_transcriber(config: &Config) -> Result<Box<dyn Transcriber>, TranscribeError> {
    match config.engine {
        TranscriptionEngine::Whisper => {
            let transcriber = create_whisper_transcriber(&config.whisper)?;
            if config.whisper.streaming {
                if config.whisper.effective_mode() != WhisperMode::Local {
                    tracing::warn!(
                        "[whisper] streaming requires mode = \"local\"; ignoring streaming"
                    );
                    Ok(transcriber)
                } else {
                    Ok(Box::new(SlidingWindowStreamingTranscriber::new(
                        Arc::from(transcriber),
                        sliding_window_config_from_whisper(config),
                    )))
                }
            } else {
                Ok(transcriber)
            }
        }
        #[cfg(feature = "parakeet")]
        TranscriptionEngine::Parakeet => {
            let parakeet_config = config.parakeet.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Parakeet engine selected but [parakeet] config section is missing".to_string(),
                )
            })?;
            if parakeet_config.streaming {
                Ok(Box::new(
                    parakeet_streaming::ParakeetStreamingTranscriber::new(parakeet_config)?,
                ))
            } else {
                Ok(Box::new(parakeet::ParakeetTranscriber::new(
                    parakeet_config,
                )?))
            }
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
        #[cfg(feature = "cohere")]
        TranscriptionEngine::Cohere => {
            let cfg = config.cohere.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Cohere engine selected but [cohere] config section is missing".to_string(),
                )
            })?;
            Ok(Box::new(cohere::CohereTranscriber::new(cfg)?))
        }
        #[cfg(not(feature = "cohere"))]
        TranscriptionEngine::Cohere => Err(TranscribeError::InitFailed(
            "Cohere engine requested but voxtype was not compiled with --features cohere"
                .to_string(),
        )),
        TranscriptionEngine::Soniox => {
            let cfg = config.soniox.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Soniox engine selected but [soniox] config section is missing".to_string(),
                )
            })?;
            Ok(Box::new(soniox::SonioxTranscriber::new(cfg.clone())?))
        }
        #[cfg(feature = "openvino-whisper")]
        TranscriptionEngine::OpenVino => {
            let default_config = crate::config::OpenVinoConfig::default();
            let openvino_config = config.openvino.as_ref().unwrap_or(&default_config);
            let transcriber = openvino_whisper::OpenVinoTranscriber::new(openvino_config)?;
            if openvino_config.streaming {
                Ok(Box::new(SlidingWindowStreamingTranscriber::new(
                    Arc::from(Box::new(transcriber) as Box<dyn Transcriber>),
                    sliding_window_config_from_openvino(config, openvino_config),
                )))
            } else {
                Ok(Box::new(transcriber))
            }
        }
        #[cfg(not(feature = "openvino-whisper"))]
        TranscriptionEngine::OpenVino => Err(TranscribeError::InitFailed(
            "OpenVINO engine requested but voxtype was not compiled with --features openvino-whisper"
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

/// Build the sliding-window engine config for the active backend.
///
/// The shared `[streaming]` section (`config.streaming`) wins when present;
/// otherwise `legacy` — that engine's own deprecated `streaming_*` fields —
/// is used, preserving the exact behavior of an old config.toml that never
/// had a `[streaming]` section. See `StreamingConfig::resolve`.
fn sliding_window_config(
    config: &Config,
    legacy: StreamingConfig,
    engine: &str,
) -> SlidingWindowConfig {
    let resolved = StreamingConfig::resolve(config.streaming.as_ref(), legacy, engine);
    SlidingWindowConfig {
        interval_s: resolved.interval_secs as f64,
        max_buffer_s: resolved.max_buffer_secs,
        // Streaming audio arrives at the configured [audio] sample rate.
        sample_rate: config.audio.sample_rate,
        min_speech_rms: resolved.min_speech_rms,
        min_audio_s: resolved.min_audio_secs,
        partial_min_words: resolved.partial_min_words,
        type_partials: resolved.type_partials,
        revision_mode: resolved.revision_mode,
    }
}

/// Build the sliding-window engine config from `[whisper]`'s deprecated
/// per-engine streaming fields, as a fallback for when `[streaming]` isn't
/// set. See `sliding_window_config`.
fn sliding_window_config_from_whisper(config: &Config) -> SlidingWindowConfig {
    let legacy = StreamingConfig {
        interval_secs: config.whisper.streaming_interval_secs,
        max_buffer_secs: config.whisper.streaming_max_buffer_secs,
        min_speech_rms: config.whisper.streaming_min_speech_rms,
        min_audio_secs: config.whisper.streaming_min_audio_secs,
        partial_min_words: config.whisper.streaming_partial_min_words,
        type_partials: config.whisper.streaming_type_partials,
        revision_mode: config.whisper.streaming_revision_mode,
    };
    sliding_window_config(config, legacy, "whisper")
}

/// Build the sliding-window engine config from `[openvino]`'s deprecated
/// per-engine streaming fields, as a fallback for when `[streaming]` isn't
/// set. See `sliding_window_config`.
#[cfg(feature = "openvino-whisper")]
fn sliding_window_config_from_openvino(
    config: &Config,
    openvino: &OpenVinoConfig,
) -> SlidingWindowConfig {
    let legacy = StreamingConfig {
        interval_secs: openvino.streaming_interval_secs,
        max_buffer_secs: openvino.streaming_max_buffer_secs,
        min_speech_rms: openvino.streaming_min_speech_rms,
        min_audio_secs: openvino.streaming_min_audio_secs,
        partial_min_words: openvino.streaming_partial_min_words,
        type_partials: openvino.streaming_type_partials,
        revision_mode: openvino.streaming_revision_mode,
    };
    sliding_window_config(config, legacy, "openvino")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An old-style config.toml — no `[streaming]` section at all, just the
    /// per-engine `streaming_*` fields that predate it — must resolve to
    /// exactly the same `SlidingWindowConfig` it always did. This is the
    /// core backward-compatibility guarantee for the `[streaming]` unification.
    #[test]
    fn old_style_whisper_config_without_shared_section_keeps_working() {
        let toml_str = r#"
            [whisper]
            model = "base.en"
            streaming = true
            streaming_interval_secs = 0.3
            streaming_max_buffer_secs = 15.0
            streaming_min_speech_rms = 0.01
            streaming_min_audio_secs = 2.0
            streaming_partial_min_words = 3
            streaming_type_partials = false
            streaming_revision_mode = true
        "#;
        let config: Config = toml::from_str(toml_str).expect("valid old-style config");
        assert!(config.streaming.is_none(), "no [streaming] section present");

        let resolved = sliding_window_config_from_whisper(&config);
        // interval_s is f64 but the source field is f32 (see
        // SlidingWindowConfig), so compare against the same f32->f64 cast
        // rather than an f64 literal that isn't bit-identical to it.
        assert_eq!(resolved.interval_s, 0.3_f32 as f64);
        assert_eq!(resolved.max_buffer_s, 15.0);
        assert_eq!(resolved.min_speech_rms, 0.01);
        assert_eq!(resolved.min_audio_s, 2.0);
        assert_eq!(resolved.partial_min_words, 3);
        assert!(!resolved.type_partials);
        assert!(resolved.revision_mode);
    }

    /// A config with no streaming settings anywhere still resolves to the
    /// documented defaults, matching pre-unification behavior exactly.
    #[test]
    fn config_with_no_streaming_settings_uses_documented_defaults() {
        let config = Config::default();
        assert!(config.streaming.is_none());
        let resolved = sliding_window_config_from_whisper(&config);
        assert_eq!(resolved.interval_s, 0.8_f32 as f64);
        assert_eq!(resolved.max_buffer_s, 29.0);
        assert_eq!(resolved.min_speech_rms, 0.005);
        assert_eq!(resolved.min_audio_s, 1.0);
        assert_eq!(resolved.partial_min_words, 1);
        assert!(resolved.type_partials);
        assert!(!resolved.revision_mode);
    }

    /// A new-style `[streaming]` section takes priority over any legacy
    /// per-engine fields that might still be sitting in `[whisper]`.
    #[test]
    fn shared_streaming_section_overrides_legacy_whisper_fields() {
        let toml_str = r#"
            [whisper]
            model = "base.en"
            streaming = true
            streaming_interval_secs = 0.3

            [streaming]
            interval_secs = 1.5
        "#;
        let config: Config = toml::from_str(toml_str).expect("valid config");
        assert!(config.streaming.is_some());

        let resolved = sliding_window_config_from_whisper(&config);
        // The shared section's value wins outright, not just for the one
        // field the user set explicitly — the rest come from
        // StreamingConfig::default(), not from [whisper]'s legacy fields.
        assert_eq!(resolved.interval_s, 1.5_f32 as f64);
        assert_eq!(resolved.partial_min_words, 1); // shared default, not legacy
    }
}
