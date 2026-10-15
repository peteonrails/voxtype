//! Whistle speech-to-text transcription via Cactus Compute's `needle` runner.
//!
//! Whistle is a 16.9 MB on-device ASR model in seven languages. It shares the
//! `.cact` container and C++ engine with Needle; the published engine folder
//! ships a small CLI runner that transcribes a WAV and prints JSON:
//!
//!   needle --model whistle.cact --audio clip.wav
//!   => {"text":"...","language":"en","ttft_ms":..,"decode_tps":..}
//!
//! Word timestamps are added with `--audio-word-timestamps`. The runner and the
//! weights are downloaded separately; see [`crate::config::WhistleConfig`].

use super::Transcriber;
use crate::config::WhistleConfig;
use crate::error::TranscribeError;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Whistle transcriber driving the `needle` runner as a subprocess.
pub struct WhistleTranscriber {
    /// Path to the needle runner binary
    runner_path: PathBuf,
    /// Path to the whistle.cact weights
    model_path: PathBuf,
    /// Forced language, or empty to detect
    language: String,
    /// Keyword-bias file
    keywords_file: Option<PathBuf>,
    /// Ask the runner for per-word timestamps
    word_timestamps: bool,
    /// Threads passed to the runner (0 = let it choose)
    threads: usize,
    /// Language reported by the most recent transcription
    last_language: Mutex<Option<String>>,
}

/// JSON output from the needle runner's `--audio` path.
#[derive(Debug, Deserialize)]
struct WhistleOutput {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    words: Vec<WhistleWord>,
}

#[derive(Debug, Deserialize)]
struct WhistleWord {
    word: String,
    #[serde(default)]
    start: f32,
    #[serde(default)]
    end: f32,
}

impl WhistleTranscriber {
    /// Create a new Whistle transcriber
    pub fn new(config: &WhistleConfig) -> Result<Self, TranscribeError> {
        let runner_path = resolve_runner_path(config.runner_path.as_deref())?;
        let model_path = resolve_model_path(&config.model)?;

        tracing::info!(
            "Using Whistle backend: {:?} with model {:?}",
            runner_path,
            model_path
        );

        if !runner_path.exists() {
            return Err(TranscribeError::InitFailed(format!(
                "needle runner not found at {:?}",
                runner_path
            )));
        }
        if !model_path.exists() {
            return Err(TranscribeError::ModelNotFound(format!(
                "whistle.cact not found at {:?}",
                model_path
            )));
        }

        let threads = match config.threads {
            Some(n) if n > 0 => n,
            _ => 0,
        };

        Ok(Self {
            runner_path,
            model_path,
            language: config.language.clone(),
            keywords_file: config.keywords_file.clone(),
            word_timestamps: config.word_timestamps,
            threads,
            last_language: Mutex::new(None),
        })
    }

    /// Write audio samples to a temporary 16 kHz mono WAV file
    fn write_temp_wav(&self, samples: &[f32]) -> Result<tempfile::NamedTempFile, TranscribeError> {
        let temp_file = tempfile::Builder::new()
            .prefix("voxtype_whistle_")
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

    /// Run the runner on a clip and parse its JSON output.
    fn run(&self, temp_wav: &tempfile::NamedTempFile) -> Result<WhistleOutput, TranscribeError> {
        let mut cmd = Command::new(&self.runner_path);
        cmd.arg("--model")
            .arg(&self.model_path)
            .arg("--audio")
            .arg(temp_wav.path());

        if !self.language.is_empty() && self.language != "auto" {
            cmd.arg("--audio-language").arg(&self.language);
        }
        if let Some(keywords) = &self.keywords_file {
            cmd.arg("--audio-keywords").arg(keywords);
        }
        if self.word_timestamps {
            cmd.arg("--audio-word-timestamps");
        }
        if self.threads > 0 {
            cmd.arg("--threads").arg(self.threads.to_string());
        }

        // Whistle's runner has telemetry on by default; keep it local.
        cmd.env("NEEDLE_TELEMETRY", "0").env("DO_NOT_TRACK", "1");

        tracing::debug!("Running needle: {:?}", cmd);

        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to run needle: {}", e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TranscribeError::InferenceFailed(format!(
                "needle failed: {}",
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_output(&stdout)
    }
}

/// Parse the runner's stdout, which is one JSON object (possibly with other
/// lines around it). Scans for the first line that parses as JSON.
fn parse_output(stdout: &str) -> Result<WhistleOutput, TranscribeError> {
    let trimmed = stdout.trim();
    if let Ok(parsed) = serde_json::from_str::<WhistleOutput>(trimmed) {
        return Ok(parsed);
    }
    for line in stdout.lines().rev() {
        let line = line.trim();
        if line.starts_with('{') {
            if let Ok(parsed) = serde_json::from_str::<WhistleOutput>(line) {
                return Ok(parsed);
            }
        }
    }
    Err(TranscribeError::InferenceFailed(format!(
        "could not parse needle JSON output: {:?}",
        trimmed
    )))
}

impl Transcriber for WhistleTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        tracing::debug!(
            "Transcribing {:.2}s of audio ({} samples) via Whistle",
            duration_secs,
            samples.len()
        );

        let start = std::time::Instant::now();
        let temp_wav = self.write_temp_wav(samples)?;
        let result = self.run(&temp_wav)?;

