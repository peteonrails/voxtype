//! Dolphin engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// Dolphin speech-to-text configuration (ONNX-based CTC encoder, dictation-optimized)
/// Requires: cargo build --features dolphin
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DolphinConfig {
    /// Model name or path to ONNX model directory
    pub model: String,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for DolphinConfig {
    fn default() -> Self {
        Self {
            model: "dolphin-base".to_string(),
            threads: None,
            on_demand_loading: false,
        }
    }
}
