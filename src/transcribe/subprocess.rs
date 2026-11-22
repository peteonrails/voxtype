//! Subprocess-based transcription for GPU isolation
//!
//! This module provides a transcriber that spawns a subprocess for each
//! transcription. When the subprocess exits, all GPU resources are fully
//! released. This solves the problem of GPU memory staying allocated
//! between transcriptions when using ggml-vulkan.
//!
//! Key benefits:
//! - GPU memory fully released after each transcription
//! - No GPU power draw between transcriptions (important for laptops)
//! - Clean separation of concerns
//!
//! Eager spawning:
//! - `prepare()` spawns the worker when recording STARTS
//! - Worker loads model while user is speaking
//! - `transcribe()` sends audio to already-ready worker
//! - Perceived latency is just transcription time, not model load + transcription

use super::worker::{READY_SIGNAL, TIMED_FLAG};
use super::{TimedSegment, Transcriber};
use crate::config::WhisperConfig;
use crate::error::TranscribeError;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use ureq::serde_json;

/// Response from the transcription worker process
#[derive(Debug, serde::Deserialize)]
struct WorkerResponse {
    ok: bool,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    error: Option<String>,
    /// Two-letter language code chosen for this transcription, if the worker
    /// tracked it. Used by output methods that benefit from a layout hint.
    /// Older workers that do not emit this field deserialize to `None`.
    #[serde(default)]
    language: Option<String>,
    /// Timed segments, when they were asked for
    #[serde(default)]
    segments: Option<Vec<TimedSegment>>,
    #[serde(default)]
    metadata: super::RuntimeMetadata,
}

/// A prepared worker process ready to receive audio
struct PreparedWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// Subprocess-based transcriber for GPU isolation
///
/// Spawns a fresh `voxtype transcribe-worker` process for each transcription.
/// The worker loads the model, transcribes, returns the result, and exits.
/// This ensures all GPU resources are released after transcription.
///
/// With eager spawning (`prepare()` called when recording starts), the worker
/// loads the model while the user is speaking, hiding load latency.
pub struct SubprocessTranscriber {
    /// Config to pass to the worker
    config: WhisperConfig,
    /// Path to the config file (if any)
    config_path: Option<std::path::PathBuf>,
    /// Vocabulary selected before the worker is spawned.
    active_vocabulary: Mutex<Vec<String>>,
    /// Pre-spawned worker (from prepare())
    prepared_worker: Mutex<Option<PreparedWorker>>,
    /// Last language reported by the worker, if any. Mirrors
    /// `WhisperTranscriber::last_language` so the daemon can derive a layout
    /// hint after transcription. See [`Transcriber::last_detected_language`].
    last_language: Mutex<Option<String>>,
    last_metadata: Mutex<super::RuntimeMetadata>,
}

impl SubprocessTranscriber {
    /// Create a new subprocess transcriber
    pub fn new(
        config: &WhisperConfig,
        config_path: Option<std::path::PathBuf>,
    ) -> Result<Self, TranscribeError> {
        Ok(Self {
            config: config.clone(),
            config_path,
            active_vocabulary: Mutex::new(Vec::new()),
            prepared_worker: Mutex::new(None),
            last_language: Mutex::new(None),
            last_metadata: Mutex::new(super::RuntimeMetadata::default()),
        })
    }

    /// Get the path to the voxtype executable
    fn get_executable_path() -> Result<std::path::PathBuf, TranscribeError> {
        std::env::current_exe().map_err(|e| {
            TranscribeError::InitFailed(format!("Cannot find voxtype executable: {}", e))
        })
    }

