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

/// Create a diarizer based on configuration.
///
/// When the configured backend cannot be used and the simple backend stands
/// in for it, the second value says why and how to fix it, so the daemon can
/// tell the user instead of quietly producing coarser speaker labels.
pub fn create_diarizer(config: &DiarizationConfig) -> (Box<dyn Diarizer>, Option<String>) {
    let fallback = |notice: String| -> (Box<dyn Diarizer>, Option<String>) {
        tracing::warn!("{}", notice);
        (Box::new(simple::SimpleDiarizer::new()), Some(notice))
    };

    match config.backend.as_str() {
        "simple" => (Box::new(simple::SimpleDiarizer::new()), None),
        "ml" => {
            #[cfg(feature = "ml-diarization")]
            {
                // Auto-download model if missing
                if !ml::MlDiarizer::default_model_path().exists() {
                    tracing::info!("Speaker embedding model not found, attempting download...");
                    crate::setup::model::ensure_ecapa_model();
                }

                let mut diarizer = ml::MlDiarizer::new(config);
                if !diarizer.model_exists() {
                    let fix = match config.model_path.as_deref() {
                        Some(path) => format!(
                            "[meeting.diarization] model_path points to {}, which does not exist.",
                            path
                        ),
                        None => format!(
                            "Download it with: {}",
                            crate::setup::model::ecapa_download_command()
                        ),
                    };
                    return fallback(format!(
                        "Speaker labels fall back to mic/remote only: the ML diarization \
                         model is missing. {}",
                        fix
                    ));
                }
                if let Err(e) = diarizer.load_model() {
                    return fallback(format!(
                        "Speaker labels fall back to mic/remote only: the ML diarization \
                         model failed to load ({}). Delete it and download it again with: {}",
                        e,
                        crate::setup::model::ecapa_download_command()
                    ));
                }
                tracing::info!("Using ML diarization with ONNX");
                (Box::new(diarizer), None)
            }
            #[cfg(not(feature = "ml-diarization"))]
            {
                fallback(
                    "Speaker labels fall back to mic/remote only: this voxtype binary was \
                     built without ML diarization. Install an ONNX build (voxtype-onnx-*) or \
                     set [meeting.diarization] backend = \"simple\"."
                        .to_string(),
                )
            }
        }
        "subprocess" => {
            // Subprocess diarizer for memory-isolated ML diarization
            (
                Box::new(subprocess::SubprocessDiarizer::new(config.clone())),
                None,
            )
        }
        other => fallback(format!(
            "Speaker labels fall back to mic/remote only: unknown diarization backend \
             '{}'. Set [meeting.diarization] backend to \"simple\" or \"ml\".",
            other
        )),
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

    fn config_with_backend(backend: &str) -> DiarizationConfig {
        DiarizationConfig {
            backend: backend.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn simple_backend_has_no_notice() {
        let (diarizer, notice) = create_diarizer(&config_with_backend("simple"));
        assert_eq!(diarizer.name(), "simple");
        assert!(notice.is_none());
    }

    #[test]
    fn unknown_backend_falls_back_with_notice() {
        let (diarizer, notice) = create_diarizer(&config_with_backend("bogus"));
        assert_eq!(diarizer.name(), "simple");
        let notice = notice.expect("fallback must be explained");
        assert!(notice.contains("'bogus'"));
        assert!(notice.contains("backend"));
    }

    #[cfg(not(feature = "ml-diarization"))]
    #[test]
    fn ml_backend_without_feature_falls_back_with_notice() {
        let (diarizer, notice) = create_diarizer(&config_with_backend("ml"));
        assert_eq!(diarizer.name(), "simple");
        assert!(notice.unwrap().contains("voxtype-onnx"));
    }

    #[test]
    fn test_default_config() {
        // All fields asserted to catch silent default drift — adding a new
        // field without adding an assertion here would land a typo unnoticed.
        let config = DiarizationConfig::default();
        assert!(config.enabled);
        assert_eq!(config.backend, "simple");
        assert_eq!(config.max_speakers, 10);
        assert_eq!(config.min_segment_ms, 500);
        assert_eq!(config.model_path, None);
        assert!((config.similarity_threshold - 0.25).abs() < f32::EPSILON);
        assert!((config.vad_window_secs - 4.0).abs() < f32::EPSILON);
        assert!((config.vad_hop_secs - 2.0).abs() < f32::EPSILON);
        assert!((config.vad_rms_floor - 0.005).abs() < f32::EPSILON);
    }

    #[test]
    fn test_vad_subwindows_empty_when_too_short() {
        // 0.5s of audio at 16kHz = 8000 samples; window is 4s = 64000. No fit.
        let samples = vec![0.5f32; 8000];
        let out = vad_subwindows(&samples, 16000, 4.0, 2.0, 0.001);
        assert!(out.is_empty(), "short segment should produce zero windows");
    }

    #[test]
    fn test_vad_subwindows_zero_window_returns_empty() {
        let samples = vec![0.5f32; 64000];
        // Zero window length → 0 samples per window → early-return empty.
        let out = vad_subwindows(&samples, 16000, 0.0, 2.0, 0.001);
        assert!(out.is_empty());
    }

    #[test]
    fn test_vad_subwindows_rms_gates_silence() {
        // 8s of true silence. Even at the lowest practical floor, every
        // window's RMS is 0.0 < floor, so the vector is empty.
        let samples = vec![0.0f32; 16000 * 8];
        let out = vad_subwindows(&samples, 16000, 4.0, 2.0, 0.001);
        assert!(out.is_empty(), "silent audio should be fully gated out");
    }

    #[test]
    fn test_vad_subwindows_admits_voiced() {
        // 8s of constant 0.5 amplitude (RMS = 0.5 ≫ floor). Window 4s, hop 2s:
        // starts at 0, 2, 4 (window 4..8 still fits). 4 windows total.
        let samples = vec![0.5f32; 16000 * 8];
        let out = vad_subwindows(&samples, 16000, 4.0, 2.0, 0.001);
        assert_eq!(out.len(), 3, "8s audio, 4s/2s window/hop → 3 windows");
        for (s, e, rms) in &out {
            assert_eq!(e - s, 16000 * 4, "window length = 4s of samples");
            assert!((rms - 0.5).abs() < 0.01, "constant 0.5 → rms ≈ 0.5");
        }
    }

    #[test]
    fn test_vad_subwindows_clamps_tiny_hop() {
        // Hop of 0.01s would normally produce 8s/0.01s ≈ 800 starts. The clamp
        // floors hop at 0.1s → 8s/0.1s = 80 starts but with 4s window that
        // fits, only floor((8-4)/0.1)+1 = 41 windows survive.
        let samples = vec![0.5f32; 16000 * 8];
        let out = vad_subwindows(&samples, 16000, 4.0, 0.01, 0.001);
        assert!(
            (40..=41).contains(&out.len()),
            "tiny hop should clamp to 100ms, got {} windows",
            out.len()
        );
    }
}
