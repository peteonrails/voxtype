//! GTCRN speech enhancement for echo/noise removal
//!
//! Uses a lightweight ONNX model (48K params, ~523KB) to clean up mic audio
//! before transcription. Removes background noise and speaker bleed-through
//! by processing STFT frames through the neural network.
//!
//! Model: GTCRN (Group Temporal Convolutional Recurrent Network)
//! Input: 16kHz mono audio → STFT frames (512-point, 256-hop, sqrt-Hann)
//! Output: Enhanced audio with noise/echo suppressed

use ort::session::Session;
use ort::value::Tensor;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use std::sync::Mutex;

/// STFT parameters matching the GTCRN model
const N_FFT: usize = 512;
const HOP_LENGTH: usize = 256;
const FREQ_BINS: usize = N_FFT / 2 + 1; // 257

/// GTCRN speech enhancer
pub struct GtcrnEnhancer {
    session: Mutex<Session>,
}

impl GtcrnEnhancer {
    /// Load the GTCRN model from the given ONNX file path
    pub fn load(model_path: &std::path::Path) -> Result<Self, String> {
        let session = Session::builder()
            .map_err(|e| format!("ONNX session builder failed: {}", e))?
            .with_intra_threads(1)
            .map_err(|e| format!("Failed to set threads: {}", e))?
            .commit_from_file(model_path)
            .map_err(|e| format!("Failed to load GTCRN model from {:?}: {}", model_path, e))?;

        tracing::info!("GTCRN speech enhancer loaded from {:?}", model_path);

        Ok(Self {
            session: Mutex::new(session),
        })
    }

    /// Enhance audio by removing noise and echo
    ///
    /// Takes 16kHz mono f32 samples, returns enhanced samples of the same length.
    pub fn enhance(&self, samples: &[f32]) -> Result<Vec<f32>, String> {
        if samples.is_empty() {
            return Ok(Vec::new());
        }

        let start = std::time::Instant::now();

        // 1. Compute STFT
        let stft_frames = stft(samples);
        let num_frames = stft_frames.len();

        if num_frames == 0 {
            return Ok(samples.to_vec());
        }

        // 2. Initialize state tensors (zeros)
        let mut conv_cache = vec![0.0f32; 2 * 16 * 16 * 33];
        let mut tra_cache = vec![0.0f32; 2 * 3 * 16];
        let mut inter_cache = vec![0.0f32; 2 * 33 * 16];

        // 3. Process each STFT frame through the model
        let mut session = self
            .session
            .lock()
            .map_err(|e| format!("Failed to lock GTCRN session: {}", e))?;

        let mut enhanced_frames: Vec<[f32; FREQ_BINS * 2]> = Vec::with_capacity(num_frames);

        for frame in &stft_frames {
            // Input: [1, 257, 1, 2] — real and imaginary parts interleaved
            let mut mix_data = vec![0.0f32; FREQ_BINS * 2];
            for (i, bin) in frame.iter().enumerate() {
                mix_data[i * 2] = bin.re;
                mix_data[i * 2 + 1] = bin.im;
            }

            let mix_tensor =
                Tensor::<f32>::from_array(([1usize, FREQ_BINS, 1, 2], mix_data)).map_err(|e| {
                    format!("Failed to create mix tensor: {}", e)
                })?;

            let conv_tensor = Tensor::<f32>::from_array((
                [2usize, 1, 16, 16, 33],
                conv_cache.clone(),
            ))
            .map_err(|e| format!("Failed to create conv_cache tensor: {}", e))?;

            let tra_tensor =
                Tensor::<f32>::from_array(([2usize, 3, 1, 1, 16], tra_cache.clone()))
                    .map_err(|e| format!("Failed to create tra_cache tensor: {}", e))?;

            let inter_tensor =
                Tensor::<f32>::from_array(([2usize, 1, 33, 16], inter_cache.clone()))
                    .map_err(|e| format!("Failed to create inter_cache tensor: {}", e))?;

            let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
                (std::borrow::Cow::Borrowed("mix"), mix_tensor.into()),
                (
                    std::borrow::Cow::Borrowed("conv_cache"),
                    conv_tensor.into(),
                ),
                (std::borrow::Cow::Borrowed("tra_cache"), tra_tensor.into()),
                (
                    std::borrow::Cow::Borrowed("inter_cache"),
                    inter_tensor.into(),
                ),
            ];

            let outputs = session
                .run(inputs)
                .map_err(|e| format!("GTCRN inference failed: {}", e))?;

            // Extract enhanced frame [1, 257, 1, 2]
            let (_, enh_data) = outputs["enh"]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("Failed to extract enhanced frame: {}", e))?;

            let mut enh_frame = [0.0f32; FREQ_BINS * 2];
            for (i, &v) in enh_data.iter().enumerate().take(FREQ_BINS * 2) {
                enh_frame[i] = v;
            }
            enhanced_frames.push(enh_frame);

