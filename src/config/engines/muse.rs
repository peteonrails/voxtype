//! Muse Voice Transcribe engine configuration.
//!
//! Hosted-only model via Meta Model API (`https://api.meta.ai/v1`).

use serde::{Deserialize, Serialize};

/// Muse Voice Transcribe configuration (hosted API).
///
/// Muse is Meta's real-time streaming ASR (80 ms chunks, diarization,
/// endpointing) available through the Meta Model API as
/// `muse-voice-transcribe-1.0` at $3 / 1k audio minutes.
///
/// API key required: either set `api_key` here or via env
/// `VOXTYPE_MUSE_API_KEY` (also accepts `MUSE_API_KEY` / `META_API_KEY`
/// for compatibility with `muse` tooling).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MuseConfig {
    /// API key for Meta Model API. If unset, falls back to env vars.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Model ID. Default: "muse-voice-transcribe-1.0".
    #[serde(default = "default_muse_model")]
    pub model: String,

    /// Override base endpoint. Default: "https://api.meta.ai/v1".
    /// Used for testing / self-hosted proxies.
    #[serde(default)]
    pub endpoint: Option<String>,

    /// Enable streaming transcription (live partials). Default: true.
    /// When true, wraps the batch transcriber in the shared sliding-window
    /// engine so text lands at the cursor incrementally, matching
    /// `[whisper] streaming` behaviour.
    #[serde(default = "default_true")]
    pub streaming: bool,

    /// Optional language hint (ISO 639-1, e.g. "en"). None = auto-detect.
    #[serde(default)]
    pub language: Option<String>,

    /// WebSocket streaming endpoint override. Default:
    /// `wss://api.meta.ai/v1/asr/realtime` (or `wss://` variant of `endpoint`).
    #[serde(default)]
    pub ws_endpoint: Option<String>,

    /// Whether to request interim (partial) results. Default: true.
    #[serde(default = "default_true")]
    pub interim_results: bool,

    /// Enable speaker diarization (prefixes "Speaker N:"). Default: false.
    #[serde(default)]
    pub diarization: bool,

    /// Vocabulary bias terms (proper names, jargon, product words).
    /// Mapped 1:1 to the server's `keywords` array in the handshake.
    #[serde(default)]
    pub keywords: Option<Vec<String>>,

    /// End-of-stream polish command (optional, disabled when unset). When set,
    /// the full stream transcript is piped through this shell command when
    /// the stream ends (stdin in, cleaned text on stdout — same contract as
    /// `[output.post_process]`) and the cleaned result replaces what was
    /// typed via a single `Replace`. Fail-open: on error, timeout, or empty
    /// output the raw transcript is kept.
    #[serde(default)]
    pub polish_command: Option<String>,

    /// Timeout for the polish command in milliseconds.
    /// Read by the daemon's end-of-stream polish step when configured.
    #[serde(default = "default_polish_timeout")]
    pub polish_timeout_ms: u64,
}

/// Default grace period for the stop drain (see `stop_drain_timeout_ms`).
pub const DEFAULT_STOP_DRAIN_TIMEOUT_MS: u64 = 3000;

fn default_muse_model() -> String {
    "muse-voice-transcribe-1.0".to_string()
}

fn default_true() -> bool {
    true
}

fn default_polish_timeout() -> u64 {
    20000 // 20 seconds — one cheap-model call per turn, fail-open on timeout
}

impl Default for MuseConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model: default_muse_model(),
            endpoint: None,
            streaming: true,
            language: None,
            ws_endpoint: None,
            interim_results: true,
            diarization: false,
            keywords: None,
            polish_command: None,
            polish_timeout_ms: default_polish_timeout(),
        }
    }
}

impl MuseConfig {
    /// Effective endpoint, always with a trailing slash trimmed.
    pub fn effective_endpoint(&self) -> String {
        self.endpoint
            .clone()
            .unwrap_or_else(|| "https://api.meta.ai/v1".to_string())
    }

    /// Effective WebSocket endpoint for streaming.
    pub fn effective_ws_endpoint(&self) -> String {
        if let Some(ws) = &self.ws_endpoint {
            return ws.trim_end_matches('/').to_string();
        }
        // Derive from REST endpoint if it's http(s), otherwise use default.
        let base = self.effective_endpoint();
        if base.starts_with("https://") {
            return base.replacen("https://", "wss://", 1) + "/asr/realtime";
        }
        if base.starts_with("http://") {
            return base.replacen("http://", "ws://", 1) + "/asr/realtime";
        }
        if base.starts_with("wss://") || base.starts_with("ws://") {
            return base;
        }
        "wss://api.meta.ai/v1/asr/realtime".to_string()
    }

    /// Resolve API key from config or env.
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key.clone().or_else(|| {
            std::env::var("VOXTYPE_MUSE_API_KEY")
                .or_else(|_| std::env::var("MUSE_API_KEY"))
                .or_else(|_| std::env::var("META_API_KEY"))
                .or_else(|_| std::env::var("META_MODEL_API_KEY"))
                .ok()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_is_voice_transcribe() {
        assert_eq!(MuseConfig::default().model, "muse-voice-transcribe-1.0");
    }

    #[test]
    fn effective_endpoint_defaults_to_meta_api() {
        let cfg = MuseConfig::default();
        assert_eq!(cfg.effective_endpoint(), "https://api.meta.ai/v1");
    }

    #[test]
    fn deserialize_muse_table() {
        let toml_str = r#"
            api_key = "sk_test"
            model = "muse-voice-transcribe-1.0"
            streaming = true
        "#;
        let cfg: MuseConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.api_key.as_deref(), Some("sk_test"));
        assert!(cfg.streaming);
    }

    #[test]
    fn deserialize_keywords_array() {
        let cfg: MuseConfig = toml::from_str(r#"keywords = ["Omarchy", "Hyprland"]"#).unwrap();
        assert_eq!(
            cfg.keywords.as_deref(),
            Some(&["Omarchy".to_string(), "Hyprland".to_string()][..])
        );
        assert!(MuseConfig::default().keywords.is_none());
    }
}
