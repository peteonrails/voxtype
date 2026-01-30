//! Parakeet-based speech-to-text transcription
//!
//! Uses NVIDIA's Parakeet model via the parakeet-rs crate for fast, local transcription.
//! This module is only available when the `parakeet` feature is enabled.
//!
//! Supports two model architectures:
//! - CTC (Connectionist Temporal Classification): faster, character-level output
//! - TDT (Token-Duration-Transducer): recommended, proper punctuation and word boundaries

use super::{StreamingTranscriber, Transcriber};
use crate::config::{ParakeetConfig, ParakeetModelType};
use crate::error::TranscribeError;
#[cfg(any(
    feature = "parakeet-cuda",
    feature = "parakeet-rocm",
    feature = "parakeet-tensorrt"
))]
use parakeet_rs::ExecutionProvider;
use parakeet_rs::{
    ExecutionConfig, Nemotron, Parakeet, ParakeetTDT, Transcriber as ParakeetTranscriberTrait,
};
use std::path::PathBuf;
use std::sync::Mutex;

/// Internal enum to hold CTC, TDT, or Nemotron model instance
enum ParakeetModel {
    /// CTC model (character-level, faster)
    Ctc(Mutex<Parakeet>),
    /// TDT model (token-level, better quality output)
    Tdt(Mutex<ParakeetTDT>),
    /// Nemotron model (streaming transducer)
    Nemotron(Mutex<Nemotron>),
}

/// Parakeet-based transcriber using ONNX Runtime
pub struct ParakeetTranscriber {
    /// Parakeet model instance (CTC or TDT)
    model: ParakeetModel,
    /// Model type for logging
    model_type: ParakeetModelType,
}

impl ParakeetTranscriber {
    /// Create a new Parakeet transcriber
    pub fn new(config: &ParakeetConfig) -> Result<Self, TranscribeError> {
        let model_path = resolve_model_path(&config.model)?;

        // Determine model type: use config override or auto-detect from directory
        let model_type = config
            .model_type
            .unwrap_or_else(|| detect_model_type(&model_path));

        tracing::info!(
            "Loading Parakeet {:?} model from {:?}",
            model_type,
            model_path
        );
        let start = std::time::Instant::now();

        // Configure execution provider based on feature flags
        let exec_config = build_execution_config();

        let model = match model_type {
            ParakeetModelType::Ctc => {
                let parakeet =
                    Parakeet::from_pretrained(&model_path, exec_config).map_err(|e| {
                        TranscribeError::InitFailed(format!("Parakeet CTC init failed: {}", e))
                    })?;
                ParakeetModel::Ctc(Mutex::new(parakeet))
            }
            ParakeetModelType::Tdt => {
                let parakeet =
                    ParakeetTDT::from_pretrained(&model_path, exec_config).map_err(|e| {
                        TranscribeError::InitFailed(format!("Parakeet TDT init failed: {}", e))
                    })?;
                ParakeetModel::Tdt(Mutex::new(parakeet))
            }
            ParakeetModelType::Nemotron => {
                let nemotron =
                    Nemotron::from_pretrained(&model_path, exec_config).map_err(|e| {
                        TranscribeError::InitFailed(format!("Nemotron init failed: {}", e))
                    })?;
                ParakeetModel::Nemotron(Mutex::new(nemotron))
            }
        };

        tracing::info!(
            "Parakeet {:?} model loaded in {:.2}s",
            model_type,
            start.elapsed().as_secs_f32()
        );

        Ok(Self { model, model_type })
    }
}

impl Transcriber for ParakeetTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with Parakeet {:?}",
            duration_secs,
            samples.len(),
            self.model_type
        );

        let start = std::time::Instant::now();

        let text = match &self.model {
            ParakeetModel::Ctc(parakeet) => {
                let mut parakeet = parakeet.lock().map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Failed to lock Parakeet mutex: {}",
                        e
                    ))
                })?;

                let result = parakeet
                    .transcribe_samples(
                        samples.to_vec(),
                        16000, // sample rate
                        1,     // mono
                        None,  // default timestamp mode
                    )
                    .map_err(|e| {
                        TranscribeError::InferenceFailed(format!(
                            "Parakeet CTC inference failed: {}",
                            e
                        ))
                    })?;

                result.text.trim().to_string()
            }
            ParakeetModel::Tdt(parakeet) => {
                let mut parakeet = parakeet.lock().map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Failed to lock Parakeet mutex: {}",
                        e
                    ))
                })?;

                let result = parakeet
                    .transcribe_samples(
                        samples.to_vec(),
                        16000, // sample rate
                        1,     // mono
                        None,  // default timestamp mode
                    )
                    .map_err(|e| {
                        TranscribeError::InferenceFailed(format!(
                            "Parakeet TDT inference failed: {}",
                            e
                        ))
                    })?;

                result.text.trim().to_string()
            }
            ParakeetModel::Nemotron(nemotron) => {
                let mut nemotron = nemotron.lock().map_err(|e| {
                    TranscribeError::InferenceFailed(format!(
                        "Failed to lock Nemotron mutex: {}",
                        e
                    ))
                })?;

                nemotron.reset();
                let text = nemotron.transcribe_audio(samples).map_err(|e| {
                    TranscribeError::InferenceFailed(format!("Nemotron inference failed: {}", e))
                })?;

                text.trim().to_string()
            }
        };

        tracing::info!(
            "Parakeet {:?} transcription completed in {:.2}s: {:?}",
            self.model_type,
            start.elapsed().as_secs_f32(),
            if text.chars().count() > 50 {
                format!("{}...", text.chars().take(50).collect::<String>())
            } else {
                text.clone()
            }
        );

        Ok(text)
    }
}

