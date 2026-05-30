//! Parakeet engine configuration.

use serde::{Deserialize, Serialize};

use super::super::whisper::default_on_demand_loading;

/// Parakeet model architecture type
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ParakeetModelType {
    /// CTC (Connectionist Temporal Classification) - faster, character-level output
    Ctc,
    /// TDT (Token-Duration-Transducer) - recommended, proper punctuation and word boundaries
    #[default]
    Tdt,
}

/// Parakeet speech-to-text configuration (ONNX-based, alternative to Whisper)
/// Requires: cargo build --features parakeet
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParakeetConfig {
    /// Path to model directory containing ONNX model files
    /// For TDT: encoder-model.onnx, decoder_joint-model.onnx, vocab.txt
    /// For CTC: model.onnx, tokenizer.json
    pub model: String,

    /// Model architecture type: "tdt" (default, recommended) or "ctc"
    /// Auto-detected from model directory structure if not specified
    #[serde(default)]
    pub model_type: Option<ParakeetModelType>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,

    /// Use the cache-aware streaming pipeline (parakeet-rs `ParakeetUnified`)
    /// instead of the batch CTC/TDT models. When true, voxtype emits live
    /// partial transcripts during recording and types the final transcript
    /// on hotkey release. Requires a streaming-capable model directory
    /// (TDT v3 family with `tokenizer.model`).
    /// Default: false (batch pipeline, identical to pre-streaming behavior).
    #[serde(default)]
    pub streaming: bool,

    /// Streaming chunk length in seconds. Smaller = lower latency, more
    /// inference overhead. Maps to `UnifiedStreamingConfig::chunk_secs`.
    #[serde(default = "default_streaming_chunk_secs")]
    pub streaming_chunk_secs: f32,

    /// Streaming left context in seconds. Maps to
    /// `UnifiedStreamingConfig::left_context_secs`.
    #[serde(default = "default_streaming_left_context_secs")]
    pub streaming_left_context_secs: f32,

    /// Streaming right context in seconds. Maps to
    /// `UnifiedStreamingConfig::right_context_secs`.
    #[serde(default = "default_streaming_right_context_secs")]
    pub streaming_right_context_secs: f32,
}

fn default_streaming_chunk_secs() -> f32 {
    0.5
}

fn default_streaming_left_context_secs() -> f32 {
    1.5
}

fn default_streaming_right_context_secs() -> f32 {
    0.5
}

impl Default for ParakeetConfig {
    fn default() -> Self {
        Self {
            model: "parakeet-tdt-0.6b-v3".to_string(),
            model_type: None, // Auto-detect
            on_demand_loading: false,
            streaming: false,
            streaming_chunk_secs: default_streaming_chunk_secs(),
            streaming_left_context_secs: default_streaming_left_context_secs(),
            streaming_right_context_secs: default_streaming_right_context_secs(),
        }
    }
}
