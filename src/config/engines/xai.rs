//! xAI Grok Speech-to-Text (`POST https://api.x.ai/v1/stt`).

use serde::{Deserialize, Serialize};

use super::super::default_true;

/// Cloud batch STT. Auth: `[xai] api_key`, env keys, or `voxtype setup xai --login`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct XaiConfig {
    #[serde(default)]
    pub api_key: Option<String>,

    /// Default `https://api.x.ai/v1/stt`. Must be https.
    #[serde(default)]
    pub endpoint: Option<String>,

    /// ISO-639-1. Empty or `auto` = autodetect (then `format` is omitted).
    #[serde(default)]
    pub language: Option<String>,

    /// Inverse text normalization. xAI requires `language` when this is true.
    #[serde(default = "default_true")]
    pub format: bool,

    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

impl Default for XaiConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            endpoint: None,
            language: None,
            format: true,
            timeout_secs: Some(120),
        }
    }
}
