//! Voice activity detection configuration.

use serde::{Deserialize, Serialize};

/// VAD backend selection
///
/// Determines which voice activity detection algorithm to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VadBackend {
    /// Auto-select based on transcription engine (default)
    /// - Whisper engine: uses Whisper VAD (requires model download)
    /// - Parakeet engine: uses Energy VAD (no model needed)
    #[default]
    Auto,
    /// Energy-based VAD using RMS amplitude analysis
    /// Fast, no model download required, works with any engine
    Energy,
    /// Whisper VAD using whisper-rs built-in Silero model (GGML format)
    /// More accurate but requires downloading ggml-silero-vad.bin
    Whisper,
}

/// Voice Activity Detection configuration
///
/// VAD filters silence-only recordings before transcription to prevent
/// Whisper hallucinations when processing silence.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VadConfig {
    /// Enable Voice Activity Detection (default: false)
    /// When enabled, recordings with no detected speech are rejected before transcription
    #[serde(default)]
    pub enabled: bool,

    /// VAD backend to use (default: auto)
    /// - auto: Whisper VAD for Whisper engine, Energy VAD for Parakeet
    /// - energy: Simple RMS-based detection, no model needed
    /// - whisper: Silero VAD via whisper-rs, requires model download
    #[serde(default)]
    pub backend: VadBackend,

    /// Speech detection threshold (0.0-1.0, default: 0.5)
    /// Higher values require more confident speech detection
    #[serde(default = "default_vad_threshold")]
    pub threshold: f32,

    /// Minimum speech duration in milliseconds (default: 100)
    /// Recordings with less speech than this are rejected
    #[serde(default = "default_min_speech_duration_ms")]
    pub min_speech_duration_ms: u32,

    /// Path to VAD model file (optional, for Whisper VAD backend)
    /// If not set, uses the default model location (~/.local/share/voxtype/models/)
    #[serde(default)]
    pub model: Option<String>,
}

fn default_vad_threshold() -> f32 {
    0.5
}

fn default_min_speech_duration_ms() -> u32 {
    100
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: VadBackend::default(),
            threshold: default_vad_threshold(),
            min_speech_duration_ms: default_min_speech_duration_ms(),
            model: None,
        }
    }
}
