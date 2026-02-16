//! Omnilingual ASR transcription (Meta MMS wav2vec2)
//!
//! Uses Meta's Massively Multilingual Speech model via ONNX Runtime for local
//! transcription. Supports 1600+ languages with a single model. CTC-based,
//! character-level tokenizer with 9812 symbols.
//!
//! The model takes raw audio waveform as input (no Fbank preprocessing).
//! Audio is mean-variance normalized before inference.
//!
//! Pipeline: Audio (f32, 16kHz) -> Normalize -> ONNX model -> CTC decode
//!
//! Languages: 1600+ (language-agnostic, no language selection)
//! Model files: model.int8.onnx (or model.onnx), tokens.txt

use super::ctc;
use super::Transcriber;
use crate::config::OmnilingualConfig;
use crate::error::TranscribeError;
use ort::session::Session;
use ort::value::Tensor;
use std::collections::HashMap;
use std::path::PathBuf;

/// Sample rate expected by Omnilingual
const SAMPLE_RATE: usize = 16000;

/// Omnilingual ASR transcriber using ONNX Runtime
pub struct OmnilingualTranscriber {
    session: std::sync::Mutex<Session>,
    tokens: HashMap<u32, String>,
}

impl OmnilingualTranscriber {
    pub fn new(config: &OmnilingualConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;

        tracing::info!("Loading Omnilingual model from {:?}", model_dir);
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
                    "Omnilingual model not found in {:?}\n  \
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
                "Omnilingual tokens.txt not found: {}\n  \
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
                    "Failed to load Omnilingual model from {:?}: {}",
                    model_file, e
                ))
            })?;

        tracing::info!(
            "Omnilingual model loaded in {:.2}s",
            start.elapsed().as_secs_f32(),
        );

        Ok(Self {
            session: std::sync::Mutex::new(session),
            tokens,
        })
    }
}

impl Transcriber for OmnilingualTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with Omnilingual",
            duration_secs,
            samples.len(),
        );

        let start = std::time::Instant::now();

        // Apply mean-variance normalization (instance normalization)
        let normalized = normalize_audio(samples);

        let num_samples = normalized.len();

        // x: shape [1, num_samples]
        let x_tensor =
            Tensor::<f32>::from_array(([1usize, num_samples], normalized)).map_err(|e| {
                TranscribeError::InferenceFailed(format!(
                    "Failed to create input tensor: {}",
                    e
                ))
            })?;

        // Run inference
        let inference_start = std::time::Instant::now();
        let mut session = self.session.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock session: {}", e))
        })?;

        let inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> =
            vec![(std::borrow::Cow::Borrowed("x"), x_tensor.into())];

        let outputs = session.run(inputs).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Omnilingual inference failed: {}", e))
        })?;

        tracing::debug!(
            "ONNX inference: {:.2}s",
            inference_start.elapsed().as_secs_f32(),
        );

        // Extract CTC logits and decode
        let logits_val = outputs
            .get("logits")
            .or_else(|| outputs.get("output"))
            .ok_or_else(|| {
                TranscribeError::InferenceFailed(
                    "Omnilingual output not found (expected 'logits' or 'output')".to_string(),
                )
            })?;

        let (shape, logits_data) = logits_val.try_extract_tensor::<f32>().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to extract logits: {}", e))
        })?;

        let shape_dims: &[i64] = shape;
        tracing::debug!("Omnilingual output shape: {:?}", shape_dims);

        // CTC output: [batch, time_steps, vocab_size]
        let result = if shape_dims.len() == 3 {
            let time_steps = shape_dims[1] as usize;
            let vocab_size = shape_dims[2] as usize;
            let config = ctc::CtcConfig {
                blank_id: 0,
                num_metadata_tokens: 0,
                sentencepiece_cleanup: false, // character-level tokenizer
            };
            ctc::ctc_greedy_decode(logits_data, time_steps, vocab_size, &self.tokens, &config)
        } else if shape_dims.len() == 2 {
            // Pre-argmaxed output
            let time_steps = shape_dims[1] as usize;
            let config = ctc::CtcConfig {
                blank_id: 0,
                num_metadata_tokens: 0,
                sentencepiece_cleanup: false,
            };
            ctc::decode_pre_argmax(&logits_data[..time_steps], &self.tokens, &config)
        } else {
            return Err(TranscribeError::InferenceFailed(format!(
                "Unexpected Omnilingual output shape: {:?}",
                shape_dims
            )));
        };

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

/// Apply mean-variance normalization (instance normalization) to audio samples
///
/// The wav2vec2-based model expects normalized audio:
/// `normalized = (samples - mean) / sqrt(variance + epsilon)`
fn normalize_audio(samples: &[f32]) -> Vec<f32> {
    let n = samples.len() as f32;
    let mean: f32 = samples.iter().sum::<f32>() / n;
    let variance: f32 = samples.iter().map(|&s| (s - mean) * (s - mean)).sum::<f32>() / n;
    let inv_stddev = 1.0 / (variance + 1e-5_f32).sqrt();

    samples.iter().map(|&s| (s - mean) * inv_stddev).collect()
}

/// Resolve model name to directory path
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
        "sherpa-onnx-omnilingual-asr-1600-languages-{}-ctc",
        model.trim_start_matches("omnilingual-")
    );
    let sherpa_path = models_dir.join(&sherpa_name);
    if sherpa_path.exists() {
        return Ok(sherpa_path);
    }

    // Also check int8 variant naming
    let sherpa_int8_name = format!("{}-int8", &sherpa_name);
    let sherpa_int8_path = models_dir.join(&sherpa_int8_name);
    if sherpa_int8_path.exists() {
        return Ok(sherpa_int8_path);
    }

    // Check with date suffix pattern
    let models_dir_read = std::fs::read_dir(&models_dir);
    if let Ok(entries) = models_dir_read {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("sherpa-onnx-omnilingual-asr") && entry.path().is_dir() {
                let has_model = entry.path().join("model.int8.onnx").exists()
                    || entry.path().join("model.onnx").exists();
                let has_tokens = entry.path().join("tokens.txt").exists();
                if has_model && has_tokens {
                    return Ok(entry.path());
                }
            }
        }
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Omnilingual model '{}' not found. Looked in:\n  \
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
    fn test_normalize_audio() {
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let normalized = normalize_audio(&samples);

        // Mean should be ~0 after normalization
        let mean: f32 = normalized.iter().sum::<f32>() / normalized.len() as f32;
        assert!(mean.abs() < 1e-5);

        // Standard deviation should be ~1
        let variance: f32 = normalized
            .iter()
            .map(|&s| (s - mean) * (s - mean))
            .sum::<f32>()
            / normalized.len() as f32;
        assert!((variance - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_normalize_audio_constant() {
        // Constant signal: all same value
        let samples = vec![5.0; 100];
        let normalized = normalize_audio(&samples);
        // Should not produce NaN/Inf thanks to epsilon
        assert!(normalized.iter().all(|x| x.is_finite()));
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
