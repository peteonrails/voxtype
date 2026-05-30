//! Soniox engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_true;

/// Soniox cloud streaming WebSocket STT configuration
/// Requires: cargo build --features soniox
///
/// Soniox is a paid cloud STT provider. API key required:
/// either set `api_key` here or via the `SONIOX_API_KEY` env var.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SonioxConfig {
    /// API key. If unset, falls back to the SONIOX_API_KEY env var.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Soniox model name. Default: "stt-rt-v4".
    #[serde(default = "default_soniox_model")]
    pub model: String,

    /// Language hints (ISO 639-1 codes). Default: ["hu", "en"].
    /// Empty array means auto-detect.
    #[serde(default = "default_soniox_language_hints")]
    pub language_hints: Vec<String>,

    /// Strictly restrict recognition to the languages in `language_hints`.
    /// When true (default), the model strongly prefers producing output
    /// only in the hinted languages, avoiding occasional drift to a third
    /// language in mid-stream partials. Ignored when `language_hints` is
    /// empty. See https://soniox.com/docs/stt/concepts/language-restrictions.
    #[serde(default = "default_true")]
    pub language_hints_strict: bool,

    /// Streaming mode. true = WebSocket session with live partials
    /// (requires [hotkey] mode = "toggle"; PTT auto-promoted to toggle).
    /// false = batch mode: buffer audio while held, send one-shot on release
    /// (PTT-compatible).
    #[serde(default = "default_true")]
    pub streaming: bool,

    /// Type partials at cursor as they arrive (streaming mode only).
    /// false = only finalized segments are typed. Default: true.
    #[serde(default = "default_true")]
    pub type_partials: bool,

    /// Free-form context text — mapped to `context.text` in Soniox's init
    /// frame. Use for short domain prose ("medical consultation",
    /// "podcast about Rust async runtime"). See
    /// https://soniox.com/docs/stt/concepts/context.
    #[serde(default)]
    pub context: Option<String>,

    /// Vocabulary boost terms (proper names, jargon, product names).
    /// Mapped to `context.terms` in Soniox's init frame. Can be combined
    /// with `terms_file`; entries are deduplicated in order.
    #[serde(default)]
    pub terms: Option<Vec<String>>,

    /// Path to a JSON file containing a list of vocabulary boost terms
    /// (`["term1", "term2", ...]`). Loaded once at daemon startup and
    /// merged into `context.terms`. Useful for sharing a single
    /// corrections list across multiple voxtype config snapshots.
    #[serde(default)]
    pub terms_file: Option<std::path::PathBuf>,

    /// Use the Soniox async transcription API (file upload + poll) instead
    /// of the realtime WebSocket. Higher accuracy, PTT-compatible, batch
    /// only (no live partials). When true, overrides `streaming` and
    /// `type_partials`. Default model becomes `stt-async-v4`.
    /// Default: false.
    #[serde(default)]
    pub async_api: bool,

    /// Maximum total wait time (seconds) for an async API job to complete
    /// before giving up. Default: 120.
    #[serde(default = "default_soniox_async_max_wait_secs")]
    pub async_max_wait_secs: u64,
}

fn default_soniox_model() -> String {
    "stt-rt-v4".to_string()
}

fn default_soniox_language_hints() -> Vec<String> {
    vec!["hu".to_string(), "en".to_string()]
}

fn default_soniox_async_max_wait_secs() -> u64 {
    120
}

impl Default for SonioxConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model: default_soniox_model(),
            language_hints: default_soniox_language_hints(),
            language_hints_strict: true,
            streaming: true,
            type_partials: true,
            context: None,
            terms: None,
            terms_file: None,
            async_api: false,
            async_max_wait_secs: default_soniox_async_max_wait_secs(),
        }
    }
}
