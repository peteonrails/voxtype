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
use crate::config::{Config, WhistleConfig};
use crate::error::TranscribeError;
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default timeout for the needle runner process (120 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

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
    /// Bounded execution timeout for needle
    timeout: Duration,
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

        let keywords_file = config.keywords_file.as_ref().map(|p| expand_tilde(p));

        Ok(Self {
            runner_path,
            model_path,
            language: config.language.clone(),
            keywords_file,
            word_timestamps: config.word_timestamps,
            threads,
            timeout: DEFAULT_TIMEOUT,
            last_language: Mutex::new(None),
        })
    }

    /// Build the Command to run needle on an audio file.
    pub(crate) fn build_command(&self, audio_path: &Path, word_timestamps: bool) -> Command {
        let mut cmd = Command::new(&self.runner_path);
        cmd.arg("--model")
            .arg(&self.model_path)
            .arg("--audio")
            .arg(audio_path);

        if !self.language.is_empty() && self.language != "auto" {
            cmd.arg("--audio-language").arg(&self.language);
        }
        if let Some(keywords) = &self.keywords_file {
            cmd.arg("--audio-keywords").arg(keywords);
        }
        if word_timestamps {
            cmd.arg("--audio-word-timestamps");
        }
        if self.threads > 0 {
            cmd.arg("--threads").arg(self.threads.to_string());
        }

        // Whistle's runner has telemetry on by default; keep it local.
        cmd.env("NEEDLE_TELEMETRY", "0").env("DO_NOT_TRACK", "1");
        cmd
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

    /// Run the needle runner on a clip and parse its JSON output with bounded timeout.
    fn run_needle(
        &self,
        temp_wav: &tempfile::NamedTempFile,
        word_timestamps: bool,
    ) -> Result<WhistleOutput, TranscribeError> {
        let mut cmd = self.build_command(temp_wav.path(), word_timestamps);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        tracing::debug!("Running needle: {:?}", cmd);

        let mut child = cmd.spawn().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to spawn needle: {}", e))
        })?;

        let deadline = Instant::now() + self.timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut stdout = Vec::new();
                    let mut stderr = Vec::new();
                    if let Some(mut out) = child.stdout.take() {
                        let _ = out.read_to_end(&mut stdout);
                    }
                    if let Some(mut err) = child.stderr.take() {
                        let _ = err.read_to_end(&mut stderr);
                    }

                    if !status.success() {
                        let stderr_str = String::from_utf8_lossy(&stderr);
                        return Err(TranscribeError::InferenceFailed(format!(
                            "needle failed: {}",
                            stderr_str.trim()
                        )));
                    }

                    let stdout_str = String::from_utf8_lossy(&stdout);
                    return parse_output(&stdout_str);
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(TranscribeError::InferenceFailed(format!(
                            "needle timed out after {}s. Check that the model file is valid \
                             and the runner is not stuck. See docs/WHISTLE.md troubleshooting.",
                            self.timeout.as_secs()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(TranscribeError::InferenceFailed(format!(
                        "Failed to wait for needle: {}",
                        e
                    )));
                }
            }
        }
    }
}

/// Expand leading `~` to the user's home directory.
fn expand_tilde(path: &Path) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    path.to_path_buf()
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

        let start = Instant::now();
        let temp_wav = self.write_temp_wav(samples)?;
        let result = self.run_needle(&temp_wav, self.word_timestamps)?;

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
        let result = self.run_needle(&temp_wav, true)?;

        if let Some(lang) = result.language.filter(|l| !l.is_empty()) {
            if let Ok(mut guard) = self.last_language.lock() {
                *guard = Some(lang);
            }
        }

        Ok(result
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
        let p = expand_tilde(path);
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
    let path = expand_tilde(&path);
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

    Err(TranscribeError::ModelNotFound(format!(
        "Whistle model '{}' not found. Looked in:\n  - {}\n\
         Download whistle.cact from https://huggingface.co/Cactus-Compute/whistle",
        model,
        model_path.display()
    )))
}

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

    #[test]
    fn resolve_missing_model_error_message() {
        let err = resolve_model_path("nonexistent_model").unwrap_err();
        let msg = err.to_string();
        // Should not check or report current directory
        assert!(!msg.contains("./nonexistent_model"));
    }

    #[test]
    fn build_command_defaults() {
        let transcriber = WhistleTranscriber {
            runner_path: PathBuf::from("/usr/bin/needle"),
            model_path: PathBuf::from("/models/whistle.cact"),
            language: String::new(),
            keywords_file: None,
            word_timestamps: false,
            threads: 0,
            timeout: DEFAULT_TIMEOUT,
            last_language: Mutex::new(None),
        };

        let cmd = transcriber.build_command(Path::new("/tmp/clip.wav"), false);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args,
            vec![
                "--model", "/models/whistle.cact",
                "--audio", "/tmp/clip.wav",
            ]
        );

        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|s| s.to_string_lossy().into_owned())))
            .collect();
        assert!(envs.contains(&("NEEDLE_TELEMETRY".into(), Some("0".into()))));
        assert!(envs.contains(&("DO_NOT_TRACK".into(), Some("1".into()))));
    }

    #[test]
    fn build_command_all_options() {
        let transcriber = WhistleTranscriber {
            runner_path: PathBuf::from("/usr/bin/needle"),
            model_path: PathBuf::from("/models/whistle.cact"),
            language: "de".to_string(),
            keywords_file: Some(PathBuf::from("/etc/keywords.txt")),
            word_timestamps: true,
            threads: 4,
            timeout: DEFAULT_TIMEOUT,
            last_language: Mutex::new(None),
        };

        let cmd = transcriber.build_command(Path::new("/tmp/clip.wav"), true);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args,
            vec![
                "--model", "/models/whistle.cact",
                "--audio", "/tmp/clip.wav",
                "--audio-language", "de",
                "--audio-keywords", "/etc/keywords.txt",
                "--audio-word-timestamps",
                "--threads", "4",
            ]
        );
    }

    #[test]
    fn build_command_auto_language_skipped() {
        let transcriber = WhistleTranscriber {
            runner_path: PathBuf::from("/usr/bin/needle"),
            model_path: PathBuf::from("/models/whistle.cact"),
            language: "auto".to_string(),
            keywords_file: None,
            word_timestamps: false,
            threads: 0,
            timeout: DEFAULT_TIMEOUT,
            last_language: Mutex::new(None),
        };

        let cmd = transcriber.build_command(Path::new("/tmp/clip.wav"), false);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(!args.contains(&"--audio-language".to_string()));
    }

    #[test]
    fn tilde_expansion_works() {
        let expanded = expand_tilde(Path::new("~/test/path.txt"));
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expanded, home.join("test/path.txt"));
        }
        let unexpanded = expand_tilde(Path::new("/var/log/test.txt"));
        assert_eq!(unexpanded, PathBuf::from("/var/log/test.txt"));
    }
}
