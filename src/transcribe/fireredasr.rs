//! FireRedASR speech-to-text transcription (encoder-decoder via ONNX Runtime)
//!
//! Uses sherpa-onnx pre-exported FireRedASR models with an autoregressive
//! encoder-decoder architecture similar to Moonshine. The encoder processes
//! Fbank features, and the decoder generates tokens autoregressively.
//!
//! Model files: encoder.int8.onnx (~1.29GB), decoder.int8.onnx (~445MB), tokens.txt
//!
//! Pipeline: Audio -> Fbank (80-dim) -> Encoder ONNX -> Decoder ONNX (autoregressive) -> text

use super::fbank::{self, FbankConfig, FbankExtractor};
use super::Transcriber;
use crate::config::FireRedAsrConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

const SAMPLE_RATE: usize = 16000;

/// Special token IDs for decoder (sherpa-onnx convention)
const DECODER_START_TOKEN_ID: i64 = 1;
const EOS_TOKEN_ID: i64 = 2;
const BLANK_ID: i64 = 0;

pub struct FireRedAsrTranscriber {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    tokens: HashMap<u32, String>,
    neg_mean: Vec<f32>,
    inv_stddev: Vec<f32>,
    fbank_extractor: FbankExtractor,
    max_tokens: usize,
}

impl FireRedAsrTranscriber {
    pub fn new(config: &FireRedAsrConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;

        tracing::info!("Loading FireRedASR model from {:?}", model_dir);
        let start = std::time::Instant::now();

        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));

        // Find encoder and decoder model files
        let encoder_file = find_encoder_file(&model_dir)?;
        let decoder_file = find_decoder_file(&model_dir)?;

        let tokens_path = model_dir.join("tokens.txt");
        if !tokens_path.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "FireRedASR tokens.txt not found: {}",
                tokens_path.display()
            )));
        }
        let tokens = fbank::load_tokens(&tokens_path)?;

        let encoder = Session::builder()
            .map_err(|e| TranscribeError::InitFailed(format!("ONNX encoder builder failed: {}", e)))?
            .with_intra_threads(threads)
            .map_err(|e| TranscribeError::InitFailed(format!("Failed to set encoder threads: {}", e)))?
            .commit_from_file(&encoder_file)
            .map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load FireRedASR encoder from {:?}: {}",
                    encoder_file, e
                ))
            })?;

        let decoder = Session::builder()
            .map_err(|e| TranscribeError::InitFailed(format!("ONNX decoder builder failed: {}", e)))?
            .with_intra_threads(threads)
            .map_err(|e| TranscribeError::InitFailed(format!("Failed to set decoder threads: {}", e)))?
            .commit_from_file(&decoder_file)
            .map_err(|e| {
                TranscribeError::InitFailed(format!(
                    "Failed to load FireRedASR decoder from {:?}: {}",
                    decoder_file, e
                ))
            })?;

        // Read CMVN from encoder metadata if available.
        // Must extract before moving encoder into Mutex.
        let (neg_mean, inv_stddev) = {
            match fbank::read_cmvn_from_metadata(&encoder) {
                Ok(cmvn) => cmvn,
                Err(_) => {
                    tracing::warn!("No CMVN stats in encoder metadata, using instance normalization");
                    (vec![], vec![])
                }
            }
        };

        let fbank_extractor = FbankExtractor::new(FbankConfig::sensevoice_default());

        tracing::info!(
            "FireRedASR model loaded in {:.2}s (encoder + decoder)",
            start.elapsed().as_secs_f32(),
        );

        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            tokens,
            neg_mean,
            inv_stddev,
            fbank_extractor,
            max_tokens: config.max_tokens,
        })
    }

    /// Autoregressive decoder loop (greedy search)
    fn decode_autoregressive(
        &self,
        encoder_output: &[f32],
        encoder_shape: &[i64],
    ) -> Result<String, TranscribeError> {
        let mut decoder = self.decoder.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock decoder: {}", e))
        })?;

        let mut generated_tokens: Vec<i64> = vec![DECODER_START_TOKEN_ID];
        let enc_time = encoder_shape[1] as usize;
        let enc_dim = encoder_shape[2] as usize;

        for step in 0..self.max_tokens {
            // Prepare decoder inputs
            let token_tensor = Tensor::<i64>::from_array((
                [1usize, generated_tokens.len()],
                generated_tokens.clone(),
            ))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to create token tensor: {}", e))
            })?;

            let enc_tensor = Tensor::<f32>::from_array((
                [1usize, enc_time, enc_dim],
                encoder_output.to_vec(),
            ))
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create encoder output tensor: {}",
                    e
                ))
            })?;

            let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
                (std::borrow::Cow::Borrowed("encoder_out"), enc_tensor.into()),
                (std::borrow::Cow::Borrowed("decoder_input"), token_tensor.into()),
            ];

            let outputs = decoder.run(inputs).map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "FireRedASR decoder step {} failed: {}",
                    step, e
                ))
            })?;

            // Extract logits from decoder output
            let logits_val = &outputs[0];
            let (shape, logits_data) = logits_val.try_extract_tensor::<f32>().map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to extract decoder logits: {}", e))
            })?;

            let shape_dims: &[i64] = shape;
            let vocab_size = *shape_dims.last().unwrap_or(&0) as usize;
            if vocab_size == 0 {
                break;
            }

            // Get logits for the last time step
            let last_step_offset = (logits_data.len() / vocab_size - 1) * vocab_size;
            let last_logits = &logits_data[last_step_offset..last_step_offset + vocab_size];

            // Greedy argmax
            let next_token = last_logits
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(idx, _)| idx as i64)
                .unwrap_or(EOS_TOKEN_ID);

            if next_token == EOS_TOKEN_ID {
                break;
            }

            generated_tokens.push(next_token);
        }

        // Convert token IDs to text (skip the start token)
        let content_tokens = &generated_tokens[1..];
        let mut result = String::new();
        for &id in content_tokens {
            if id == BLANK_ID || id == EOS_TOKEN_ID || id == DECODER_START_TOKEN_ID {
                continue;
            }
            if let Some(token_str) = self.tokens.get(&(id as u32)) {
                result.push_str(&token_str.replace('\u{2581}', " "));
            }
        }

        Ok(result.trim().to_string())
    }
}

