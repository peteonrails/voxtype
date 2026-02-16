//! SenseVoice-based speech-to-text transcription
//!
//! Uses Alibaba's SenseVoice model via ONNX Runtime for local transcription.
//! SenseVoice is an encoder-only CTC model (no autoregressive decoder loop),
//! making inference a single forward pass. The complexity is in preprocessing:
//! audio must be converted to 80-dim Fbank features, stacked via LFR to 560-dim,
//! then CMVN-normalized before feeding to the ONNX model.
//!
//! Pipeline: Audio (f32, 16kHz) -> Fbank (80-dim) -> LFR (560-dim) -> CMVN -> ONNX -> CTC decode
//!
//! Supports languages: auto, zh, en, ja, ko, yue
//! Model files: model.int8.onnx (or model.onnx), tokens.txt

use super::Transcriber;
use crate::config::SenseVoiceConfig;
use crate::error::TranscribeError;
use ndarray::Array2;
use ort::session::Session;
use ort::value::Tensor;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use std::collections::HashMap;
use std::path::PathBuf;

/// Sample rate expected by SenseVoice
const SAMPLE_RATE: usize = 16000;

/// FFT size for Fbank computation
const FFT_SIZE: usize = 512;

/// Number of mel filterbank channels
const NUM_MELS: usize = 80;

/// Frame length in samples (25ms at 16kHz)
const FRAME_LENGTH: usize = 400;

/// Frame shift in samples (10ms at 16kHz)
const FRAME_SHIFT: usize = 160;

/// LFR window size (stack 7 consecutive frames)
const LFR_M: usize = 7;

/// LFR stride (advance by 6 frames)
const LFR_N: usize = 6;

/// Pre-emphasis coefficient
const PREEMPH_COEFF: f32 = 0.97;

/// Blank token ID for CTC decoding
const BLANK_ID: u32 = 0;

/// Number of metadata tokens to skip at the start of CTC output
/// (language, emotion, event, ITN flag)
const NUM_METADATA_TOKENS: usize = 4;

/// SenseVoice-based transcriber using ONNX Runtime
pub struct SenseVoiceTranscriber {
    session: std::sync::Mutex<Session>,
    tokens: HashMap<u32, String>,
    neg_mean: Vec<f32>,
    inv_stddev: Vec<f32>,
    language_id: i32,
    text_norm_id: i32,
    mel_filterbank: Vec<Vec<f32>>,
}

