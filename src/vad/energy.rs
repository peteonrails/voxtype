//! Energy-based Voice Activity Detection
//!
//! A simple but effective VAD that uses RMS energy to detect speech.
//! Works well for filtering completely silent recordings without
//! requiring external model downloads.

use crate::config::VadConfig;
use crate::error::VadError;

use super::{VadResult, VoiceActivityDetector};

/// Energy-based VAD using RMS amplitude analysis
///
/// This implementation analyzes audio in short frames (20ms) and determines
/// speech presence based on energy levels exceeding a threshold. It's designed
/// to filter out completely silent or near-silent recordings that would cause
/// Whisper to hallucinate.
pub struct EnergyVad {
    /// Energy threshold for speech detection (0.0 - 1.0)
    /// Frames with RMS energy above this are considered speech
    threshold: f32,
    /// Minimum speech duration in milliseconds
    min_speech_duration_ms: u32,
}

impl EnergyVad {
    /// Create a new energy-based VAD instance
    pub fn new(config: &VadConfig) -> Self {
        // Map the config threshold (0.0-1.0) to an energy threshold
        // Default 0.5 maps to ~0.01 RMS, which filters silence but allows quiet speech
        let energy_threshold = map_threshold_to_energy(config.threshold);

        Self {
            threshold: energy_threshold,
            min_speech_duration_ms: config.min_speech_duration_ms,
        }
    }

    /// Calculate RMS energy of a sample slice
    fn calculate_rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_squares: f32 = samples.iter().map(|&s| s * s).sum();
        (sum_squares / samples.len() as f32).sqrt()
    }
}

/// Map config threshold (0.0-1.0) to energy threshold
///
/// - 0.0 = very sensitive (energy threshold ~0.001, detects quiet whispers)
/// - 0.5 = balanced (energy threshold ~0.01, filters silence)
/// - 1.0 = aggressive (energy threshold ~0.1, requires louder speech)
fn map_threshold_to_energy(config_threshold: f32) -> f32 {
    // Exponential mapping: lower config values = lower energy threshold
    // Range: 0.001 to 0.1
    let t = config_threshold.clamp(0.0, 1.0);
    0.001 * (100.0_f32).powf(t)
}

impl VoiceActivityDetector for EnergyVad {
    fn detect(&self, samples: &[f32]) -> Result<VadResult, VadError> {
        if samples.is_empty() {
            return Ok(VadResult {
                has_speech: false,
                speech_duration_secs: 0.0,
                speech_ratio: 0.0,
                rms_energy: 0.0,
            });
        }

        const SAMPLE_RATE: usize = 16000;
        const FRAME_MS: usize = 20;
        const FRAME_SIZE: usize = SAMPLE_RATE * FRAME_MS / 1000; // 320 samples

        let mut speech_frames = 0usize;
        let mut total_frames = 0usize;
        let mut total_energy = 0.0f32;

        // Process audio in frames
        for frame in samples.chunks(FRAME_SIZE) {
            let rms = Self::calculate_rms(frame);
            total_energy += rms;
            total_frames += 1;

            if rms >= self.threshold {
                speech_frames += 1;
            }
        }

        let avg_rms = if total_frames > 0 {
            total_energy / total_frames as f32
        } else {
            0.0
        };

        let speech_duration_secs = (speech_frames * FRAME_MS) as f32 / 1000.0;
        let speech_ratio = if total_frames > 0 {
            speech_frames as f32 / total_frames as f32
        } else {
            0.0
        };

        // Determine if there's enough speech
        let min_speech_secs = self.min_speech_duration_ms as f32 / 1000.0;
        let has_speech = speech_duration_secs >= min_speech_secs;

        tracing::debug!(
            "VAD result: has_speech={}, speech_duration={:.2}s ({} frames), \
             speech_ratio={:.1}%, avg_rms={:.4}, threshold={:.4}",
            has_speech,
            speech_duration_secs,
            speech_frames,
            speech_ratio * 100.0,
            avg_rms,
            self.threshold
        );

        Ok(VadResult {
            has_speech,
            speech_duration_secs,
            speech_ratio,
            rms_energy: avg_rms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_energy_vad_creation() {
        let config = VadConfig::default();
        let vad = EnergyVad::new(&config);
        assert!(vad.threshold > 0.0);
    }

    #[test]
    fn test_detect_silence() {
        let config = VadConfig::default();
        let vad = EnergyVad::new(&config);

        // Create 1 second of silence
        let silence: Vec<f32> = vec![0.0; 16000];
        let result = vad.detect(&silence).unwrap();

        assert!(!result.has_speech);
        assert_eq!(result.speech_duration_secs, 0.0);
        assert_eq!(result.rms_energy, 0.0);
    }

    #[test]
    fn test_detect_loud_audio() {
        let config = VadConfig::default();
        let vad = EnergyVad::new(&config);

        // Create 1 second of "loud" audio (sine wave)
        let samples: Vec<f32> = (0..16000)
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.5)
            .collect();
        let result = vad.detect(&samples).unwrap();

        assert!(result.has_speech);
        assert!(result.speech_ratio > 0.9);
        assert!(result.rms_energy > 0.1);
    }

    #[test]
    fn test_detect_quiet_audio() {
        let config = VadConfig::default();
        let vad = EnergyVad::new(&config);

        // Create 1 second of very quiet audio
        let samples: Vec<f32> = (0..16000)
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.001)
            .collect();
        let result = vad.detect(&samples).unwrap();

        // Very quiet audio should be detected as silence with default threshold
        assert!(!result.has_speech);
    }

    #[test]
    fn test_detect_empty_audio() {
        let config = VadConfig::default();
        let vad = EnergyVad::new(&config);

        let result = vad.detect(&[]).unwrap();
        assert!(!result.has_speech);
        assert_eq!(result.speech_duration_secs, 0.0);
    }

    #[test]
    fn test_threshold_mapping() {
        // Test threshold mapping function
        let low = map_threshold_to_energy(0.0);
        let mid = map_threshold_to_energy(0.5);
        let high = map_threshold_to_energy(1.0);

        assert!(low < mid);
        assert!(mid < high);
        assert!(low >= 0.001);
        assert!(high <= 0.1);
    }

    #[test]
    fn test_min_speech_duration() {
        let mut config = VadConfig::default();
        config.min_speech_duration_ms = 500; // 500ms minimum
        let vad = EnergyVad::new(&config);

        // Create 200ms of loud audio followed by silence
        // This is less than the 500ms minimum
        let mut samples: Vec<f32> = (0..3200) // 200ms
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.5)
            .collect();
        samples.extend(vec![0.0; 12800]); // 800ms silence

        let result = vad.detect(&samples).unwrap();

        // Should not pass because speech duration < min_speech_duration
        assert!(!result.has_speech);
    }

    #[test]
    fn test_calculate_rms() {
        // RMS of constant 1.0 should be 1.0
        let ones = vec![1.0f32; 100];
        assert!((EnergyVad::calculate_rms(&ones) - 1.0).abs() < 0.001);

        // RMS of constant 0.0 should be 0.0
        let zeros = vec![0.0f32; 100];
        assert_eq!(EnergyVad::calculate_rms(&zeros), 0.0);

        // RMS of sine wave with amplitude 1.0 should be ~0.707
        let sine: Vec<f32> = (0..1000)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI / 100.0).sin())
            .collect();
        let rms = EnergyVad::calculate_rms(&sine);
        assert!((rms - 0.707).abs() < 0.01);
    }
}
