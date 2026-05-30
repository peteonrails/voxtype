//! SenseVoice engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

use super::super::default_true;

/// SenseVoice speech-to-text configuration (ONNX-based, CTC encoder-only ASR)
/// Requires: cargo build --features sensevoice
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SenseVoiceConfig {
    /// Model name or path to directory containing ONNX model files
    /// Expects: model.int8.onnx (or model.onnx), tokens.txt
    /// Short name: "sensevoice-small" (default)
    pub model: String,

    /// Language for transcription: "auto", "zh", "en", "ja", "ko", "yue" (default: "auto")
    #[serde(default = "default_sensevoice_language")]
    pub language: String,

    /// Enable inverse text normalization (adds punctuation) (default: true)
    #[serde(default = "default_true")]
    pub use_itn: bool,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

fn default_sensevoice_language() -> String {
    "auto".to_string()
}

impl Default for SenseVoiceConfig {
    fn default() -> Self {
        Self {
            model: "sensevoice-small".to_string(),
            language: "auto".to_string(),
            use_itn: true,
            threads: None,
            on_demand_loading: false,
        }
    }
}
