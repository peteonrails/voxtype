//! Cohere Transcribe feature extractor.
//!
//! Matches the `CohereAsrFeatureExtractor` shipped in HuggingFace's
//! `processor_config.json` for `CohereLabs/cohere-transcribe-03-2026`:
//!
//! - 16 kHz input
//! - Pre-emphasis 0.97 applied to the time-domain signal up front
//! - Hann window, length 400 (25 ms), hop 160 (10 ms)
//! - n_fft = 512, 128 mel bins (Slaney scale, fmin=0, fmax=8000)
//! - Power spectrum (`|X|^2`)
//! - Log mel
//! - Per-utterance, per-feature mean/std normalization
//!
//! Notes vs the SenseVoice/Paraformer fbank:
//! - No `* 32768.0` Kaldi scale (NeMo works in [-1, 1]).
//! - Hann not Hamming.
//! - Normalization is per-feature mean/std computed across this clip's
//!   frames, not pre-baked CMVN constants.
//!
//! The output layout is `[frames, 128]`, ready to be unsqueezed to a
//! `[1, frames, 128]` batch tensor for the encoder.

use ndarray::Array2;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

const SAMPLE_RATE: usize = 16000;
const FFT_SIZE: usize = 512;
const NUM_MELS: usize = 128;
const FRAME_LENGTH: usize = 400;
const FRAME_SHIFT: usize = 160;
const PREEMPH: f32 = 0.97;
/// Floor for `log` so silent frames don't blow up to -inf.
const LOG_FLOOR: f32 = 1.0e-10;
/// Stability epsilon added to per-feature stddev before division.
const NORM_EPS: f32 = 1.0e-5;

/// Pre-computed mel filterbank shared across calls. 128 banks × 257 FFT bins.
pub struct CohereFbank {
    mel_filterbank: Vec<Vec<f32>>,
    hann: Vec<f32>,
}

impl Default for CohereFbank {
    fn default() -> Self {
        Self::new()
    }
}

impl CohereFbank {
    pub fn new() -> Self {
        let mel_filterbank = compute_mel_filterbank(NUM_MELS, FFT_SIZE, SAMPLE_RATE as f32);
        // Periodic Hann (HF/NeMo convention, matches torch.hann_window default
        // periodic=True and what librosa/STFT uses).
        let hann: Vec<f32> = (0..FRAME_LENGTH)
            .map(|n| {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * n as f32 / FRAME_LENGTH as f32).cos()
            })
            .collect();
        Self {
            mel_filterbank,
            hann,
        }
    }

    pub fn num_mels(&self) -> usize {
        NUM_MELS
    }

    /// Extract log-mel features then apply per-feature CMVN.
    /// Output: `[num_frames, 128]`. Caller adds the batch dim.
    pub fn extract(&self, samples: &[f32]) -> Array2<f32> {
        // Pre-emphasis on the whole signal before framing.
        let mut emphasized = Vec::with_capacity(samples.len());
        if !samples.is_empty() {
            emphasized.push(samples[0]);
            for i in 1..samples.len() {
                emphasized.push(samples[i] - PREEMPH * samples[i - 1]);
            }
        }

        let num_frames = if emphasized.len() >= FRAME_LENGTH {
            (emphasized.len() - FRAME_LENGTH) / FRAME_SHIFT + 1
        } else {
            0
        };

        if num_frames == 0 {
            return Array2::zeros((0, NUM_MELS));
        }

        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let num_bins = FFT_SIZE / 2 + 1;

        let mut features = Array2::<f32>::zeros((num_frames, NUM_MELS));
        let mut fft_buf: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); FFT_SIZE];

        for frame_idx in 0..num_frames {
            let start = frame_idx * FRAME_SHIFT;

            for i in 0..FFT_SIZE {
                fft_buf[i] = if i < FRAME_LENGTH {
                    Complex::new(emphasized[start + i] * self.hann[i], 0.0)
                } else {
                    Complex::new(0.0, 0.0)
                };
            }
            fft.process(&mut fft_buf);

            // Power spectrum (first num_bins bins).
            for mel_idx in 0..NUM_MELS {
                let mut energy = 0.0_f32;
                let bank = &self.mel_filterbank[mel_idx];
                for (b, &w) in bank.iter().enumerate().take(num_bins) {
                    if w == 0.0 {
                        continue;
                    }
                    energy += w * fft_buf[b].norm_sqr();
                }
                features[[frame_idx, mel_idx]] = energy.max(LOG_FLOOR).ln();
            }
        }

        per_feature_normalize(&mut features);
        features
    }
}