impl Transcriber for FireRedAsrTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".to_string()));
        }

        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!("Transcribing {:.2}s of audio with FireRedASR", duration_secs);

        let start = std::time::Instant::now();

        // 1. Fbank extraction
        let fbank_features = self.fbank_extractor.extract(samples);
        if fbank_features.nrows() == 0 {
            return Err(TranscribeError::AudioFormat(
                "Audio too short for feature extraction".to_string(),
            ));
        }

        // 2. Normalization (CMVN if available, otherwise instance norm)
        let mut features = fbank_features;
        if !self.neg_mean.is_empty() {
            fbank::apply_cmvn(&mut features, &self.neg_mean, &self.inv_stddev);
        } else {
            fbank::apply_instance_norm(&mut features);
        }

        // 3. Encode
        let num_frames = features.nrows();
        let feat_dim = features.ncols();

        let (x_data, _offset) = features.into_raw_vec_and_offset();
        let x_tensor = Tensor::<f32>::from_array(([1usize, num_frames, feat_dim], x_data))
            .map_err(|e| TranscribeError::InferenceFailed(format!("Failed to create input tensor: {}", e)))?;

        let x_length_tensor = Tensor::<i32>::from_array(([1usize], vec![num_frames as i32]))
            .map_err(|e| TranscribeError::InferenceFailed(format!("Failed to create length tensor: {}", e)))?;

        let encoder_start = std::time::Instant::now();

        // Run encoder and extract output data before releasing the lock.
        // The ONNX outputs borrow from the session, so we must copy the data out.
        let (enc_data_owned, enc_shape_owned) = {
            let mut encoder = self.encoder.lock().map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to lock encoder: {}", e))
            })?;

            let enc_inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = vec![
                (std::borrow::Cow::Borrowed("speech"), x_tensor.into()),
                (std::borrow::Cow::Borrowed("speech_lengths"), x_length_tensor.into()),
            ];

            let enc_outputs = encoder.run(enc_inputs).map_err(|e| {
                TranscribeError::InferenceFailed(format!("FireRedASR encoder failed: {}", e))
            })?;

            let enc_val = &enc_outputs[0];
            let (enc_shape, enc_data) = enc_val.try_extract_tensor::<f32>().map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to extract encoder output: {}", e))
            })?;

            (enc_data.to_vec(), enc_shape.to_vec())
        };

        tracing::debug!("Encoder: {:.2}s", encoder_start.elapsed().as_secs_f32());

        if enc_shape_owned.len() != 3 {
            return Err(TranscribeError::InferenceFailed(format!(
                "Unexpected encoder output shape: {:?}",
                enc_shape_owned
            )));
        }

        // 4. Autoregressive decode
        let decoder_start = std::time::Instant::now();
        let result = self.decode_autoregressive(&enc_data_owned, &enc_shape_owned)?;
        tracing::debug!("Decoder: {:.2}s", decoder_start.elapsed().as_secs_f32());

        tracing::info!(
            "FireRedASR transcription completed in {:.2}s: {:?}",
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

fn find_encoder_file(model_dir: &std::path::Path) -> Result<PathBuf, TranscribeError> {
    for name in ["encoder.int8.onnx", "encoder.onnx", "encoder_model.onnx"] {
        let path = model_dir.join(name);
        if path.exists() {
            return Ok(path);
        }
    }
    Err(TranscribeError::ModelNotFound(format!(
        "FireRedASR encoder not found in {:?}\n  Expected encoder.int8.onnx or encoder.onnx",
        model_dir
    )))
}

fn find_decoder_file(model_dir: &std::path::Path) -> Result<PathBuf, TranscribeError> {
    for name in ["decoder.int8.onnx", "decoder.onnx", "decoder_model.onnx"] {
        let path = model_dir.join(name);
        if path.exists() {
            return Ok(path);
        }
    }
    Err(TranscribeError::ModelNotFound(format!(
        "FireRedASR decoder not found in {:?}\n  Expected decoder.int8.onnx or decoder.onnx",
        model_dir
    )))
}

fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    let model_dir_name = if model.starts_with("firered-") {
        model.to_string()
    } else {
        format!("firered-{}", model)
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
        "FireRedASR model '{}' not found.\n  Run 'voxtype setup model' to download.",
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
