//! Speech-to-text transcription module
//!
//! Provides transcription via:
//! - Local whisper.cpp inference (whisper-rs crate)
//! - Remote OpenAI-compatible Whisper API (whisper.cpp server, OpenAI, etc.)
//! - CLI subprocess using whisper-cli (fallback for glibc 2.42+ compatibility)
//! - Subprocess isolation for GPU memory release
//! - Optionally ElevenLabs Scribe cloud transcription (when `elevenlabs` feature is enabled)
//! - Optionally NVIDIA Parakeet via ONNX Runtime (when `parakeet` feature is enabled)
//!   or transcribe.cpp GGUF in voxtype-gguf-worker (when `parakeet-gguf` is enabled)
//! - Optionally Moonshine via ONNX Runtime (when `moonshine` feature is enabled)
//! - Optionally SenseVoice via ONNX Runtime (when `sensevoice` feature is enabled)
//! - Optionally Paraformer via ONNX Runtime (when `paraformer` feature is enabled)
//! - Optionally Dolphin via ONNX Runtime (when `dolphin` feature is enabled)
//! - Optionally Omnilingual via ONNX Runtime (when `omnilingual` feature is enabled)
//! - Optionally OpenVINO Whisper for Intel NPU/CPU/GPU (when `openvino-whisper` feature is enabled)

pub mod cli;
#[cfg(feature = "elevenlabs")]
mod elevenlabs;
pub mod lines;
mod metadata;
pub use metadata::RuntimeMetadata;
#[cfg(feature = "parakeet")]
pub mod parakeet_streaming;
pub mod remote;
pub mod sliding_window;
pub mod soniox;
pub mod streaming;
pub mod subprocess;
pub mod whisper;
/// Long-audio windowing shared by the engines with a bounded input length.
/// Deliberately not feature-gated: see the module docs.
pub mod window;
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

/// Drives voxtype-gguf-worker, which runs the transcribe.cpp GGUF engines out of
/// process. voxtype never links transcribe.cpp: its ggml collides with
/// whisper-rs's.
#[cfg(any(feature = "cohere-gguf", feature = "parakeet-gguf", feature = "gguf"))]
pub mod gguf_subprocess;

#[cfg(any(feature = "cohere", feature = "cohere-gguf", feature = "gguf"))]
// Without the ONNX engine, voxtype only validates the settings; the chunker
// itself runs in voxtype-gguf-worker, which compiles this same file.
#[cfg_attr(not(feature = "cohere"), allow(dead_code))]
mod cohere_chunking;

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
use std::time::Duration;

/// A timed segment from transcription (word or sentence level)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TimedSegment {
    pub text: String,
    /// Start time in seconds relative to the audio input
    pub start_secs: f32,
    /// End time in seconds relative to the audio input
    pub end_secs: f32,
}

