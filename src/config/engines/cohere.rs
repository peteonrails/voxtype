//! Cohere engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// Cohere Transcribe speech-to-text configuration (ONNX or GGUF).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CohereConfig {
    /// Model name, ONNX directory, or path to a transcribe.cpp GGUF file.
    /// Expects HuggingFace Optimum layout:
    ///   encoder_model.onnx (+ .onnx_data),
    ///   decoder_model_merged.onnx (+ .onnx_data),
    ///   tokenizer.json
    /// Short names: "cohere-transcribe-q4f16" (default, ~1.5 GB),
    ///              "cohere-transcribe-q4", "cohere-transcribe-int8",
    ///              "cohere-transcribe-fp16"
    pub model: String,

    /// transcribe.cpp compute backend for GGUF models (auto, cpu, vulkan, etc.).
    #[serde(default = "default_gguf_backend")]
    pub gguf_backend: String,

    /// Maximum audio duration passed to Cohere in one inference, in seconds.
    #[serde(default = "default_max_chunk_secs")]
    pub max_chunk_secs: u32,

    /// Search radius around a balanced split for the quietest boundary.
    #[serde(default = "default_boundary_search_secs")]
    pub boundary_search_secs: f32,

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

fn default_gguf_backend() -> String {
    "auto".to_string()
}

fn default_max_chunk_secs() -> u32 {
    35
}

fn default_boundary_search_secs() -> f32 {
    2.5
}

impl Default for CohereConfig {
    fn default() -> Self {
        Self {
            model: "cohere-transcribe-q4f16".to_string(),
            gguf_backend: default_gguf_backend(),
            max_chunk_secs: default_max_chunk_secs(),
            boundary_search_secs: default_boundary_search_secs(),
            language: default_cohere_language(),
            threads: None,
            on_demand_loading: false,
        }
    }
}
