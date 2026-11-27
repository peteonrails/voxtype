//! CLI-based speech-to-text transcription
//!
//! Uses whisper-cli (from whisper.cpp) as an external process for transcription.
//! This is a fallback for systems where the whisper-rs FFI bindings don't work
//! (e.g., Ubuntu 25.10 with glibc 2.42+).
//!
//! The whisper-cli binary must be installed separately or built from whisper.cpp.

use super::whisper::{timed_segments, RawSegment};
use super::{TimedSegment, Transcriber};
use crate::config::{Config, WhisperConfig};
use crate::error::TranscribeError;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

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
    /// Vocabulary selected for the current recording.
    active_vocabulary: Mutex<Vec<String>>,
}

/// JSON output structure from whisper-cli
#[derive(Debug, Deserialize)]
struct WhisperCliOutput {
    transcription: Vec<Segment>,
}

#[derive(Debug, Deserialize)]
struct Segment {
    text: String,
    /// Start and end in milliseconds. Every whisper-cli that writes JSON
    /// has them; absent, the segment is placed over the whole clip.
    #[serde(default)]
    offsets: Option<Offsets>,
}

#[derive(Debug, Deserialize)]
struct Offsets {
    from: i64,
    to: i64,
}

impl WhisperCliOutput {
    /// Every segment's text, joined: what dictation types.
    fn text(&self) -> String {
        self.transcription
            .iter()
            .map(|s| s.text.trim())
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string()
    }

    /// The segments with their times, as whisper.cpp in-process gives
    /// them: trimmed, clamped to the clip, wordless ones dropped.
    fn timed(&self, duration_secs: f32) -> Vec<TimedSegment> {
        let whole_clip = (duration_secs * 100.0).round() as i64;
        let raw: Vec<RawSegment> = self
            .transcription
            .iter()
            .map(|s| {
                let (from, to) = s
                    .offsets
                    .as_ref()
                    .map_or((0, whole_clip * 10), |o| (o.from, o.to));
                RawSegment {
                    start_cs: from / 10,
                    end_cs: (to.max(0) + 9) / 10,
                    text: s.text.clone(),
                }
            })
            .collect();
        timed_segments(&raw, duration_secs)
    }
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
                "whisper-cli not found at {:?}. {}",
                cli_path, MISSING_CLI_FIX
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
            active_vocabulary: Mutex::new(Vec::new()),
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
    fn set_vocabulary(&self, terms: &[String]) {
        if let Ok(mut active) = self.active_vocabulary.lock() {
            active.clear();
            active.extend_from_slice(terms);
        }
    }

    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let start = std::time::Instant::now();
        let text = self.run(samples)?.text();
        super::log_transcription("Transcription", start, &text);
        Ok(text)
    }

    /// whisper-cli's JSON already times each segment, so a meeting chunk
    /// gets a line per segment from the same single run per chunk.
    fn transcribe_timed(&self, samples: &[f32]) -> Result<Vec<TimedSegment>, TranscribeError> {
        let duration_secs = samples.len() as f32 / 16000.0;
        Ok(self.run(samples)?.timed(duration_secs))
    }
}

impl CliTranscriber {
    /// Run whisper-cli once over `samples` and read its JSON.
    fn run(&self, samples: &[f32]) -> Result<WhisperCliOutput, TranscribeError> {
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

        // Preserve the configured prompt and append the recording's terms.
        let vocabulary = self
            .active_vocabulary
            .lock()
            .map(|terms| terms.clone())
            .unwrap_or_default();
        if let Some(prompt) =
            super::prompt_with_vocabulary(self.initial_prompt.as_deref(), &vocabulary)
        {
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

        serde_json::from_str(&json_content).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to parse JSON output: {}", e))
        })
    }
}

/// The two ways out of `[whisper] mode = "cli"` without `whisper-cli`,
/// shown wherever that is reported: the daemon's error, `voxtype doctor`,
/// `voxtype config set` and `voxtype configure`. Omarchy #18694 reached the
/// cli mode by accident, and the old message only offered the install.
pub const MISSING_CLI_FIX: &str = "Install whisper.cpp's whisper-cli (the whisper-cpp \
     package on Arch and Omarchy), or transcribe in-process instead: \
     voxtype config set whisper.mode local";

