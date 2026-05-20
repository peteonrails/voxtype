//! Speaker diarization for meeting transcription
//!
//! Provides speaker identification and attribution for meeting transcripts.
//!
//! # Backends
//!
//! - **Simple**: Source-based attribution using mic vs loopback (Phase 2)
//! - **ML**: ONNX-based speaker embeddings with clustering (Phase 3)
//! - **Subprocess**: Memory-isolated ML diarization for resource-constrained systems

pub mod ml;
pub mod simple;
pub mod subprocess;

use crate::meeting::data::AudioSource;
use std::collections::HashMap;

/// Speaker identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SpeakerId {
    /// The local user (from microphone)
    You,
    /// Remote participant(s) (from loopback)
    Remote,
    /// Unknown speaker
    Unknown,
    /// Identified speaker with label
    Named(String),
    /// Auto-generated speaker ID (e.g., SPEAKER_00)
    Auto(u32),
}

impl SpeakerId {
    /// Get display name for this speaker
    pub fn display_name(&self) -> String {
        match self {
            SpeakerId::You => "You".to_string(),
            SpeakerId::Remote => "Remote".to_string(),
            SpeakerId::Unknown => "Unknown".to_string(),
            SpeakerId::Named(name) => name.clone(),
            SpeakerId::Auto(id) => format!("SPEAKER_{:02}", id),
        }
    }
}

impl std::fmt::Display for SpeakerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// A segment with speaker attribution
#[derive(Debug, Clone)]
pub struct DiarizedSegment {
    /// Speaker who said this
    pub speaker: SpeakerId,
    /// Start time in milliseconds
    pub start_ms: u64,
    /// End time in milliseconds
    pub end_ms: u64,
    /// Transcribed text
    pub text: String,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
}

/// Speaker labels mapping auto IDs to names
pub type SpeakerLabels = HashMap<SpeakerId, String>;

/// Split audio into overlapping voiced sub-windows by RMS gating.
///
/// Returns `(start_sample, end_sample, rms)` tuples for windows whose
/// RMS energy meets or exceeds `rms_floor`. ECAPA-TDNN performs best on
/// 2-5s segments, so callers should use `window_secs ≈ 4.0`, `hop_secs ≈ 2.0`.
pub fn vad_subwindows(
    samples: &[f32],
    sample_rate: u32,
    window_secs: f32,
    hop_secs: f32,
    rms_floor: f32,
) -> Vec<(usize, usize, f32)> {
    // Clamp at the seconds level: a hop of 0 or a very small fraction would
    // otherwise produce hundreds of thousands of overlapping windows per
    // segment and overwhelm ECAPA inference. 100 ms is the lowest hop that
    // still makes sense for speaker fingerprinting.
    let hop_secs = hop_secs.max(0.1);
    let win = (window_secs * sample_rate as f32) as usize;
    let hop = (hop_secs * sample_rate as f32) as usize;
    if win == 0 || hop == 0 || samples.len() < win {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    while start + win <= samples.len() {
        let segment = &samples[start..start + win];
        let sum_sq: f32 = segment.iter().map(|s| s * s).sum();
        let rms = (sum_sq / segment.len() as f32).sqrt();
        if rms >= rms_floor {
            out.push((start, start + win, rms));
        }
        start += hop;
    }
    out
}

/// Trait for diarization backends
pub trait Diarizer: Send + Sync {
    /// Process audio samples and return diarized segments
    fn diarize(
        &self,
        samples: &[f32],
        source: AudioSource,
        transcript_segments: &[crate::meeting::TranscriptSegment],
    ) -> Vec<DiarizedSegment>;

    /// Get the backend name
    fn name(&self) -> &'static str;
}

/// Diarization configuration
#[derive(Debug, Clone)]
pub struct DiarizationConfig {
    /// Enable diarization
    pub enabled: bool,
    /// Backend to use: "simple", "ml", or "remote"
    pub backend: String,
    /// Maximum number of speakers to detect
    pub max_speakers: u32,
    /// Minimum segment duration in milliseconds
    pub min_segment_ms: u64,
    /// Path to ONNX model for ML backend
    pub model_path: Option<String>,
    /// Cosine similarity threshold for matching new embeddings to existing
    /// speakers. Lower = more merging (fewer speakers detected); higher =
    /// more fragmentation. Empirically 0.20-0.30 is the useful range for
    /// ECAPA-TDNN on 4s windows.
    pub similarity_threshold: f32,
    /// VAD sub-window length in seconds for ECAPA feeding
    pub vad_window_secs: f32,
    /// VAD sub-window hop in seconds
    pub vad_hop_secs: f32,
    /// RMS floor below which a sub-window is treated as silence
    pub vad_rms_floor: f32,
}

impl Default for DiarizationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: "simple".to_string(),
            max_speakers: 10,
            min_segment_ms: 500,
            model_path: None,
            similarity_threshold: 0.25,
            vad_window_secs: 4.0,
            vad_hop_secs: 2.0,
            vad_rms_floor: 0.005,
        }
    }
}

/// Create a diarizer based on configuration
pub fn create_diarizer(config: &DiarizationConfig) -> Box<dyn Diarizer> {
    match config.backend.as_str() {
        "simple" => Box::new(simple::SimpleDiarizer::new()),
        "ml" => {
            #[cfg(feature = "ml-diarization")]
            {
                // Auto-download model if missing
                if !ml::MlDiarizer::default_model_path().exists() {
                    tracing::info!("Speaker embedding model not found, attempting download...");
                    crate::setup::model::ensure_ecapa_model();
                }

                let mut diarizer = ml::MlDiarizer::new(config);
                if diarizer.model_exists() {
                    if let Err(e) = diarizer.load_model() {
                        tracing::warn!("Failed to load ML diarization model: {}", e);
                        tracing::info!("Falling back to simple diarization");
                        return Box::new(simple::SimpleDiarizer::new());
                    }
                    tracing::info!("Using ML diarization with ONNX");
                    return Box::new(diarizer);
                } else {
                    tracing::warn!("ML diarization model not found, falling back to simple");
                    return Box::new(simple::SimpleDiarizer::new());
                }
            }
            #[cfg(not(feature = "ml-diarization"))]
            {
                tracing::warn!(
                    "ML diarization requires the 'ml-diarization' feature, falling back to simple"
                );
                Box::new(simple::SimpleDiarizer::new())
            }
        }
        "subprocess" => {
            // Subprocess diarizer for memory-isolated ML diarization
            Box::new(subprocess::SubprocessDiarizer::new(config.clone()))
        }
        _ => {
            tracing::warn!(
                "Unknown diarizer backend '{}', using simple",
                config.backend
            );
            Box::new(simple::SimpleDiarizer::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_speaker_id_display() {
        assert_eq!(SpeakerId::You.display_name(), "You");
        assert_eq!(SpeakerId::Remote.display_name(), "Remote");
        assert_eq!(SpeakerId::Auto(0).display_name(), "SPEAKER_00");
        assert_eq!(SpeakerId::Auto(5).display_name(), "SPEAKER_05");
        assert_eq!(
            SpeakerId::Named("Alice".to_string()).display_name(),
            "Alice"
        );
    }

    #[test]
    fn test_default_config() {
        let config = DiarizationConfig::default();
        assert!(config.enabled);
        assert_eq!(config.backend, "simple");
        assert_eq!(config.max_speakers, 10);
    }
}
