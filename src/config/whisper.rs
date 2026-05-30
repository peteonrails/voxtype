//! Whisper-specific configuration.

use serde::{Deserialize, Serialize};

use super::LanguageConfig;

/// Whisper execution mode (how whisper runs)
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum WhisperMode {
    /// Local transcription using whisper.cpp FFI
    #[default]
    Local,
    /// Remote transcription via OpenAI-compatible API
    Remote,
    /// CLI transcription using whisper-cli subprocess
    /// Fallback for systems where whisper-rs FFI doesn't work (e.g., glibc 2.42+)
    Cli,
}

/// Whisper speech-to-text configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhisperConfig {
    /// Execution mode: "local" or "remote" (preferred field name)
    #[serde(default)]
    pub mode: Option<WhisperMode>,

    /// DEPRECATED: Use `mode` instead. Kept for backwards compatibility.
    #[serde(default)]
    pub backend: Option<WhisperMode>,

    /// Model name: tiny, base, small, medium, large-v3, large-v3-turbo
    /// Can also be an absolute path to a .bin file
    #[serde(default = "default_whisper_model")]
    pub model: String,

    /// Language configuration: single code, "auto", or array of allowed languages
    /// Examples: "en", "auto", ["en", "fr"]
    #[serde(default)]
    pub language: LanguageConfig,

    /// Translate to English if source language is not English
    #[serde(default)]
    pub translate: bool,

    /// Number of threads for inference (None = auto-detect)
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,

    /// Enable GPU memory isolation mode (default: false)
    /// When true, transcription runs in a subprocess that exits after each
    /// transcription, ensuring GPU memory is fully released between recordings.
    /// This is especially useful on laptops with hybrid graphics to prevent
    /// the GPU from staying active when not in use.
    /// Note: This option only applies when mode = "local".
    #[serde(default)]
    pub gpu_isolation: bool,

    /// GPU device index for Vulkan/CUDA/Metal backend selection.
    /// On multi-GPU systems, whisper.cpp may select the integrated GPU (index 0)
    /// instead of the discrete GPU, causing slower transcription.
    /// Set this to the index of your preferred GPU (e.g., 1 for the second device).
    /// Leave unset to use the default device (index 0).
    /// You can also use the GGML_VK_VISIBLE_DEVICES env var for Vulkan filtering.
    #[serde(default)]
    pub gpu_device: Option<i32>,

    /// Enable flash attention for GPU inference (default: false)
    /// Reduces memory bandwidth pressure in the attention layers.
    /// Requires a compatible GPU backend (CUDA or Vulkan).
    #[serde(default)]
    pub flash_attention: bool,

    /// Optimize context window for short recordings (default: true)
    /// When enabled, uses a smaller context window proportional to audio length
    /// for clips under 22.5 seconds. This significantly speeds up transcription
    /// on both CPU and GPU. If transcription seems unstable, set this to false.
    #[serde(default = "default_context_window_optimization")]
    pub context_window_optimization: bool,

    // --- Eager processing settings ---
    /// Enable eager input processing (transcribe chunks while recording continues)
    /// When enabled, audio is split into chunks and transcribed in parallel with
    /// continued recording. This reduces perceived latency on slower machines.
    #[serde(default)]
    pub eager_processing: bool,

    /// Duration of each audio chunk in seconds for eager processing
    #[serde(default = "default_eager_chunk_secs")]
    pub eager_chunk_secs: f32,

    /// Overlap between adjacent chunks in seconds for eager processing
    /// Overlap helps catch words at chunk boundaries
    #[serde(default = "default_eager_overlap_secs")]
    pub eager_overlap_secs: f32,

    /// Initial prompt to provide context for transcription
    /// Use this to hint at terminology, proper nouns, or formatting conventions.
    /// Example: "Technical discussion about Rust, TypeScript, and Kubernetes."
    #[serde(default)]
    pub initial_prompt: Option<String>,

    // --- Multi-model settings ---
    /// Secondary model to use when hotkey.model_modifier is held
    /// Example: "large-v3-turbo" for difficult audio
    #[serde(default)]
    pub secondary_model: Option<String>,

    /// List of available models that can be selected via CLI --model flag
    /// These models can be loaded on-demand when requested
    #[serde(default)]
    pub available_models: Vec<String>,

    /// Maximum number of models to keep loaded in memory (LRU eviction)
    /// Default: 2 (primary model + one secondary)
    /// Only applies when gpu_isolation = false
    #[serde(default = "default_max_loaded_models")]
    pub max_loaded_models: usize,

    /// Seconds before unloading idle secondary models from memory
    /// Default: 300 (5 minutes). Set to 0 to never auto-unload.
    /// Only applies when gpu_isolation = false
    #[serde(default = "default_cold_model_timeout")]
    pub cold_model_timeout_secs: u64,

    // --- Remote backend settings ---
    /// Remote server endpoint URL (e.g., "http://192.168.1.100:8080")
    /// Required when mode = "remote"
    #[serde(default)]
    pub remote_endpoint: Option<String>,

    /// Model name to send to remote server (default: "whisper-1")
    #[serde(default)]
    pub remote_model: Option<String>,

    /// API key for remote server (optional, can also use VOXTYPE_WHISPER_API_KEY env var)
    #[serde(default)]
    pub remote_api_key: Option<String>,

    /// Timeout for remote requests in seconds (default: 30)
    #[serde(default)]
    pub remote_timeout_secs: Option<u64>,

    // --- CLI backend settings ---
    /// Path to whisper-cli binary (optional, searches PATH if not set)
    /// Used when mode = "cli"
    #[serde(default)]
    pub whisper_cli_path: Option<String>,
}

