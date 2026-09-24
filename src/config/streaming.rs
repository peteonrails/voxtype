//! Shared sliding-window streaming tuning config.
//!
//! `transcribe::sliding_window::SlidingWindowStreamingTranscriber` wraps any
//! batch `Transcriber` (whisper.cpp, OpenVINO GenAI, …) to add live
//! incremental transcription. Every backend that wraps itself in that engine
//! needs the exact same seven tuning knobs, so they live here once instead of
//! being copied into each engine's own config struct.
//!
//! This is deliberately separate from each engine's own `streaming: bool`
//! switch (e.g. `[whisper] streaming`, `[openvino] streaming`), which stays
//! per-engine: it is what selects the sliding-window engine for *that*
//! backend in the first place (and, for whisper, is gated on `mode =
//! "local"`), not a tunable knob shared across backends.
//!
//! Before this section existed, `[whisper]` and `[openvino]` each carried
//! their own verbatim copy of these seven fields (prefixed
//! `streaming_*`). Those per-engine fields are kept — unchanged, still
//! read straight out of `config.toml` — purely as a fallback for old
//! configs; see [`StreamingConfig::resolve`].

use serde::{Deserialize, Serialize};

/// Tuning knobs for the shared sliding-window streaming engine, configured
/// once under `[streaming]` instead of once per engine.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct StreamingConfig {
    /// Seconds between re-transcriptions of the rolling buffer. Lower =
    /// more responsive partials, higher CPU/NPU cost.
    pub interval_secs: f32,

    /// Maximum buffered audio (seconds) before the window slides (drops old
    /// samples to respect the model's context limit).
    pub max_buffer_secs: f32,

    /// Skip transcription while whole-buffer RMS is below this.
    pub min_speech_rms: f32,

    /// Minimum buffered audio (seconds) before the first partial is
    /// attempted.
    pub min_audio_secs: f32,

    /// Minimum number of new stable words before a delta is
    /// committed/typed.
    pub partial_min_words: usize,

    /// Type committed deltas live at the cursor (`true`) or only commit
    /// whole segments at once (`false`).
    pub type_partials: bool,

    /// Experimental: type the current best-guess tail immediately and
    /// correct it later via backspace + retype if a later tick disagrees,
    /// instead of withholding it until two consecutive ticks agree. See
    /// `transcribe::sliding_window`'s "Revision mode" doc section.
    pub revision_mode: bool,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            interval_secs: 0.8,
            max_buffer_secs: 29.0,
            min_speech_rms: 0.005,
            min_audio_secs: 1.0,
            partial_min_words: 1,
            type_partials: true,
            revision_mode: false,
        }
    }
}

impl StreamingConfig {
    /// Resolve the effective sliding-window tuning knobs for one engine.
    ///
    /// The shared `[streaming]` section (`shared`) wins outright when
    /// present in config.toml. Otherwise, fall back to `legacy` — that
    /// engine's own deprecated per-field copies (`[whisper]
    /// streaming_interval_secs`, etc.), which used to be the only place
    /// these settings lived. A one-time warning is logged the first time
    /// the fallback is actually carrying a customization (i.e. `legacy`
    /// isn't just the defaults again), so a user who genuinely relies on
    /// the old fields is told to migrate; a user who never touched them
    /// gets identical behavior either way and no noise.
    pub fn resolve(
        shared: Option<&StreamingConfig>,
        legacy: StreamingConfig,
        engine: &str,
    ) -> StreamingConfig {
        if let Some(shared) = shared {
            return *shared;
        }

        if legacy != StreamingConfig::default() {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                tracing::warn!(
                    "[{engine}] streaming_* fields are deprecated in favor of a shared \
                     [streaming] section (same field names, minus the 'streaming_' \
                     prefix, e.g. streaming_interval_secs -> [streaming] interval_secs). \
                     The old per-engine fields still work for now."
                );
            });
        }

        legacy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_shared_section_when_present() {
        let shared = StreamingConfig {
            interval_secs: 0.3,
            ..StreamingConfig::default()
        };
        let legacy = StreamingConfig {
            interval_secs: 5.0,
            ..StreamingConfig::default()
        };
        let resolved = StreamingConfig::resolve(Some(&shared), legacy, "whisper");
        assert_eq!(resolved, shared);
    }

    #[test]
    fn resolve_falls_back_to_legacy_when_shared_absent() {
        let legacy = StreamingConfig {
            interval_secs: 5.0,
            ..StreamingConfig::default()
        };
        let resolved = StreamingConfig::resolve(None, legacy, "whisper");
        assert_eq!(resolved, legacy);
    }

    #[test]
    fn resolve_with_default_legacy_and_no_shared_matches_default() {
        let resolved = StreamingConfig::resolve(None, StreamingConfig::default(), "whisper");
        assert_eq!(resolved, StreamingConfig::default());
    }

    #[test]
    fn default_config_omits_the_streaming_table() {
        // A None `Option<StreamingConfig>` field must serialize with no
        // [streaming] table at all, so `parse_config_with_defaults`'s
        // defaults-then-merge dance doesn't fabricate a shared section for
        // configs that never had one (see config::parse).
        #[derive(Serialize, Default)]
        struct Holder {
            #[serde(default)]
            streaming: Option<StreamingConfig>,
        }
        let value = toml::Value::try_from(Holder::default()).unwrap();
        assert!(value.get("streaming").is_none());
    }
}
