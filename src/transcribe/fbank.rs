//! Shared Fbank (log-mel filterbank) feature extraction
//!
//! Used by SenseVoice, Paraformer, and FireRedASR backends. These models share
//! identical preprocessing: 80-dim Fbank features, LFR stacking (m=7, n=6),
//! and CMVN normalization with the same constants (16kHz, 25ms/10ms frames,
//! Hamming window, 0.97 pre-emphasis).
//!
//! Pipeline: Audio (f32, 16kHz) -> Fbank (80-dim) -> LFR (560-dim) -> CMVN

use ndarray::Array2;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Default sample rate for Fbank extraction
const DEFAULT_SAMPLE_RATE: usize = 16000;

/// Default FFT size
const DEFAULT_FFT_SIZE: usize = 512;

/// Default number of mel filterbank channels
const DEFAULT_NUM_MELS: usize = 80;

/// Default frame length in samples (25ms at 16kHz)
const DEFAULT_FRAME_LENGTH: usize = 400;

/// Default frame shift in samples (10ms at 16kHz)
const DEFAULT_FRAME_SHIFT: usize = 160;

/// Default pre-emphasis coefficient
const DEFAULT_PREEMPH_COEFF: f32 = 0.97;

/// Default LFR window size (stack 7 consecutive frames)
const DEFAULT_LFR_M: usize = 7;

/// Default LFR stride (advance by 6 frames)
const DEFAULT_LFR_N: usize = 6;

/// Configuration for Fbank feature extraction
pub struct FbankConfig {
    pub sample_rate: usize,
    pub fft_size: usize,
    pub num_mels: usize,
    pub frame_length: usize,
    pub frame_shift: usize,
    pub preemph_coeff: f32,
}

impl Default for FbankConfig {
    fn default() -> Self {
        Self {
            sample_rate: DEFAULT_SAMPLE_RATE,
            fft_size: DEFAULT_FFT_SIZE,
            num_mels: DEFAULT_NUM_MELS,
            frame_length: DEFAULT_FRAME_LENGTH,
            frame_shift: DEFAULT_FRAME_SHIFT,
            preemph_coeff: DEFAULT_PREEMPH_COEFF,
        }
    }
}

/// Configuration for LFR (Low Frame Rate) stacking
pub struct LfrConfig {
    pub m: usize,
    pub n: usize,
}

impl Default for LfrConfig {
    fn default() -> Self {
        Self {
            m: DEFAULT_LFR_M,
            n: DEFAULT_LFR_N,
        }
    }
}

/// Fbank feature extractor with pre-computed mel filterbank matrix
pub struct FbankExtractor {
    config: FbankConfig,
    mel_filterbank: Vec<Vec<f32>>,
}

impl FbankExtractor {
    /// Create a new FbankExtractor with the given configuration
    pub fn new(config: FbankConfig) -> Self {
        let mel_filterbank =
            compute_mel_filterbank(config.num_mels, config.fft_size, config.sample_rate as f32);
        Self {
            config,
            mel_filterbank,
        }
    }

    /// Create a new FbankExtractor with default SenseVoice/Paraformer settings
    pub fn new_default() -> Self {
        Self::new(FbankConfig::default())
    }

    /// Number of mel channels in the output
    pub fn num_mels(&self) -> usize {
        self.config.num_mels
    }

