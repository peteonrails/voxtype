//! Volcengine Seed-ASR engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_true;

/// Default Seed-ASR 2.0 bidirectional streaming endpoint.
pub const DEFAULT_SEEDASR_URL: &str = "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async";

/// Default duration-based Seed-ASR 2.0 resource identifier.
pub const DEFAULT_SEEDASR_RESOURCE_ID: &str = "volc.seedasr.sauc.duration";

/// Volcengine Seed-ASR WebSocket configuration.
///
/// New-console credentials use `api_key`. Legacy-console credentials use
/// `app_id` and `access_token`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SeedAsrConfig {
    /// New-console API key. Falls back to `SEEDASR_API_KEY`.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Legacy-console application ID. Falls back to `SEEDASR_APP_ID`.
    #[serde(default)]
    pub app_id: Option<String>,

    /// Legacy-console access token. Falls back to `SEEDASR_ACCESS_TOKEN`.
    #[serde(default)]
    pub access_token: Option<String>,

    /// Volcengine service resource ID.
    #[serde(default = "default_resource_id")]
    pub resource_id: String,

    /// WebSocket endpoint. Override this for a compatible regional endpoint
    /// or a local protocol test server.
    #[serde(default = "default_url")]
    pub url: String,

    /// Use the native bidirectional streaming pipeline. When false, voxtype
    /// buffers the recording and uses the same endpoint as a one-shot request.
    #[serde(default = "default_true")]
    pub streaming: bool,

    /// Type stable partial results while recording. Disabled by default to
    /// avoid visible cursor churn when the model revises its current sentence.
    #[serde(default)]
    pub type_partials: bool,

    /// Optional recognition language code. Omit for automatic detection.
    #[serde(default)]
    pub language: Option<String>,

    /// Enable inverse text normalization.
    #[serde(default = "default_true")]
    pub enable_itn: bool,

    /// Enable punctuation.
    #[serde(default = "default_true")]
    pub enable_punc: bool,

    /// Enable semantic smoothing and filler-word removal.
    #[serde(default)]
    pub enable_ddc: bool,

    /// Server-side silence window in milliseconds used to finalize an
    /// utterance. Volcengine recommends 800-1000 ms for realtime dictation.
    #[serde(default = "default_end_window_ms")]
    pub end_window_ms: u32,
}

fn default_resource_id() -> String {
    DEFAULT_SEEDASR_RESOURCE_ID.to_string()
}

fn default_url() -> String {
    DEFAULT_SEEDASR_URL.to_string()
}

fn default_end_window_ms() -> u32 {
    800
}

impl Default for SeedAsrConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            app_id: None,
            access_token: None,
            resource_id: default_resource_id(),
            url: default_url(),
            streaming: true,
            type_partials: false,
            language: None,
            enable_itn: true,
            enable_punc: true,
            enable_ddc: false,
            end_window_ms: default_end_window_ms(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, TranscriptionEngine};

    #[test]
    fn defaults_target_seedasr_2_bidirectional_streaming() {
        let cfg = SeedAsrConfig::default();
        assert_eq!(cfg.url, DEFAULT_SEEDASR_URL);
        assert_eq!(cfg.resource_id, DEFAULT_SEEDASR_RESOURCE_ID);
        assert!(cfg.streaming);
        assert!(!cfg.type_partials);
        assert_eq!(cfg.end_window_ms, 800);
    }

    #[test]
    fn parses_legacy_configuration_and_enables_streaming_gate() {
        let cfg: Config = toml::from_str(
            r#"
                engine = "seedasr"

                [seedasr]
                app_id = "app"
                access_token = "token"
            "#,
        )
        .unwrap();

        assert_eq!(cfg.engine, TranscriptionEngine::SeedAsr);
        assert!(cfg.streaming_active());
        let seedasr = cfg.seedasr.unwrap();
        assert_eq!(seedasr.resource_id, DEFAULT_SEEDASR_RESOURCE_ID);
        assert_eq!(seedasr.url, DEFAULT_SEEDASR_URL);
    }
}