/// Nemotron streaming transcriber for real-time output during recording
pub struct NemotronStreamingTranscriber {
    model: Nemotron,
    /// Text already returned to the caller (for computing deltas)
    last_transcript_len: usize,
}

impl NemotronStreamingTranscriber {
    pub fn new(config: &ParakeetConfig) -> Result<Self, TranscribeError> {
        let model_path = resolve_model_path(&config.model)?;

        tracing::info!("Loading Nemotron streaming model from {:?}", model_path);
        let start = std::time::Instant::now();

        let exec_config = build_execution_config();

        let model = Nemotron::from_pretrained(&model_path, exec_config)
            .map_err(|e| TranscribeError::InitFailed(format!("Nemotron init failed: {}", e)))?;

        tracing::info!(
            "Nemotron streaming model loaded in {:.2}s",
            start.elapsed().as_secs_f32()
        );

        Ok(Self {
            model,
            last_transcript_len: 0,
        })
    }
}

impl StreamingTranscriber for NemotronStreamingTranscriber {
    fn transcribe_chunk(&mut self, chunk: &[f32]) -> Result<String, TranscribeError> {
        let full_text = self.model.transcribe_chunk(chunk).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Nemotron chunk inference failed: {}", e))
        })?;

        // Return only the new text (delta)
        let delta = if full_text.len() > self.last_transcript_len {
            full_text[self.last_transcript_len..].to_string()
        } else {
            String::new()
        };
        self.last_transcript_len = full_text.len();

        Ok(delta)
    }

    fn flush(&mut self) -> Result<String, TranscribeError> {
        // Feed 3 silence chunks to drain the decoder
        let silence = vec![0.0f32; self.chunk_size()];
        let mut flushed = String::new();
        for _ in 0..3 {
            let delta = self.transcribe_chunk(&silence)?;
            flushed.push_str(&delta);
        }
        Ok(flushed)
    }

    fn reset(&mut self) {
        self.model.reset();
        self.last_transcript_len = 0;
    }

    fn get_transcript(&self) -> String {
        self.model.get_transcript()
    }

    fn chunk_size(&self) -> usize {
        // 560ms at 16kHz = 8960 samples
        8960
    }
}

/// Factory function to create a Nemotron streaming transcriber
pub fn create_nemotron_streaming(
    config: &ParakeetConfig,
) -> Result<Box<dyn StreamingTranscriber>, TranscribeError> {
    Ok(Box::new(NemotronStreamingTranscriber::new(config)?))
}

/// Build execution config based on compile-time feature flags
fn build_execution_config() -> Option<ExecutionConfig> {
    #[cfg(feature = "parakeet-cuda")]
    {
        tracing::info!("Configuring CUDA execution provider for NVIDIA GPU acceleration");
        return Some(ExecutionConfig::new().with_execution_provider(ExecutionProvider::Cuda));
    }

    #[cfg(feature = "parakeet-tensorrt")]
    {
        tracing::info!("Configuring TensorRT execution provider for NVIDIA GPU acceleration");
        return Some(ExecutionConfig::new().with_execution_provider(ExecutionProvider::TensorRT));
    }

    #[cfg(feature = "parakeet-rocm")]
    {
        tracing::info!("Configuring ROCm execution provider for AMD GPU acceleration");
        return Some(ExecutionConfig::new().with_execution_provider(ExecutionProvider::ROCm));
    }

    #[cfg(not(any(
        feature = "parakeet-cuda",
        feature = "parakeet-tensorrt",
        feature = "parakeet-rocm"
    )))]
    {
        None
    }
}

