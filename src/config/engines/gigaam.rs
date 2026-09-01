//! GigaAM engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// GigaAM speech-to-text configuration (SberDevices GigaAM-v3 e2e RNN-T
/// via ONNX Runtime). Russian ASR with built-in punctuation and text
/// normalization.
/// Requires: cargo build --features gigaam
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GigaAMConfig {
    /// Model name or path to ONNX model directory
    /// Expects: encoder.onnx, decoder.onnx, joint.onnx, tokens.txt
    pub model: String,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for GigaAMConfig {
    fn default() -> Self {
        Self {
            model: "gigaam-v3-e2e-rnnt".to_string(),
            threads: None,
            on_demand_loading: false,
        }
    }
}
