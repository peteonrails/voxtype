//! Dolphin-based speech-to-text transcription
//!
//! Uses DataoceanAI's Dolphin model via ONNX Runtime for local transcription.
//! Dolphin is a CTC-based E-Branchformer model optimized for Eastern languages
//! (40 languages + 22 Chinese dialects). No English support.
//!
//! The ONNX model expects 80-dim Fbank features as input, preprocessed with
//! the shared Fbank pipeline (same as SenseVoice/Paraformer) and normalized
//! with CMVN stats from model metadata.
//!
//! Pipeline: Audio (f32, 16kHz) -> Fbank (80-dim) -> CMVN -> ONNX model -> CTC decode
//!
//! Languages: zh, ja, ko, th, vi, id, ms, ar, hi, ur, bn, ta, and 28 more
//! Model files: model.int8.onnx (or model.onnx), tokens.txt

use super::ctc;
use super::fbank::{self, FbankExtractor};
use super::Transcriber;
use crate::config::DolphinConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::PathBuf;

/// Sample rate expected by Dolphin
const SAMPLE_RATE: usize = 16000;

/// Dolphin-based transcriber using ONNX Runtime
pub struct DolphinTranscriber {
    session: std::sync::Mutex<Session>,
    tokens: HashMap<u32, String>,
    neg_mean: Vec<f32>,
    inv_stddev: Vec<f32>,
    fbank_extractor: FbankExtractor,
}

impl DolphinTranscriber {
    pub fn new(config: &DolphinConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;

        tracing::info!("Loading Dolphin model from {:?}", model_dir);
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
                    "Dolphin model not found in {:?}\n  \
                     Expected model.int8.onnx or model.onnx\n  \
                     Run: voxtype setup model",
                    model_dir
                )));
            }
        };

        // Load tokens.txt
        let tokens_path = model_dir.join("tokens.txt");
        if !tokens_path.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "Dolphin tokens.txt not found: {}\n  \
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
                    "Failed to load Dolphin model from {:?}: {}",
                    model_file, e
                ))
            })?;

        // Read CMVN stats from model metadata
        // Dolphin uses "mean"/"invstd" naming (mean is positive, needs negation)
        let (neg_mean, inv_stddev) = read_cmvn_from_metadata(&session)?;

        let fbank_extractor = FbankExtractor::new_default();

        tracing::info!(
            "Dolphin model loaded in {:.2}s",
            start.elapsed().as_secs_f32(),
        );

        Ok(Self {
            session: std::sync::Mutex::new(session),
            tokens,
            neg_mean,
            inv_stddev,
            fbank_extractor,
        })
    }
}

impl Transcriber for DolphinTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with Dolphin",
            duration_secs,
            samples.len(),
        );

        let start = std::time::Instant::now();

        // 1. Extract Fbank features (80-dim, same pipeline as SenseVoice)
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

        // 2. CMVN normalization (no LFR stacking - Dolphin takes 80-dim directly)
        let mut features = fbank_features;
        fbank::apply_cmvn(&mut features, &self.neg_mean, &self.inv_stddev);

        let num_frames = features.nrows();
        let feat_dim = features.ncols();

        // x: shape [1, T, 80]
        let (x_data, _offset) = features.into_raw_vec_and_offset();
        let x_tensor =
            Tensor::<f32>::from_array(([1usize, num_frames, feat_dim], x_data)).map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create input tensor: {}",
                    e
                ))
            })?;

        // x_len: shape [1] (i64)
        let x_len_tensor = Tensor::<i64>::from_array(([1usize], vec![num_frames as i64]))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create length tensor: {}",
                    e
                ))
            })?;

        // Run inference
        let inference_start = std::time::Instant::now();
        let mut session = self.session.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock session: {}", e))
        })?;

        let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
            (std::borrow::Cow::Borrowed("x"), x_tensor.into()),
            (
                std::borrow::Cow::Borrowed("x_len"),
                x_len_tensor.into(),
            ),
        ];

        let outputs = session.run(inputs).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Dolphin inference failed: {}", e))
        })?;

        tracing::debug!(
            "ONNX inference: {:.2}s",
            inference_start.elapsed().as_secs_f32(),
        );

        // Extract CTC log-probs and decode
        let logits_val = outputs
            .get("lob_probs")
            .or_else(|| outputs.get("logits"))
            .or_else(|| outputs.get("output"))
            .ok_or_else(|| {
                TranscribeError::InferenceFailed(
                    "Dolphin output not found (expected 'lob_probs', 'logits', or 'output')"
                        .to_string(),
                )
            })?;

        let (shape, logits_data) = logits_val.try_extract_tensor::<f32>().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to extract logits: {}", e))
        })?;

        let shape_dims: &[i64] = shape;
        tracing::debug!("Dolphin output shape: {:?}", shape_dims);

        // Dolphin CTC output: [batch, time_steps, vocab_size]
        let raw_text = if shape_dims.len() == 3 {
            let time_steps = shape_dims[1] as usize;
            let vocab_size = shape_dims[2] as usize;
            let config = ctc::CtcConfig {
                blank_id: 0,
                num_metadata_tokens: 0,
                sentencepiece_cleanup: true,
            };
            ctc::ctc_greedy_decode(logits_data, time_steps, vocab_size, &self.tokens, &config)
        } else if shape_dims.len() == 2 {
            // Pre-argmaxed output
            let time_steps = shape_dims[1] as usize;
            let config = ctc::CtcConfig {
                blank_id: 0,
                num_metadata_tokens: 0,
                sentencepiece_cleanup: true,
            };
            ctc::decode_pre_argmax(&logits_data[..time_steps], &self.tokens, &config)
        } else {
            return Err(TranscribeError::InferenceFailed(format!(
                "Unexpected Dolphin output shape: {:?}",
                shape_dims
            )));
        };

        // Filter language/region tokens from output (e.g., <zh>, <CN>, <ja>, <JP>)
        let result = filter_language_tokens(&raw_text);

        tracing::info!(
            "Dolphin transcription completed in {:.2}s: {:?}",
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

/// Remove language and region tokens from CTC output
///
/// Dolphin prepends tokens like <zh>, <CN>, <ja>, <JP> to its output.
/// These are useful for language identification but should not appear in
/// the final transcription text.
fn filter_language_tokens(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c == '<' {
            // Consume everything up to and including '>'
            let mut found_close = false;
            for inner in chars.by_ref() {
                if inner == '>' {
                    found_close = true;
                    break;
                }
            }
            if !found_close {
                // Malformed tag, just skip the '<'
                result.push(c);
            }
        } else {
            result.push(c);
            chars.next();
        }
    }

    result.trim().to_string()
}