impl SenseVoiceTranscriber {
    pub fn new(config: &SenseVoiceConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;

        tracing::info!("Loading SenseVoice model from {:?}", model_dir);
        let start = std::time::Instant::now();

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));

        // Find model file (prefer int8 quantized)
        let model_file = {
            let int8 = model_dir.join("model.int8.onnx");
            let full = model_dir.join("model.onnx");
            if int8.exists() {
                int8
            } else if full.exists() {
                tracing::info!("Using full-precision model (model.int8.onnx not found)");
                full
            } else {
                return Err(TranscribeError::ModelNotFound(format!(
                    "SenseVoice model not found in {:?}\n  \
                     Expected model.int8.onnx or model.onnx\n  \
                     Download from: https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
                    model_dir
                )));
            }
        };

        // Load tokens.txt
        let tokens_path = model_dir.join("tokens.txt");
        if !tokens_path.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "SenseVoice tokens.txt not found: {}\n  \
                 Ensure tokens.txt is in the model directory.",
                tokens_path.display()
            )));
        }
        let tokens = load_tokens(&tokens_path)?;
        tracing::debug!("Loaded {} tokens", tokens.len());

        // Create ONNX session
        let session = Session::builder()
            .map_err(|e| {
                TranscribeError::InitFailed(format!("ONNX session builder failed: {}", e))
            })?
            .with_intra_threads(threads)
            .map_err(|e| {
                TranscribeError::InitFailed(format!("Failed to set threads: {}", e))
            })?
            .commit_from_file(&model_file)
            .map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load SenseVoice model from {:?}: {}",
                    model_file, e
                ))
            })?;

        // Read CMVN stats from model metadata
        let (neg_mean, inv_stddev) = read_cmvn_from_metadata(&session)?;

        // Map language config to ID
        let language_id = language_to_id(&config.language);
        let text_norm_id = if config.use_itn { 14 } else { 15 };

        // Pre-compute mel filterbank matrix
        let mel_filterbank = compute_mel_filterbank(NUM_MELS, FFT_SIZE, SAMPLE_RATE as f32);

        tracing::info!(
            "SenseVoice model loaded in {:.2}s (language={}, use_itn={})",
            start.elapsed().as_secs_f32(),
            config.language,
            config.use_itn,
        );

        Ok(Self {
            session: std::sync::Mutex::new(session),
            tokens,
            neg_mean,
            inv_stddev,
            language_id,
            text_norm_id,
            mel_filterbank,
        })
    }

    /// Extract 80-dim log-mel filterbank features from audio
    fn extract_fbank(&self, samples: &[f32]) -> Array2<f32> {
        // Scale to int16 range (SenseVoice/kaldi convention)
        let scaled: Vec<f32> = samples.iter().map(|&s| s * 32768.0).collect();

        // Pre-emphasis
        let mut emphasized = Vec::with_capacity(scaled.len());
        emphasized.push(scaled[0]);
        for i in 1..scaled.len() {
            emphasized.push(scaled[i] - PREEMPH_COEFF * scaled[i - 1]);
        }

        // Compute number of frames
        let num_frames = if emphasized.len() >= FRAME_LENGTH {
            (emphasized.len() - FRAME_LENGTH) / FRAME_SHIFT + 1
        } else {
            0
        };

        if num_frames == 0 {
            return Array2::zeros((0, NUM_MELS));
        }

        // Pre-compute Hamming window
        let hamming: Vec<f32> = (0..FRAME_LENGTH)
            .map(|n| 0.54 - 0.46 * (2.0 * std::f32::consts::PI * n as f32 / (FRAME_LENGTH as f32 - 1.0)).cos())
            .collect();

        // Set up FFT
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        let mut fbank = Array2::zeros((num_frames, NUM_MELS));

        for frame_idx in 0..num_frames {
            let start = frame_idx * FRAME_SHIFT;

            // Window the frame
            let mut fft_input: Vec<Complex<f32>> = Vec::with_capacity(FFT_SIZE);
            for i in 0..FRAME_LENGTH {
                fft_input.push(Complex::new(emphasized[start + i] * hamming[i], 0.0));
            }
            // Zero-pad to FFT_SIZE
            fft_input.resize(FFT_SIZE, Complex::new(0.0, 0.0));

            // FFT
            fft.process(&mut fft_input);

            // Power spectrum (only need first FFT_SIZE/2 + 1 bins)
            let num_bins = FFT_SIZE / 2 + 1;
            let power: Vec<f32> = fft_input[..num_bins]
                .iter()
                .map(|c| c.norm_sqr())
                .collect();

            // Apply mel filterbank and take log
            for mel_idx in 0..NUM_MELS {
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

    /// Apply LFR (Low Frame Rate) stacking: concatenate 7 frames with stride 6
    fn apply_lfr(&self, fbank: &Array2<f32>) -> Array2<f32> {
        let num_frames = fbank.nrows();
        if num_frames == 0 {
            return Array2::zeros((0, NUM_MELS * LFR_M));
        }

        // Left-pad with 3 copies of the first frame
        let pad = (LFR_M - 1) / 2; // = 3
        let padded_len = pad + num_frames;
        let output_frames = padded_len.div_ceil(LFR_N);

        let mut output = Array2::zeros((output_frames, NUM_MELS * LFR_M));

        for out_idx in 0..output_frames {
            let center = out_idx * LFR_N; // index into padded sequence
            for j in 0..LFR_M {
                let padded_idx = center + j;
                // Map padded index to actual frame index
                let frame_idx = if padded_idx < pad {
                    0 // replicate first frame
                } else {
                    (padded_idx - pad).min(num_frames - 1)
                };

                let col_start = j * NUM_MELS;
                for k in 0..NUM_MELS {
                    output[[out_idx, col_start + k]] = fbank[[frame_idx, k]];
                }
            }
        }

        output
    }

    /// Apply CMVN normalization: (features + neg_mean) * inv_stddev
    fn apply_cmvn(&self, features: &mut Array2<f32>) {
        let feat_dim = features.ncols();
        for row in features.rows_mut() {
            for (j, val) in row.into_iter().enumerate() {
                if j < feat_dim && j < self.neg_mean.len() {
                    *val = (*val + self.neg_mean[j]) * self.inv_stddev[j];
                }
            }
        }
    }

    /// CTC greedy decoding: argmax, collapse duplicates, remove blanks
    fn ctc_decode(&self, logits: &[f32], time_steps: usize, vocab_size: usize) -> String {
        let mut token_ids: Vec<u32> = Vec::new();
        let mut prev_id: Option<u32> = None;

        for t in 0..time_steps {
            let offset = t * vocab_size;
            let frame_logits = &logits[offset..offset + vocab_size];

            // Argmax
            let best_id = frame_logits
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(idx, _)| idx as u32)
                .unwrap_or(BLANK_ID);

            // Collapse consecutive duplicates and skip blanks
            if best_id != BLANK_ID && Some(best_id) != prev_id {
                token_ids.push(best_id);
            }
            prev_id = Some(best_id);
        }

        // Skip metadata tokens (language, emotion, event, ITN flag)
        let content_tokens = if token_ids.len() > NUM_METADATA_TOKENS {
            &token_ids[NUM_METADATA_TOKENS..]
        } else {
            &[]
        };

        // Map token IDs to strings, replacing SentencePiece word boundary markers
        let mut result = String::new();
        for &id in content_tokens {
            if let Some(token_str) = self.tokens.get(&id) {
                result.push_str(&token_str.replace('\u{2581}', " "));
            }
        }

        result.trim().to_string()
    }
}

impl Transcriber for SenseVoiceTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with SenseVoice",
            duration_secs,
            samples.len(),
        );

        let start = std::time::Instant::now();

        // 1. Extract Fbank features
        let fbank_start = std::time::Instant::now();
        let fbank = self.extract_fbank(samples);
        tracing::debug!(
            "Fbank extraction: {:.2}s ({} frames x {})",
            fbank_start.elapsed().as_secs_f32(),
            fbank.nrows(),
            fbank.ncols(),
        );

        if fbank.nrows() == 0 {
            return Err(TranscribeError::AudioFormat(
                "Audio too short for feature extraction".to_string(),
            ));
        }

        // 2. LFR stacking
        let lfr = self.apply_lfr(&fbank);
        tracing::debug!("LFR output: {} frames x {}", lfr.nrows(), lfr.ncols());

        // 3. CMVN normalization
        let mut features = lfr;
        self.apply_cmvn(&mut features);

        // 4. Build ONNX inputs
        let num_frames = features.nrows();
        let feat_dim = features.ncols();

        // x: shape [1, T, 560]
        let (x_data, _offset) = features.into_raw_vec_and_offset();
        let x_tensor = Tensor::<f32>::from_array(([1usize, num_frames, feat_dim], x_data))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create input tensor: {}",
                    e
                ))
            })?;

        // x_length: shape [1]
        let x_length_tensor = Tensor::<i32>::from_array(([1usize], vec![num_frames as i32]))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create length tensor: {}",
                    e
                ))
            })?;

        // language: shape [1]
        let language_tensor = Tensor::<i32>::from_array(([1usize], vec![self.language_id]))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create language tensor: {}",
                    e
                ))
            })?;

        // text_norm: shape [1]
        let text_norm_tensor = Tensor::<i32>::from_array(([1usize], vec![self.text_norm_id]))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create text_norm tensor: {}",
                    e
                ))
            })?;

        // 5. Run inference
        let inference_start = std::time::Instant::now();
        let mut session = self.session.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock session: {}", e))
        })?;

        let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
            (std::borrow::Cow::Borrowed("x"), x_tensor.into()),
            (std::borrow::Cow::Borrowed("x_length"), x_length_tensor.into()),
            (std::borrow::Cow::Borrowed("language"), language_tensor.into()),
            (std::borrow::Cow::Borrowed("text_norm"), text_norm_tensor.into()),
        ];

        let outputs = session.run(inputs).map_err(|e| {
            TranscribeError::InferenceFailed(format!("SenseVoice inference failed: {}", e))
        })?;

        tracing::debug!(
            "ONNX inference: {:.2}s",
            inference_start.elapsed().as_secs_f32(),
        );

        // 6. Extract logits and decode
        let logits_val = &outputs["logits"];
        let (shape, logits_data) = logits_val.try_extract_tensor::<f32>().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to extract logits: {}", e))
        })?;

        let shape_dims: &[i64] = shape;
        tracing::debug!("Logits shape: {:?}", shape_dims);

        // logits shape: [batch=1, time_steps] or [batch=1, time_steps, vocab_size]
        let (time_steps, vocab_size) = if shape_dims.len() == 3 {
            (shape_dims[1] as usize, shape_dims[2] as usize)
        } else if shape_dims.len() == 2 {
            // Pre-argmaxed output: each value is already a token ID
            let time_steps = shape_dims[1] as usize;
            let result = self.decode_pre_argmax(&logits_data[..time_steps]);
            tracing::info!(
                "SenseVoice transcription completed in {:.2}s: {:?}",
                start.elapsed().as_secs_f32(),
                if result.chars().count() > 50 {
                    format!("{}...", result.chars().take(50).collect::<String>())
                } else {
                    result.clone()
                }
            );
            return Ok(result);
        } else {
            return Err(TranscribeError::InferenceFailed(format!(
                "Unexpected logits shape: {:?}",
                shape_dims
            )));
        };

        let result = self.ctc_decode(logits_data, time_steps, vocab_size);

        tracing::info!(
            "SenseVoice transcription completed in {:.2}s: {:?}",
            start.elapsed().as_secs_f32(),
            if result.chars().count() > 50 {
                format!("{}...", result.chars().take(50).collect::<String>())
            } else {
                result.clone()
            }
        );

        Ok(result)
    }
}

