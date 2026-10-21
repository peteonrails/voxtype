//! Resident Cohere GGUF inference through transcribe.cpp's Rust binding.

use crate::config::CohereConfig;
use crate::error::TranscribeError;
use crate::transcribe::cohere_chunking::CohereChunking;
use crate::transcribe::gguf::{self, GgufEngine, GgufLoad, GgufSession};
use crate::transcribe::Transcriber;
use transcribe_cpp::RunOptions;

const ENGINE: GgufEngine = GgufEngine {
    name: "Cohere",
    section: "cohere",
};

pub struct CohereGgufTranscriber {
    session: GgufSession,
    language: String,
    chunking: CohereChunking,
}

impl CohereGgufTranscriber {
    pub fn new(config: &CohereConfig) -> Result<Self, TranscribeError> {
        let chunking = CohereChunking::new(config)?;
        let session = GgufSession::load(
            &ENGINE,
            &GgufLoad {
                model: &config.model,
                backend: &config.gguf_backend,
                threads: config.threads,
            },
        )?;
        Ok(Self {
            session,
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
        let mut session = self.session.lock()?;
        let options = RunOptions {
            language: Some(self.language.clone()),
            ..Default::default()
        };
        self.chunking.transcribe(samples, |chunk| {
            gguf::run(&mut session, chunk, &options).map(|result| result.text)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transcribe_cpp::Backend;

    #[test]
    fn validates_backend_and_model_header() {
        assert_eq!(
            gguf::parse_backend("VULKAN", "cohere").unwrap(),
            Backend::Vulkan
        );
        assert!(gguf::parse_backend("bogus", "cohere").is_err());
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