            // Update caches
            let (_, conv_data) = outputs["conv_cache_out"]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("Failed to extract conv_cache: {}", e))?;
            conv_cache = conv_data.to_vec();

            let (_, tra_data) = outputs["tra_cache_out"]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("Failed to extract tra_cache: {}", e))?;
            tra_cache = tra_data.to_vec();

            let (_, inter_data) = outputs["inter_cache_out"]
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("Failed to extract inter_cache: {}", e))?;
            inter_cache = inter_data.to_vec();
        }

        // 4. Convert enhanced STFT frames back to complex
        let enhanced_complex: Vec<Vec<Complex<f32>>> = enhanced_frames
            .iter()
            .map(|frame| {
                (0..FREQ_BINS)
                    .map(|i| Complex::new(frame[i * 2], frame[i * 2 + 1]))
                    .collect()
            })
            .collect();

        // 5. Inverse STFT
        let result = istft(&enhanced_complex, samples.len());

        tracing::debug!(
            "GTCRN enhanced {} samples ({} frames) in {:.2}s",
            samples.len(),
            num_frames,
            start.elapsed().as_secs_f32()
        );

        Ok(result)
    }
}

/// Compute STFT with sqrt-Hann window
/// Returns a Vec of frames, each containing FREQ_BINS complex values
fn stft(samples: &[f32]) -> Vec<Vec<Complex<f32>>> {
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(N_FFT);

    // sqrt-Hann window
    let window: Vec<f32> = (0..N_FFT)
        .map(|i| {
            let hann = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / N_FFT as f32).cos());
            hann.sqrt()
        })
        .collect();

    let mut frames = Vec::new();
    let mut pos = 0;

    while pos + N_FFT <= samples.len() {
        // Apply window and create complex buffer
        let mut buffer: Vec<Complex<f32>> = (0..N_FFT)
            .map(|i| Complex::new(samples[pos + i] * window[i], 0.0))
            .collect();

        fft.process(&mut buffer);

        // Keep only positive frequencies (0..N_FFT/2+1)
        frames.push(buffer[..FREQ_BINS].to_vec());
        pos += HOP_LENGTH;
    }

    // Handle last partial frame with zero-padding
    if pos < samples.len() {
        let remaining = samples.len() - pos;
        let mut buffer: Vec<Complex<f32>> = (0..N_FFT)
            .map(|i| {
                if i < remaining {
                    Complex::new(samples[pos + i] * window[i], 0.0)
                } else {
                    Complex::new(0.0, 0.0)
                }
            })
            .collect();

        fft.process(&mut buffer);
        frames.push(buffer[..FREQ_BINS].to_vec());
    }

    frames
}

/// Inverse STFT with sqrt-Hann window and overlap-add
fn istft(frames: &[Vec<Complex<f32>>], target_len: usize) -> Vec<f32> {
    if frames.is_empty() {
        return Vec::new();
    }

    let mut planner = FftPlanner::new();
    let ifft = planner.plan_fft_inverse(N_FFT);

    // sqrt-Hann window (same as forward)
    let window: Vec<f32> = (0..N_FFT)
        .map(|i| {
            let hann = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / N_FFT as f32).cos());
            hann.sqrt()
        })
        .collect();

    // Output buffer with overlap-add
    let output_len = (frames.len() - 1) * HOP_LENGTH + N_FFT;
    let mut output = vec![0.0f32; output_len];
    let mut window_sum = vec![0.0f32; output_len];

    for (frame_idx, frame) in frames.iter().enumerate() {
        // Reconstruct full spectrum (mirror conjugate for negative frequencies)
        let mut buffer = vec![Complex::new(0.0f32, 0.0); N_FFT];
        buffer[..FREQ_BINS].copy_from_slice(frame);
        for (j, buf) in buffer[FREQ_BINS..].iter_mut().enumerate() {
            *buf = frame[FREQ_BINS - 2 - j].conj();
        }

        ifft.process(&mut buffer);

        // Apply window and overlap-add
        // rustfft doesn't normalize, so divide by N_FFT
        let pos = frame_idx * HOP_LENGTH;
        for i in 0..N_FFT {
            if pos + i < output.len() {
                let sample = buffer[i].re / N_FFT as f32;
                output[pos + i] += sample * window[i];
                window_sum[pos + i] += window[i] * window[i];
            }
        }
    }

    // Normalize by window sum (avoid division by zero)
    for (o, &w) in output.iter_mut().zip(window_sum.iter()) {
        if w > 1e-8 {
            *o /= w;
        }
    }

    // Trim to target length
    output.truncate(target_len);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stft_istft_roundtrip() {
        // Generate a simple test signal
        let samples: Vec<f32> = (0..16000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin())
            .collect();

        let frames = stft(&samples);
        assert!(!frames.is_empty());
        assert_eq!(frames[0].len(), FREQ_BINS);

        let reconstructed = istft(&frames, samples.len());
        assert_eq!(reconstructed.len(), samples.len());

        // Check roundtrip accuracy (allow some tolerance from windowing)
        // Skip first and last windows which have edge effects
        let start = N_FFT;
        let end = samples.len().saturating_sub(N_FFT);
        if end > start {
            let max_error: f32 = samples[start..end]
                .iter()
                .zip(reconstructed[start..end].iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(
                max_error < 0.05,
                "STFT/ISTFT roundtrip error too high: {}",
                max_error
            );
        }
    }
}