impl WhisperConfig {
    /// Get the effective execution mode, preferring `mode` over deprecated `backend`
    pub fn effective_mode(&self) -> WhisperMode {
        // Prefer `mode` if set
        if let Some(mode) = self.mode {
            return mode;
        }
        // Fall back to deprecated `backend` with warning
        if let Some(backend) = self.backend {
            tracing::warn!("DEPRECATED: [whisper] backend is deprecated, use 'mode' instead");
            tracing::warn!(
                "  Change 'backend = \"{}\"' to 'mode = \"{}\"' in config.toml",
                match backend {
                    WhisperMode::Local => "local",
                    WhisperMode::Remote => "remote",
                    WhisperMode::Cli => "cli",
                },
                match backend {
                    WhisperMode::Local => "local",
                    WhisperMode::Remote => "remote",
                    WhisperMode::Cli => "cli",
                }
            );
            return backend;
        }
        WhisperMode::default()
    }
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            mode: None,    // Defaults to Local via effective_mode()
            backend: None, // Deprecated alias
            model: "base.en".to_string(),
            language: LanguageConfig::default(),
            translate: false,
            threads: None,
            on_demand_loading: default_on_demand_loading(),
            gpu_isolation: false,
            gpu_device: None,
            flash_attention: false,
            context_window_optimization: default_context_window_optimization(),
            eager_processing: false,
            eager_chunk_secs: default_eager_chunk_secs(),
            eager_overlap_secs: default_eager_overlap_secs(),
            initial_prompt: None,
            secondary_model: None,
            available_models: vec![],
            max_loaded_models: default_max_loaded_models(),
            cold_model_timeout_secs: default_cold_model_timeout(),
            remote_endpoint: None,
            remote_model: None,
            remote_api_key: None,
            remote_timeout_secs: None,
            whisper_cli_path: None,
        }
    }
}

pub(crate) fn default_on_demand_loading() -> bool {
    false
}

fn default_context_window_optimization() -> bool {
    false
}

fn default_max_loaded_models() -> usize {
    2 // Primary model + one secondary
}

fn default_cold_model_timeout() -> u64 {
    300 // 5 minutes
}

fn default_eager_chunk_secs() -> f32 {
    5.0
}

fn default_eager_overlap_secs() -> f32 {
    0.5
}

fn default_whisper_model() -> String {
    "base.en".to_string()
}
