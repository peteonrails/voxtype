//! Deepgram engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_true;

/// Default Deepgram realtime WebSocket endpoint.
pub const DEFAULT_DEEPGRAM_ENDPOINT: &str = "wss://api.deepgram.com/v1/listen";

/// Deepgram cloud streaming WebSocket STT configuration.
/// Requires: cargo build --features deepgram
///
/// Deepgram is a paid cloud STT provider. API key required:
/// either set `api_key` here or via the `DEEPGRAM_API_KEY` env var.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeepgramConfig {
    /// API key. If unset, falls back to the DEEPGRAM_API_KEY env var.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Deepgram model name. Default: "nova-3".
    #[serde(default = "default_deepgram_model")]
    pub model: String,

    /// Language code (BCP-47, e.g. "en", "en-US"). Default: "en".
    #[serde(default = "default_deepgram_language")]
    pub language: String,

    /// WebSocket endpoint. Default: Deepgram's production realtime endpoint.
    /// Override to point at a self-hosted or on-prem Deepgram instance.
    /// Only the scheme+host is used (an optional trailing `/v1/listen` is
    /// stripped); custom path prefixes on the endpoint are not preserved.
    #[serde(default = "default_deepgram_endpoint")]
    pub endpoint: String,

    /// Enable Deepgram smart formatting (punctuation, numerals, etc.).
    /// Default: true.
    #[serde(default = "default_true")]
    pub smart_format: bool,

    /// Endpointing duration in milliseconds: how long Deepgram waits after
    /// silence before finalizing a transcript segment. `None` uses
    /// Deepgram's default. Recommended: 300-500ms for conversational
    /// speech, 100-200ms for snappier finalization.
    #[serde(default)]
    pub endpointing_ms: Option<u32>,

    /// Streaming mode. true = live WebSocket session (requires
    /// [hotkey] mode = "toggle"; PTT auto-promoted to toggle).
    /// false = batch: buffer audio while held, send a single one-shot
    /// streaming round on release (PTT-compatible).
    #[serde(default = "default_true")]
    pub streaming: bool,

    /// Timeout in seconds for finalizing transcription after recording
    /// stops. Longer recordings may need more time for Deepgram to flush
    /// remaining segments. Default: 15 seconds.
    #[serde(default = "default_deepgram_finish_timeout_secs")]
    pub finish_timeout_secs: u64,
}

fn default_deepgram_model() -> String {
    "nova-3".to_string()
}

fn default_deepgram_language() -> String {
    "en".to_string()
}

fn default_deepgram_endpoint() -> String {
    DEFAULT_DEEPGRAM_ENDPOINT.to_string()
}

fn default_deepgram_finish_timeout_secs() -> u64 {
    15
}

impl Default for DeepgramConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model: default_deepgram_model(),
            language: default_deepgram_language(),
            endpoint: default_deepgram_endpoint(),
            smart_format: true,
            endpointing_ms: None,
            streaming: true,
            finish_timeout_secs: default_deepgram_finish_timeout_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DeepgramConfig::default();
        assert_eq!(config.model, "nova-3");
        assert_eq!(config.language, "en");
        assert_eq!(config.endpoint, DEFAULT_DEEPGRAM_ENDPOINT);
        assert!(config.smart_format);
        assert!(config.streaming);
        assert_eq!(config.finish_timeout_secs, 15);
        assert!(config.endpointing_ms.is_none());
    }

    #[test]
    fn test_parse_minimal() {
        let toml_str = r#"
            api_key = "secret"
            model = "nova-2"
        "#;
        let config: DeepgramConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.api_key.as_deref(), Some("secret"));
        assert_eq!(config.model, "nova-2");
        // Unspecified fields fall back to defaults.
        assert_eq!(config.language, "en");
        assert!(config.streaming);
    }
}