/// Find the whisper-cli binary: `configured` when set (and only then), else
/// PATH and the usual install locations. `None` when there is none.
pub fn find_whisper_cli(configured: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = configured {
        let p = PathBuf::from(path);
        return p.exists().then_some(p);
    }

    cli_candidates(
        |n| which::which(n).ok(),
        directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()),
    )
    .into_iter()
    .find(|c| c.exists())
}

/// What is wrong with `config` running in cli mode on this machine, or
/// `None` when it is not in cli mode or whisper-cli is there. The fix is
/// [`MISSING_CLI_FIX`].
pub fn cli_mode_problem(config: &WhisperConfig) -> Option<String> {
    if config.effective_mode() != crate::config::WhisperMode::Cli {
        return None;
    }
    let configured = config.whisper_cli_path.as_deref();
    if find_whisper_cli(configured).is_some() {
        return None;
    }
    Some(match configured {
        Some(path) => {
            format!("[whisper] mode = \"cli\" but whisper_cli_path ({path}) does not exist")
        }
        None => "[whisper] mode = \"cli\" runs whisper-cli for every transcription, \
                 and whisper-cli is not installed or not on PATH"
            .to_string(),
    })
}

/// Resolve whisper-cli path
fn resolve_cli_path(configured_path: Option<&str>) -> Result<PathBuf, TranscribeError> {
    if let Some(path) = find_whisper_cli(configured_path) {
        return Ok(path);
    }
    Err(TranscribeError::InitFailed(match configured_path {
        Some(path) => format!(
            "Configured whisper_cli_path not found: {path}. Fix the path, or: {MISSING_CLI_FIX}"
        ),
        None => format!("whisper-cli not found. {MISSING_CLI_FIX}"),
    }))
}

