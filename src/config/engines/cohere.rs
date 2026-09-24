//! Cohere engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// Inference backend for the Cohere encoder. The decoder stays on ONNX Runtime CPU.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CohereEncoderBackend {
    /// Existing ONNX Runtime encoder path.
    #[default]
    Onnx,
    /// Experimental native OpenVINO encoder on Intel GPU, with no CPU fallback.
    OpenvinoGpu,
}

impl std::str::FromStr for CohereEncoderBackend {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "onnx" => Ok(Self::Onnx),
            "openvino_gpu" => Ok(Self::OpenvinoGpu),
            _ => Err(format!(
                "Invalid Cohere encoder backend '{}'. Valid options: onnx, openvino_gpu",
                value
            )),
        }
    }
}

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

    /// Encoder inference backend. OpenvinoGpu requires the cohere-openvino feature.
    #[serde(default)]
    pub encoder_backend: CohereEncoderBackend,

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
            encoder_backend: CohereEncoderBackend::default(),
            language: default_cohere_language(),
            threads: None,
            on_demand_loading: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_cohere_config_defaults_to_onnx() {
        let config: CohereConfig = toml::from_str("model = \"cohere-transcribe-q4f16\"").unwrap();
        assert_eq!(config.encoder_backend, CohereEncoderBackend::Onnx);
        assert_eq!(
            CohereConfig::default().encoder_backend,
            CohereEncoderBackend::Onnx
        );
    }

    #[test]
    fn cohere_encoder_backend_parsing_and_serde() {
        for (name, expected) in [
            ("onnx", CohereEncoderBackend::Onnx),
            ("openvino_gpu", CohereEncoderBackend::OpenvinoGpu),
        ] {
            assert_eq!(name.parse::<CohereEncoderBackend>().unwrap(), expected);
            let config: CohereConfig = toml::from_str(&format!(
                "model = \"cohere-transcribe-q4f16\"\nencoder_backend = \"{name}\""
            ))
            .unwrap();
            assert_eq!(config.encoder_backend, expected);
            assert_eq!(serde_json::to_value(expected).unwrap(), name);
        }
    }

    #[test]
    fn cohere_encoder_backend_rejects_invalid_selection() {
        for name in ["", "gpu", "openvino", "openvino-gpu", "auto", "ONNX"] {
            let error = name.parse::<CohereEncoderBackend>().unwrap_err();
            assert!(error.contains("Valid options: onnx, openvino_gpu"));
            let error = toml::from_str::<CohereConfig>(&format!(
                "model = \"cohere-transcribe-q4f16\"\nencoder_backend = \"{name}\""
            ))
            .unwrap_err()
            .to_string();
            assert!(error.contains("onnx"));
            assert!(error.contains("openvino_gpu"));
        }
    }
}
