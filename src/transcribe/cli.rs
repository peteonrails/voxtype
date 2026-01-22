//! CLI-based speech-to-text transcription
//!
//! Uses whisper-cli (from whisper.cpp) as an external process for transcription.
//! This is a fallback for systems where the whisper-rs FFI bindings don't work
//! (e.g., Ubuntu 25.10 with glibc 2.42+).
//!
//! The whisper-cli binary must be installed separately or built from whisper.cpp.

use super::Transcriber;
use crate::config::{Config, WhisperConfig};
use crate::error::TranscribeError;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// CLI-based transcriber using whisper-cli subprocess
pub struct CliTranscriber {
    /// Path to whisper-cli binary
    cli_path: PathBuf,
    /// Path to model file
    model_path: PathBuf,
    /// Language for transcription
    language: String,
    /// Whether to translate to English
    translate: bool,
    /// Number of threads to use
    threads: usize,
    /// Initial prompt for context
    initial_prompt: Option<String>,
}

/// JSON output structure from whisper-cli
#[derive(Debug, Deserialize)]
struct WhisperCliOutput {
    transcription: Vec<Segment>,
}

#[derive(Debug, Deserialize)]
struct Segment {
    text: String,
}

impl CliTranscriber {
    /// Create a new CLI-based transcriber
    pub fn new(config: &WhisperConfig) -> Result<Self, TranscribeError> {
        let cli_path = resolve_cli_path(config.whisper_cli_path.as_deref())?;
        let model_path = resolve_model_path(&config.model)?;

        tracing::info!(
            "Using whisper-cli backend: {:?} with model {:?}",
            cli_path,
            model_path
        );

        // Verify cli exists and is executable
        if !cli_path.exists() {
            return Err(TranscribeError::InitFailed(format!(
                "whisper-cli not found at {:?}",
                cli_path
            )));
        }

        // threads = 0 or None means auto-detect, use a sensible default
        let threads = match config.threads {
            Some(0) | None => num_cpus::get().min(4),
            Some(n) => n,
        };

        // Get language - use primary language from config
        let language = config.language.primary().to_string();

        Ok(Self {
            cli_path,
            model_path,
            language,
            translate: config.translate,
            threads,
            initial_prompt: config.initial_prompt.clone(),
        })
    }

    /// Write audio samples to a temporary WAV file
    fn write_temp_wav(&self, samples: &[f32]) -> Result<tempfile::NamedTempFile, TranscribeError> {
        let temp_file = tempfile::Builder::new()
            .prefix("voxtype_")
            .suffix(".wav")
            .tempfile()
            .map_err(|e| {
                TranscribeError::AudioFormat(format!("Failed to create temp file: {}", e))
            })?;

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = hound::WavWriter::create(temp_file.path(), spec).map_err(|e| {
            TranscribeError::AudioFormat(format!("Failed to create WAV writer: {}", e))
        })?;

        for &sample in samples {
            // Convert f32 [-1.0, 1.0] to i16
            let clamped = sample.clamp(-1.0, 1.0);
            let scaled = (clamped * 32767.0) as i16;
            writer.write_sample(scaled).map_err(|e| {
                TranscribeError::AudioFormat(format!("Failed to write sample: {}", e))
            })?;
        }

        writer
            .finalize()
            .map_err(|e| TranscribeError::AudioFormat(format!("Failed to finalize WAV: {}", e)))?;

        Ok(temp_file)
    }
}