/// In-place per-utterance, per-feature mean/std normalization.
///
/// For each mel bin, compute mean and std across all frames, then
/// `(x - mean) / (std + eps)`. Matches NeMo's `normalize="per_feature"`.
fn per_feature_normalize(features: &mut Array2<f32>) {
    let num_frames = features.nrows();
    let num_mels = features.ncols();
    if num_frames == 0 {
        return;
    }
    let n = num_frames as f32;
    for mel in 0..num_mels {
        let mut sum = 0.0_f32;
        for frame in 0..num_frames {
            sum += features[[frame, mel]];
        }
        let mean = sum / n;
        let mut var_sum = 0.0_f32;
        for frame in 0..num_frames {
            let d = features[[frame, mel]] - mean;
            var_sum += d * d;
        }
        let std = (var_sum / n).sqrt();
        let inv = 1.0 / (std + NORM_EPS);
        for frame in 0..num_frames {
            features[[frame, mel]] = (features[[frame, mel]] - mean) * inv;
        }
    }
}

/// Slaney-scale mel filterbank, fmin=0, fmax=sample_rate/2.
/// Returns `num_mels` banks each of length `fft_size/2 + 1`.
fn compute_mel_filterbank(num_mels: usize, fft_size: usize, sample_rate: f32) -> Vec<Vec<f32>> {
    let num_bins = fft_size / 2 + 1;
    let fmin = 0.0_f32;
    let fmax = sample_rate / 2.0;

    let hz_to_mel = |hz: f32| 2595.0 * (1.0 + hz / 700.0).log10();
    let mel_to_hz = |mel: f32| 700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0);

    let mel_min = hz_to_mel(fmin);
    let mel_max = hz_to_mel(fmax);

    // num_mels + 2 mel-spaced points (left edge, peaks, right edge).
    let mel_points: Vec<f32> = (0..=num_mels + 1)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (num_mels + 1) as f32)
        .collect();
    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();

    let bin_freq: Vec<f32> = (0..num_bins)
        .map(|b| sample_rate * b as f32 / fft_size as f32)
        .collect();

    let mut banks = vec![vec![0.0_f32; num_bins]; num_mels];
    for m in 0..num_mels {
        let f_left = hz_points[m];
        let f_center = hz_points[m + 1];
        let f_right = hz_points[m + 2];
        for (b, &f) in bin_freq.iter().enumerate() {
            let weight = if f < f_left || f > f_right {
                0.0
            } else if f <= f_center {
                (f - f_left) / (f_center - f_left)
            } else {
                (f_right - f) / (f_right - f_center)
            };
            banks[m][b] = weight;
        }
    }
    banks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fbank_shape_one_second() {
        let extractor = CohereFbank::new();
        let samples = vec![0.0_f32; SAMPLE_RATE]; // 1 second of silence
        let f = extractor.extract(&samples);
        assert_eq!(f.ncols(), NUM_MELS);
        // 1s @ 16kHz, frame 25ms, hop 10ms -> ~98 frames
        assert!(
            f.nrows() >= 95 && f.nrows() <= 100,
            "got {} frames",
            f.nrows()
        );
    }

    #[test]
    fn fbank_normalization_zero_mean_unit_std() {
        // Deterministic noise should give per-feature mean ~0, std ~1 after CMVN.
        let extractor = CohereFbank::new();
        let mut samples = vec![0.0_f32; SAMPLE_RATE];
        let mut rng = 0x12345_u32;
        for s in samples.iter_mut() {
            rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
            *s = (rng as f32 / u32::MAX as f32 - 0.5) * 0.1;
        }
        let f = extractor.extract(&samples);
        let n = f.nrows() as f32;
        for mel in 0..NUM_MELS {
            let mean: f32 = (0..f.nrows()).map(|r| f[[r, mel]]).sum::<f32>() / n;
            let var: f32 = (0..f.nrows())
                .map(|r| (f[[r, mel]] - mean).powi(2))
                .sum::<f32>()
                / n;
            let std = var.sqrt();
            assert!(mean.abs() < 1e-3, "mel {mel} mean {mean}");
            assert!(
                (std - 1.0).abs() < 1e-2 || std < 1e-2,
                "mel {mel} std {std}"
            );
        }
    }
}
