//! Meeting-mode configuration.

use serde::{Deserialize, Serialize};

use super::default_true;

/// Meeting transcription configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingConfig {
    /// Enable meeting mode
    #[serde(default)]
    pub enabled: bool,

    /// Duration of each audio chunk in seconds
    #[serde(default = "default_chunk_duration")]
    pub chunk_duration_secs: u32,

    /// Storage path for meetings ("auto" for default location)
    /// Default: ~/.local/share/voxtype/meetings/
    #[serde(default = "default_storage_path")]
    pub storage_path: String,

    /// Retain raw audio files after transcription
    #[serde(default)]
    pub retain_audio: bool,

    /// Maximum meeting duration in minutes (0 = unlimited)
    #[serde(default = "default_max_duration")]
    pub max_duration_mins: u32,

    /// Meeting audio configuration
    #[serde(default)]
    pub audio: MeetingAudioConfig,

    /// Diarization configuration
    #[serde(default)]
    pub diarization: MeetingDiarizationConfig,

    /// Summarization configuration
    #[serde(default)]
    pub summary: MeetingSummaryConfig,
}

/// Meeting audio configuration for dual capture
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingAudioConfig {
    /// Microphone device (uses main audio.device if not specified)
    #[serde(default = "default_mic_device")]
    pub mic_device: String,

    /// Loopback device for capturing remote participants
    /// Options: "auto" (detect), "disabled", or specific device name
    #[serde(default = "default_loopback")]
    pub loopback_device: String,

    /// Echo cancellation mode for removing speaker bleed-through from mic
    /// Options: "auto" (GTCRN neural enhancement + transcript dedup), "disabled"
    /// The GTCRN model (~523KB) is auto-downloaded on first meeting start.
    /// For system-level echo cancellation, configure PipeWire's echo-cancel module
    /// and set this to "disabled".
    #[serde(default = "default_echo_cancel")]
    pub echo_cancel: String,

    /// RMS threshold for meeting chunk voice activity detection.
    /// Lower values are more permissive; 0.0 disables the pre-transcription gate.
    #[serde(default = "default_meeting_vad_threshold")]
    pub vad_threshold: f32,
}

fn default_mic_device() -> String {
    "default".to_string()
}

fn default_loopback() -> String {
    "auto".to_string()
}

fn default_echo_cancel() -> String {
    "auto".to_string()
}

fn default_meeting_vad_threshold() -> f32 {
    0.01
}

impl Default for MeetingAudioConfig {
    fn default() -> Self {
        Self {
            mic_device: default_mic_device(),
            loopback_device: default_loopback(),
            echo_cancel: default_echo_cancel(),
            vad_threshold: default_meeting_vad_threshold(),
        }
    }
}

/// Meeting diarization configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingDiarizationConfig {
    /// Enable speaker diarization
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Diarization backend: "simple", "ml", or "remote"
    #[serde(default = "default_diarization_backend")]
    pub backend: String,

    /// Maximum number of speakers to detect
    #[serde(default = "default_max_speakers")]
    pub max_speakers: u32,

    /// Path to ONNX model for ML backend (uses default if not set)
    #[serde(default)]
    pub model_path: Option<String>,

    /// Minimum segment duration in milliseconds for ML embedding extraction
    #[serde(default = "default_min_segment_ms")]
    pub min_segment_ms: u64,

    // The four fields below apply only to backend = "ml"; the "simple" and
    // "remote" backends ignore them.
    /// Cosine similarity threshold for the ML backend (0.20-0.30 typical for ECAPA on 4s windows)
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f32,

    /// VAD sub-window length in seconds for ECAPA feeding
    #[serde(default = "default_vad_window_secs")]
    pub vad_window_secs: f32,

    /// VAD sub-window hop in seconds
    #[serde(default = "default_vad_hop_secs")]
    pub vad_hop_secs: f32,

    /// RMS floor for treating a sub-window as silence
    #[serde(default = "default_vad_rms_floor")]
    pub vad_rms_floor: f32,
}

fn default_diarization_backend() -> String {
    "simple".to_string()
}

fn default_max_speakers() -> u32 {
    10
}

fn default_min_segment_ms() -> u64 {
    500
}

// Empirically tuned against multi-speaker test clips (4-person roundtable,
// 3-person panel, 1h talk). The previous 0.75 anchor was far too strict for
// 4s ECAPA windows and produced overwhelmingly Unknown labels in practice.
fn default_similarity_threshold() -> f32 {
    0.25
}

fn default_vad_window_secs() -> f32 {
    4.0
}

fn default_vad_hop_secs() -> f32 {
    2.0
}

fn default_vad_rms_floor() -> f32 {
    0.005
}

fn default_chunk_duration() -> u32 {
    30
}

fn default_storage_path() -> String {
    "auto".to_string()
}

fn default_max_duration() -> u32 {
    180
}

impl Default for MeetingDiarizationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: default_diarization_backend(),
            max_speakers: default_max_speakers(),
            model_path: None,
            min_segment_ms: default_min_segment_ms(),
            similarity_threshold: default_similarity_threshold(),
            vad_window_secs: default_vad_window_secs(),
            vad_hop_secs: default_vad_hop_secs(),
            vad_rms_floor: default_vad_rms_floor(),
        }
    }
}

/// Meeting summary configuration (Phase 5)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingSummaryConfig {
    /// Summarization backend: "local", "remote", or "disabled"
    #[serde(default = "default_summary_backend")]
    pub backend: String,

    /// Ollama URL for local backend
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,

    /// Ollama model name
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,

    /// Remote API endpoint for remote backend
    #[serde(default)]
    pub remote_endpoint: Option<String>,

    /// Remote API key
    #[serde(default)]
    pub remote_api_key: Option<String>,

    /// Request timeout in seconds
    #[serde(default = "default_summary_timeout")]
    pub timeout_secs: u64,
}

fn default_summary_backend() -> String {
    "disabled".to_string()
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_ollama_model() -> String {
    "llama3.2".to_string()
}

fn default_summary_timeout() -> u64 {
    120
}

impl Default for MeetingSummaryConfig {
    fn default() -> Self {
        Self {
            backend: default_summary_backend(),
            ollama_url: default_ollama_url(),
            ollama_model: default_ollama_model(),
            remote_endpoint: None,
            remote_api_key: None,
            timeout_secs: default_summary_timeout(),
        }
    }
}

impl Default for MeetingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            chunk_duration_secs: default_chunk_duration(),
            storage_path: default_storage_path(),
            retain_audio: false,
            max_duration_mins: default_max_duration(),
            audio: MeetingAudioConfig::default(),
            diarization: MeetingDiarizationConfig::default(),
            summary: MeetingSummaryConfig::default(),
        }
    }
}