    /// Extract 80-dim log-mel filterbank features from audio samples
    ///
    /// Input: f32 samples at the configured sample rate (default 16kHz)
    /// Output: Array2<f32> of shape (num_frames, num_mels)
    pub fn extract(&self, samples: &[f32]) -> Array2<f32> {
        let num_mels = self.config.num_mels;
        let frame_length = self.config.frame_length;
        let frame_shift = self.config.frame_shift;
        let fft_size = self.config.fft_size;

        // Scale to int16 range (kaldi convention)
        let scaled: Vec<f32> = samples.iter().map(|&s| s * 32768.0).collect();

        // Pre-emphasis
        let mut emphasized = Vec::with_capacity(scaled.len());
        emphasized.push(scaled[0]);
        for i in 1..scaled.len() {
            emphasized.push(scaled[i] - self.config.preemph_coeff * scaled[i - 1]);
        }

        // Compute number of frames
        let num_frames = if emphasized.len() >= frame_length {
            (emphasized.len() - frame_length) / frame_shift + 1
        } else {
            0
        };

        if num_frames == 0 {
            return Array2::zeros((0, num_mels));
        }

        // Pre-compute Hamming window
        let hamming: Vec<f32> = (0..frame_length)
            .map(|n| {
                0.54 - 0.46
                    * (2.0 * std::f32::consts::PI * n as f32 / (frame_length as f32 - 1.0)).cos()
            })
            .collect();

        // Set up FFT
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        let mut fbank = Array2::zeros((num_frames, num_mels));

        for frame_idx in 0..num_frames {
            let start = frame_idx * frame_shift;

            // Window the frame
            let mut fft_input: Vec<Complex<f32>> = Vec::with_capacity(fft_size);
            for i in 0..frame_length {
                fft_input.push(Complex::new(emphasized[start + i] * hamming[i], 0.0));
            }
            // Zero-pad to fft_size
            fft_input.resize(fft_size, Complex::new(0.0, 0.0));

            // FFT
            fft.process(&mut fft_input);

            // Power spectrum (only need first fft_size/2 + 1 bins)
            let num_bins = fft_size / 2 + 1;
            let power: Vec<f32> = fft_input[..num_bins].iter().map(|c| c.norm_sqr()).collect();

            // Apply mel filterbank and take log
            for mel_idx in 0..num_mels {
                let energy: f32 = self.mel_filterbank[mel_idx]
                    .iter()
                    .zip(power.iter())
                    .map(|(&w, &p)| w * p)
                    .sum();
                fbank[[frame_idx, mel_idx]] = energy.max(1e-10).ln();
            }
        }

        fbank
    }
}

/// Apply LFR (Low Frame Rate) stacking: concatenate m frames with stride n
///
/// Left-pads with copies of the first frame. Output dimension is num_mels * m.
/// Default: m=7 consecutive frames, stride n=6, producing 560-dim features from 80-dim Fbank.
pub fn apply_lfr(fbank: &Array2<f32>, config: &LfrConfig) -> Array2<f32> {
    let num_mels = fbank.ncols();
    let num_frames = fbank.nrows();
    if num_frames == 0 {
        return Array2::zeros((0, num_mels * config.m));
    }

    // Left-pad with copies of the first frame
    let pad = (config.m - 1) / 2;
    let padded_len = pad + num_frames;
    let output_frames = padded_len.div_ceil(config.n);

    let mut output = Array2::zeros((output_frames, num_mels * config.m));

    for out_idx in 0..output_frames {
        let center = out_idx * config.n;
        for j in 0..config.m {
            let padded_idx = center + j;
            let frame_idx = if padded_idx < pad {
                0
            } else {
                (padded_idx - pad).min(num_frames - 1)
            };

            let col_start = j * num_mels;
            for k in 0..num_mels {
                output[[out_idx, col_start + k]] = fbank[[frame_idx, k]];
            }
        }
    }

    output
}

/// Apply CMVN (Cepstral Mean and Variance Normalization)
///
/// Formula: normalized = (features + neg_mean) * inv_stddev
/// Applied element-wise per feature dimension.
pub fn apply_cmvn(features: &mut Array2<f32>, neg_mean: &[f32], inv_stddev: &[f32]) {
    let feat_dim = features.ncols();
    for row in features.rows_mut() {
        for (j, val) in row.into_iter().enumerate() {
            if j < feat_dim && j < neg_mean.len() {
                *val = (*val + neg_mean[j]) * inv_stddev[j];
            }
        }
    }
}