/// Where whisper-cli may be, in search order. Only the name `whisper-cli`
/// is accepted: a bare `whisper` on PATH is OpenAI's Python CLI, which
/// takes different arguments and would fail every transcription, or worse,
/// run a second, slow model. whisper.cpp itself has shipped the binary as
/// `whisper-cli` since it retired `main`, never as `whisper`, so no legacy
/// name needs probing.
fn cli_candidates(which: impl Fn(&str) -> Option<PathBuf>, home: Option<PathBuf>) -> Vec<PathBuf> {
    [
        // In PATH
        which("whisper-cli"),
        // Local builds
        Some(PathBuf::from("./whisper-cli")),
        Some(PathBuf::from("./build/bin/whisper-cli")),
        // System locations
        Some(PathBuf::from("/usr/local/bin/whisper-cli")),
        Some(PathBuf::from("/usr/bin/whisper-cli")),
        // Home directory
        home.map(|h| h.join(".local/bin/whisper-cli")),
    ]
    .into_iter()
    .flatten()
    .collect()
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

    /// A bare `whisper` on PATH is OpenAI's Python CLI, not whisper.cpp.
    #[test]
    fn a_bare_whisper_on_path_is_never_a_candidate() {
        let python_whisper = PathBuf::from("/usr/bin/whisper");
        let which = |name: &str| (name == "whisper").then(|| python_whisper.clone());
        let found = cli_candidates(which, Some(PathBuf::from("/home/u")));
        assert!(!found.contains(&python_whisper), "{found:?}");
        assert!(found
            .iter()
            .all(|p| p.file_name().and_then(|n| n.to_str()) == Some("whisper-cli")));
    }

    #[test]
    fn whisper_cli_on_path_is_searched_first() {
        let on_path = PathBuf::from("/opt/wcpp/bin/whisper-cli");
        let which = |name: &str| (name == "whisper-cli").then(|| on_path.clone());
        let found = cli_candidates(which, None);
        assert_eq!(found.first(), Some(&on_path));
        assert!(found.contains(&PathBuf::from("/usr/bin/whisper-cli")));
    }

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

    /// whisper-cli's `--output-json` for a 10 s clip: two sentences with a
    /// pause, a wordless segment, and one running past the end.
    const JSON: &str = r#"{"systeminfo": "AVX = 1", "transcription": [
        {"timestamps": {"from": "00:00:00,000", "to": "00:00:02,400"},
         "offsets": {"from": 0, "to": 2400}, "text": " Hello there."},
        {"timestamps": {"from": "00:00:02,400", "to": "00:00:07,815"},
         "offsets": {"from": 2400, "to": 7815}, "text": " See you at three."},
        {"offsets": {"from": 7815, "to": 8000}, "text": " ..."},
        {"offsets": {"from": 8000, "to": 12000}, "text": " Bye."}
    ]}"#;

    fn spans(segments: &[TimedSegment]) -> Vec<(&str, f32, f32)> {
        segments
            .iter()
            .map(|s| (s.text.as_str(), s.start_secs, s.end_secs))
            .collect()
    }

    #[test]
    fn json_segments_keep_their_times() {
        let out: WhisperCliOutput = serde_json::from_str(JSON).unwrap();
        assert_eq!(out.text(), "Hello there. See you at three. ... Bye.");
        assert_eq!(
            spans(&out.timed(10.0)),
            vec![
                ("Hello there.", 0.0, 2.4),
                ("See you at three.", 2.4, 7.82),
                ("Bye.", 8.0, 10.0),
            ],
            "times in seconds, the wordless segment dropped, the last clamped"
        );
        // A whisper-cli without offsets: one segment over the whole clip
        // each, as the default transcribe_timed would give.
        let bare: WhisperCliOutput =
            serde_json::from_str(r#"{"transcription": [{"text": " Hi."}]}"#).unwrap();
        assert_eq!(spans(&bare.timed(3.0)), vec![("Hi.", 0.0, 3.0)]);
    }

    fn cli_config(path: Option<&str>) -> WhisperConfig {
        WhisperConfig {
            mode: Some(crate::config::WhisperMode::Cli),
            whisper_cli_path: path.map(str::to_string),
            ..WhisperConfig::default()
        }
    }

    /// Omarchy #18694: the error has to offer switching back to local, not
    /// only installing whisper-cli.
    #[test]
    fn a_missing_configured_whisper_cli_names_both_fixes() {
        let err = resolve_cli_path(Some("/nonexistent/voxtype-test/whisper-cli"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("whisper-cpp"), "{err}");
        assert!(
            err.contains("voxtype config set whisper.mode local"),
            "{err}"
        );

        let problem = cli_mode_problem(&cli_config(Some("/nonexistent/voxtype-test/whisper-cli")))
            .expect("missing binary in cli mode is a problem");
        assert!(
            problem.contains("/nonexistent/voxtype-test/whisper-cli"),
            "{problem}"
        );
    }

    #[test]
    fn cli_mode_with_whisper_cli_present_or_other_modes_are_fine() {
        let dir = tempfile::TempDir::new().unwrap();
        let cli = dir.path().join("whisper-cli");
        std::fs::write(&cli, "").unwrap();
        assert_eq!(cli_mode_problem(&cli_config(cli.to_str())), None);

        for mode in [
            crate::config::WhisperMode::Local,
            crate::config::WhisperMode::Remote,
        ] {
            let config = WhisperConfig {
                mode: Some(mode),
                whisper_cli_path: Some("/nonexistent/voxtype-test/whisper-cli".into()),
                ..WhisperConfig::default()
            };
            assert_eq!(cli_mode_problem(&config), None, "{mode:?}");
        }
    }

    /// A fake whisper-cli writes the JSON where it is told to; the meeting
    /// path gets the segments, dictation the joined text, from one run each.
    #[test]
    fn transcribe_timed_runs_whisper_cli_once_and_reads_its_segments() {
        let dir = tempfile::TempDir::new().unwrap();
        let runs = dir.path().join("runs");
        let json = dir.path().join("out.json");
        std::fs::write(&json, JSON).unwrap();
        let cli = dir.path().join("whisper-cli");
        crate::test_support::write_script(
            &cli,
            &format!(
                "#!/bin/sh\necho run >> {runs}\nwhile [ $# -gt 0 ]; do \
                 [ \"$1\" = --output-file ] && cp {json} \"$2.json\"; shift; done\n",
                runs = runs.display(),
                json = json.display()
            ),
        );
        let t = CliTranscriber {
            cli_path: cli,
            model_path: dir.path().join("model.bin"),
            language: "en".into(),
            translate: false,
            threads: 1,
            initial_prompt: None,
            active_vocabulary: Mutex::new(Vec::new()),
        };
        let audio = vec![0.0f32; 16000 * 10];
        assert_eq!(t.transcribe_timed(&audio).unwrap().len(), 3);
        assert_eq!(
            t.transcribe(&audio).unwrap(),
            "Hello there. See you at three. ... Bye."
        );
        assert_eq!(std::fs::read_to_string(&runs).unwrap().lines().count(), 2);
        assert!(t.transcribe_timed(&[]).is_err());
    }
}
