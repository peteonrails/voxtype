//! Transcription engine selection and per-engine configuration modules.

use serde::{Deserialize, Serialize};

mod cohere;
mod dolphin;
mod moonshine;
mod omnilingual;
mod paraformer;
mod parakeet;
mod sensevoice;
mod soniox;

pub use cohere::CohereConfig;
pub use dolphin::DolphinConfig;
pub use moonshine::MoonshineConfig;
pub use omnilingual::OmnilingualConfig;
pub use paraformer::ParaformerConfig;
pub use parakeet::{ParakeetConfig, ParakeetModelType};
pub use sensevoice::SenseVoiceConfig;
pub use soniox::SonioxConfig;

/// Transcription engine selection (which ASR technology to use)
#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    Default,
    strum::IntoStaticStr,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum TranscriptionEngine {
    /// Use Whisper (whisper.cpp via whisper-rs)
    #[default]
    Whisper,
    /// Use Parakeet (NVIDIA's FastConformer via ONNX Runtime)
    /// Requires: cargo build --features parakeet
    Parakeet,
    /// Use Moonshine (encoder-decoder ASR via ONNX Runtime)
    /// Requires: cargo build --features moonshine
    Moonshine,
    /// Use SenseVoice (Alibaba FunAudioLLM CTC model via ONNX Runtime)
    /// Requires: cargo build --features sensevoice
    SenseVoice,
    /// Use Paraformer (FunASR CTC encoder via ONNX Runtime)
    /// Requires: cargo build --features paraformer
    Paraformer,
    /// Use Dolphin (dictation-optimized CTC encoder via ONNX Runtime)
    /// Requires: cargo build --features dolphin
    Dolphin,
    /// Use Omnilingual (FunASR 50+ language CTC encoder via ONNX Runtime)
    /// Requires: cargo build --features omnilingual
    Omnilingual,
    /// Use Cohere Transcribe (encoder-decoder via ONNX Runtime, Whisper-style
    /// task tokens). Top of the Open ASR Leaderboard.
    /// Requires: cargo build --features cohere
    Cohere,
    /// Use Soniox (cloud streaming WebSocket STT).
    /// Requires: cargo build --features soniox
    Soniox,
}

impl TranscriptionEngine {
    /// Canonical lowercase name, matching this enum's serde representation.
    /// Backed by `strum::IntoStaticStr` so a new variant added to the enum
    /// picks up its `serialize_all = "lowercase"` name automatically — no
    /// match block to keep in sync. The hand-written `name()` method
    /// remains as a stable public API so call sites read `engine.name()`
    /// rather than the less-obvious `(*engine).into()`.
    pub fn name(self) -> &'static str {
        self.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use strum::IntoEnumIterator;

    /// Pin `crate::cli::ENGINE_NAMES_CSV` to the `TranscriptionEngine` enum.
    ///
    /// The constant lives in `src/cli/mod.rs` (not here) because `build.rs`
    /// includes the CLI tree via `#[path]` and cannot see `crate::config`.
    /// This test enforces that whenever a new engine variant is added to
    /// the enum, the CLI string is updated too. Otherwise the help text
    /// drifts (the bug that motivated this test: `soniox` was missing from
    /// two CLI help strings for a release because the lists were hand-maintained).
    #[test]
    fn cli_engine_names_csv_lists_every_variant() {
        let from_enum: Vec<&str> = TranscriptionEngine::iter().map(|e| e.name()).collect();
        let from_const: Vec<&str> = crate::cli::ENGINE_NAMES_CSV.split(", ").collect();
        assert_eq!(
            from_enum, from_const,
            "src/cli/mod.rs::ENGINE_NAMES_CSV is out of sync with TranscriptionEngine. \
             Update the constant to match every variant's name() in declaration order."
        );
    }

    #[test]
    fn test_parse_engine_whisper() {
        let toml_str = r#"
            engine = "whisper"

            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.engine, TranscriptionEngine::Whisper);
    }

    #[test]
    fn test_parse_engine_parakeet() {
        let toml_str = r#"
            engine = "parakeet"

            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [parakeet]
            model = "parakeet-tdt-0.6b-v3"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.engine, TranscriptionEngine::Parakeet);
        assert!(config.parakeet.is_some());
        assert_eq!(
            config.parakeet.as_ref().unwrap().model,
            "parakeet-tdt-0.6b-v3"
        );
    }

    #[test]
    fn test_engine_defaults_to_whisper() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.engine, TranscriptionEngine::Whisper);
    }
}
