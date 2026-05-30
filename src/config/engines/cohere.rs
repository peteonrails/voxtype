//! Cohere engine configuration.

use serde::{Deserialize, Serialize};

use super::super::whisper::default_on_demand_loading;

/// Cohere Transcribe speech-to-text configuration (ONNX-based, encoder-decoder).
/// Requires: cargo build --features cohere
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CohereConfig {
    /// Model name or directory containing the Cohere ONNX files.
    /// Expects HuggingFace Optimum layout:
    ///   encoder_model.onnx (+ .onnx_data),
    ///   decoder_model_merged.onnx (+ .onnx_data),
    ///   tokenizer.json
    /// Short names: "cohere-transcribe-q4f16" (default, ~1.5 GB),
    ///              "cohere-transcribe-q4", "cohere-transcribe-int8",
    ///              "cohere-transcribe-fp16"
    pub model: String,

    /// Language for transcription. Two-letter ISO 639-1 codes
    /// (e.g. "en", "fr", "de"). Cohere supports 14 languages.
    #[serde(default = "default_cohere_language")]
    pub language: String,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

fn default_cohere_language() -> String {
    "en".to_string()
}

impl Default for CohereConfig {
    fn default() -> Self {
        Self {
            model: "cohere-transcribe-q4f16".to_string(),
            language: default_cohere_language(),
            threads: None,
            on_demand_loading: false,
        }
    }
}
