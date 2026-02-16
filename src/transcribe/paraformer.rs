//! Paraformer speech-to-text transcription (FunASR ONNX-based CTC encoder)
//!
//! Uses FunASR's Paraformer model via ONNX Runtime. Paraformer is a CTC encoder
//! with the same Fbank preprocessing as SenseVoice (Hamming window, 25ms frame,
//! 10ms shift, LFR stacking from model metadata, CMVN normalization).
//!
//! Pipeline: Audio (f32, 16kHz) -> Fbank (80-dim) -> LFR -> CMVN -> ONNX -> CTC decode -> text
//!
//! Available models:
//! - paraformer-zh (~220MB) - Chinese + English (offline)
//! - paraformer-en - English (offline)
//! - paraformer-small - Smaller variant

use super::ctc;
use super::fbank::{self, FbankConfig, FbankExtractor};
use super::Transcriber;
use crate::config::ParaformerConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::PathBuf;

const SAMPLE_RATE: usize = 16000;
const BLANK_ID: u32 = 0;

/// Default LFR parameters (may be overridden by model metadata)
const DEFAULT_LFR_M: usize = 7;
const DEFAULT_LFR_N: usize = 6;

pub struct ParaformerTranscriber {
    session: std::sync::Mutex<Session>,
    tokens: HashMap<u32, String>,
    neg_mean: Vec<f32>,
    inv_stddev: Vec<f32>,
    lfr_m: usize,
    lfr_n: usize,
    fbank_extractor: FbankExtractor,
}

impl ParaformerTranscriber {
    pub fn new(config: &ParaformerConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;

        tracing::info!("Loading Paraformer model from {:?}", model_dir);
        let start = std::time::Instant::now();

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));

        let model_file = find_model_file(&model_dir)?;

        let tokens_path = model_dir.join("tokens.txt");
        if !tokens_path.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "Paraformer tokens.txt not found: {}",
                tokens_path.display()
            )));
        }
        let tokens = fbank::load_tokens(&tokens_path)?;

        let session = Session::builder()
            .map_err(|e| TranscribeError::InitFailed(format!("ONNX session builder failed: {}", e)))?
            .with_intra_threads(threads)
            .map_err(|e| TranscribeError::InitFailed(format!("Failed to set threads: {}", e)))?
            .commit_from_file(&model_file)
            .map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load Paraformer model from {:?}: {}",
                    model_file, e
                ))
            })?;

        let (neg_mean, inv_stddev) = fbank::read_cmvn_from_metadata(&session)?;

        // Read LFR parameters from model metadata if available.
        // Must extract values before moving session into Mutex.
        let (lfr_m, lfr_n) = {
            let metadata = session.metadata().ok();
            let m = metadata
                .as_ref()
                .and_then(|m| m.custom("lfr_m"))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(DEFAULT_LFR_M);
            let n = metadata
                .as_ref()
                .and_then(|m| m.custom("lfr_n"))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(DEFAULT_LFR_N);
            (m, n)
        };

        let fbank_extractor = FbankExtractor::new(FbankConfig::sensevoice_default());

        tracing::info!(
            "Paraformer model loaded in {:.2}s (lfr_m={}, lfr_n={})",
            start.elapsed().as_secs_f32(),
            lfr_m,
            lfr_n,
        );

        Ok(Self {
            session: std::sync::Mutex::new(session),
            tokens,
            neg_mean,
            inv_stddev,
            lfr_m,
            lfr_n,
            fbank_extractor,
        })
    }
}

impl Transcriber for ParaformerTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".to_string()));
        }

        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!("Transcribing {:.2}s of audio with Paraformer", duration_secs);

        let start = std::time::Instant::now();

        // 1. Fbank extraction
        let fbank_features = self.fbank_extractor.extract(samples);
        if fbank_features.nrows() == 0 {
            return Err(TranscribeError::AudioFormat(
                "Audio too short for feature extraction".to_string(),
            ));
        }

        // 2. LFR stacking
        let lfr = fbank::apply_lfr(&fbank_features, self.lfr_m, self.lfr_n);

        // 3. CMVN normalization
        let mut features = lfr;
        fbank::apply_cmvn(&mut features, &self.neg_mean, &self.inv_stddev);

        // 4. Build ONNX inputs
        let num_frames = features.nrows();
        let feat_dim = features.ncols();

        let (x_data, _offset) = features.into_raw_vec_and_offset();
        let x_tensor = Tensor::<f32>::from_array(([1usize, num_frames, feat_dim], x_data))
            .map_err(|e| TranscribeError::InferenceFailed(format!("Failed to create input tensor: {}", e)))?;

        let x_length_tensor = Tensor::<i32>::from_array(([1usize], vec![num_frames as i32]))
            .map_err(|e| TranscribeError::InferenceFailed(format!("Failed to create length tensor: {}", e)))?;

        // 5. Inference
        let inference_start = std::time::Instant::now();
        let mut session = self.session.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock session: {}", e))
        })?;

        let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
            (std::borrow::Cow::Borrowed("speech"), x_tensor.into()),
            (std::borrow::Cow::Borrowed("speech_lengths"), x_length_tensor.into()),
        ];

        let outputs = session.run(inputs).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Paraformer inference failed: {}", e))
        })?;

        tracing::debug!("ONNX inference: {:.2}s", inference_start.elapsed().as_secs_f32());

        // 6. CTC decode
        let logits_val = &outputs[0];
        let (shape, logits_data) = logits_val.try_extract_tensor::<f32>().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to extract logits: {}", e))
        })?;

        let shape_dims: &[i64] = shape;
        let (time_steps, vocab_size) = if shape_dims.len() == 3 {
            (shape_dims[1] as usize, shape_dims[2] as usize)
        } else {
            return Err(TranscribeError::InferenceFailed(format!(
                "Unexpected logits shape: {:?}",
                shape_dims
            )));
        };

        let result = ctc::ctc_greedy_decode(logits_data, time_steps, vocab_size, BLANK_ID, &self.tokens);

        tracing::info!(
            "Paraformer transcription completed in {:.2}s: {:?}",
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

fn find_model_file(model_dir: &std::path::Path) -> Result<PathBuf, TranscribeError> {
    let int8 = model_dir.join("model.int8.onnx");
    let full = model_dir.join("model.onnx");
    if int8.exists() {
        Ok(int8)
    } else if full.exists() {
        Ok(full)
    } else {
        Err(TranscribeError::ModelNotFound(format!(
            "Paraformer model not found in {:?}\n  Expected model.int8.onnx or model.onnx",
            model_dir
        )))
    }
}

fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    let model_dir_name = if model.starts_with("paraformer-") {
        model.to_string()
    } else {
        format!("paraformer-{}", model)
    };

    let models_dir = crate::config::Config::models_dir();

    for candidate in [
        models_dir.join(&model_dir_name),
        models_dir.join(model),
        PathBuf::from(&model_dir_name),
        PathBuf::from("models").join(&model_dir_name),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Paraformer model '{}' not found.\n  Run 'voxtype setup model' to download.",
        model
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_model_path_not_found() {
        let result = resolve_model_path("/nonexistent/model");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_model_path_absolute() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();
        std::fs::write(model_path.join("model.onnx"), b"dummy").unwrap();
        let resolved = resolve_model_path(model_path.to_str().unwrap());
        assert!(resolved.is_ok());
    }
}
