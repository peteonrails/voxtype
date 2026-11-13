use serde::{Deserialize, Serialize};

/// Shared offline transcribe.cpp engine for curated GGUF families that do not
/// have a legacy Voxtype runtime.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GgufConfig {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub architecture: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_backend")]
    pub gguf_backend: String,
    #[serde(default)]
    pub threads: Option<usize>,
    #[serde(default)]
    pub on_demand_loading: bool,
    #[serde(default)]
    pub streaming: bool,
}

fn default_language() -> String {
    "auto".to_string()
}

fn default_backend() -> String {
    "auto".to_string()
}

impl Default for GgufConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            architecture: String::new(),
            language: default_language(),
            gguf_backend: default_backend(),
            threads: None,
            on_demand_loading: false,
            streaming: false,
        }
    }
}
