//! Shared log-mel filterbank (Fbank) feature extraction
//!
//! Converts raw audio samples into log-mel filterbank features used by
//! ONNX-based ASR models (SenseVoice, Paraformer, Dolphin, Omnilingual).
//!
//! The pipeline is configurable via `FbankConfig` to support different models:
//! - Window function (Hamming vs Hann)
//! - Frame length and shift
//! - Pre-emphasis coefficient
//! - LFR (Low Frame Rate) stacking parameters
//! - CMVN normalization

use ndarray::Array2;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Window function type for frame windowing
#[derive(Debug, Clone, Copy)]
pub enum WindowFn {
    /// Hamming window: 0.54 - 0.46 * cos(2π n / (N-1))
    /// Used by SenseVoice, Paraformer
    Hamming,
    /// Hann window: 0.5 * (1 - cos(2π n / (N-1)))
    /// Used by Dolphin
    Hann,
}

/// Configuration for Fbank feature extraction
#[derive(Debug, Clone)]
pub struct FbankConfig {
    pub num_mels: usize,
    pub frame_length_ms: f32,
    pub frame_shift_ms: f32,
    pub sample_rate: u32,
    pub window_fn: WindowFn,
    pub pre_emphasis: Option<f32>,
}

impl FbankConfig {
    /// Frame length in samples
    fn frame_length_samples(&self) -> usize {
        (self.frame_length_ms * self.sample_rate as f32 / 1000.0) as usize
    }

    /// Frame shift in samples
    fn frame_shift_samples(&self) -> usize {
        (self.frame_shift_ms * self.sample_rate as f32 / 1000.0) as usize
    }

    /// FFT size (next power of 2 >= frame length)
    fn fft_size(&self) -> usize {
        let frame_len = self.frame_length_samples();
        frame_len.next_power_of_two()
    }

    /// SenseVoice/Paraformer default: Hamming, 25ms frame, 10ms shift, pre-emphasis 0.97
    pub fn sensevoice_default() -> Self {
        Self {
            num_mels: 80,
            frame_length_ms: 25.0,
            frame_shift_ms: 10.0,
            sample_rate: 16000,
            window_fn: WindowFn::Hamming,
            pre_emphasis: Some(0.97),
        }
    }

    /// Dolphin default: Hann, 31.25ms frame, 10ms shift, no pre-emphasis
    pub fn dolphin_default() -> Self {
        Self {
            num_mels: 80,
            frame_length_ms: 31.25,
            frame_shift_ms: 10.0,
            sample_rate: 16000,
            window_fn: WindowFn::Hann,
            pre_emphasis: None,
        }
    }

    /// Omnilingual default: Hamming, 25ms frame, 20ms shift, pre-emphasis 0.97
    pub fn omnilingual_default() -> Self {
        Self {
            num_mels: 80,
            frame_length_ms: 25.0,
            frame_shift_ms: 20.0,
            sample_rate: 16000,
            window_fn: WindowFn::Hamming,
            pre_emphasis: Some(0.97),
        }
    }
}

/// Pre-computed Fbank extractor for efficient repeated use
pub struct FbankExtractor {
    config: FbankConfig,
    window: Vec<f32>,
    mel_filterbank: Vec<Vec<f32>>,
}

impl FbankExtractor {
    /// Create a new Fbank extractor with pre-computed window and filterbank
    pub fn new(config: FbankConfig) -> Self {
        let frame_len = config.frame_length_samples();
        let fft_size = config.fft_size();

        let window = compute_window(config.window_fn, frame_len);
        let mel_filterbank =
            compute_mel_filterbank(config.num_mels, fft_size, config.sample_rate as f32);

        Self {
            config,
            window,
            mel_filterbank,
        }
    }