    /// Build the command to spawn a worker
    fn build_worker_command(&self) -> Result<Command, TranscribeError> {
        let exe_path = Self::get_executable_path()?;

        let mut cmd = Command::new(&exe_path);

        // Pass config path BEFORE the subcommand — --config is a parent-level
        // arg in clap, so it must appear before "transcribe-worker"
        if let Some(ref config_path) = self.config_path {
            cmd.arg("--config").arg(config_path);
        }

        cmd.arg("transcribe-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Pass essential config via command-line arguments
        cmd.arg("--model").arg(&self.config.model);
        // Serialize language config as comma-separated string for CLI
        // Single: "en", Auto: "auto", Multiple: "en,fr,de"
        let language_str = self.config.language.as_vec().join(",");
        cmd.arg("--language").arg(&language_str);
        if self.config.translate {
            cmd.arg("--translate");
        }
        if let Some(threads) = self.config.threads {
            cmd.arg("--threads").arg(threads.to_string());
        }
        let vocabulary = self
            .active_vocabulary
            .lock()
            .map(|terms| terms.clone())
            .unwrap_or_default();
        if let Some(prompt) =
            super::prompt_with_vocabulary(self.config.initial_prompt.as_deref(), &vocabulary)
        {
            cmd.arg("--initial-prompt").arg(prompt);
        }

        Ok(cmd)
    }

    /// Spawn a worker process and wait for it to be ready
    fn spawn_and_wait_ready(&self) -> Result<PreparedWorker, TranscribeError> {
        let mut cmd = self.build_worker_command()?;

        let mut child = cmd.spawn().map_err(|e| {
            TranscribeError::InitFailed(format!("Failed to spawn transcribe-worker: {}", e))
        })?;

        // Get handles
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| TranscribeError::InitFailed("Worker stdin not available".to_string()))?;

        let stdout = child.stdout.take().ok_or_else(|| {
            TranscribeError::InitFailed("Worker stdout not available".to_string())
        })?;

        let mut stdout = BufReader::new(stdout);

        // Wait for READY signal (model loaded)
        let mut ready_line = String::new();
        stdout.read_line(&mut ready_line).map_err(|e| {
            TranscribeError::InitFailed(format!("Failed to read READY signal: {}", e))
        })?;

        if ready_line.trim() != READY_SIGNAL {
            // Worker failed during model load - try to get error from JSON
            if let Ok(response) = serde_json::from_str::<WorkerResponse>(&ready_line) {
                if let Some(error) = response.error {
                    return Err(TranscribeError::InitFailed(error));
                }
            }
            return Err(TranscribeError::InitFailed(format!(
                "Worker failed to load model (got: {:?})",
                ready_line.trim()
            )));
        }

        tracing::debug!("Worker ready (model loaded)");

        Ok(PreparedWorker {
            child,
            stdin,
            stdout,
        })
    }

