//! Deepgram speech-to-text engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_true;

/// The model a new `[deepgram]` table starts with.
pub const DEEPGRAM_DEFAULT_MODEL: &str = "nova-3";

/// Deepgram speech-to-text configuration.
///
/// Credentials are resolved from `VOXTYPE_DEEPGRAM_API_KEY` or
/// `DEEPGRAM_API_KEY` first. `api_key` exists as a fallback for environments
/// where injecting an environment variable is impractical, but storing
/// secrets in the config file is discouraged.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeepgramConfig {
    /// API key fallback. Prefer the DEEPGRAM_API_KEY environment variable.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Deepgram model identifier.
    #[serde(default = "default_model")]
    pub model: String,

    /// BCP-47 language code, `auto` for language detection, or `multi` for
    /// Nova multilingual code-switching. Deepgram detects the language of a
    /// finished recording only, so a live stream with `auto` is English.
    #[serde(default = "default_language")]
    pub language: String,

    /// Apply punctuation, paragraphs, numerals, dates, and related formatting.
    #[serde(default = "default_true")]
    pub smart_format: bool,

    /// Opt audio out of Deepgram's Model Improvement Program.
    #[serde(default = "default_true")]
    pub mip_opt_out: bool,

    /// Stream audio to Deepgram's realtime API while recording, so the
    /// transcript is ready when recording stops. `false` sends the finished
    /// recording to the pre-recorded API instead.
    #[serde(default = "default_true")]
    pub streaming: bool,

    /// Streaming only: how many milliseconds of silence end a segment.
    /// Unset uses Deepgram's default.
    #[serde(default)]
    pub endpointing_ms: Option<u32>,

    /// How long to wait for Deepgram, in seconds: the whole request for a
    /// finished recording, and the connection and the last transcript after
    /// recording stops when streaming.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    /// Deepgram's `/v1/listen` endpoint. The realtime API uses the same URL
    /// over `wss://`, so either scheme works. Configurable for regional and
    /// self-hosted deployments as well as deterministic local tests.
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
}

fn default_model() -> String {
    DEEPGRAM_DEFAULT_MODEL.to_string()
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
            streaming: true,
            endpointing_ms: None,
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
        assert!(config.streaming);
        assert_eq!(config.endpointing_ms, None);
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
        assert!(config.streaming);
    }
}
