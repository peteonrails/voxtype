//! Parakeet-based speech-to-text transcription
//!
//! Uses NVIDIA's Parakeet model via the parakeet-rs crate for fast, local transcription.
//! This module is only available when the `parakeet` feature is enabled.

use super::Transcriber;
use crate::config::ParakeetConfig;
use crate::error::TranscribeError;
use parakeet_rs::{Parakeet, Transcriber as ParakeetTranscriberTrait};
use std::path::PathBuf;
use std::sync::Mutex;

/// Parakeet-based transcriber using ONNX Runtime
pub struct ParakeetTranscriber {
    /// Parakeet model instance wrapped in Mutex for interior mutability
    /// parakeet-rs requires &mut self for transcribe_samples, but our trait uses &self
    parakeet: Mutex<Parakeet>,
}

impl ParakeetTranscriber {
    /// Create a new Parakeet transcriber
    pub fn new(config: &ParakeetConfig) -> Result<Self, TranscribeError> {
        let model_path = resolve_model_path(&config.model)?;

        tracing::info!("Loading Parakeet model from {:?}", model_path);
        let start = std::time::Instant::now();

        let parakeet = Parakeet::from_pretrained(&model_path, None)
            .map_err(|e| TranscribeError::InitFailed(format!("Parakeet init failed: {}", e)))?;

        tracing::info!("Parakeet model loaded in {:.2}s", start.elapsed().as_secs_f32());

        Ok(Self {
            parakeet: Mutex::new(parakeet),
        })
    }
}

impl Transcriber for ParakeetTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat("Empty audio buffer".to_string()));
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) with Parakeet",
            duration_secs,
            samples.len()
        );

        let start = std::time::Instant::now();

        // Lock the mutex to get mutable access to the Parakeet instance
        let mut parakeet = self.parakeet.lock().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to lock Parakeet mutex: {}", e))
        })?;

        let result = parakeet
            .transcribe_samples(
                samples.to_vec(),
                16000, // sample rate - Whisper/Parakeet expect 16kHz
                1,     // mono
                None,  // default timestamp mode
            )
            .map_err(|e| TranscribeError::InferenceFailed(format!("Parakeet inference failed: {}", e)))?;

        let text = result.text.trim().to_string();

        tracing::info!(
            "Parakeet transcription completed in {:.2}s: {:?}",
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
        "Parakeet model '{}' not found. Looked in:\n  - {}\n  - {}\n  - {}\n\nDownload from: https://huggingface.co/nvidia/parakeet-ctc-0.6b",
        model,
        model_path.display(),
        cwd_path.display(),
        local_models_path.display()
    )))
}
