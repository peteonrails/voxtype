//! Resident Parakeet GGUF inference through transcribe.cpp's Rust binding.
//!
//! The same `parakeet` engine as the ONNX path, selected by a `.gguf` model.
//! Its reason to exist is the GPU: on Vulkan it is several times faster than
//! ONNX Runtime on the CPU at the same accuracy, and it needs no ONNX Runtime.

use crate::config::ParakeetConfig;
use crate::error::TranscribeError;
use crate::transcribe::gguf::{self, GgufEngine, GgufLoad, GgufSession};
use crate::transcribe::window;
use crate::transcribe::Transcriber;
use transcribe_cpp::RunOptions;

const ENGINE: GgufEngine = GgufEngine {
    name: "Parakeet",
    section: "parakeet",
};

/// Parakeet models are fixed at 16kHz mono.
const SAMPLE_RATE: usize = 16_000;

pub struct ParakeetGgufTranscriber {
    session: GgufSession,
    /// Longest audio, in samples, sent through the model in one pass.
    /// Zero disables windowing.
    max_window: usize,
    /// Audio shared between consecutive windows, in samples.
    window_overlap: usize,
}

impl ParakeetGgufTranscriber {
    pub fn new(config: &ParakeetConfig) -> Result<Self, TranscribeError> {
        // Streaming runs on parakeet-rs's cache-aware ONNX pipeline. Loading
        // the GGUF in batch mode instead would quietly drop the live partials
        // the user asked for.
        if config.streaming {
            return Err(TranscribeError::ConfigError(
                "[parakeet] streaming = true needs an ONNX streaming model; GGUF models \
                 run in batch mode only. Set streaming = false, or use an ONNX model \
                 such as parakeet-unified-en-0.6b"
                    .to_string(),
            ));
        }
        let start = std::time::Instant::now();
        let session = GgufSession::load(
            &ENGINE,
            &GgufLoad {
                model: &config.model,
                backend: &config.gguf_backend,
                threads: config.threads,
            },
        )?;
        let arch = session.arch()?;
        if arch != "parakeet" {
            return Err(TranscribeError::InitFailed(format!(
                "{} is a '{arch}' GGUF, not a Parakeet model",
                config.model
            )));
        }
        tracing::info!(
            "Parakeet GGUF model loaded in {:.2}s",
            start.elapsed().as_secs_f32()
        );
        Ok(Self {
            session,
            max_window: (config.max_window_secs.max(0.0) * SAMPLE_RATE as f32) as usize,
            window_overlap: (config.window_overlap_secs.max(0.0) * SAMPLE_RATE as f32) as usize,
        })
    }
}

impl Transcriber for ParakeetGgufTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let mut session = self.session.lock()?;
        // v3 detects the language itself; a hint only selects a prompt on
        // the prompt-conditioned variants.
        let options = RunOptions::default();
        let start = std::time::Instant::now();

        // transcribe.cpp's Parakeet encoder uses full attention, so memory
        // grows with the square of the input length and a long recording
        // can exhaust the GPU. Window it exactly as the ONNX path does.
        let text = if self.max_window > 0 && samples.len() > self.max_window {
            let spans = window::windows(samples.len(), self.max_window, self.window_overlap);
            tracing::info!(
                "Parakeet GGUF: {:.1}s exceeds the {:.0}s window; transcribing in {} \
                 overlapping passes",
                samples.len() as f32 / SAMPLE_RATE as f32,
                self.max_window as f32 / SAMPLE_RATE as f32,
                spans.len()
            );
            let mut merged = String::new();
            for (begin, end) in spans {
                let pass = gguf::run(&mut session, &samples[begin..end], &options)?;
                merged = window::merge_overlapping(&merged, &pass.text);
            }
            merged
        } else {
            gguf::run(&mut session, samples, &options)?
                .text
                .trim()
                .to_string()
        };

        tracing::info!(
            "Parakeet GGUF transcribed {:.2}s of audio in {:.2}s",
            samples.len() as f32 / SAMPLE_RATE as f32,
            start.elapsed().as_secs_f32()
        );
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_samples(path: &str) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.bits_per_sample, 16);
        reader
            .samples::<i16>()
            .map(|sample| sample.unwrap() as f32 / 32768.0)
            .collect()
    }

    #[test]
    fn rejects_a_file_without_a_gguf_header() {
        let dir = tempfile::tempdir().unwrap();
        let invalid = dir.path().join("invalid.gguf");
        std::fs::write(&invalid, b"not a gguf").unwrap();
        let config = ParakeetConfig {
            model: invalid.to_string_lossy().into_owned(),
            ..ParakeetConfig::default()
        };
        let err = ParakeetGgufTranscriber::new(&config).err().unwrap();
        assert!(err.to_string().contains("is not a GGUF model"), "{err}");
    }

    /// Streaming needs parakeet-rs's ONNX pipeline. Accepting the setting and
    /// running batch would drop the live partials without saying so.
    #[test]
    fn streaming_with_a_gguf_model_is_a_config_error() {
        let config = ParakeetConfig {
            model: "parakeet-tdt-0.6b-v3-F16.gguf".into(),
            streaming: true,
            ..ParakeetConfig::default()
        };
        assert!(matches!(
            ParakeetGgufTranscriber::new(&config),
            Err(TranscribeError::ConfigError(_))
        ));
    }

    #[test]
    fn invalid_backend_names_the_parakeet_key() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("header-only.gguf");
        std::fs::write(&model, b"GGUF").unwrap();
        let config = ParakeetConfig {
            model: model.to_string_lossy().into_owned(),
            gguf_backend: "opencl".into(),
            ..ParakeetConfig::default()
        };
        let err = ParakeetGgufTranscriber::new(&config).err().unwrap();
        assert!(err.to_string().contains("parakeet.gguf_backend"), "{err}");
    }

    /// Run with VOXTYPE_PARAKEET_GGUF_MODEL and VOXTYPE_PARAKEET_GGUF_WAV set.
    /// Also forces the windowed path on the same clip, so the stitcher is
    /// exercised on real model output, not only on the unit fixtures.
    #[test]
    #[ignore = "requires a Parakeet GGUF, speech WAV, and a Vulkan device"]
    fn transcribes_twice_with_one_resident_vulkan_session() {
        let config = ParakeetConfig {
            model: std::env::var("VOXTYPE_PARAKEET_GGUF_MODEL").unwrap(),
            gguf_backend: "vulkan".into(),
            ..ParakeetConfig::default()
        };
        let samples = wav_samples(&std::env::var("VOXTYPE_PARAKEET_GGUF_WAV").unwrap());
        let transcriber = ParakeetGgufTranscriber::new(&config).unwrap();
        let first = transcriber.transcribe(&samples).unwrap();
        let second = transcriber.transcribe(&samples).unwrap();
        println!("single pass: {first}");
        assert!(!first.is_empty());
        assert_eq!(first, second);

        let duration = samples.len() as f32 / SAMPLE_RATE as f32;
        let windowed = ParakeetGgufTranscriber::new(&ParakeetConfig {
            max_window_secs: (duration / 2.0).max(2.0),
            window_overlap_secs: 1.0,
            ..config
        })
        .unwrap()
        .transcribe(&samples)
        .unwrap();
        println!("windowed:    {windowed}");
        let words = |text: &str| text.split_whitespace().count() as f32;
        assert!(
            (words(&windowed) - words(&first)).abs() <= words(&first) * 0.2 + 3.0,
            "windowed transcript diverged: {windowed}"
        );
    }
}