/// Log a finished transcription: how long `what` took and how long the text
/// is at info, the text itself only at debug.
///
/// The service logs to the journal at info, and the journal keeps what it is
/// given, so a line carrying the words would hold everything a user dictates
/// whatever they set for history, failed recordings and undelivered text.
/// Every engine reports its result through here so none of them can log the
/// words at the default level. `-v` or `RUST_LOG=voxtype=debug` brings the
/// text back for a recognition report.
pub(crate) fn log_transcription(what: &str, started: std::time::Instant, text: &str) {
    tracing::info!(
        "{} completed in {:.2}s ({} chars)",
        what,
        started.elapsed().as_secs_f32(),
        text.chars().count()
    );
    tracing::debug!("{} text: {:?}", what, text);
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

    /// Whether transcribing a short clip costs proportionally less than a
    /// long one. Meeting mode transcribes each utterance on its own, to give
    /// it a timestamp, only for engines where that is so. Whisper pads every
    /// call to a 30 s window, and a process or HTTP request per call has a
    /// fixed cost, so the default is no.
    fn short_clips_are_cheap(&self) -> bool {
        false
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

    /// Set vocabulary hints for the next recording without rebuilding the
    /// resident model. Backends with a native vocabulary mechanism override
    /// this; the default leaves engines without one unchanged.
    fn set_vocabulary(&self, _terms: &[String]) {}

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

    /// Runtime measurements from this engine instance, not compile-time GPU
    /// capabilities. Backends without a measurement leave it unknown.
    fn runtime_metadata(&self) -> RuntimeMetadata {
        RuntimeMetadata::default()
    }
}

/// Append profile vocabulary to an explicitly configured Whisper prompt.
/// The configured prompt always remains first and unchanged.
pub(crate) fn prompt_with_vocabulary(configured: Option<&str>, terms: &[String]) -> Option<String> {
    let configured = configured.filter(|prompt| !prompt.trim().is_empty());
    if terms.is_empty() {
        return configured.map(str::to_string);
    }
    let vocabulary = terms
        .iter()
        .map(|term| term.trim())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    if vocabulary.is_empty() {
        return configured.map(str::to_string);
    }
    Some(match configured {
        Some(prompt) => format!("{prompt}\nVocabulary: {vocabulary}"),
        None => vocabulary,
    })
}

/// Factory function to create transcriber based on configured engine
pub fn create_transcriber(config: &Config) -> Result<Box<dyn Transcriber>, TranscribeError> {
    crate::model_selection::validate_config_backend(config).map_err(TranscribeError::InitFailed)?;
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
        TranscriptionEngine::Parakeet => {
            let parakeet_config = config.parakeet.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Parakeet engine selected but [parakeet] config section is missing".to_string(),
                )
            })?;
            if is_gguf_model(&parakeet_config.model) {
                #[cfg(feature = "parakeet-gguf")]
                {
                    return Ok(Box::new(
                        gguf_subprocess::GgufSubprocessTranscriber::parakeet(parakeet_config)?,
                    ));
                }
                #[cfg(not(feature = "parakeet-gguf"))]
                {
                    return Err(TranscribeError::InitFailed(
                        "Parakeet GGUF requires --features parakeet-gguf (or gpu-vulkan)"
                            .to_string(),
                    ));
                }
            }
            #[cfg(feature = "parakeet")]
            {
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
            {
                Err(TranscribeError::InitFailed(
                    "Parakeet engine requested but voxtype was not compiled with --features parakeet"
                        .to_string(),
                ))
            }
        }
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
        TranscriptionEngine::Cohere => {
            let cfg = config.cohere.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "Cohere engine selected but [cohere] config section is missing".to_string(),
                )
            })?;
            if is_gguf_model(&cfg.model) {
                #[cfg(feature = "cohere-gguf")]
                { return Ok(Box::new(gguf_subprocess::GgufSubprocessTranscriber::cohere(cfg)?)); }
                #[cfg(not(feature = "cohere-gguf"))]
                { return Err(TranscribeError::InitFailed("Cohere GGUF requires --features cohere-gguf".to_string())); }
            }
            #[cfg(feature = "cohere")]
            { Ok(Box::new(cohere::CohereTranscriber::new(cfg)?)) }
            #[cfg(not(feature = "cohere"))]
            { Err(TranscribeError::InitFailed("Cohere ONNX requires --features cohere".to_string())) }
        }
        TranscriptionEngine::Gguf => {
            let cfg = config.gguf.as_ref().ok_or_else(|| {
                TranscribeError::InitFailed(
                    "GGUF engine selected but [gguf] config section is missing".to_string(),
                )
            })?;
            #[cfg(feature = "gguf")]
            {
                Ok(Box::new(
                    gguf_subprocess::GgufSubprocessTranscriber::generic(cfg)?,
                ))
            }
            #[cfg(not(feature = "gguf"))]
            {
                let _ = cfg;
                Err(TranscribeError::InitFailed(
                    "GGUF engine requires --features gguf (or gpu-vulkan)".to_string(),
                ))
            }
        }
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
        #[cfg(feature = "elevenlabs")]
        TranscriptionEngine::ElevenLabs => {
            let provider_config = config.elevenlabs.clone().unwrap_or_default();
            Ok(Box::new(elevenlabs::ElevenLabsTranscriber::new(
                &provider_config,
            )?))
        }
        #[cfg(not(feature = "elevenlabs"))]
        TranscriptionEngine::ElevenLabs => Err(TranscribeError::InitFailed(
            "ElevenLabs engine requested but voxtype was not compiled with --features elevenlabs"
                .to_string(),
        )),
    }
}

