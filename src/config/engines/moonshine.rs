//! Moonshine engine configuration.

use serde::{Deserialize, Serialize};

use super::super::whisper::default_on_demand_loading;

use super::super::default_true;

/// Moonshine speech-to-text configuration (ONNX-based, encoder-decoder ASR)
/// Requires: cargo build --features moonshine
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MoonshineConfig {
    /// Model name or path to directory containing ONNX model files
    /// Expects: encoder_model.onnx, decoder_model_merged.onnx, tokenizer.json
    /// Short names: "tiny" (27M params), "base" (61M params)
    pub model: String,

    /// Use quantized model variants for faster CPU inference (default: true)
    /// Falls back to full precision if quantized files are not found
    #[serde(default = "default_true")]
    pub quantized: bool,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for MoonshineConfig {
    fn default() -> Self {
        Self {
            model: "base".to_string(),
            quantized: true,
            threads: None,
            on_demand_loading: false,
        }
    }
}
