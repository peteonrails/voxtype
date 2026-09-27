//! Resident Cohere GGUF inference through transcribe.cpp's Rust binding.

use crate::config::{CohereConfig, Config};
use crate::error::TranscribeError;
use crate::transcribe::cohere_chunking::CohereChunking;
use crate::transcribe::Transcriber;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use transcribe_cpp::{Backend, Model, ModelOptions, RunOptions, Session, SessionOptions};

pub struct CohereGgufTranscriber {
    session: Mutex<Session>,
    language: String,
    chunking: CohereChunking,
}

impl CohereGgufTranscriber {
    pub fn new(config: &CohereConfig) -> Result<Self, TranscribeError> {
        let chunking = CohereChunking::new(config)?;
        let configured = PathBuf::from(&config.model);
        let model_path = if configured.exists() || configured.is_absolute() {
            configured
        } else {
            Config::models_dir().join(configured)
        };
        let mut file = std::fs::File::open(&model_path).map_err(|e| {
            TranscribeError::ModelNotFound(format!("{}: {e}", model_path.display()))
        })?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic).map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Cannot read GGUF header at {}: {e}",
                model_path.display()
            ))
        })?;
        if &magic != b"GGUF" {
            return Err(TranscribeError::InitFailed(format!(
                "{} is not a GGUF model",
                model_path.display()
            )));
        }

        let backend = parse_backend(&config.gguf_backend)?;
        let model = Model::load_with(
            &model_path,
            &ModelOptions {
                backend,
                ..Default::default()
            },
        )
        .map_err(|e| TranscribeError::InitFailed(format!("GGUF model load: {e}")))?;
        let device = model
            .device()
            .map_err(|e| TranscribeError::InitFailed(format!("GGUF device query: {e}")))?;
        if backend == Backend::Vulkan {
            let name = format!("{} {}", device.name, device.description).to_ascii_lowercase();
            if device.kind != "vulkan"
                || ["llvmpipe", "lavapipe", "swiftshader", "software"]
                    .iter()
                    .any(|renderer| name.contains(renderer))
            {
                return Err(TranscribeError::InitFailed(format!(
                    "Cohere requested hardware Vulkan but transcribe.cpp selected {} [{}] ({})",
                    device.name, device.kind, device.description
                )));
            }
        }
        tracing::info!(
            "Cohere GGUF loaded on {} [{}] ({})",
            device.name,
            model.backend(),
            device.description
        );
        let session = model
            .session_with(&SessionOptions {
                n_threads: config.threads.unwrap_or(0).try_into().map_err(|_| {
                    TranscribeError::ConfigError("cohere.threads exceeds i32::MAX".into())
                })?,
                ..Default::default()
            })
            .map_err(|e| TranscribeError::InitFailed(format!("GGUF session init: {e}")))?;
        Ok(Self {
            session: Mutex::new(session),
            language: config.language.clone(),
            chunking,
        })
    }
}

impl Transcriber for CohereGgufTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let mut session = self
            .session
            .lock()
            .map_err(|e| TranscribeError::InferenceFailed(format!("GGUF session lock: {e}")))?;
        let options = RunOptions {
            language: Some(self.language.clone()),
            ..Default::default()
        };
        self.chunking.transcribe(samples, |chunk| {
            session
                .run(chunk, &options)
                .map(|result| result.text)
                .map_err(|e| TranscribeError::InferenceFailed(format!("GGUF inference: {e}")))
        })
    }
}

fn parse_backend(value: &str) -> Result<Backend, TranscribeError> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(Backend::Auto),
        "cpu" => Ok(Backend::Cpu),
        "cpu_accel" => Ok(Backend::CpuAccel),
        "vulkan" => Ok(Backend::Vulkan),
        "metal" => Ok(Backend::Metal),
        "cuda" => Ok(Backend::Cuda),
        "rocm" => Ok(Backend::Rocm),
        _ => Err(TranscribeError::ConfigError(format!(
            "Invalid cohere.gguf_backend: {value}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_backend_and_model_header() {
        assert_eq!(parse_backend("VULKAN").unwrap(), Backend::Vulkan);
        assert!(parse_backend("bogus").is_err());
        let dir = tempfile::tempdir().unwrap();
        let invalid = dir.path().join("invalid.gguf");
        std::fs::write(&invalid, b"not a gguf").unwrap();
        let config = CohereConfig {
            model: invalid.to_string_lossy().into_owned(),
            ..CohereConfig::default()
        };
        assert!(CohereGgufTranscriber::new(&config).is_err());
    }

    /// Run with VOXTYPE_COHERE_GGUF_MODEL and VOXTYPE_COHERE_GGUF_WAV set.
    #[test]
    #[ignore = "requires a Cohere GGUF, speech WAV, and a Vulkan device"]
    fn transcribes_twice_with_one_resident_vulkan_session() {
        let config = CohereConfig {
            model: std::env::var("VOXTYPE_COHERE_GGUF_MODEL").unwrap(),
            gguf_backend: "vulkan".into(),
            ..CohereConfig::default()
        };
        let wav = std::env::var("VOXTYPE_COHERE_GGUF_WAV").unwrap();
        let mut reader = hound::WavReader::open(wav).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.bits_per_sample, 16);
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|sample| sample.unwrap() as f32 / 32768.0)
            .collect();
        let transcriber = CohereGgufTranscriber::new(&config).unwrap();
        let first = transcriber.transcribe(&samples).unwrap();
        let second = transcriber.transcribe(&samples).unwrap();
        assert!(first.to_ascii_lowercase().contains("country"), "{first}");
        assert_eq!(first, second);
    }
}
