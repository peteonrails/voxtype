//! Torchaudio-compatible log-mel feature extraction for GigaAM.
//!
//! GigaAM's ONNX encoder takes 64-dim log-mel features produced by
//! torchaudio's `MelSpectrogram` (n_fft=320, hop=160, win=320, htk mel
//! scale, norm=None, center=False, power=2.0) followed by
//! `ln(clamp(x, 1e-9, 1e9))`. torch.stft does not export to ONNX, so the
//! features are computed here. The extractor is pinned against a
//! Python-generated reference in `test_mel_matches_python_reference`.

use ndarray::Array2;
use rustfft::{num_complex::Complex, FftPlanner};

const SAMPLE_RATE: usize = 16000;
const N_FFT: usize = 320;
const HOP_LENGTH: usize = 160;
const WIN_LENGTH: usize = 320;
const N_MELS: usize = 64;
const F_MIN: f64 = 0.0;
const F_MAX: f64 = SAMPLE_RATE as f64 / 2.0;

/// Allow the encoder graph to override the mel dimension (default 64).
fn expected_feature_dim() -> usize {
    EXPECTED_FEATURE_DIM.load(std::sync::atomic::Ordering::Relaxed)
}

static EXPECTED_FEATURE_DIM: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(N_MELS);

/// Called by the engine after loading the encoder ONNX graph.
pub fn set_expected_feature_dim(dim: usize) {
    EXPECTED_FEATURE_DIM.store(dim.max(1), std::sync::atomic::Ordering::Relaxed);
}

/// HTK mel scale (mel_scale="htk")
fn hz_to_mel(hz: f64) -> f64 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f64) -> f64 {
    700.0 * (10f64.powf(mel / 2595.0) - 1.0)
}

/// Periodic Hann window (torch.hann_window(periodic=True))
fn hann_periodic(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()))
        .collect()
}

/// Triangular mel filterbank, torchaudio conventions: filter points are
/// equally spaced in mel, converted back to Hz, and the triangles are
/// built in the Hz domain (norm=None → no per-filter normalization).
fn mel_filterbank(n_mels: usize, n_freqs: usize) -> Vec<Vec<f64>> {
    let mel_min = hz_to_mel(F_MIN);
    let mel_max = hz_to_mel(F_MAX);
    let n_mels_p2 = n_mels + 2;
    let f_pts: Vec<f64> = (0..n_mels_p2)
        .map(|i| {
            let m = mel_min + (mel_max - mel_min) * i as f64 / (n_mels_p2 - 1) as f64;
            mel_to_hz(m)
        })
        .collect();
    let fdiff: Vec<f64> = f_pts.windows(2).map(|w| w[1] - w[0]).collect();

    // frequency of each fft bin
    let bin_freqs: Vec<f64> = (0..n_freqs)
        .map(|k| k as f64 * SAMPLE_RATE as f64 / N_FFT as f64)
        .collect();

    let mut fb = vec![vec![0.0f64; n_mels]; n_freqs];
    for (k, &freq) in bin_freqs.iter().enumerate() {
        for j in 0..n_mels {
            // rising edge: (freq - f_pts[j]) / fdiff[j]
            let lower = (freq - f_pts[j]) / fdiff[j];
            // falling edge: (f_pts[j+2] - freq) / fdiff[j+1]
            let upper = (f_pts[j + 2] - freq) / fdiff[j + 1];
            let w = lower.min(upper).max(0.0);
            if w > 0.0 {
                fb[k][j] = w;
            }
        }
    }
    fb
}

/// Log-mel extractor matching torchaudio MelSpectrogram + GigaAM SpecScaler.
pub struct GigaAMMelExtractor {
    window: Vec<f64>,
    fb: Vec<Vec<f64>>, // [n_freqs][n_mels]
    n_mels: usize,
}

impl GigaAMMelExtractor {
    pub fn new_default() -> Self {
        let n_mels = expected_feature_dim();
        let n_freqs = N_FFT / 2 + 1;
        Self {
            window: hann_periodic(WIN_LENGTH),
            fb: mel_filterbank(n_mels, n_freqs),
            n_mels,
        }
    }