impl SenseVoiceTranscriber {
    /// Decode pre-argmaxed output (2D logits where values are token IDs)
    fn decode_pre_argmax(&self, token_ids_f32: &[f32]) -> String {
        let mut token_ids: Vec<u32> = Vec::new();
        let mut prev_id: Option<u32> = None;

        for &val in token_ids_f32 {
            let id = val as u32;
            if id != BLANK_ID && Some(id) != prev_id {
                token_ids.push(id);
            }
            prev_id = Some(id);
        }

        let content_tokens = if token_ids.len() > NUM_METADATA_TOKENS {
            &token_ids[NUM_METADATA_TOKENS..]
        } else {
            &[]
        };

        let mut result = String::new();
        for &id in content_tokens {
            if let Some(token_str) = self.tokens.get(&id) {
                result.push_str(token_str);
            }
        }

        result.trim().to_string()
    }
}

/// Map language string to SenseVoice language ID
fn language_to_id(language: &str) -> i32 {
    match language.to_lowercase().as_str() {
        "auto" => 0,
        "zh" | "chinese" => 3,
        "en" | "english" => 4,
        "yue" | "cantonese" => 7,
        "ja" | "japanese" => 11,
        "ko" | "korean" => 12,
        _ => {
            tracing::warn!(
                "Unknown SenseVoice language '{}', falling back to auto-detect",
                language
            );
            0
        }
    }
}

