//! Transcription engine selection and per-engine configuration modules.

use serde::{Deserialize, Serialize};

pub mod cohere;
pub mod dolphin;
pub mod moonshine;
pub mod omnilingual;
pub mod paraformer;
pub mod parakeet;
pub mod sensevoice;
pub mod soniox;

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
