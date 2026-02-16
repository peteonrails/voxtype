//! Omnilingual speech-to-text transcription (FunASR 50+ language CTC encoder)
//!
//! Uses FunASR's Omnilingual model via ONNX Runtime. Key differences from
//! SenseVoice/Paraformer:
//! - 20ms frame shift (vs 10ms)
//! - Instance normalization instead of global CMVN
//! - No LFR stacking
//! - Large model (~1.3GB, 50+ languages)
//!
//! Pipeline: Audio (f32, 16kHz) -> Fbank (80-dim, 25ms frame, 20ms shift) -> Instance Norm -> ONNX -> CTC decode -> text

use super::ctc;
use super::fbank::{self, FbankConfig, FbankExtractor};
use super::Transcriber;
use crate::config::OmnilingualConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::PathBuf;

const SAMPLE_RATE: usize = 16000;
const BLANK_ID: u32 = 0;

pub struct OmnilingualTranscriber {
    session: std::sync::Mutex<Session>,
    tokens: HashMap<u32, String>,
    fbank_extractor: FbankExtractor,
}

impl OmnilingualTranscriber {
    pub fn new(config: &OmnilingualConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;

        tracing::info!("Loading Omnilingual model from {:?}", model_dir);
        let start = std::time::Instant::now();

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));

        let model_file = find_model_file(&model_dir)?;

        let tokens_path = model_dir.join("tokens.txt");
        if !tokens_path.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "Omnilingual tokens.txt not found: {}",
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
                    "Failed to load Omnilingual model from {:?}: {}",
                    model_file, e
                ))
            })?;

        // Omnilingual uses 25ms frame, 20ms shift (different from standard 10ms)
        let fbank_extractor = FbankExtractor::new(FbankConfig::omnilingual_default());

        tracing::info!(
            "Omnilingual model loaded in {:.2}s",
            start.elapsed().as_secs_f32(),
        );

        Ok(Self {
            session: std::sync::Mutex::new(session),
            tokens,
            fbank_extractor,
        })
    }
}

impl Transcriber for OmnilingualTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".to_string()));
        }

        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!("Transcribing {:.2}s of audio with Omnilingual", duration_secs);

        let start = std::time::Instant::now();

        // 1. Fbank extraction (25ms frame, 20ms shift)
        let fbank_features = self.fbank_extractor.extract(samples);
        if fbank_features.nrows() == 0 {
            return Err(TranscribeError::AudioFormat(
                "Audio too short for feature extraction".to_string(),
            ));
        }

        // 2. Instance normalization (per-utterance, no global CMVN)
        let mut features = fbank_features;
        fbank::apply_instance_norm(&mut features);

        // 3. Build ONNX inputs
        let num_frames = features.nrows();
        let feat_dim = features.ncols();

        let (x_data, _offset) = features.into_raw_vec_and_offset();
        let x_tensor = Tensor::<f32>::from_array(([1usize, num_frames, feat_dim], x_data))
            .map_err(|e| TranscribeError::InferenceFailed(format!("Failed to create input tensor: {}", e)))?;

        let x_length_tensor = Tensor::<i32>::from_array(([1usize], vec![num_frames as i32]))
            .map_err(|e| TranscribeError::InferenceFailed(format!("Failed to create length tensor: {}", e)))?;

        // 4. Inference
        let inference_start = std::time::Instant::now();
        let mut session = self.session.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock session: {}", e))
        })?;

        let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
            (std::borrow::Cow::Borrowed("speech"), x_tensor.into()),
            (std::borrow::Cow::Borrowed("speech_lengths"), x_length_tensor.into()),
        ];

        let outputs = session.run(inputs).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Omnilingual inference failed: {}", e))
        })?;

        tracing::debug!("ONNX inference: {:.2}s", inference_start.elapsed().as_secs_f32());

        // 5. CTC decode
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
            "Omnilingual transcription completed in {:.2}s: {:?}",
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
            "Omnilingual model not found in {:?}\n  Expected model.int8.onnx or model.onnx",
            model_dir
        )))
    }
}

fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    let model_dir_name = if model.starts_with("omnilingual-") {
        model.to_string()
    } else {
        format!("omnilingual-{}", model)
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
        "Omnilingual model '{}' not found.\n  Run 'voxtype setup model' to download.",
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
}
