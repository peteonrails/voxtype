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

use super::worker::READY_SIGNAL;
use super::Transcriber;
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
    /// Pre-spawned worker (from prepare())
    prepared_worker: Mutex<Option<PreparedWorker>>,
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
            prepared_worker: Mutex::new(None),
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
        cmd.arg("transcribe-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Pass config path if we have one
        if let Some(ref config_path) = self.config_path {
            cmd.arg("--config").arg(config_path);
        }

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
    ) -> Result<(), TranscribeError> {
        // Write sample count (u32 little-endian)
        let count = samples.len() as u32;
        stdin.write_all(&count.to_le_bytes()).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to write sample count: {}", e))
        })?;

        // Write samples (f32 little-endian)
        let samples_bytes = unsafe {
            std::slice::from_raw_parts(
                samples.as_ptr() as *const u8,
                samples.len() * std::mem::size_of::<f32>(),
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
}

impl Transcriber for SubprocessTranscriber {
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
        Self::write_audio_to_worker(&mut worker.stdin, samples)?;
        drop(worker.stdin); // Close stdin to signal EOF

        // Read response
        let response = Self::read_worker_response(&mut worker.stdout)?;

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

        // Handle response
        if response.ok {
            response.text.ok_or_else(|| {
                TranscribeError::InferenceFailed("Worker returned ok but no text".to_string())
            })
        } else {
            Err(TranscribeError::InferenceFailed(
                response
                    .error
                    .unwrap_or_else(|| "Unknown worker error".to_string()),
            ))
        }
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

        let error: WorkerResponse =
            serde_json::from_str(r#"{"ok": false, "error": "Model not found"}"#).unwrap();
        assert!(!error.ok);
        assert_eq!(error.error, Some("Model not found".to_string()));
    }
}