impl Transcriber for CliTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) via whisper-cli",
            duration_secs,
            samples.len()
        );

        let start = std::time::Instant::now();

        // Write audio to temp WAV file
        let temp_wav = self.write_temp_wav(samples)?;

        // Create temp file for JSON output
        let temp_json = tempfile::Builder::new()
            .prefix("voxtype_out_")
            .suffix("") // whisper-cli adds .json
            .tempfile()
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to create temp file: {}", e))
            })?;

        let output_base = temp_json
            .path()
            .to_str()
            .ok_or_else(|| TranscribeError::InferenceFailed("Invalid temp path".to_string()))?;

        // Build command
        let mut cmd = Command::new(&self.cli_path);
        cmd.arg("--model")
            .arg(&self.model_path)
            .arg("--file")
            .arg(temp_wav.path())
            .arg("--output-json")
            .arg("--output-file")
            .arg(output_base)
            .arg("--threads")
            .arg(self.threads.to_string())
            .arg("--no-prints"); // Suppress progress output

        // Set language (skip if auto-detect)
        if self.language != "auto" {
            cmd.arg("--language").arg(&self.language);
        }

        // Translation
        if self.translate {
            cmd.arg("--translate");
        }

        // Initial prompt
        if let Some(prompt) = &self.initial_prompt {
            cmd.arg("--prompt").arg(prompt);
        }

        tracing::debug!("Running whisper-cli: {:?}", cmd);

        // Run whisper-cli
        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to run whisper-cli: {}", e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TranscribeError::InferenceFailed(format!(
                "whisper-cli failed: {}",
                stderr
            )));
        }

        // Read JSON output
        let json_path = format!("{}.json", output_base);
        let json_content = std::fs::read_to_string(&json_path).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to read output: {}", e))
        })?;

        // Clean up JSON file
        let _ = std::fs::remove_file(&json_path);

        // Parse JSON
        let result: WhisperCliOutput = serde_json::from_str(&json_content).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to parse JSON output: {}", e))
        })?;

        // Combine all segments
        let text: String = result
            .transcription
            .iter()
            .map(|s| s.text.trim())
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();

        tracing::info!(
            "Transcription completed in {:.2}s: {:?}",
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

/// Resolve whisper-cli path
fn resolve_cli_path(configured_path: Option<&str>) -> Result<PathBuf, TranscribeError> {
    // If explicitly configured, use that
    if let Some(path) = configured_path {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
        return Err(TranscribeError::InitFailed(format!(
            "Configured whisper-cli path not found: {}",
            path
        )));
    }

    // Check common locations
    let candidates = [
        // In PATH
        which::which("whisper-cli").ok(),
        which::which("whisper").ok(),
        // Local builds
        Some(PathBuf::from("./whisper-cli")),
        Some(PathBuf::from("./build/bin/whisper-cli")),
        // System locations
        Some(PathBuf::from("/usr/local/bin/whisper-cli")),
        Some(PathBuf::from("/usr/bin/whisper-cli")),
        // Home directory
        directories::BaseDirs::new().map(|d| d.home_dir().join(".local/bin/whisper-cli")),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(TranscribeError::InitFailed(
        "whisper-cli not found. Install from https://github.com/ggerganov/whisper.cpp or set whisper_cli_path in config.".to_string()
    ))
}

/// Resolve model name to file path
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    // If it's already an absolute path, use it directly
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    // Map model names to file names
    let model_filename = match model {
        "tiny" => "ggml-tiny.bin",
        "tiny.en" => "ggml-tiny.en.bin",
        "base" => "ggml-base.bin",
        "base.en" => "ggml-base.en.bin",
        "small" => "ggml-small.bin",
        "small.en" => "ggml-small.en.bin",
        "medium" => "ggml-medium.bin",
        "medium.en" => "ggml-medium.en.bin",
        "large" | "large-v1" => "ggml-large-v1.bin",
        "large-v2" => "ggml-large-v2.bin",
        "large-v3" => "ggml-large-v3.bin",
        "large-v3-turbo" => "ggml-large-v3-turbo.bin",
        other if other.ends_with(".bin") => other,
        other => {
            return Err(TranscribeError::ModelNotFound(format!(
                "Unknown model: '{}'. Valid models: tiny, base, small, medium, large-v3, large-v3-turbo",
                other
            )));
        }
    };

    // Look in the data directory
    let models_dir = Config::models_dir();
    let model_path = models_dir.join(model_filename);

    if model_path.exists() {
        return Ok(model_path);
    }

    // Also check current directory
    let cwd_path = PathBuf::from(model_filename);
    if cwd_path.exists() {
        return Ok(cwd_path);
    }

    // Also check ./models/
    let local_models_path = PathBuf::from("models").join(model_filename);
    if local_models_path.exists() {
        return Ok(local_models_path);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Model '{}' not found. Looked in:\n  - {}\n  - {}\n  - {}",
        model,
        model_path.display(),
        cwd_path.display(),
        local_models_path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_cli_not_found() {
        // Should fail gracefully when whisper-cli is not installed
        let result = resolve_cli_path(Some("/nonexistent/whisper-cli"));
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_model_path_unknown() {
        let result = resolve_model_path("nonexistent-model");
        assert!(result.is_err());
    }
}