/// Compute mel filterbank matrix
///
/// Returns num_mels triangular filters, each with fft_size/2+1 coefficients.
/// Uses the standard mel scale: mel = 1127 * ln(1 + f/700)
pub fn compute_mel_filterbank(
    num_mels: usize,
    fft_size: usize,
    sample_rate: f32,
) -> Vec<Vec<f32>> {
    let num_bins = fft_size / 2 + 1;
    let max_freq = sample_rate / 2.0;

    let hz_to_mel = |f: f32| -> f32 { 1127.0 * (1.0 + f / 700.0).ln() };
    let mel_to_hz = |m: f32| -> f32 { 700.0 * ((m / 1127.0).exp() - 1.0) };

    let mel_low = hz_to_mel(0.0);
    let mel_high = hz_to_mel(max_freq);

    // Mel center frequencies (num_mels + 2 points for triangular filters)
    let mel_points: Vec<f32> = (0..num_mels + 2)
        .map(|i| mel_low + (mel_high - mel_low) * i as f32 / (num_mels + 1) as f32)
        .collect();

    // Convert back to Hz and then to FFT bin indices
    let bin_points: Vec<f32> = mel_points
        .iter()
        .map(|&m| mel_to_hz(m) * fft_size as f32 / sample_rate)
        .collect();

    // Build triangular filters
    let mut filterbank = Vec::with_capacity(num_mels);
    for i in 0..num_mels {
        let mut filter = vec![0.0f32; num_bins];
        let left = bin_points[i];
        let center = bin_points[i + 1];
        let right = bin_points[i + 2];

        for (j, val) in filter.iter_mut().enumerate() {
            let freq = j as f32;
            if freq >= left && freq < center && center > left {
                *val = (freq - left) / (center - left);
            } else if freq >= center && freq <= right && right > center {
                *val = (right - freq) / (right - center);
            }
        }
        filterbank.push(filter);
    }

    filterbank
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mel_filterbank_shape() {
        let fb = compute_mel_filterbank(80, 512, 16000.0);
        assert_eq!(fb.len(), 80);
        assert_eq!(fb[0].len(), 257); // FFT_SIZE/2 + 1
    }

    #[test]
    fn test_mel_filterbank_triangular() {
        let fb = compute_mel_filterbank(80, 512, 16000.0);
        for filter in &fb {
            for &val in filter {
                assert!(val >= 0.0, "Filter values should be non-negative");
            }
        }
        for (i, filter) in fb.iter().enumerate() {
            let sum: f32 = filter.iter().sum();
            assert!(sum > 0.0, "Filter {} should have non-zero area", i);
        }
    }

    #[test]
    fn test_fbank_extractor_default() {
        let extractor = FbankExtractor::new_default();
        assert_eq!(extractor.num_mels(), 80);
    }

    #[test]
    fn test_fbank_empty_audio() {
        let extractor = FbankExtractor::new_default();
        // Audio shorter than one frame (400 samples at 16kHz = 25ms)
        let short_audio = vec![0.0f32; 100];
        let result = extractor.extract(&short_audio);
        assert_eq!(result.nrows(), 0);
    }

    #[test]
    fn test_fbank_one_second() {
        let extractor = FbankExtractor::new_default();
        // 1 second of silence at 16kHz
        let audio = vec![0.0f32; 16000];
        let result = extractor.extract(&audio);
        // Expected frames: (16000 - 400) / 160 + 1 = 98
        assert_eq!(result.nrows(), 98);
        assert_eq!(result.ncols(), 80);
    }

    #[test]
    fn test_lfr_default() {
        let config = LfrConfig::default();
        assert_eq!(config.m, 7);
        assert_eq!(config.n, 6);
    }

    #[test]
    fn test_lfr_stacking() {
        let fbank = Array2::ones((100, 80));
        let config = LfrConfig::default();
        let result = apply_lfr(&fbank, &config);
        // Output dim should be 80 * 7 = 560
        assert_eq!(result.ncols(), 560);
        // Output frames: ceil((3 + 100) / 6) = ceil(103/6) = 18
        assert_eq!(result.nrows(), 18);
    }

    #[test]
    fn test_lfr_empty() {
        let fbank = Array2::zeros((0, 80));
        let config = LfrConfig::default();
        let result = apply_lfr(&fbank, &config);
        assert_eq!(result.nrows(), 0);
        assert_eq!(result.ncols(), 560);
    }

    #[test]
    fn test_cmvn() {
        let mut features = Array2::from_elem((2, 3), 1.0f32);
        let neg_mean = vec![-1.0, -1.0, -1.0]; // (1.0 + (-1.0)) = 0.0
        let inv_stddev = vec![2.0, 2.0, 2.0]; // 0.0 * 2.0 = 0.0
        apply_cmvn(&mut features, &neg_mean, &inv_stddev);
        for val in features.iter() {
            assert!((val - 0.0).abs() < 1e-6);
        }
    }
}
