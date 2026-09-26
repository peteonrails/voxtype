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

    /// The threshold for this recording: the configured one, lowered for a
    /// quiet microphone to `NOISE_FLOOR_MULTIPLE` times the recording's own
    /// noise floor. A fixed RMS level depends on input gain, and at low gain
    /// real speech never reached it, so short dictations were discarded.
    /// Only ever lowers the configured threshold, so a loud mic behaves as
    /// before, and never below the most sensitive setting.
    fn effective_threshold(&self, frame_rms: &[f32]) -> f32 {
        let mut sorted = frame_rms.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let floor = sorted[((sorted.len() - 1) as f32 * NOISE_FLOOR_PERCENTILE) as usize];
        (floor * NOISE_FLOOR_MULTIPLE)
            .max(MIN_ENERGY_THRESHOLD)
            .min(self.threshold)
    }
}

/// Speech must be this many times louder than the recording's noise floor
/// (about 10 dB) when that bar is below the configured threshold.
const NOISE_FLOOR_MULTIPLE: f32 = 3.0;
/// Frame-RMS percentile taken as the recording's noise floor.
const NOISE_FLOOR_PERCENTILE: f32 = 0.10;
/// The lowest threshold ever used, equal to the most sensitive setting.
const MIN_ENERGY_THRESHOLD: f32 = 0.001;
/// Consecutive loud frames (60ms) needed before they count as speech. A lower
/// adaptive threshold would otherwise let keyboard clicks and pops, one or two
/// 20ms frames each, add up to the minimum speech duration.
const MIN_RUN_FRAMES: usize = 3;

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

        let frame_rms: Vec<f32> = samples
            .chunks(FRAME_SIZE)
            .map(Self::calculate_rms)
            .collect();
        let total_frames = frame_rms.len();
        let threshold = self.effective_threshold(&frame_rms);

        // Count loud frames only in runs long enough to be speech.
        let mut speech_frames = 0usize;
        let mut run = 0usize;
        for &rms in &frame_rms {
            if rms >= threshold {
                run += 1;
            } else {
                if run >= MIN_RUN_FRAMES {
                    speech_frames += run;
                }
                run = 0;
            }
        }
        if run >= MIN_RUN_FRAMES {
            speech_frames += run;
        }

        let avg_rms = frame_rms.iter().sum::<f32>() / total_frames as f32;

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
             speech_ratio={:.1}%, avg_rms={:.4}, threshold={:.4} (configured {:.4})",
            has_speech,
            speech_duration_secs,
            speech_frames,
            speech_ratio * 100.0,
            avg_rms,
            threshold,
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
        let config = VadConfig {
            min_speech_duration_ms: 500, // 500ms minimum
            ..VadConfig::default()
        };
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

    /// Deterministic uniform noise in [-amplitude, amplitude].
    fn noise(len: usize, amplitude: f32, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * amplitude
            })
            .collect()
    }

    fn tone(len: usize, amplitude: f32) -> Vec<f32> {
        (0..len)
            .map(|i| (i as f32 * 220.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * amplitude)
            .collect()
    }

    /// Mix a tone burst into `audio` at `start` for `len` samples.
    fn add_burst(audio: &mut [f32], start: usize, len: usize, amplitude: f32) {
        for (sample, t) in audio[start..start + len]
            .iter_mut()
            .zip(tone(len, amplitude))
        {
            *sample += t;
        }
    }

    #[test]
    fn quiet_speech_on_a_low_gain_mic_is_detected() {
        // Measured on a webcam mic at 40% input volume: speech averaged RMS
        // 0.003-0.005 against a fixed 0.01 threshold, and short dictations
        // were discarded. Two 300ms bursts at RMS ~0.004 over a ~0.0003
        // floor must count as speech.
        let mut audio = noise(32000, 0.0005, 7);
        add_burst(&mut audio, 6400, 4800, 0.006);
        add_burst(&mut audio, 17600, 4800, 0.006);
        let result = EnergyVad::new(&VadConfig::default())
            .detect(&audio)
            .unwrap();
        assert!(result.has_speech, "{result:?}");
    }

    #[test]
    fn a_noise_only_recording_is_still_rejected() {
        let audio = noise(32000, 0.002, 11);
        let result = EnergyVad::new(&VadConfig::default())
            .detect(&audio)
            .unwrap();
        assert!(!result.has_speech, "{result:?}");
    }

    #[test]
    fn keyboard_clicks_in_silence_are_not_speech() {
        // Ten 20ms clicks is 200ms of loud frames, twice the minimum, but
        // each is a single frame, not a run.
        let mut audio = noise(48000, 0.0005, 3);
        for k in 0..10 {
            add_burst(&mut audio, 3200 + k * 4000, 320, 0.2);
        }
        let result = EnergyVad::new(&VadConfig::default())
            .detect(&audio)
            .unwrap();
        assert!(!result.has_speech, "{result:?}");
    }

    #[test]
    fn a_loud_mic_keeps_the_configured_threshold() {
        // The adaptive bar only ever lowers the threshold.
        let vad = EnergyVad::new(&VadConfig::default());
        let loud_floor = vec![0.02f32; 50];
        assert_eq!(vad.effective_threshold(&loud_floor), vad.threshold);
        let digital_silence = vec![0.0f32; 50];
        assert_eq!(
            vad.effective_threshold(&digital_silence),
            MIN_ENERGY_THRESHOLD
        );
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