    /// Write audio samples to the worker's stdin
    fn write_audio_to_worker(
        stdin: &mut ChildStdin,
        samples: &[f32],
        timed: bool,
    ) -> Result<(), TranscribeError> {
        // Write sample count (u32 little-endian), flagged for a timed decode
        let count = samples.len() as u32 | if timed { TIMED_FLAG } else { 0 };
        stdin.write_all(&count.to_le_bytes()).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to write sample count: {}", e))
        })?;

        // Write samples (f32 little-endian)
        let samples_bytes = unsafe {
            std::slice::from_raw_parts(
                samples.as_ptr() as *const u8,
                std::mem::size_of_val(samples),
            )
        };
        stdin.write_all(samples_bytes).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to write audio samples: {}", e))
        })?;

        stdin.flush().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to flush stdin: {}", e))
        })?;

        Ok(())
    }

    /// Read the JSON response from the worker's stdout
    fn read_worker_response(
        stdout: &mut BufReader<ChildStdout>,
    ) -> Result<WorkerResponse, TranscribeError> {
        let mut line = String::new();
        stdout.read_line(&mut line).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to read worker output: {}", e))
        })?;

        serde_json::from_str(&line).map_err(|e| {
            TranscribeError::InferenceFailed(format!(
                "Failed to parse worker response: {} (output: {:?})",
                e, line
            ))
        })
    }

    fn run(&self, samples: &[f32], timed: bool) -> Result<WorkerResponse, TranscribeError> {
        // What a call learned belongs to that call. Clear both before
        // anything can fail: an empty buffer, a worker that will not start,
        // a broken pipe or a garbled reply must not leave the previous
        // call's language or metadata looking like this one's.
        *self
            .last_metadata
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = super::RuntimeMetadata::default();
        *self
            .last_language
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / 16000.0;

        // Try to use prepared worker, or spawn a new one
        let mut prepared = self.prepared_worker.lock().unwrap();
        let mut worker = match prepared.take() {
            Some(w) => {
                tracing::debug!(
                    "Using pre-spawned worker for {:.2}s of audio",
                    duration_secs
                );
                w
            }
            None => {
                tracing::debug!(
                    "No prepared worker, spawning new one for {:.2}s of audio",
                    duration_secs
                );
                self.spawn_and_wait_ready()?
            }
        };
        drop(prepared); // Release lock

        let start = std::time::Instant::now();

        // Write audio to worker
        Self::write_audio_to_worker(&mut worker.stdin, samples, timed)?;
        drop(worker.stdin); // Close stdin to signal EOF

        // Read response
        let response = Self::read_worker_response(&mut worker.stdout)?;
        *self
            .last_metadata
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = response.metadata.clone();

        // Wait for process to exit
        let status = worker.child.wait().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to wait for worker: {}", e))
        })?;

        if !status.success() {
            // Try to get stderr for error details
            if let Some(mut stderr) = worker.child.stderr.take() {
                let mut err_output = String::new();
                let _ = stderr.read_to_string(&mut err_output);
                if !err_output.is_empty() {
                    tracing::warn!("Worker stderr: {}", err_output.trim());
                }
            }
        }

        tracing::debug!(
            "Subprocess transcription completed in {:.2}s",
            start.elapsed().as_secs_f32()
        );

        // Record reported language for layout-aware output methods. Missing
        // (older worker) leaves the previous value untouched-by-success
        // semantics: clear it on every successful call so stale language
        // hints don't leak across transcriptions when the user switches
        // engines or recordings.
        if let Ok(mut guard) = self.last_language.lock() {
            *guard = response.language.clone();
        }

        if response.ok {
            Ok(response)
        } else {
            Err(TranscribeError::InferenceFailed(
                response
                    .error
                    .unwrap_or_else(|| "Unknown worker error".to_string()),
            ))
        }
    }
}

impl Transcriber for SubprocessTranscriber {
    fn set_vocabulary(&self, terms: &[String]) {
        if let Ok(mut active) = self.active_vocabulary.lock() {
            active.clear();
            active.extend_from_slice(terms);
        }
    }

