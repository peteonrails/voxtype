//! GigaAM v3 RNN-T engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// GigaAM v3 RNN-T configuration (ONNX-based Russian ASR).
/// Requires: cargo build --features gigaam
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GigaamConfig {
    /// Model directory name under the models dir, or an absolute path.
    /// Expects: v3_rnnt_encoder_int8.onnx, v3_rnnt_decoder.onnx,
    /// v3_rnnt_joint.onnx, v3_vocab.txt
    pub model: String,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for GigaamConfig {
    fn default() -> Self {
        Self {
            model: "gigaam-v3-rnnt-int8".to_string(),
            threads: None,
            on_demand_loading: false,
        }
    }
}
