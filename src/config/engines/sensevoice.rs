//! SenseVoice engine configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::super::default_on_demand_loading;

use super::super::default_true;

/// Which runtime executes the SenseVoice encoder.
///
/// `onnx` (default) runs in-process through ONNX Runtime, so the weights occupy
/// the daemon's RAM. `ggml` shells out to the FunASR llama.cpp runtime, which
/// can place the weights in VRAM and keeps them out of the daemon's heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SenseVoiceRuntime {
    /// ONNX Runtime, in-process (weights in system RAM)
    #[default]
    Onnx,
    /// FunASR llama.cpp runtime, one child process per transcription
    Ggml,
}

impl SenseVoiceRuntime {
    /// Config value / display name for this runtime
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Onnx => "onnx",
            Self::Ggml => "ggml",
        }
    }
}

/// Compute backend passed to the ggml runtime as `--backend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GgmlBackend {
    /// Vulkan, weights uploaded to GPU memory
    #[default]
    Vulkan,
    /// Plain CPU
    Cpu,
    /// CUDA (requires a runtime binary built with GGML_CUDA)
    Cuda,
}

impl GgmlBackend {
    /// Value for the runtime's `--backend` flag
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Vulkan => "vulkan",
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
        }
    }

    /// Config value / display name for this backend
    pub fn as_str(self) -> &'static str {
        self.as_arg()
    }
}

/// SenseVoice speech-to-text configuration (CTC encoder-only ASR)
/// Requires: cargo build --features sensevoice
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SenseVoiceConfig {
    /// Model name or path to directory containing ONNX model files
    /// Expects: model.int8.onnx (or model.onnx), tokens.txt
    /// Short name: "sensevoice-small" (default)
    /// Used when `runtime = "onnx"`.
    pub model: String,

    /// Language for transcription: "auto", "zh", "en", "ja", "ko", "yue" (default: "auto")
    /// Only honoured by `runtime = "onnx"`; the ggml runtime always auto-detects.
    #[serde(default = "default_sensevoice_language")]
    pub language: String,

    /// Enable inverse text normalization (adds punctuation) (default: true)
    /// The ggml runtime always applies ITN; with `runtime = "ggml"` this is
    /// reported but not enforced.
    #[serde(default = "default_true")]
    pub use_itn: bool,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    ///
    /// With `runtime = "ggml"` no weights are held in-process either way, so
    /// this only decides when the configured paths are validated.
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,

    /// Which runtime executes the encoder (default: "onnx")
    #[serde(default)]
    pub runtime: SenseVoiceRuntime,

    /// Path to the FunASR llama.cpp runtime binary (`llama-funasr-sensevoice`
    /// from the `runtime-llamacpp-v0.2.6` release). Required when
    /// `runtime = "ggml"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ggml_binary: Option<PathBuf>,

    /// Path to the SenseVoice GGUF weights (`sensevoice-small-q8.gguf` or
    /// `sensevoice-small-f16.gguf`). Required when `runtime = "ggml"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ggml_model: Option<PathBuf>,

    /// Optional FSMN-VAD GGUF. When set, the runtime segments long audio
    /// internally (`--vad`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ggml_vad: Option<PathBuf>,

    /// Compute backend for the ggml runtime: "vulkan" (default), "cpu", "cuda".
    /// "vulkan" places the weights in VRAM rather than system RAM.
    #[serde(default)]
    pub ggml_backend: GgmlBackend,
}

fn default_sensevoice_language() -> String {
    "auto".to_string()
}

impl Default for SenseVoiceConfig {
    fn default() -> Self {
        Self {
            model: "sensevoice-small".to_string(),
            language: "auto".to_string(),
            use_itn: true,
            threads: None,
            on_demand_loading: false,
            runtime: SenseVoiceRuntime::Onnx,
            ggml_binary: None,
            ggml_model: None,
            ggml_vad: None,
            ggml_backend: GgmlBackend::Vulkan,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_defaults_to_onnx_and_backend_to_vulkan() {
        let config: SenseVoiceConfig = toml::from_str("model = \"sensevoice-small\"").unwrap();
        assert_eq!(config.runtime, SenseVoiceRuntime::Onnx);
        assert_eq!(config.ggml_backend, GgmlBackend::Vulkan);
        assert!(config.ggml_binary.is_none());
    }

    #[test]
    fn ggml_settings_parse_from_toml() {
        let config: SenseVoiceConfig = toml::from_str(
            r#"
            model = "sensevoice-small"
            runtime = "ggml"
            ggml_binary = "/usr/local/bin/llama-funasr-sensevoice"
            ggml_model = "/usr/share/voxtype/sensevoice-small-q8.gguf"
            ggml_vad = "/usr/share/voxtype/fsmn-vad.gguf"
            ggml_backend = "cpu"
            "#,
        )
        .unwrap();
        assert_eq!(config.runtime, SenseVoiceRuntime::Ggml);
        assert_eq!(config.ggml_backend, GgmlBackend::Cpu);
        assert_eq!(
            config.ggml_model.as_deref().unwrap().to_str().unwrap(),
            "/usr/share/voxtype/sensevoice-small-q8.gguf"
        );
    }

    #[test]
    fn ggml_backend_args_match_the_runtime_flags() {
        assert_eq!(GgmlBackend::Vulkan.as_arg(), "vulkan");
        assert_eq!(GgmlBackend::Cpu.as_arg(), "cpu");
        assert_eq!(GgmlBackend::Cuda.as_arg(), "cuda");
    }

    #[test]
    fn unknown_backend_is_rejected() {
        let parsed: Result<SenseVoiceConfig, _> =
            toml::from_str("model = \"x\"\nggml_backend = \"opencl\"");
        assert!(parsed.is_err());
    }
}
