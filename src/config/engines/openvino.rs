//! OpenVINO engine configuration (Intel NPU / GPU / CPU via OpenVINO GenAI).
//!
//! Requires: cargo build --features openvino

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// OpenVINO GenAI Whisper configuration.
///
/// Runs Whisper through OpenVINO GenAI's `WhisperPipeline` on the Intel NPU
/// (or GPU/CPU). The model is an OpenVINO IR directory (`.xml` + `.bin`),
/// e.g. `OpenVINO/whisper-base.en-int8-ov` from HuggingFace.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenVinoConfig {
    /// Model name or path to a directory containing OpenVINO IR files
    /// (openvino_encoder_model.xml, openvino_decoder_model.xml, ...).
    /// Short names: "tiny.en", "base.en", "small.en", "base", "small"
    pub model: String,

    /// Inference device: "NPU", "GPU", or "CPU" (default: "NPU").
    /// Falls back to CPU if the requested device fails to initialize.
    #[serde(default = "default_openvino_device")]
    pub device: String,

    /// Language for transcription (default: "en")
    #[serde(default = "default_openvino_language")]
    pub language: String,

    /// Number of CPU threads for OpenVINO inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,

    // --- Sliding-window streaming settings (same knobs as [whisper]) ---
    /// Enable live streaming transcription via the shared sliding-window engine.
    #[serde(default)]
    pub streaming: bool,

    /// Seconds between re-transcriptions of the rolling buffer.
    #[serde(default = "default_streaming_interval_secs")]
    pub streaming_interval_secs: f32,

    /// Maximum buffered audio (seconds) before the window slides.
    #[serde(default = "default_streaming_max_buffer_secs")]
    pub streaming_max_buffer_secs: f32,

    /// Skip transcription while whole-buffer RMS is below this.
    #[serde(default = "default_streaming_min_speech_rms")]
    pub streaming_min_speech_rms: f32,

    /// Minimum buffered audio (seconds) before the first partial is attempted.
    #[serde(default = "default_streaming_min_audio_secs")]
    pub streaming_min_audio_secs: f32,

    /// Minimum number of new stable words before a delta is committed/typed.
    #[serde(default = "default_streaming_partial_min_words")]
    pub streaming_partial_min_words: usize,

    /// Type committed deltas live at the cursor (`true`) or commit whole
    /// segments at once (`false`).
    #[serde(default = "default_streaming_type_partials")]
    pub streaming_type_partials: bool,
}

fn default_openvino_device() -> String {
    "NPU".to_string()
}

fn default_openvino_language() -> String {
    "en".to_string()
}

fn default_streaming_interval_secs() -> f32 {
    0.8
}

fn default_streaming_max_buffer_secs() -> f32 {
    29.0
}

fn default_streaming_min_speech_rms() -> f32 {
    0.005
}

fn default_streaming_min_audio_secs() -> f32 {
    1.0
}

fn default_streaming_partial_min_words() -> usize {
    1
}

fn default_streaming_type_partials() -> bool {
    true
}

impl Default for OpenVinoConfig {
    fn default() -> Self {
        Self {
            model: "base.en".to_string(),
            device: "NPU".to_string(),
            language: "en".to_string(),
            threads: None,
            on_demand_loading: false,
            streaming: false,
            streaming_interval_secs: default_streaming_interval_secs(),
            streaming_max_buffer_secs: default_streaming_max_buffer_secs(),
            streaming_min_speech_rms: default_streaming_min_speech_rms(),
            streaming_min_audio_secs: default_streaming_min_audio_secs(),
            streaming_partial_min_words: default_streaming_partial_min_words(),
            streaming_type_partials: default_streaming_type_partials(),
        }
    }
}
