//! Detection for audio captures that open but never deliver sound.

use std::f32::consts::PI;

use tokio::sync::mpsc;

use crate::config::DeadInputConfig;

const CUTOFF_HZ: f32 = 150.0;
const FRAME_MS: u32 = 100;
const SETTLE_MS: u32 = 100;
const MIN_WINDOW_MS: u32 = 300;

/// The result of observing the start of a capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// More samples are required before a decision can be made.
    Pending,
    /// The capture delivered a frame at or above the configured threshold.
    Live,
    /// The observation window ended without an audible frame.
    Dead,
}

/// A second-order Butterworth high-pass filter, primed to the first sample.
struct HighPass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
    primed: bool,
}

impl HighPass {
    fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;

        Self {
            b0: ((1.0 + cos_w0) / 2.0) / a0,
            b1: (-(1.0 + cos_w0)) / a0,
            b2: ((1.0 + cos_w0) / 2.0) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
            primed: false,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        if !self.primed {
            self.x1 = input;
            self.x2 = input;
            self.primed = true;
            // A high-pass filter sees the initial transition to a DC offset
            // as sound unless its input history starts at that offset.
        }

        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

/// Measures the first frames of a capture for audible input.
pub struct DeadInputDetector {
    filter: HighPass,
    threshold: f32,
    frame_len: usize,
    settle_left: usize,
    window_left: usize,
    frame_energy: f64,
    frame_filled: usize,
    peak_rms: f32,
    verdict: Verdict,
}

impl DeadInputDetector {
    /// Create a detector for a capture running at `sample_rate`.
    pub fn new(sample_rate: u32, config: &DeadInputConfig) -> Self {
        let sample_rate = sample_rate.max(1);
        let samples_per_ms = sample_rate as f32 / 1_000.0;
        let frame_len = ((FRAME_MS as f32 * samples_per_ms) as usize).max(1);
        let window_ms = effective_window_ms(config);

        Self {
            filter: HighPass::new(CUTOFF_HZ, sample_rate as f32),
            threshold: dbfs_to_linear(config.threshold_dbfs),
            frame_len,
            settle_left: (SETTLE_MS as f32 * samples_per_ms) as usize,
            window_left: ((window_ms as f32 * samples_per_ms) as usize).max(frame_len),
            frame_energy: 0.0,
            frame_filled: 0,
            peak_rms: 0.0,
            verdict: Verdict::Pending,
        }
    }

    /// Observe a chunk and return the current verdict.
    pub fn push(&mut self, chunk: &[f32]) -> Verdict {
        if self.verdict != Verdict::Pending {
            return self.verdict;
        }

        for &sample in chunk {
            let filtered = self.filter.process(sample);
            if self.settle_left > 0 {
                self.settle_left -= 1;
                continue;
            }

            self.frame_energy += f64::from(filtered) * f64::from(filtered);
            self.frame_filled += 1;
            self.window_left -= 1;

            if self.frame_filled == self.frame_len {
                let rms = (self.frame_energy / self.frame_filled as f64).sqrt() as f32;
                self.peak_rms = self.peak_rms.max(rms);
                self.frame_energy = 0.0;
                self.frame_filled = 0;
                if rms >= self.threshold {
                    self.verdict = Verdict::Live;
                    return self.verdict;
                }
            }

            if self.window_left == 0 {
                self.verdict = Verdict::Dead;
                return self.verdict;
            }
        }

        self.verdict
    }

    /// The loudest observed frame in dBFS.
    pub fn peak_dbfs(&self) -> f32 {
        linear_to_dbfs(self.peak_rms)
    }
}

/// The information the daemon needs to report a dead input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeadInputReport {
    /// The loudest 100 ms frame observed, in dBFS.
    pub peak_dbfs: f32,
    /// The configured level required for a live input, in dBFS.
    pub threshold_dbfs: f32,
    /// The observation window the detector applied, in milliseconds: the
    /// configured window, raised to the shortest window the guard supports.
    pub window_ms: u32,
}

/// A detector wired to the daemon's one-shot report channel.
pub struct DeadInputGuard {
    detector: DeadInputDetector,
    report_tx: mpsc::Sender<DeadInputReport>,
    threshold_dbfs: f32,
    window_ms: u32,
}

impl DeadInputGuard {
    /// Create a guard and the receiver for its single possible dead-input report.
    pub fn new(
        sample_rate: u32,
        config: &DeadInputConfig,
    ) -> (Self, mpsc::Receiver<DeadInputReport>) {
        let (report_tx, report_rx) = mpsc::channel(1);
        (
            Self {
                detector: DeadInputDetector::new(sample_rate, config),
                report_tx,
                threshold_dbfs: config.threshold_dbfs,
                window_ms: effective_window_ms(config),
            },
            report_rx,
        )
    }

