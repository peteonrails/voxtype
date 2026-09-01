//! Deepgram batch speech-to-text engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_true;

/// Deepgram pre-recorded speech-to-text configuration.
///
/// Credentials are resolved from `DEEPGRAM_API_KEY` first. `api_key` exists
/// as a fallback for environments where injecting an environment variable is
/// impractical, but storing secrets in the config file is discouraged.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeepgramConfig {
    /// API key fallback. Prefer the DEEPGRAM_API_KEY environment variable.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Deepgram model identifier.
    #[serde(default = "default_model")]
    pub model: String,

    /// BCP-47 language code, `auto` for language detection, or `multi` for
    /// Nova multilingual code-switching.
    #[serde(default = "default_language")]
    pub language: String,

    /// Apply punctuation, paragraphs, numerals, dates, and related formatting.
    #[serde(default = "default_true")]
    pub smart_format: bool,

    /// Opt audio out of Deepgram's Model Improvement Program.
    #[serde(default = "default_true")]
    pub mip_opt_out: bool,

    /// Total request timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    /// Pre-recorded transcription endpoint. Configurable for regional and
    /// self-hosted deployments as well as deterministic local tests.
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
}

fn default_model() -> String {
    "nova-3".to_string()
}

fn default_language() -> String {
    "en".to_string()
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_endpoint() -> String {
    "https://api.deepgram.com/v1/listen".to_string()
}

impl Default for DeepgramConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model: default_model(),
            language: default_language(),
            smart_format: true,
            mip_opt_out: true,
            timeout_secs: default_timeout_secs(),
            endpoint: default_endpoint(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_dictation_friendly() {
        let config = DeepgramConfig::default();
        assert_eq!(config.model, "nova-3");
        assert_eq!(config.language, "en");
        assert!(config.smart_format);
        assert!(config.mip_opt_out);
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.endpoint, "https://api.deepgram.com/v1/listen");
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let config: DeepgramConfig = toml::from_str("language = \"multi\"").unwrap();
        assert_eq!(config.model, "nova-3");
        assert_eq!(config.language, "multi");
        assert!(config.smart_format);
        assert!(config.mip_opt_out);
    }
}