        if let Some(lang) = result.language.filter(|l| !l.is_empty()) {
            if let Ok(mut guard) = self.last_language.lock() {
                *guard = Some(lang);
            }
        }

        let text = result.text.trim().to_string();
        tracing::info!(
            "Whistle transcription completed in {:.2}s: {:?}",
            start.elapsed().as_secs_f32(),
            if text.chars().count() > 50 {
                format!("{}...", text.chars().take(50).collect::<String>())
            } else {
                text.clone()
            }
        );
        Ok(text)
    }

    fn transcribe_timed(
        &self,
        samples: &[f32],
    ) -> Result<Vec<super::TimedSegment>, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        // Whistle only returns per-word times when asked, so run with the flag
        // regardless of the configured default.
        let temp_wav = self.write_temp_wav(samples)?;
        let mut cmd = Command::new(&self.runner_path);
        cmd.arg("--model")
            .arg(&self.model_path)
            .arg("--audio")
            .arg(temp_wav.path())
            .arg("--audio-word-timestamps");
        if !self.language.is_empty() && self.language != "auto" {
            cmd.arg("--audio-language").arg(&self.language);
        }
        if let Some(keywords) = &self.keywords_file {
            cmd.arg("--audio-keywords").arg(keywords);
        }
        if self.threads > 0 {
            cmd.arg("--threads").arg(self.threads.to_string());
        }
        cmd.env("NEEDLE_TELEMETRY", "0").env("DO_NOT_TRACK", "1");

        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to run needle: {}", e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TranscribeError::InferenceFailed(format!(
                "needle failed: {}",
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed = parse_output(&stdout)?;
        Ok(parsed
            .words
            .into_iter()
            .map(|w| super::TimedSegment {
                text: w.word,
                start_secs: w.start,
                end_secs: w.end,
            })
            .collect())
    }

    fn last_detected_language(&self) -> Option<String> {
        self.last_language.lock().ok().and_then(|g| g.clone())
    }
}

/// Resolve the needle runner path
fn resolve_runner_path(
    configured_path: Option<&std::path::Path>,
) -> Result<PathBuf, TranscribeError> {
    if let Some(path) = configured_path {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
        return Err(TranscribeError::InitFailed(format!(
            "Configured needle runner path not found: {}",
            p.display()
        )));
    }

    let candidates = [
        which::which("needle").ok(),
        Some(PathBuf::from("./needle")),
        Some(PathBuf::from("/usr/local/bin/needle")),
        Some(PathBuf::from("/usr/bin/needle")),
        directories::BaseDirs::new().map(|d| d.home_dir().join(".local/bin/needle")),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(TranscribeError::InitFailed(
        "needle runner not found. Download it from https://huggingface.co/Cactus-Compute/needle3 \
         or set runner_path in the [whistle] config section."
            .to_string(),
    ))
}

/// Resolve the whistle.cact model path
fn resolve_model_path(model: &str) -> Result<PathBuf, TranscribeError> {
    let path = PathBuf::from(model);
    if path.is_absolute() && path.exists() {
        return Ok(path);
    }

    let filename = if model.ends_with(".cact") {
        model.to_string()
    } else {
        format!("{}.cact", model)
    };

    let models_dir = Config::models_dir();
    let model_path = models_dir.join(&filename);
    if model_path.exists() {
        return Ok(model_path);
    }

    let cwd_path = PathBuf::from(&filename);
    if cwd_path.exists() {
        return Ok(cwd_path);
    }

    Err(TranscribeError::ModelNotFound(format!(
        "Whistle model '{}' not found. Looked in:\n  - {}\n  - {}\n\
         Download whistle.cact from https://huggingface.co/Cactus-Compute/whistle",
        model,
        model_path.display(),
        cwd_path.display()
    )))
}

use crate::config::Config;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_output_reads_text_and_language() {
        let out = r#"{"text":"hello world","language":"en","ttft_ms":12.0,"decode_tps":120.0}"#;
        let parsed = parse_output(out).unwrap();
        assert_eq!(parsed.text, "hello world");
        assert_eq!(parsed.language.as_deref(), Some("en"));
        assert!(parsed.words.is_empty());
    }

    #[test]
    fn parse_output_reads_words() {
        let out = r#"{"text":"hi","language":"en","words":[{"word":"hi","start":0.0,"end":0.4,"probability":0.9}],"ttft_ms":1.0,"decode_tps":2.0}"#;
        let parsed = parse_output(out).unwrap();
        assert_eq!(parsed.words.len(), 1);
        assert_eq!(parsed.words[0].word, "hi");
        assert_eq!(parsed.words[0].end, 0.4);
    }

    #[test]
    fn parse_output_tolerates_surrounding_lines() {
        let out = "starting\n{\"text\":\"ok\",\"language\":\"de\"}\n";
        let parsed = parse_output(out).unwrap();
        assert_eq!(parsed.text, "ok");
        assert_eq!(parsed.language.as_deref(), Some("de"));
    }

    #[test]
    fn parse_output_rejects_non_json() {
        assert!(parse_output("not json").is_err());
    }

    #[test]
    fn resolve_missing_runner_errors() {
        let missing: Option<&std::path::Path> = Some(std::path::Path::new("/nonexistent/needle"));
        assert!(resolve_runner_path(missing).is_err());
    }
}
