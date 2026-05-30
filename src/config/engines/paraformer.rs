//! Paraformer engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// Paraformer speech-to-text configuration (FunASR ONNX-based CTC encoder)
/// Requires: cargo build --features paraformer
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParaformerConfig {
    /// Model name or path to ONNX model directory
    /// Expects: model.onnx (or model.int8.onnx), tokens.txt
    pub model: String,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for ParaformerConfig {
    fn default() -> Self {
        Self {
            model: "paraformer-zh".to_string(),
            threads: None,
            on_demand_loading: false,
        }
    }
}