/// Whether an engine's `model` setting names a transcribe.cpp GGUF file
/// rather than an ONNX model directory. Cohere and Parakeet both dispatch on
/// it, so the same spelling selects the same runtime for either.
fn is_gguf_model(model: &str) -> bool {
    model.to_ascii_lowercase().ends_with(".gguf")
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

    // Before anything touches ggml. Backends are registered once per process
    // at first model load, so if the graphics driver has not finished binding
    // its render node yet we are pinned to CPU for the life of the daemon —
    // and on a login-started service that is the common case, not the rare
    // one (#611). Costs nothing when the node is already there.
    if cfg!(feature = "gpu-vulkan") && matches!(config.effective_mode(), WhisperMode::Local) {
        let waited = gpu::wait_for_render_node(Duration::from_secs(config.gpu_wait_secs));
        if !waited {
            tracing::warn!(
                "No GPU render node found under /dev/dri after waiting {}s. This \
                 build has Vulkan support, so transcription will fall back to CPU \
                 and stay there until the daemon restarts. If your GPU works but \
                 appears late in boot, raise [whisper] gpu_wait_secs.",
                config.gpu_wait_secs
            );
        }
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

    /// Every engine reports its result through `log_transcription`: the
    /// timing and the length at info, the words only at debug.
    #[test]
    fn a_finished_transcription_is_logged_without_its_text_at_info() {
        use crate::test_support::{own_process, LogCapture};
        if !own_process(
            "transcribe::tests::a_finished_transcription_is_logged_without_its_text_at_info",
            |_| {},
        ) {
            return;
        }
        let text = "meet me at the quarry at nine";

        let capture = LogCapture::at(tracing::Level::INFO);
        log_transcription("Whisper transcription", std::time::Instant::now(), text);
        let logged = capture.text();
        drop(capture);
        assert!(
            logged.contains("Whisper transcription completed in ")
                && logged.contains("s (29 chars)"),
            "{logged}"
        );
        for word in ["meet", "quarry", "nine"] {
            assert!(!logged.contains(word), "{logged}");
        }

        let capture = LogCapture::at(tracing::Level::DEBUG);
        log_transcription("Whisper transcription", std::time::Instant::now(), text);
        assert!(capture.text().contains(text), "{}", capture.text());
    }

    #[test]
    fn gguf_is_chosen_by_extension_in_any_case() {
        assert!(is_gguf_model("parakeet-tdt-0.6b-v3-F16.gguf"));
        assert!(is_gguf_model("/models/Parakeet.GGUF"));
        assert!(!is_gguf_model("parakeet-tdt-0.6b-v3"));
        assert!(!is_gguf_model("/models/parakeet-tdt-0.6b-v3-gguf"));
    }

    fn parakeet_config(model: &str) -> Config {
        Config {
            engine: TranscriptionEngine::Parakeet,
            parakeet: Some(crate::config::ParakeetConfig {
                model: model.to_string(),
                ..Default::default()
            }),
            ..Config::default()
        }
    }

    fn init_error(config: &Config) -> String {
        match create_transcriber(config) {
            Ok(_) => panic!("a missing model must not load"),
            Err(e) => e.to_string(),
        }
    }

    /// A `.gguf` Parakeet model goes to transcribe.cpp, which reports the
    /// missing file by path, or says which feature the build lacks.
    #[test]
    fn parakeet_gguf_models_dispatch_to_transcribe_cpp() {
        let err = init_error(&parakeet_config("/nonexistent/voxtype/parakeet.gguf"));
        if cfg!(feature = "parakeet-gguf") {
            assert!(err.contains("/nonexistent/voxtype/parakeet.gguf"), "{err}");
            assert!(!err.contains("Parakeet model '"), "{err}");
        } else {
            assert!(err.contains("parakeet-gguf"), "{err}");
        }
    }

    /// Anything else stays on the ONNX path, exactly as before GGUF support.
    #[test]
    fn parakeet_onnx_models_dispatch_to_onnx_runtime() {
        let err = init_error(&parakeet_config("/nonexistent/voxtype/parakeet-tdt"));
        if cfg!(feature = "parakeet") {
            assert!(err.contains("Parakeet model '"), "{err}");
        } else {
            assert!(err.contains("--features parakeet"), "{err}");
            assert!(!err.contains("parakeet-gguf"), "{err}");
        }
    }

    #[test]
    fn profile_vocabulary_appends_to_configured_prompt() {
        let terms = vec!["Voxtype".to_string(), "PipeWire".to_string()];
        assert_eq!(
            prompt_with_vocabulary(Some("Meeting notes."), &terms).as_deref(),
            Some("Meeting notes.\nVocabulary: Voxtype, PipeWire")
        );
        assert_eq!(
            prompt_with_vocabulary(None, &terms).as_deref(),
            Some("Voxtype, PipeWire")
        );
        assert_eq!(
            prompt_with_vocabulary(Some("Keep this prompt"), &[]).as_deref(),
            Some("Keep this prompt")
        );
    }

    #[test]
    fn cohere_model_format_keeps_gguf_and_legacy_dispatch_separate() {
        for (model, gguf) in [
            ("/nonexistent/voxtype/cohere.gguf", true),
            ("/nonexistent/voxtype/cohere-onnx", false),
        ] {
            let config = Config {
                engine: TranscriptionEngine::Cohere,
                cohere: Some(crate::config::CohereConfig {
                    model: model.to_string(),
                    ..Default::default()
                }),
                ..Config::default()
            };
            let err = init_error(&config);
            if gguf {
                if cfg!(feature = "cohere-gguf") {
                    assert!(err.contains(model), "{err}");
                    assert!(!err.contains("--features cohere"), "{err}");
                } else {
                    assert!(err.contains("cohere-gguf"), "{err}");
                }
            } else {
                assert!(!err.contains("cohere-gguf"), "{err}");
                if cfg!(feature = "cohere") {
                    assert!(err.contains(model), "{err}");
                } else {
                    assert!(err.contains("--features cohere"), "{err}");
                }
            }
        }
    }

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

    fn factory_error(config: &Config) -> TranscribeError {
        match create_transcriber(config) {
            Ok(_) => panic!("expected transcriber construction to fail"),
            Err(error) => error,
        }
    }

    #[cfg(not(feature = "elevenlabs"))]
    #[test]
    fn elevenlabs_factory_reports_feature_disabled_remediation() {
        let config = Config {
            engine: TranscriptionEngine::ElevenLabs,
            ..Config::default()
        };

        let error = factory_error(&config).to_string();

        assert!(error.contains("--features elevenlabs"));
    }

    #[cfg(feature = "elevenlabs")]
    mod elevenlabs {
        use super::*;
        use crate::config::{ElevenLabsConfig, ElevenLabsMode};

        const API_KEY: &str = "test-elevenlabs-key";

        fn config(mode: ElevenLabsMode) -> Config {
            Config {
                engine: TranscriptionEngine::ElevenLabs,
                elevenlabs: Some(ElevenLabsConfig {
                    api_key: Some(API_KEY.to_string()),
                    mode,
                    ..ElevenLabsConfig::default()
                }),
                ..Config::default()
            }
        }

        #[test]
        fn factory_materializes_defaults_before_reporting_missing_api_key() {
            let config = Config {
                engine: TranscriptionEngine::ElevenLabs,
                elevenlabs: None,
                ..Config::default()
            };

            let error = factory_error(&config).to_string();

            assert!(error.contains(
                "ElevenLabs API key required: set [elevenlabs] api_key or ELEVENLABS_API_KEY"
            ));
            assert!(!error.contains("config section is missing"));
        }

        #[test]
        fn factory_rejects_api_keys_that_are_invalid_http_headers() {
            let invalid_key = "secret\nheader";
            let mut config = config(ElevenLabsMode::Realtime);
            config.elevenlabs.as_mut().unwrap().api_key = Some(invalid_key.to_string());

            let error = factory_error(&config).to_string();

            assert!(error.contains("cannot be used in an HTTP header"));
            assert!(!error.contains(invalid_key));
        }

        #[test]
        fn factory_rejects_invalid_vad_silence_threshold() {
            let mut config = config(ElevenLabsMode::Realtime);
            config
                .elevenlabs
                .as_mut()
                .unwrap()
                .vad_silence_threshold_secs = f32::NAN;

            let error = factory_error(&config).to_string();

            assert!(error.contains("vad_silence_threshold_secs"));
            assert!(error.contains("positive finite number"));
        }
    }
}