    /// Extract log-mel features: returns [n_mels, n_frames].
    ///
    /// center=False: n_frames = 1 + (n_samples - win_length) / hop.
    /// SpecScaler: ln(clamp(energy, 1e-9, 1e9)).
    pub fn extract(&self, samples: &[f32]) -> Array2<f32> {
        if samples.len() < WIN_LENGTH {
            return Array2::zeros((self.n_mels, 0));
        }
        let n_frames = 1 + (samples.len() - WIN_LENGTH) / HOP_LENGTH;
        let n_freqs = N_FFT / 2 + 1;

        let mut planner = FftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(N_FFT);

        let mut features = Array2::zeros((self.n_mels, n_frames));

        let mut buf: Vec<Complex<f64>> = vec![Complex::new(0.0, 0.0); N_FFT];
        for f in 0..n_frames {
            let start = f * HOP_LENGTH;
            // windowed frame (win_length == n_fft, no padding needed)
            for i in 0..N_FFT {
                buf[i] = Complex::new(samples[start + i] as f64 * self.window[i], 0.0);
            }
            fft.process(&mut buf);

            // mel energies: dot(power spectrum, filterbank)
            let mut col = features.column_mut(f);
            for m in 0..self.n_mels {
                let mut energy = 0.0f64;
                for (k, b) in buf.iter().enumerate().take(n_freqs) {
                    let w = self.fb[k][m];
                    if w != 0.0 {
                        energy += w * (b.re * b.re + b.im * b.im);
                    }
                }
                // SpecScaler: ln(clamp(x, 1e-9, 1e9))
                col[m] = energy.clamp(1e-9, 1e9).ln() as f32;
            }
        }
        features
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test signal: deterministic chirp shared with the Python generator
    /// (scripts/export_gigaam_onnx.py --emit-mel-reference). Computed in
    /// f64 and cast to f32, matching the generator exactly — deep sidelobes
    /// of a chirp are numerically fragile in f32.
    fn chirp() -> Vec<f32> {
        let n = 32000usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / SAMPLE_RATE as f64;
                ((2.0 * std::f64::consts::PI * (200.0 + 600.0 * t) * t).sin() * 0.5) as f32
            })
            .collect()
    }

    /// Reference features generated by the Python GigaAM preprocessor
    /// (tests/gigaam_ref_feats.txt, 64 rows x 50 columns).
    #[test]
    fn test_mel_matches_python_reference() {
        let ref_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/gigaam_ref_feats.txt");
        let Ok(content) = std::fs::read_to_string(&ref_path) else {
            eprintln!("reference not found at {:?}, skipping", ref_path);
            return;
        };
        let ref_feats: Vec<Vec<f32>> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                l.split_whitespace()
                    .map(|v| v.parse::<f32>().unwrap())
                    .collect()
            })
            .collect();
        assert_eq!(ref_feats.len(), 64, "expected 64 mel bins");

        let extractor = GigaAMMelExtractor::new_default();
        let feats = extractor.extract(&chirp());
        assert_eq!(feats.nrows(), 64);

        let n_check = 50usize.min(feats.ncols());
        let mut max_diff: f32 = 0.0;
        for m in 0..64 {
            for t in 0..n_check {
                let got = feats[[m, t]];
                let want = ref_feats[m][t];
                let d = (got - want).abs();
                if d > max_diff {
                    max_diff = d;
                }
                // Relative log-scale tolerance: torchaudio computes in
                // f32, we accumulate in f64 then round-trip through f32,
                // so small-energy bins drift by a few percent.
                let tol = (0.02 * want.abs()).max(1e-3);
                assert!(
                    d < tol,
                    "mel mismatch at [{}][{}]: got {:.6}, want {:.6} (tol {:.4})",
                    m,
                    t,
                    got,
                    want,
                    tol
                );
            }
        }
        eprintln!("max mel diff vs python reference: {:.2e}", max_diff);
        assert!(max_diff < 0.2);
    }

    #[test]
    fn test_frame_count_center_false() {
        let extractor = GigaAMMelExtractor::new_default();
        // 180640 samples -> 1 + (180640 - 320)/160 = 1128 frames
        let samples = vec![0.01f32; 180640];
        let feats = extractor.extract(&samples);
        assert_eq!(feats.ncols(), 1128);
    }

    #[test]
    fn test_hann_periodic_symmetry() {
        let w = hann_periodic(8);
        assert!(w[0].abs() < 1e-12);
        assert!((w[4] - 1.0).abs() < 1e-12);
        assert!((w[1] - w[7]).abs() < 1e-12);
    }
}