/// Read CMVN stats from ONNX model metadata
///
/// Dolphin uses "mean"/"invstd" keys where mean is positive (needs negation).
/// Falls back to "neg_mean"/"inv_stddev" if those aren't found.
fn read_cmvn_from_metadata(session: &Session) -> Result<(Vec<f32>, Vec<f32>), TranscribeError> {
    let metadata = session.metadata().map_err(|e| {
        TranscribeError::InitFailed(format!("Failed to read model metadata: {}", e))
    })?;

    // Try Dolphin naming first: "mean" and "invstd"
    // Despite the key name "mean", the values are already negated (same as SenseVoice's
    // "neg_mean"), so we use them directly without negation.
    let (neg_mean, inv_stddev) = if let Some(mean_str) = metadata.custom("mean") {
        let invstd_str = metadata.custom("invstd").ok_or_else(|| {
            TranscribeError::InitFailed("Model metadata has 'mean' but no 'invstd'".to_string())
        })?;

        let neg_mean: Vec<f32> = mean_str
            .split(',')
            .filter_map(|s: &str| s.trim().parse::<f32>().ok())
            .collect();
        let inv_stddev: Vec<f32> = invstd_str
            .split(',')
            .filter_map(|s: &str| s.trim().parse::<f32>().ok())
            .collect();

        (neg_mean, inv_stddev)
    } else if let Some(neg_mean_str) = metadata.custom("neg_mean") {
        // SenseVoice-style naming (already negated)
        let inv_stddev_str = metadata.custom("inv_stddev").ok_or_else(|| {
            TranscribeError::InitFailed(
                "Model metadata has 'neg_mean' but no 'inv_stddev'".to_string(),
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
        (neg_mean, inv_stddev)
    } else {
        return Err(TranscribeError::InitFailed(
            "Dolphin model metadata missing CMVN stats. \
             Expected 'mean'/'invstd' or 'neg_mean'/'inv_stddev' keys."
                .to_string(),
        ));
    };

    if neg_mean.is_empty() || inv_stddev.is_empty() {
        return Err(TranscribeError::InitFailed(format!(
            "CMVN stats malformed (neg_mean: {} values, inv_stddev: {} values)",
            neg_mean.len(),
            inv_stddev.len()
        )));
    }

    tracing::debug!(
        "Loaded CMVN stats: {} dimensions",
        neg_mean.len()
    );

    Ok((neg_mean, inv_stddev))
}

/// Resolve model name to directory path
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    let model_dir_name = if model.starts_with("dolphin-") {
        model.to_string()
    } else {
        format!("dolphin-{}", model)
    };

    let models_dir = crate::config::Config::models_dir();
    let model_path = models_dir.join(&model_dir_name);
    if model_path.exists() {
        return Ok(model_path);
    }

    let alt_path = models_dir.join(model);
    if alt_path.exists() {
        return Ok(alt_path);
    }

    // Check sherpa-onnx naming convention
    let sherpa_name = format!(
        "sherpa-onnx-{}-ctc-multi-lang",
        model_dir_name
    );
    let sherpa_path = models_dir.join(&sherpa_name);
    if sherpa_path.exists() {
        return Ok(sherpa_path);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Dolphin model '{}' not found. Looked in:\n  \
         - {}\n  \
         - {}\n  \
         - {}\n\n\
         Run: voxtype setup model",
        model,
        model_path.display(),
        alt_path.display(),
        sherpa_path.display(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_language_tokens() {
        assert_eq!(filter_language_tokens("<zh><CN>你好世界"), "你好世界");
        assert_eq!(filter_language_tokens("<ja><JP>こんにちは"), "こんにちは");
        assert_eq!(filter_language_tokens("no tags here"), "no tags here");
        assert_eq!(filter_language_tokens("<zh>你好<CN>世界"), "你好世界");
        assert_eq!(filter_language_tokens(""), "");
    }

    #[test]
    fn test_resolve_model_path_not_found() {
        let result = resolve_model_path("/nonexistent/path");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TranscribeError::ModelNotFound(_)
        ));
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
