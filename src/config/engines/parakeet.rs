//! Parakeet engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// Nemotron 3.5 locales documented by NVIDIA, plus automatic detection.
pub const NEMOTRON_LANGUAGE_CHOICES: &[&str] = &[
    "auto", "en-US", "en-GB", "es-US", "es-ES", "fr-FR", "fr-CA", "it-IT", "pt-BR", "pt-PT",
    "nl-NL", "de-DE", "tr-TR", "ru-RU", "ar-AR", "hi-IN", "ja-JP", "ko-KR", "vi-VN", "uk-UA",
    "pl-PL", "sv-SE", "cs-CZ", "nb-NO", "da-DK", "bg-BG", "fi-FI", "hr-HR", "sk-SK", "zh-CN",
    "hu-HU", "ro-RO", "et-EE", "el-GR", "lt-LT", "lv-LV", "mt-MT", "sl-SI", "he-IL", "th-TH",
    "nn-NO",
];

/// Parakeet model architecture type
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ParakeetModelType {
    /// CTC (Connectionist Temporal Classification) - faster, character-level output
    Ctc,
    /// TDT (Token-Duration-Transducer) - recommended, proper punctuation and word boundaries
    #[default]
    Tdt,
    /// Nemotron cache-aware RNNT streaming model
    Nemotron,
}

/// Parakeet speech-to-text configuration (ONNX-based, alternative to Whisper)
/// Requires: cargo build --features parakeet
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParakeetConfig {
    /// Path to model directory containing ONNX model files
    /// For TDT: encoder-model.onnx, decoder_joint-model.onnx, vocab.txt
    /// For CTC: model.onnx, tokenizer.json
    pub model: String,

    /// Model architecture type: "tdt", "ctc", or "nemotron"
    /// Auto-detected from model directory structure if not specified
    #[serde(default)]
    pub model_type: Option<ParakeetModelType>,

    /// Target language for multilingual Nemotron models.
    /// Use "auto" for automatic language detection.
    #[serde(default = "default_language")]
    pub language: String,

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

fn default_language() -> String {
    "auto".to_string()
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
            language: default_language(),
            on_demand_loading: false,
            streaming: false,
            streaming_chunk_secs: default_streaming_chunk_secs(),
            streaming_left_context_secs: default_streaming_left_context_secs(),
            streaming_right_context_secs: default_streaming_right_context_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_parse_parakeet_model_type_tdt() {
        let toml_str = r#"
            engine = "parakeet"

            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [parakeet]
            model = "parakeet-tdt-0.6b-v3"
            model_type = "tdt"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let parakeet = config.parakeet.unwrap();
        assert_eq!(parakeet.model, "parakeet-tdt-0.6b-v3");
        assert_eq!(parakeet.model_type, Some(ParakeetModelType::Tdt));
    }

    #[test]
    fn test_parse_parakeet_model_type_ctc() {
        let toml_str = r#"
            engine = "parakeet"

            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [parakeet]
            model = "parakeet-ctc-0.6b"
            model_type = "ctc"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let parakeet = config.parakeet.unwrap();
        assert_eq!(parakeet.model, "parakeet-ctc-0.6b");
        assert_eq!(parakeet.model_type, Some(ParakeetModelType::Ctc));
    }

    #[test]
    fn test_parse_nemotron_language() {
        let toml_str = r#"
            engine = "parakeet"

            [parakeet]
            model = "nemotron-3.5-asr-streaming-0.6b-int8"
            model_type = "nemotron"
            language = "en-US"
            streaming = true
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let parakeet = config.parakeet.unwrap();
        assert_eq!(parakeet.model_type, Some(ParakeetModelType::Nemotron));
        assert_eq!(parakeet.language, "en-US");
        assert!(parakeet.streaming);
    }

    #[test]
    fn test_parakeet_model_type_defaults_to_none_for_auto_detection() {
        let toml_str = r#"
            engine = "parakeet"

            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [parakeet]
            model = "parakeet-tdt-0.6b-v3"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let parakeet = config.parakeet.unwrap();
        // model_type should be None (will be auto-detected at runtime)
        assert!(parakeet.model_type.is_none());
    }

    #[test]
    fn test_parakeet_config_default() {
        let config = ParakeetConfig::default();
        assert_eq!(config.model, "parakeet-tdt-0.6b-v3");
        assert!(config.model_type.is_none());
        assert_eq!(config.language, "auto");
        assert!(!config.on_demand_loading);
    }

    #[test]
    fn test_parakeet_model_type_enum_default() {
        // ParakeetModelType defaults to Tdt
        assert_eq!(ParakeetModelType::default(), ParakeetModelType::Tdt);
    }
}
