//! Omnilingual engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// Omnilingual speech-to-text configuration (FunASR ONNX-based, 50+ languages)
/// Requires: cargo build --features omnilingual
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OmnilingualConfig {
    /// Model name or path to ONNX model directory
    pub model: String,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for OmnilingualConfig {
    fn default() -> Self {
        Self {
            model: "omnilingual-large".to_string(),
            threads: None,
            on_demand_loading: false,
        }
    }
}