    /// Extract log-mel filterbank features from audio samples
    ///
    /// Input: f32 samples, mono, at config.sample_rate
    /// Output: Array2<f32> with shape (num_frames, num_mels)
    pub fn extract(&self, samples: &[f32]) -> Array2<f32> {
        let frame_len = self.config.frame_length_samples();
        let frame_shift = self.config.frame_shift_samples();
        let fft_size = self.config.fft_size();
        let num_mels = self.config.num_mels;

        // Scale to int16 range (kaldi convention)
        let scaled: Vec<f32> = samples.iter().map(|&s| s * 32768.0).collect();

        // Optional pre-emphasis
        let signal = if let Some(coeff) = self.config.pre_emphasis {
            let mut emphasized = Vec::with_capacity(scaled.len());
            emphasized.push(scaled[0]);
            for i in 1..scaled.len() {
                emphasized.push(scaled[i] - coeff * scaled[i - 1]);
            }
            emphasized
        } else {
            scaled
        };

        // Compute number of frames
        let num_frames = if signal.len() >= frame_len {
            (signal.len() - frame_len) / frame_shift + 1
        } else {
            0
        };

        if num_frames == 0 {
            return Array2::zeros((0, num_mels));
        }

        // Set up FFT
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        let num_bins = fft_size / 2 + 1;
        let mut fbank = Array2::zeros((num_frames, num_mels));

        for frame_idx in 0..num_frames {
            let start = frame_idx * frame_shift;

            // Window the frame
            let mut fft_input: Vec<Complex<f32>> = Vec::with_capacity(fft_size);
            for i in 0..frame_len {
                fft_input.push(Complex::new(signal[start + i] * self.window[i], 0.0));
            }
            // Zero-pad to FFT size
            fft_input.resize(fft_size, Complex::new(0.0, 0.0));

            // FFT
            fft.process(&mut fft_input);

            // Power spectrum
            let power: Vec<f32> = fft_input[..num_bins]
                .iter()
                .map(|c| c.norm_sqr())
                .collect();

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

/// Apply LFR (Low Frame Rate) stacking: concatenate lfr_m frames with stride lfr_n
///
/// This reduces the frame rate by stacking consecutive frames together.
/// Left-pads with copies of the first frame to maintain alignment.
pub fn apply_lfr(fbank: &Array2<f32>, lfr_m: usize, lfr_n: usize) -> Array2<f32> {
    let num_frames = fbank.nrows();
    let num_mels = fbank.ncols();

    if num_frames == 0 {
        return Array2::zeros((0, num_mels * lfr_m));
    }

    let pad = (lfr_m - 1) / 2;
    let padded_len = pad + num_frames;
    let output_frames = padded_len.div_ceil(lfr_n);

    let mut output = Array2::zeros((output_frames, num_mels * lfr_m));

    for out_idx in 0..output_frames {
        let center = out_idx * lfr_n;
        for j in 0..lfr_m {
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
pub fn apply_cmvn(features: &mut Array2<f32>, neg_mean: &[f32], inv_stddev: &[f32]) {
    for row in features.rows_mut() {
        for (j, val) in row.into_iter().enumerate() {
            if j < neg_mean.len() {
                *val = (*val + neg_mean[j]) * inv_stddev[j];
            }
        }
    }
}

/// Apply instance normalization (per-utterance mean/variance normalization)
///
/// Used by Omnilingual instead of global CMVN stats.
pub fn apply_instance_norm(features: &mut Array2<f32>) {
    let num_frames = features.nrows();
    if num_frames == 0 {
        return;
    }
    let feat_dim = features.ncols();

    for j in 0..feat_dim {
        let mut sum = 0.0f32;
        let mut sum_sq = 0.0f32;
        for i in 0..num_frames {
            let v = features[[i, j]];
            sum += v;
            sum_sq += v * v;
        }
        let mean = sum / num_frames as f32;
        let var = (sum_sq / num_frames as f32 - mean * mean).max(1e-10);
        let inv_std = 1.0 / var.sqrt();

        for i in 0..num_frames {
            features[[i, j]] = (features[[i, j]] - mean) * inv_std;
        }
    }
}

/// Compute window function coefficients
fn compute_window(window_fn: WindowFn, length: usize) -> Vec<f32> {
    (0..length)
        .map(|n| {
            let phase = 2.0 * std::f32::consts::PI * n as f32 / (length as f32 - 1.0);
            match window_fn {
                WindowFn::Hamming => 0.54 - 0.46 * phase.cos(),
                WindowFn::Hann => 0.5 * (1.0 - phase.cos()),
            }
        })
        .collect()
}

/// Compute mel filterbank matrix
///
/// Returns num_mels filters, each with fft_size/2+1 coefficients
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

    let mel_points: Vec<f32> = (0..num_mels + 2)
        .map(|i| mel_low + (mel_high - mel_low) * i as f32 / (num_mels + 1) as f32)
        .collect();

    let bin_points: Vec<f32> = mel_points
        .iter()
        .map(|&m| mel_to_hz(m) * fft_size as f32 / sample_rate)
        .collect();

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

/// Read CMVN stats (neg_mean and inv_stddev) from ONNX model metadata
///
/// Many sherpa-onnx models store CMVN as comma-separated floats in metadata.
pub fn read_cmvn_from_metadata(
    session: &ort::session::Session,
) -> Result<(Vec<f32>, Vec<f32>), crate::error::TranscribeError> {
    use crate::error::TranscribeError;

    let metadata = session.metadata().map_err(|e| {
        TranscribeError::InitFailed(format!("Failed to read model metadata: {}", e))
    })?;

    let neg_mean_str = metadata.custom("neg_mean").ok_or_else(|| {
        TranscribeError::InitFailed(
            "Model metadata missing 'neg_mean' key. Is this a sherpa-onnx model?".to_string(),
        )
    })?;

    let inv_stddev_str = metadata.custom("inv_stddev").ok_or_else(|| {
        TranscribeError::InitFailed(
            "Model metadata missing 'inv_stddev' key. Is this a sherpa-onnx model?".to_string(),
        )
    })?;

    let neg_mean: Vec<f32> = neg_mean_str
        .split(',')
        .filter_map(|s: &str| s.trim().parse::<f32>().ok())
        .collect();

    let inv_stddev: Vec<f32> = inv_stddev_str
        .split(',')
        .filter_map(|s: &str| s.trim().parse::<f32>().ok())
        .collect();

    if neg_mean.is_empty() || inv_stddev.is_empty() {
        return Err(TranscribeError::InitFailed(format!(
            "CMVN stats appear malformed (neg_mean: {} values, inv_stddev: {} values)",
            neg_mean.len(),
            inv_stddev.len()
        )));
    }

    tracing::debug!(
        "CMVN stats loaded: neg_mean[{}], inv_stddev[{}]",
        neg_mean.len(),
        inv_stddev.len()
    );

    Ok((neg_mean, inv_stddev))
}

/// Load tokens.txt into a HashMap<u32, String>
///
/// Format: each line is "token_string token_id" (space-separated).
/// Common across SenseVoice, Paraformer, Dolphin, Omnilingual.
pub fn load_tokens(
    path: &std::path::Path,
) -> Result<std::collections::HashMap<u32, String>, crate::error::TranscribeError> {
    use crate::error::TranscribeError;

    let content = std::fs::read_to_string(path).map_err(|e| {
        TranscribeError::InitFailed(format!("Failed to read tokens.txt: {}", e))
    })?;

    let mut tokens = std::collections::HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(last_space) = line.rfind(' ') {
            let token_str = &line[..last_space];
            let id_str = &line[last_space + 1..];
            if let Ok(id) = id_str.parse::<u32>() {
                tokens.insert(id, token_str.to_string());
            }
        }
    }

    if tokens.is_empty() {
        return Err(TranscribeError::InitFailed(
            "tokens.txt appears empty or malformed".to_string(),
        ));
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

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
    fn test_hamming_window() {
        let w = compute_window(WindowFn::Hamming, 400);
        assert_eq!(w.len(), 400);
        // Hamming window starts and ends at 0.08
        assert!((w[0] - 0.08).abs() < 0.01);
        assert!((w[399] - 0.08).abs() < 0.01);
        // Peak in the middle
        assert!(w[200] > 0.9);
    }

    #[test]
    fn test_hann_window() {
        let w = compute_window(WindowFn::Hann, 500);
        assert_eq!(w.len(), 500);
        // Hann window starts and ends at 0
        assert!(w[0].abs() < 0.001);
        assert!(w[499].abs() < 0.001);
        // Peak in the middle
        assert!((w[250] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_fbank_extract_basic() {
        let config = FbankConfig::sensevoice_default();
        let extractor = FbankExtractor::new(config);

        // 1 second of silence
        let samples = vec![0.0f32; 16000];
        let fbank = extractor.extract(&samples);

        // At 25ms frame, 10ms shift: (16000 - 400) / 160 + 1 = 98 frames
        assert_eq!(fbank.nrows(), 98);
        assert_eq!(fbank.ncols(), 80);
    }

    #[test]
    fn test_fbank_extract_short_audio() {
        let config = FbankConfig::sensevoice_default();
        let extractor = FbankExtractor::new(config);

        // Too short for even 1 frame
        let samples = vec![0.0f32; 100];
        let fbank = extractor.extract(&samples);
        assert_eq!(fbank.nrows(), 0);
    }

    #[test]
    fn test_apply_lfr() {
        let fbank = Array2::from_shape_fn((20, 80), |(i, j)| (i * 80 + j) as f32);
        let lfr = apply_lfr(&fbank, 7, 6);

        // Output should have stacked features: 80 * 7 = 560 dims
        assert_eq!(lfr.ncols(), 560);
        // With padding 3 + 20 frames, stride 6: ceil(23/6) = 4 output frames
        assert_eq!(lfr.nrows(), 4);
    }

    #[test]
    fn test_apply_lfr_empty() {
        let fbank = Array2::zeros((0, 80));
        let lfr = apply_lfr(&fbank, 7, 6);
        assert_eq!(lfr.nrows(), 0);
    }

    #[test]
    fn test_apply_cmvn() {
        let mut features = Array2::from_shape_fn((2, 3), |(_i, _j)| 1.0f32);
        let neg_mean = vec![-1.0, -1.0, -1.0]; // features + neg_mean = 0
        let inv_stddev = vec![2.0, 2.0, 2.0];

        apply_cmvn(&mut features, &neg_mean, &inv_stddev);
        // (1.0 + (-1.0)) * 2.0 = 0.0
        for row in features.rows() {
            for &val in row {
                assert!((val - 0.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn test_apply_instance_norm() {
        // Create features where column 0 has values [2, 4, 6] => mean=4, std~=1.63
        let mut features = Array2::from_shape_fn((3, 2), |(i, _j)| (i as f32 + 1.0) * 2.0);
        apply_instance_norm(&mut features);

        // After normalization, each column should have mean ~0 and std ~1
        for j in 0..2 {
            let mut sum = 0.0f32;
            for i in 0..3 {
                sum += features[[i, j]];
            }
            assert!((sum / 3.0).abs() < 1e-5, "Mean should be ~0");
        }
    }

    #[test]
    fn test_load_tokens() {
        let temp_dir = TempDir::new().unwrap();
        let tokens_path = temp_dir.path().join("tokens.txt");
        fs::write(
            &tokens_path,
            "<blank> 0\n<sos/eos> 1\nhello 2\nworld 3\n",
        )
        .unwrap();

        let tokens = load_tokens(&tokens_path).unwrap();
        assert_eq!(tokens.get(&0), Some(&"<blank>".to_string()));
        assert_eq!(tokens.get(&2), Some(&"hello".to_string()));
        assert_eq!(tokens.get(&3), Some(&"world".to_string()));
    }

    #[test]
    fn test_load_tokens_empty() {
        let temp_dir = TempDir::new().unwrap();
        let tokens_path = temp_dir.path().join("tokens.txt");
        fs::write(&tokens_path, "").unwrap();

        let result = load_tokens(&tokens_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_dolphin_fbank_config() {
        let config = FbankConfig::dolphin_default();
        assert_eq!(config.frame_length_samples(), 500); // 31.25ms * 16 = 500
        assert_eq!(config.frame_shift_samples(), 160); // 10ms * 16 = 160
        assert_eq!(config.fft_size(), 512); // next power of 2 >= 500
        assert!(config.pre_emphasis.is_none());
    }

    #[test]
    fn test_omnilingual_fbank_config() {
        let config = FbankConfig::omnilingual_default();
        assert_eq!(config.frame_length_samples(), 400); // 25ms * 16 = 400
        assert_eq!(config.frame_shift_samples(), 320); // 20ms * 16 = 320
        assert_eq!(config.fft_size(), 512);
    }
}