    /// Observe a chunk. Returns `true` once no further observations are needed.
    pub fn observe(&mut self, chunk: &[f32]) -> bool {
        match self.detector.push(chunk) {
            Verdict::Pending => false,
            Verdict::Live => true,
            Verdict::Dead => {
                let report = DeadInputReport {
                    peak_dbfs: self.detector.peak_dbfs(),
                    threshold_dbfs: self.threshold_dbfs,
                    window_ms: self.window_ms,
                };
                if let Err(error) = self.report_tx.try_send(report) {
                    tracing::debug!(%error, "dead-input report receiver dropped");
                }
                true
            }
        }
    }
}

/// Convert a dBFS value to a linear RMS amplitude.
fn dbfs_to_linear(dbfs: f32) -> f32 {
    10.0_f32.powf(dbfs / 20.0)
}

/// Convert a linear RMS amplitude to dBFS.
fn linear_to_dbfs(linear: f32) -> f32 {
    if linear <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * linear.log10()
    }
}

/// The observation window the detector applies: the configured window,
/// raised to the shortest window the guard supports.
fn effective_window_ms(config: &DeadInputConfig) -> u32 {
    config.window_ms.max(MIN_WINDOW_MS)
}

/// Resolve the source name to show in a dead-input alert.
pub async fn source_label(configured_device: &str) -> String {
    if configured_device != "default" {
        return configured_device.to_owned();
    }

    match tokio::process::Command::new("pactl")
        .arg("get-default-source")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let source = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if source.is_empty() {
                "default".to_owned()
            } else {
                source
            }
        }
        _ => "default".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 16_000;

    fn config() -> DeadInputConfig {
        DeadInputConfig::default()
    }

    fn verdict_of(samples: &[f32], config: &DeadInputConfig) -> Verdict {
        let mut detector = DeadInputDetector::new(SAMPLE_RATE, config);
        detector.push(samples)
    }

    fn samples_for(seconds: f32) -> usize {
        (SAMPLE_RATE as f32 * seconds) as usize
    }

    fn noise(length: usize, rms: f32) -> Vec<f32> {
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let unit = (state >> 11) as f32 / (1_u64 << 53) as f32 * 2.0 - 1.0;
                unit * rms * 1.732
            })
            .collect()
    }

    fn tone(length: usize, frequency: f32, amplitude: f32, offset: f32) -> Vec<f32> {
        (0..length)
            .map(|index| {
                offset
                    + amplitude * (2.0 * PI * frequency * index as f32 / SAMPLE_RATE as f32).sin()
            })
            .collect()
    }

    #[test]
    fn digital_silence_is_dead() {
        assert_eq!(
            verdict_of(&vec![0.0; samples_for(2.0)], &config()),
            Verdict::Dead
        );
    }

    #[test]
    fn constant_dc_offset_is_dead() {
        assert_eq!(
            verdict_of(&vec![-0.19; samples_for(2.0)], &config()),
            Verdict::Dead
        );
    }

    #[test]
    fn room_noise_is_live() {
        assert_eq!(
            verdict_of(&noise(samples_for(2.0), 0.001), &config()),
            Verdict::Live
        );
    }

    #[test]
    fn room_noise_on_dc_offset_is_live() {
        let mut samples = noise(samples_for(2.0), 0.001);
        for sample in &mut samples {
            *sample -= 0.19;
        }
        assert_eq!(verdict_of(&samples, &config()), Verdict::Live);
    }

    #[test]
    fn speech_band_tone_is_live() {
        assert_eq!(
            verdict_of(&tone(samples_for(2.0), 440.0, 0.01, -0.19), &config()),
            Verdict::Live
        );
    }

    #[test]
    fn sub_audible_drift_is_dead() {
        assert_eq!(
            verdict_of(&tone(samples_for(2.0), 3.0, 0.01, -0.19), &config()),
            Verdict::Dead
        );
    }

    #[test]
    fn leading_silence_before_speech_is_live() {
        let mut samples = vec![0.0; samples_for(1.1)];
        samples.extend(noise(samples_for(1.5), 0.001));
        let config = DeadInputConfig {
            window_ms: 2_500,
            ..config()
        };
        assert_eq!(verdict_of(&samples, &config), Verdict::Live);
    }

    #[test]
    fn silence_remains_pending_until_the_window_closes() {
        let mut detector = DeadInputDetector::new(SAMPLE_RATE, &config());
        assert_eq!(detector.push(&vec![0.0; 25_599]), Verdict::Pending);
        assert_eq!(detector.push(&[0.0]), Verdict::Dead);
    }

    #[test]
    fn too_short_a_window_is_raised_to_the_minimum() {
        let config = DeadInputConfig {
            window_ms: 0,
            ..config()
        };
        let mut detector = DeadInputDetector::new(SAMPLE_RATE, &config);
        assert_eq!(detector.push(&vec![0.0; 6_399]), Verdict::Pending);
        assert_eq!(detector.push(&[0.0]), Verdict::Dead);
    }

    #[test]
    fn chunked_samples_match_a_single_buffer() {
        let samples = noise(samples_for(2.0), 0.001);
        let mut detector = DeadInputDetector::new(SAMPLE_RATE, &config());
        let mut verdict = Verdict::Pending;
        for chunk in samples.chunks(137) {
            verdict = detector.push(chunk);
            if verdict != Verdict::Pending {
                break;
            }
        }
        assert_eq!(verdict, Verdict::Live);
        assert_eq!(verdict_of(&samples, &config()), Verdict::Live);
    }

    #[test]
    fn a_decided_verdict_never_changes() {
        let mut detector = DeadInputDetector::new(SAMPLE_RATE, &config());
        assert_eq!(detector.push(&vec![0.0; samples_for(2.0)]), Verdict::Dead);
        assert_eq!(detector.push(&noise(samples_for(2.0), 0.1)), Verdict::Dead);
    }

    #[test]
    fn startup_click_is_ignored() {
        let mut samples = noise(samples_for(0.01), 0.05);
        samples.extend(vec![-0.19; samples_for(2.0)]);
        assert_eq!(verdict_of(&samples, &config()), Verdict::Dead);
    }

    #[test]
    fn threshold_is_honoured() {
        let config = DeadInputConfig {
            threshold_dbfs: -40.0,
            ..config()
        };
        assert_eq!(
            verdict_of(&noise(samples_for(2.0), 0.001), &config),
            Verdict::Dead
        );
    }

    #[test]
    fn peak_dbfs_reports_the_loudest_frame() {
        let mut detector = DeadInputDetector::new(SAMPLE_RATE, &config());
        detector.push(&vec![0.0; samples_for(2.0)]);
        assert_eq!(detector.peak_dbfs(), f32::NEG_INFINITY);

        let mut detector = DeadInputDetector::new(SAMPLE_RATE, &config());
        detector.push(&noise(samples_for(2.0), 0.01));
        assert!((detector.peak_dbfs() + 40.0).abs() < 3.0);
    }

    #[test]
    fn dbfs_conversion_round_trips() {
        assert!((linear_to_dbfs(dbfs_to_linear(-90.0)) + 90.0).abs() < 1e-3);
        assert!((dbfs_to_linear(0.0) - 1.0).abs() < 1e-6);
    }
}
