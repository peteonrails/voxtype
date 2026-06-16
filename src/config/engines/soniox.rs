//! Soniox engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_true;

/// Default realtime (WebSocket) model id, used when `model` is unset.
pub const DEFAULT_REALTIME_MODEL: &str = "stt-rt-v5";

/// Default async (REST) model id, used when `model` is unset and `async_api`
/// is enabled (including meeting mode).
pub const DEFAULT_ASYNC_MODEL: &str = "stt-async-v5";

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

    /// Soniox model id. When unset (the default), voxtype picks the model
    /// for the active mode: `stt-rt-v5` for the realtime WebSocket, or
    /// `stt-async-v5` when `async_api = true` (including meeting mode, which
    /// always uses the async API). Set an explicit value to override; an
    /// explicit model is used verbatim in both modes (no auto-swap).
    #[serde(default)]
    pub model: Option<String>,

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
    /// `type_partials`. With `model` unset, the default model becomes
    /// `stt-async-v5`. Default: false.
    #[serde(default)]
    pub async_api: bool,

    /// Maximum total wait time (seconds) for an async API job to complete
    /// before giving up. Default: 120.
    #[serde(default = "default_soniox_async_max_wait_secs")]
    pub async_max_wait_secs: u64,
}

impl SonioxConfig {
    /// Resolve the Soniox model id for the active mode. An explicit `model`
    /// is used verbatim; when unset, the per-mode default is chosen
    /// (`stt-async-v5` under `async_api`, otherwise `stt-rt-v5`).
    pub fn resolved_model(&self) -> &str {
        match self.model.as_deref() {
            Some(m) => m,
            None if self.async_api => DEFAULT_ASYNC_MODEL,
            None => DEFAULT_REALTIME_MODEL,
        }
    }
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
            model: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_model_resolves_to_realtime_default() {
        let cfg = SonioxConfig::default();
        assert!(!cfg.async_api);
        assert_eq!(cfg.resolved_model(), DEFAULT_REALTIME_MODEL);
    }

    #[test]
    fn unset_model_resolves_to_async_default_under_async_api() {
        let cfg = SonioxConfig {
            async_api: true,
            ..Default::default()
        };
        assert_eq!(cfg.resolved_model(), DEFAULT_ASYNC_MODEL);
    }

    #[test]
    fn explicit_model_is_used_verbatim_in_both_modes() {
        let realtime = SonioxConfig {
            model: Some("stt-rt-v5".into()),
            async_api: false,
            ..Default::default()
        };
        assert_eq!(realtime.resolved_model(), "stt-rt-v5");

        // Explicit model survives async_api: no auto-swap. A user who pins a
        // model owns the choice in both modes.
        let async_pinned = SonioxConfig {
            model: Some("stt-rt-v5".into()),
            async_api: true,
            ..Default::default()
        };
        assert_eq!(async_pinned.resolved_model(), "stt-rt-v5");
    }

    #[test]
    fn unset_model_omitted_from_serialized_toml() {
        // None must not write a `model = ...` line, so meeting mode keeps
        // resolving to the async default for default configs.
        let toml = toml::to_string(&SonioxConfig::default()).unwrap();
        assert!(
            !toml.contains("model ="),
            "unexpected model key in:\n{toml}"
        );
    }
}