    fn prepare(&self) {
        tracing::debug!("Preparing subprocess transcriber (spawning worker)...");
        let start = std::time::Instant::now();

        match self.spawn_and_wait_ready() {
            Ok(worker) => {
                let mut guard = self.prepared_worker.lock().unwrap();
                *guard = Some(worker);
                tracing::info!(
                    "Worker prepared in {:.2}s (model loaded while recording)",
                    start.elapsed().as_secs_f32()
                );
            }
            Err(e) => {
                tracing::warn!("Failed to prepare worker: {} (will retry on transcribe)", e);
            }
        }
    }

    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        self.run(samples, false)?.text.ok_or_else(|| {
            TranscribeError::InferenceFailed("Worker returned ok but no text".to_string())
        })
    }

    /// The worker decodes with Whisper's own segment timestamps, so a
    /// meeting chunk gets one line per utterance from one GPU-isolated call.
    fn transcribe_timed(&self, samples: &[f32]) -> Result<Vec<TimedSegment>, TranscribeError> {
        let response = self.run(samples, true)?;
        match (response.segments, response.text) {
            (Some(segments), _) => Ok(segments),
            (None, Some(text)) if !text.trim().is_empty() => Ok(vec![TimedSegment {
                text,
                start_secs: 0.0,
                end_secs: samples.len() as f32 / 16000.0,
            }]),
            (None, _) => Ok(Vec::new()),
        }
    }

    fn last_detected_language(&self) -> Option<String> {
        self.last_language.lock().ok().and_then(|g| g.clone())
    }

    fn runtime_metadata(&self) -> super::RuntimeMetadata {
        self.last_metadata
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_response_parsing() {
        let success: WorkerResponse =
            serde_json::from_str(r#"{"ok": true, "text": "Hello world"}"#).unwrap();
        assert!(success.ok);
        assert_eq!(success.text, Some("Hello world".to_string()));
        // Backward compat: older workers don't emit "language".
        assert_eq!(success.language, None);

        let error: WorkerResponse =
            serde_json::from_str(r#"{"ok": false, "error": "Model not found"}"#).unwrap();
        assert!(!error.ok);
        assert_eq!(error.error, Some("Model not found".to_string()));
    }

    #[test]
    fn test_worker_response_parsing_with_language() {
        let success: WorkerResponse =
            serde_json::from_str(r#"{"ok": true, "text": "Privet", "language": "ru"}"#).unwrap();
        assert!(success.ok);
        assert_eq!(success.language, Some("ru".to_string()));
    }

    #[test]
    #[ignore = "administrative private worker helper, reexecuted by metadata lifecycle test"]
    fn history_metadata_worker_helper() {
        let response =
            std::env::var("VOXTYPE_HISTORY_WORKER_RESPONSE").expect("private worker marker");
        println!("HISTORY_PRIVATE_WORKER_READY");
        std::io::stdout().flush().unwrap();
        let mut input = std::io::stdin().lock();
        let mut count = [0u8; 4];
        input.read_exact(&mut count).unwrap();
        let count = u32::from_le_bytes(count);
        assert_eq!(count, 16);
        let mut samples = [0u8; 64];
        input.read_exact(&mut samples).unwrap();
        assert!(samples.iter().all(|byte| *byte == 0));
        println!("{response}");
        std::io::stdout().flush().unwrap();
    }

    /// A worker that is already "ready" and will answer one request with
    /// `reply`, verbatim: the private helper test above, re-executed.
    fn prepared_worker_replying(fixture: &std::path::Path, reply: &str) -> PreparedWorker {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "transcribe::subprocess::tests::history_metadata_worker_helper",
                "--nocapture",
            ])
            .env_clear()
            .env("HOME", fixture)
            .env("TMPDIR", fixture)
            .env("XDG_CONFIG_HOME", fixture.join("config"))
            .env("XDG_DATA_HOME", fixture.join("data"))
            .env("XDG_RUNTIME_DIR", fixture.join("runtime"))
            .env("RUST_MIN_STACK", "16777216")
            .env("VOXTYPE_HISTORY_WORKER_RESPONSE", reply)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert!(
                stdout.read_line(&mut line).unwrap() > 0,
                "Private worker exited before readiness"
            );
            if line.contains("HISTORY_PRIVATE_WORKER_READY") {
                break;
            }
        }
        PreparedWorker {
            child,
            stdin,
            stdout,
        }
    }

    #[test]
    fn a_failed_call_does_not_keep_the_previous_calls_language() {
        let fixture = tempfile::tempdir().unwrap();
        let engine = SubprocessTranscriber::new(&WhisperConfig::default(), None).unwrap();
        let measured = super::super::RuntimeMetadata {
            model: Some("actual-fixture.bin".into()),
            backend: Some("fixture-vulkan".into()),
            model_load_ms: Some(17),
        };
        let russian =
            serde_json::json!({"ok":true,"text":"Privet","language":"ru","metadata":measured})
                .to_string();

        // A reply that is not JSON: the pipe worked, the answer did not.
        *engine.prepared_worker.lock().unwrap() =
            Some(prepared_worker_replying(fixture.path(), &russian));
        assert_eq!(engine.transcribe(&[0.0; 16]).unwrap(), "Privet");
        assert_eq!(engine.last_detected_language().as_deref(), Some("ru"));
        assert_eq!(engine.runtime_metadata(), measured);
        *engine.prepared_worker.lock().unwrap() =
            Some(prepared_worker_replying(fixture.path(), "this is not json"));
        assert!(engine.transcribe(&[0.0; 16]).is_err());
        assert_eq!(
            engine.last_detected_language(),
            None,
            "stale language after a garbled reply"
        );
        assert_eq!(
            engine.runtime_metadata(),
            super::super::RuntimeMetadata::default(),
            "stale metadata after a garbled reply"
        );

        // The timed path, and an empty buffer, which fails before any worker.
        *engine.prepared_worker.lock().unwrap() =
            Some(prepared_worker_replying(fixture.path(), &russian));
        engine.transcribe(&[0.0; 16]).unwrap();
        assert_eq!(engine.last_detected_language().as_deref(), Some("ru"));
        assert!(engine.transcribe_timed(&[]).is_err());
        assert_eq!(
            engine.last_detected_language(),
            None,
            "stale language after an empty buffer"
        );

        // A reply from an older worker, with no language at all.
        *engine.prepared_worker.lock().unwrap() =
            Some(prepared_worker_replying(fixture.path(), &russian));
        engine.transcribe(&[0.0; 16]).unwrap();
        *engine.prepared_worker.lock().unwrap() = Some(prepared_worker_replying(
            fixture.path(),
            &serde_json::json!({"ok":true,"text":"hello"}).to_string(),
        ));
        assert_eq!(engine.transcribe(&[0.0; 16]).unwrap(), "hello");
        assert_eq!(engine.last_detected_language(), None);
    }

    #[test]
    fn history_subprocess_metadata_follows_the_current_worker_response() {
        let fixture = tempfile::tempdir().unwrap();
        let engine = SubprocessTranscriber::new(&WhisperConfig::default(), None).unwrap();
        let measured = super::super::RuntimeMetadata {
            model: Some("actual-fixture.bin".into()),
            backend: Some("fixture-vulkan".into()),
            model_load_ms: Some(17),
        };
        for (response, ok, expected) in [
            (
                serde_json::json!({"ok":true,"text":"fixture","metadata":measured}),
                true,
                measured.clone(),
            ),
            (
                serde_json::json!({"ok":true,"text":"fixture"}),
                true,
                super::super::RuntimeMetadata::default(),
            ),
            (
                serde_json::json!({"ok":false,"error":"synthetic failure","metadata":measured}),
                false,
                measured.clone(),
            ),
        ] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "transcribe::subprocess::tests::history_metadata_worker_helper",
                    "--nocapture",
                ])
                .env_clear()
                .env("HOME", fixture.path())
                .env("TMPDIR", fixture.path())
                .env("XDG_CONFIG_HOME", fixture.path().join("config"))
                .env("XDG_DATA_HOME", fixture.path().join("data"))
                .env("XDG_RUNTIME_DIR", fixture.path().join("runtime"))
                .env("RUST_MIN_STACK", "16777216")
                .env("VOXTYPE_HISTORY_WORKER_RESPONSE", response.to_string())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let stdin = child.stdin.take().unwrap();
            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            loop {
                let mut line = String::new();
                assert!(
                    stdout.read_line(&mut line).unwrap() > 0,
                    "Private worker exited before readiness"
                );
                if line.contains("HISTORY_PRIVATE_WORKER_READY") {
                    break;
                }
            }
            *engine.prepared_worker.lock().unwrap() = Some(PreparedWorker {
                child,
                stdin,
                stdout,
            });
            assert_eq!(engine.transcribe(&[0.0; 16]).is_ok(), ok);
            assert_eq!(
                engine.runtime_metadata(),
                expected,
                "No capability guesses or stale metadata from the previous worker"
            );
        }
    }
}