/// Load tokens.txt into a HashMap<u32, String>
/// Format: each line is "token_string token_id" (space-separated)
fn load_tokens(path: &std::path::Path) -> Result<HashMap<u32, String>, TranscribeError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        TranscribeError::InitFailed(format!("Failed to read tokens.txt: {}", e))
    })?;

    let mut tokens = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Format: "token_string id" - split from the right to handle tokens containing spaces
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

/// Read CMVN stats (neg_mean and inv_stddev) from ONNX model metadata
/// The sherpa-onnx SenseVoice model stores these as comma-separated floats
/// in metadata keys "neg_mean" and "inv_stddev"
fn read_cmvn_from_metadata(session: &Session) -> Result<(Vec<f32>, Vec<f32>), TranscribeError> {
    let metadata = session.metadata().map_err(|e| {
        TranscribeError::InitFailed(format!("Failed to read model metadata: {}", e))
    })?;

    let neg_mean_str = metadata.custom("neg_mean").ok_or_else(|| {
        TranscribeError::InitFailed(
            "Model metadata missing 'neg_mean' key. Is this a sherpa-onnx SenseVoice model?"
                .to_string(),
        )
    })?;

    let inv_stddev_str = metadata.custom("inv_stddev").ok_or_else(|| {
        TranscribeError::InitFailed(
            "Model metadata missing 'inv_stddev' key. Is this a sherpa-onnx SenseVoice model?"
                .to_string(),
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

/// Compute mel filterbank matrix
/// Returns NUM_MELS filters, each with FFT_SIZE/2+1 coefficients
fn compute_mel_filterbank(num_mels: usize, fft_size: usize, sample_rate: f32) -> Vec<Vec<f32>> {
    let num_bins = fft_size / 2 + 1;
    let max_freq = sample_rate / 2.0;

    // Mel scale conversion functions
    let hz_to_mel = |f: f32| -> f32 { 1127.0 * (1.0 + f / 700.0).ln() };
    let mel_to_hz = |m: f32| -> f32 { 700.0 * ((m / 1127.0).exp() - 1.0) };

    let mel_low = hz_to_mel(0.0);
    let mel_high = hz_to_mel(max_freq);

    // Compute mel center frequencies (num_mels + 2 points for triangular filters)
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

/// Resolve model name to directory path
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    // If it's already an absolute path, use it directly
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    // Map short names to directory names
    let model_dir_name = if model.starts_with("sensevoice-") {
        model.to_string()
    } else {
        format!("sensevoice-{}", model)
    };

    // Check models directory
    let models_dir = crate::config::Config::models_dir();
    let model_path = models_dir.join(&model_dir_name);

    if model_path.exists() {
        return Ok(model_path);
    }

    // Also check without prefix (user might pass "sensevoice-small" or just "small")
    let alt_path = models_dir.join(model);
    if alt_path.exists() {
        return Ok(alt_path);
    }

    // Check current directory
    let cwd_path = PathBuf::from(&model_dir_name);
    if cwd_path.exists() {
        return Ok(cwd_path);
    }

    // Check ./models/
    let local_models_path = PathBuf::from("models").join(&model_dir_name);
    if local_models_path.exists() {
        return Ok(local_models_path);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "SenseVoice model '{}' not found. Looked in:\n  \
         - {}\n  \
         - {}\n  \
         - {}\n  \
         - {}\n\n\
         Manual download:\n  \
         mkdir -p {}\n  \
         cd {} && wget https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx\n  \
         cd {} && wget https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt",
        model,
        model_path.display(),
        alt_path.display(),
        cwd_path.display(),
        local_models_path.display(),
        model_path.display(),
        model_path.display(),
        model_path.display(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_language_to_id() {
        assert_eq!(language_to_id("auto"), 0);
        assert_eq!(language_to_id("zh"), 3);
        assert_eq!(language_to_id("en"), 4);
        assert_eq!(language_to_id("yue"), 7);
        assert_eq!(language_to_id("ja"), 11);
        assert_eq!(language_to_id("ko"), 12);
        assert_eq!(language_to_id("unknown"), 0); // falls back to auto
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
    fn test_mel_filterbank_shape() {
        let fb = compute_mel_filterbank(80, 512, 16000.0);
        assert_eq!(fb.len(), 80);
        assert_eq!(fb[0].len(), 257); // FFT_SIZE/2 + 1
    }

    #[test]
    fn test_mel_filterbank_triangular() {
        let fb = compute_mel_filterbank(80, 512, 16000.0);
        // Each filter should have non-negative values
        for filter in &fb {
            for &val in filter {
                assert!(val >= 0.0, "Filter values should be non-negative");
            }
        }
        // Each filter should have at least one non-zero value
        for (i, filter) in fb.iter().enumerate() {
            let sum: f32 = filter.iter().sum();
            assert!(sum > 0.0, "Filter {} should have non-zero area", i);
        }
    }

    #[test]
    fn test_resolve_model_path_not_found() {
        let result = resolve_model_path("/nonexistent/path/to/model");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TranscribeError::ModelNotFound(_)));
    }

    #[test]
    fn test_resolve_model_path_absolute() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();
        fs::write(model_path.join("model.int8.onnx"), b"dummy").unwrap();

        let resolved = resolve_model_path(model_path.to_str().unwrap());
        assert!(resolved.is_ok());
        assert_eq!(resolved.unwrap(), model_path);
    }

    #[test]
    fn test_ctc_decode_basic() {
        // Create a minimal transcriber-like test for CTC decode
        let mut tokens = HashMap::new();
        tokens.insert(0, "<blank>".to_string());
        tokens.insert(1, "<lang>".to_string());
        tokens.insert(2, "<emo>".to_string());
        tokens.insert(3, "<event>".to_string());
        tokens.insert(4, "<itn>".to_string());
        tokens.insert(5, "h".to_string());
        tokens.insert(6, "i".to_string());

        // Simulate CTC output: blank, lang, emo, event, itn, h, h, blank, i
        // vocab_size = 7
        let vocab_size = 7;
        let time_steps = 9;
        let mut logits = vec![0.0f32; time_steps * vocab_size];

        // Helper to set argmax for a frame
        let set_max = |logits: &mut Vec<f32>, t: usize, id: usize| {
            logits[t * vocab_size + id] = 10.0;
        };

        set_max(&mut logits, 0, 0); // blank
        set_max(&mut logits, 1, 1); // lang (metadata)
        set_max(&mut logits, 2, 2); // emo (metadata)
        set_max(&mut logits, 3, 3); // event (metadata)
        set_max(&mut logits, 4, 4); // itn (metadata)
        set_max(&mut logits, 5, 5); // h
        set_max(&mut logits, 6, 5); // h (duplicate, should be collapsed)
        set_max(&mut logits, 7, 0); // blank
        set_max(&mut logits, 8, 6); // i

        // We can't call ctc_decode without a full transcriber, so test the logic directly
        let mut token_ids: Vec<u32> = Vec::new();
        let mut prev_id: Option<u32> = None;
        for t in 0..time_steps {
            let offset = t * vocab_size;
            let frame_logits = &logits[offset..offset + vocab_size];
            let best_id = frame_logits
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(idx, _)| idx as u32)
                .unwrap_or(0);
            if best_id != 0 && Some(best_id) != prev_id {
                token_ids.push(best_id);
            }
            prev_id = Some(best_id);
        }

        // Skip 4 metadata tokens
        let content = &token_ids[4..];
        let mut result = String::new();
        for &id in content {
            if let Some(s) = tokens.get(&id) {
                result.push_str(s);
            }
        }
        assert_eq!(result, "hi");
    }
}