/// Auto-detect model type from directory structure
///
/// Nemotron models have: encoder.onnx, encoder.onnx.data, decoder_joint.onnx, tokenizer.model
/// TDT models have: encoder-model.onnx, decoder_joint-model.onnx, vocab.txt
/// CTC models have: model.onnx (or model_int8.onnx), tokenizer.json
fn detect_model_type(path: &PathBuf) -> ParakeetModelType {
    // Check for Nemotron model structure (must come before TDT since both have encoder/decoder)
    // Nemotron uses non-hyphenated names: encoder.onnx (not encoder-model.onnx)
    let has_nemotron_encoder =
        path.join("encoder.onnx").exists() || path.join("encoder.onnx.data").exists();
    let has_nemotron_decoder = path.join("decoder_joint.onnx").exists();
    let has_sentencepiece = path.join("tokenizer.model").exists();

    if has_nemotron_encoder && has_nemotron_decoder && has_sentencepiece {
        tracing::debug!(
            "Auto-detected Nemotron model (found encoder.onnx + decoder_joint.onnx + tokenizer.model)"
        );
        return ParakeetModelType::Nemotron;
    }

    // Check for TDT model structure (hyphenated names: encoder-model.onnx)
    let has_encoder =
        path.join("encoder-model.onnx").exists() || path.join("encoder-model.onnx.data").exists();
    let has_decoder = path.join("decoder_joint-model.onnx").exists();

    if has_encoder && has_decoder {
        tracing::debug!("Auto-detected TDT model (found encoder + decoder ONNX files)");
        return ParakeetModelType::Tdt;
    }

    // Check for CTC model structure
    let has_ctc_model = path.join("model.onnx").exists() || path.join("model_int8.onnx").exists();
    let has_tokenizer = path.join("tokenizer.json").exists();

    if has_ctc_model && has_tokenizer {
        tracing::debug!("Auto-detected CTC model (found model.onnx + tokenizer.json)");
        return ParakeetModelType::Ctc;
    }

    // Default to TDT (recommended for most use cases)
    tracing::warn!(
        "Could not auto-detect model type from {:?}, defaulting to TDT. \
        Set model_type in config to override.",
        path
    );
    ParakeetModelType::Tdt
}

/// Resolve model name to directory path
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    // If it's already an absolute path, use it directly
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    // Check models directory
    let models_dir = crate::config::Config::models_dir();
    let model_path = models_dir.join(model);

    if model_path.exists() {
        return Ok(model_path);
    }

    // Check current directory
    let cwd_path = PathBuf::from(model);
    if cwd_path.exists() {
        return Ok(cwd_path);
    }

    // Check ./models/
    let local_models_path = PathBuf::from("models").join(model);
    if local_models_path.exists() {
        return Ok(local_models_path);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Parakeet model '{}' not found. Looked in:\n  - {}\n  - {}\n  - {}\n\n\
        Download TDT (recommended): https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx\n\
        Download CTC: https://huggingface.co/nvidia/parakeet-ctc-0.6b",
        model,
        model_path.display(),
        cwd_path.display(),
        local_models_path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_detect_model_type_tdt_with_encoder_and_decoder() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Create TDT model structure
        fs::write(model_path.join("encoder-model.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("decoder_joint-model.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("vocab.txt"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_detect_model_type_tdt_with_encoder_data_file() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // TDT model with .onnx.data file (large models split data)
        fs::write(model_path.join("encoder-model.onnx.data"), b"dummy").unwrap();
        fs::write(model_path.join("decoder_joint-model.onnx"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_detect_model_type_ctc_with_model_and_tokenizer() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Create CTC model structure
        fs::write(model_path.join("model.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("tokenizer.json"), b"{}").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Ctc);
    }

    #[test]
    fn test_detect_model_type_ctc_with_int8_model() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // CTC model with quantized int8 variant
        fs::write(model_path.join("model_int8.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("tokenizer.json"), b"{}").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Ctc);
    }

    #[test]
    fn test_detect_model_type_nemotron() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Create Nemotron model structure (non-hyphenated encoder.onnx)
        fs::write(model_path.join("encoder.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("encoder.onnx.data"), b"dummy").unwrap();
        fs::write(model_path.join("decoder_joint.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("tokenizer.model"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Nemotron);
    }

    #[test]
    fn test_detect_model_type_nemotron_without_data_file() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Nemotron with encoder.onnx (no .data file) + decoder + tokenizer.model
        fs::write(model_path.join("encoder.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("decoder_joint.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("tokenizer.model"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Nemotron);
    }

    #[test]
    fn test_detect_model_type_tdt_not_confused_with_nemotron() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // TDT uses hyphenated names - should NOT match Nemotron
        fs::write(model_path.join("encoder-model.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("decoder_joint-model.onnx"), b"dummy").unwrap();
        fs::write(model_path.join("vocab.txt"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_detect_model_type_defaults_to_tdt_when_ambiguous() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Empty directory - should default to TDT
        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_detect_model_type_defaults_to_tdt_with_partial_files() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Only encoder without decoder - ambiguous, defaults to TDT
        fs::write(model_path.join("encoder-model.onnx"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_detect_model_type_ctc_requires_both_files() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Only model.onnx without tokenizer - should not detect as CTC
        fs::write(model_path.join("model.onnx"), b"dummy").unwrap();

        let detected = detect_model_type(&model_path);
        // Falls through to default (TDT) because CTC requires both files
        assert_eq!(detected, ParakeetModelType::Tdt);
    }

    #[test]
    fn test_resolve_model_path_absolute() {
        let temp_dir = TempDir::new().unwrap();
        let model_path = temp_dir.path().to_path_buf();

        // Create a dummy file so the path exists
        fs::write(model_path.join("model.onnx"), b"dummy").unwrap();

        let resolved = resolve_model_path(model_path.to_str().unwrap());
        assert!(resolved.is_ok());
        assert_eq!(resolved.unwrap(), model_path);
    }

    #[test]
    fn test_resolve_model_path_not_found() {
        let result = resolve_model_path("/nonexistent/path/to/model");
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(matches!(err, TranscribeError::ModelNotFound(_)));
    }
}
