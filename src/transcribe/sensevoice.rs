//! SenseVoice-based speech-to-text transcription
//!
//! Uses Alibaba's SenseVoice model via ONNX Runtime for local transcription.
//! SenseVoice is an encoder-only CTC model (no autoregressive decoder loop),
//! making inference a single forward pass. Preprocessing uses the shared Fbank
//! pipeline (fbank.rs) and CTC decoding uses the shared decoder (ctc.rs).
//!
//! Pipeline: Audio (f32, 16kHz) -> Fbank (80-dim) -> LFR (560-dim) -> CMVN -> ONNX -> CTC decode
//!
//! Supports languages: auto, zh, en, ja, ko, yue
//! Model files: model.int8.onnx (or model.onnx), tokens.txt

use super::fbank::{self, FbankExtractor, LfrConfig};
use super::ctc::{self, CtcConfig};
use super::Transcriber;
use crate::config::SenseVoiceConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::PathBuf;

/// Sample rate expected by SenseVoice
const SAMPLE_RATE: usize = 16000;

/// SenseVoice-based transcriber using ONNX Runtime
pub struct SenseVoiceTranscriber {
    session: std::sync::Mutex<Session>,
    tokens: HashMap<u32, String>,
    neg_mean: Vec<f32>,
    inv_stddev: Vec<f32>,
    language_id: i32,
    text_norm_id: i32,
    fbank_extractor: FbankExtractor,
    ctc_config: CtcConfig,
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
        let tokens = ctc::load_tokens(&tokens_path)?;
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

        // Create shared Fbank extractor with default settings
        let fbank_extractor = FbankExtractor::new_default();

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
            fbank_extractor,
            ctc_config: CtcConfig::sensevoice(),
        })
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

        // 1. Extract Fbank features (shared pipeline)
        let fbank_start = std::time::Instant::now();
        let fbank_features = self.fbank_extractor.extract(samples);
        tracing::debug!(
            "Fbank extraction: {:.2}s ({} frames x {})",
            fbank_start.elapsed().as_secs_f32(),
            fbank_features.nrows(),
            fbank_features.ncols(),
        );

        if fbank_features.nrows() == 0 {
            return Err(TranscribeError::AudioFormat(
                "Audio too short for feature extraction".to_string(),
            ));
        }

        // 2. LFR stacking (shared)
        let lfr = fbank::apply_lfr(&fbank_features, &LfrConfig::default());
        tracing::debug!("LFR output: {} frames x {}", lfr.nrows(), lfr.ncols());

        // 3. CMVN normalization (shared)
        let mut features = lfr;
        fbank::apply_cmvn(&mut features, &self.neg_mean, &self.inv_stddev);

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
        let result = if shape_dims.len() == 3 {
            let time_steps = shape_dims[1] as usize;
            let vocab_size = shape_dims[2] as usize;
            ctc::ctc_greedy_decode(
                logits_data,
                time_steps,
                vocab_size,
                &self.tokens,
                &self.ctc_config,
            )
        } else if shape_dims.len() == 2 {
            // Pre-argmaxed output: each value is already a token ID
            let time_steps = shape_dims[1] as usize;
            ctc::decode_pre_argmax(
                &logits_data[..time_steps],
                &self.tokens,
                &self.ctc_config,
            )
        } else {
            return Err(TranscribeError::InferenceFailed(format!(
                "Unexpected logits shape: {:?}",
                shape_dims
            )));
        };

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

/// Read CMVN stats (neg_mean and inv_stddev) from ONNX model metadata
///
/// The sherpa-onnx SenseVoice model stores these as comma-separated floats
/// in metadata keys "neg_mean" and "inv_stddev". This is SenseVoice-specific;
/// Paraformer reads CMVN from a separate am.mvn file.
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
    fn test_resolve_model_path_not_found() {
        let result = resolve_model_path("/nonexistent/path/to/model");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TranscribeError::ModelNotFound(_)));
    }

    #[test]
    fn test_resolve_model_path_absolute() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();
        std::fs::write(model_path.join("model.int8.onnx"), b"dummy").unwrap();

        let resolved = resolve_model_path(model_path.to_str().unwrap());
        assert!(resolved.is_ok());
        assert_eq!(resolved.unwrap(), model_path);
    }
}
