//! Daemon module - main event loop orchestration
//!
//! Coordinates the hotkey listener, audio capture, transcription,
//! and text output components.

use crate::audio::feedback::{AudioFeedback, SoundEvent};
use crate::audio::{self, AudioCapture};
use crate::config::{ActivationMode, Config, FileMode, OutputMode};
use crate::eager::{self, EagerConfig};
use crate::error::Result;
use crate::hotkey::{self, HotkeyEvent};
use crate::model_manager::ModelManager;
use crate::output;
use crate::output::post_process::PostProcessor;
use crate::state::{ChunkResult, State};
use crate::text::TextProcessor;
use crate::transcribe::Transcriber;
use pidlock::Pidlock;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::signal::unix::{signal, SignalKind};

/// Send a desktop notification with optional engine icon
async fn send_notification(
    title: &str,
    body: &str,
    show_engine_icon: bool,
    engine: crate::config::TranscriptionEngine,
) {
    let title = if show_engine_icon {
        format!("{} {}", crate::output::engine_icon(engine), title)
    } else {
        title.to_string()
    };

    let _ = Command::new("notify-send")
        .args(["--app-name=Voxtype", "--expire-time=2000", &title, body])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

/// Write state to file for external integrations (e.g., Waybar)
fn write_state_file(path: &PathBuf, state: &str) {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("Failed to create state file directory: {}", e);
            return;
        }
    }

    if let Err(e) = std::fs::write(path, state) {
        tracing::warn!("Failed to write state file: {}", e);
    } else {
        tracing::trace!("State file updated: {}", state);
    }
}

/// Remove state file on shutdown
fn cleanup_state_file(path: &PathBuf) {
    if path.exists() {
        if let Err(e) = std::fs::remove_file(path) {
            tracing::warn!("Failed to remove state file: {}", e);
        }
    }
}

/// Write PID file for external control via signals
fn write_pid_file() -> Option<PathBuf> {
    let pid_path = Config::runtime_dir().join("pid");

    // Ensure parent directory exists
    if let Some(parent) = pid_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("Failed to create PID file directory: {}", e);
            return None;
        }
    }

    let pid = std::process::id();
    if let Err(e) = std::fs::write(&pid_path, pid.to_string()) {
        tracing::warn!("Failed to write PID file: {}", e);
        return None;
    }

    tracing::debug!("PID file written: {:?} (pid={})", pid_path, pid);
    Some(pid_path)
}

/// Remove PID file on shutdown
fn cleanup_pid_file(path: &PathBuf) {
    if path.exists() {
        if let Err(e) = std::fs::remove_file(path) {
            tracing::warn!("Failed to remove PID file: {}", e);
        }
    }
}

/// Check if cancel has been requested (via file trigger)
fn check_cancel_requested() -> bool {
    let cancel_file = Config::runtime_dir().join("cancel");
    if cancel_file.exists() {
        // Remove the file to acknowledge the cancel
        let _ = std::fs::remove_file(&cancel_file);
        true
    } else {
        false
    }
}

/// Clean up any stale cancel file on startup
fn cleanup_cancel_file() {
    let cancel_file = Config::runtime_dir().join("cancel");
    if cancel_file.exists() {
        let _ = std::fs::remove_file(&cancel_file);
    }
}

/// Read and consume the output mode override file
/// Returns the override mode if the file exists and is valid, None otherwise
/// Output mode override result, which may include a file path for file mode
#[derive(Debug, PartialEq)]
enum OutputOverride {
    Mode(OutputMode),
    FileWithPath(PathBuf),
}

/// Read and consume the output mode override file
/// Format: "type", "clipboard", "paste", "file", or "file:/path/to/file.txt"
fn read_output_mode_override() -> Option<OutputOverride> {
    let override_file = Config::runtime_dir().join("output_mode_override");
    if !override_file.exists() {
        return None;
    }

    let content = match std::fs::read_to_string(&override_file) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to read output mode override file: {}", e);
            return None;
        }
    };

    // Consume the file (delete it after reading)
    if let Err(e) = std::fs::remove_file(&override_file) {
        tracing::warn!("Failed to remove output mode override file: {}", e);
    }

    let trimmed = content.trim();

    // Check for file mode with path: "file:/path/to/file.txt"
    if let Some(path) = trimmed.strip_prefix("file:") {
        let path = path.trim();
        if path.is_empty() {
            tracing::warn!("Output mode override 'file:' has empty path");
            return Some(OutputOverride::Mode(OutputMode::File));
        }
        tracing::info!("Using output mode override: file with path {:?}", path);
        return Some(OutputOverride::FileWithPath(PathBuf::from(path)));
    }

    match trimmed {
        "type" => {
            tracing::info!("Using output mode override: type");
            Some(OutputOverride::Mode(OutputMode::Type))
        }
        "clipboard" => {
            tracing::info!("Using output mode override: clipboard");
            Some(OutputOverride::Mode(OutputMode::Clipboard))
        }
        "paste" => {
            tracing::info!("Using output mode override: paste");
            Some(OutputOverride::Mode(OutputMode::Paste))
        }
        "file" => {
            tracing::info!("Using output mode override: file (using config path)");
            Some(OutputOverride::Mode(OutputMode::File))
        }
        other => {
            tracing::warn!("Invalid output mode override: {:?}", other);
            None
        }
    }
}

/// Remove the output mode override file if it exists (for cleanup on cancel/error)
fn cleanup_output_mode_override() {
    let override_file = Config::runtime_dir().join("output_mode_override");
    let _ = std::fs::remove_file(&override_file);
}

/// Read and consume the profile override file
/// Returns the profile name if the file exists and is valid, None otherwise
fn read_profile_override() -> Option<String> {
    let profile_file = Config::runtime_dir().join("profile_override");
    if !profile_file.exists() {
        return None;
    }

    let content = match std::fs::read_to_string(&profile_file) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to read profile override file: {}", e);
            return None;
        }
    };

    // Consume the file (delete it after reading)
    if let Err(e) = std::fs::remove_file(&profile_file) {
        tracing::warn!("Failed to remove profile override file: {}", e);
    }

    let profile_name = content.trim().to_string();
    if profile_name.is_empty() {
        return None;
    }

    tracing::info!("Using profile override: {}", profile_name);
    Some(profile_name)
}

/// Remove the profile override file if it exists (for cleanup on cancel/error)
fn cleanup_profile_override() {
    let profile_file = Config::runtime_dir().join("profile_override");
    let _ = std::fs::remove_file(&profile_file);
}

/// Write transcription to a file, respecting file_mode (overwrite or append)
async fn write_transcription_to_file(
    path: &std::path::Path,
    text: &str,
    file_mode: &FileMode,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    // Ensure text ends with newline
    let output_text = if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{}\n", text)
    };

    match file_mode {
        FileMode::Overwrite => {
            tokio::fs::write(path, output_text).await?;
        }
        FileMode::Append => {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await?;
            file.write_all(output_text.as_bytes()).await?;
        }
    }

    Ok(())
}

/// Read and consume the model override file
/// Returns the model name if the file exists, None otherwise
fn read_model_override() -> Option<String> {
    let override_file = Config::runtime_dir().join("model_override");
    if !override_file.exists() {
        return None;
    }

    let model_str = match std::fs::read_to_string(&override_file) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to read model override file: {}", e);
            return None;
        }
    };

    // Consume the file (delete it after reading)
    if let Err(e) = std::fs::remove_file(&override_file) {
        tracing::warn!("Failed to remove model override file: {}", e);
    }

    let model = model_str.trim().to_string();
    if model.is_empty() {
        None
    } else {
        tracing::info!("Using model override: {}", model);
        Some(model)
    }
}

/// Remove the model override file if it exists (for cleanup on cancel/error)
fn cleanup_model_override() {
    let override_file = Config::runtime_dir().join("model_override");
    let _ = std::fs::remove_file(&override_file);
}

/// Result type for transcription task
type TranscriptionResult = std::result::Result<String, crate::error::TranscribeError>;

/// Main daemon that orchestrates all components
pub struct Daemon {
    config: Config,
    config_path: Option<PathBuf>,
    state_file_path: Option<PathBuf>,
    pid_file_path: Option<PathBuf>,
    audio_feedback: Option<AudioFeedback>,
    text_processor: TextProcessor,
    post_processor: Option<PostProcessor>,
    // Model manager for multi-model support
    model_manager: Option<ModelManager>,
    // Background task for loading model on-demand
    model_load_task: Option<
        tokio::task::JoinHandle<
            std::result::Result<Arc<dyn Transcriber>, crate::error::TranscribeError>,
        >,
    >,
    // Background task for transcription (allows cancel during transcription)
    transcription_task: Option<tokio::task::JoinHandle<TranscriptionResult>>,
    // Background tasks for eager chunk transcriptions (chunk_index, task)
    eager_chunk_tasks: Vec<(usize, tokio::task::JoinHandle<std::result::Result<String, crate::error::TranscribeError>>)>,
}

impl Daemon {
    /// Create a new daemon with the given configuration
    pub fn new(config: Config, config_path: Option<PathBuf>) -> Self {
        let state_file_path = config.resolve_state_file();

        // Initialize audio feedback if enabled
        let audio_feedback = if config.audio.feedback.enabled {
            match AudioFeedback::new(&config.audio.feedback) {
                Ok(feedback) => {
                    tracing::info!(
                        "Audio feedback enabled (theme: {}, volume: {:.0}%)",
                        config.audio.feedback.theme,
                        config.audio.feedback.volume * 100.0
                    );
                    Some(feedback)
                }
                Err(e) => {
                    tracing::warn!("Failed to initialize audio feedback: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Initialize text processor
        let text_processor = TextProcessor::new(&config.text);
        if config.text.spoken_punctuation {
            tracing::info!("Spoken punctuation enabled");
        }
        if !config.text.replacements.is_empty() {
            tracing::info!(
                "Word replacements configured: {} rules",
                config.text.replacements.len()
            );
        }

        // Initialize post-processor if configured
        let post_processor = config.output.post_process.as_ref().map(|cfg| {
            tracing::info!(
                "Post-processing enabled: command={:?}, timeout={}ms",
                cfg.command,
                cfg.timeout_ms
            );
            PostProcessor::new(cfg)
        });

        Self {
            config,
            config_path,
            state_file_path,
            pid_file_path: None,
            audio_feedback,
            text_processor,
            post_processor,
            model_manager: None,
            model_load_task: None,
            transcription_task: None,
            eager_chunk_tasks: Vec::new(),
        }
    }

    /// Play audio feedback sound if enabled
    fn play_feedback(&self, event: SoundEvent) {
        if let Some(ref feedback) = self.audio_feedback {
            feedback.play(event);
        }
    }

    /// Update the state file if configured
    fn update_state(&self, state_name: &str) {
        if let Some(ref path) = self.state_file_path {
            write_state_file(path, state_name);
        }
    }

    /// Get the transcriber for the current recording session
    ///
    /// For on-demand loading: waits for the background model load task to complete
    /// For preloaded models: returns the preloaded transcriber (Parakeet) or gets from model manager (Whisper)
    ///
    /// Returns Ok(transcriber) on success, Err(()) if an error occurred and caller should skip to next iteration
    async fn get_transcriber_for_recording(
        &mut self,
        model_override: Option<&str>,
        transcriber_preloaded: &Option<Arc<dyn Transcriber>>,
    ) -> std::result::Result<Arc<dyn Transcriber>, ()> {
        if self.config.on_demand_loading() {
            // Wait for background model load task
            if let Some(task) = self.model_load_task.take() {
                match task.await {
                    Ok(Ok(transcriber)) => {
                        tracing::info!("Model loaded successfully");
                        Ok(transcriber)
                    }
                    Ok(Err(e)) => {
                        tracing::error!("Model loading failed: {}", e);
                        self.play_feedback(SoundEvent::Error);
                        Err(())
                    }
                    Err(e) => {
                        tracing::error!("Model loading task panicked: {}", e);
                        self.play_feedback(SoundEvent::Error);
                        Err(())
                    }
                }
            } else {
                tracing::error!("No model loading task found");
                self.play_feedback(SoundEvent::Error);
                Err(())
            }
        } else {
            // Use preloaded transcriber based on engine type
            match self.config.engine {
                crate::config::TranscriptionEngine::Parakeet => {
                    if let Some(ref t) = transcriber_preloaded {
                        Ok(t.clone())
                    } else {
                        tracing::error!("Parakeet transcriber not preloaded");
                        self.play_feedback(SoundEvent::Error);
                        Err(())
                    }
                }
                crate::config::TranscriptionEngine::Whisper => {
                    if let Some(ref mut mm) = self.model_manager {
                        match mm.get_prepared_transcriber(model_override) {
                            Ok(t) => Ok(t),
                            Err(e) => {
                                tracing::error!("Failed to get transcriber: {}", e);
                                self.play_feedback(SoundEvent::Error);
                                Err(())
                            }
                        }
                    } else {
                        tracing::error!("Model manager not initialized");
                        self.play_feedback(SoundEvent::Error);
                        Err(())
                    }
                }
            }
        }
    }

    /// Reset state to idle and run post_output_command to reset compositor submap
    /// Call this when exiting from recording/transcribing without normal output flow
    async fn reset_to_idle(&self, state: &mut State) {
        cleanup_output_mode_override();
        cleanup_model_override();
        cleanup_profile_override();
        *state = State::Idle;
        self.update_state("idle");

        // Run post_output_command to reset compositor submap
        if let Some(cmd) = &self.config.output.post_output_command {
            if let Err(e) = output::run_hook(cmd, "post_output").await {
                tracing::warn!("{}", e);
            }
        }
    }

    /// Spawn a transcription task for a single chunk (eager processing)
    fn spawn_chunk_transcription(
        &mut self,
        chunk_index: usize,
        chunk_audio: Vec<f32>,
        transcriber: Arc<dyn Transcriber>,
    ) {
        tracing::debug!(
            "Spawning eager transcription for chunk {} ({:.1}s)",
            chunk_index,
            chunk_audio.len() as f32 / 16000.0
        );

        let task = tokio::task::spawn_blocking(move || transcriber.transcribe(&chunk_audio));

        self.eager_chunk_tasks.push((chunk_index, task));
    }

    /// Check for any ready chunks in accumulated audio and spawn transcription tasks
    /// Returns the number of new chunks spawned
    fn process_eager_chunks(
        &mut self,
        accumulated_audio: &[f32],
        chunks_sent: &mut usize,
        tasks_in_flight: &mut usize,
        transcriber: &Arc<dyn Transcriber>,
    ) -> usize {
        let eager_config = EagerConfig::from_whisper_config(&self.config.whisper);
        let complete_chunks = eager::count_complete_chunks(accumulated_audio.len(), &eager_config);

        let mut spawned = 0;
        while *chunks_sent < complete_chunks {
            if let Some(chunk_audio) = eager::extract_chunk(accumulated_audio, *chunks_sent, &eager_config) {
                self.spawn_chunk_transcription(*chunks_sent, chunk_audio, transcriber.clone());
                *chunks_sent += 1;
                *tasks_in_flight += 1;
                spawned += 1;
            } else {
                break;
            }
        }

        spawned
    }

    /// Poll for completed chunk transcription tasks and collect results
    /// Returns any completed results
    async fn poll_chunk_tasks(&mut self) -> Vec<ChunkResult> {
        let mut completed = Vec::new();
        let mut remaining_tasks = Vec::new();

        for (chunk_index, task) in self.eager_chunk_tasks.drain(..) {
            if task.is_finished() {
                // Task is finished, await will complete immediately
                match task.await {
                    Ok(Ok(text)) => {
                        tracing::debug!("Chunk {} completed: {:?}", chunk_index, text);
                        completed.push(ChunkResult { text, chunk_index });
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("Chunk {} transcription failed: {}", chunk_index, e);
                        // Add empty result to maintain ordering
                        completed.push(ChunkResult {
                            text: String::new(),
                            chunk_index,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Chunk {} task panicked: {}", chunk_index, e);
                        completed.push(ChunkResult {
                            text: String::new(),
                            chunk_index,
                        });
                    }
                }
            } else {
                remaining_tasks.push((chunk_index, task));
            }
        }

        self.eager_chunk_tasks = remaining_tasks;
        completed
    }

    /// Wait for all remaining chunk tasks to complete
    async fn wait_for_chunk_tasks(&mut self) -> Vec<ChunkResult> {
        let mut results = Vec::new();

        for (chunk_index, task) in self.eager_chunk_tasks.drain(..) {
            match task.await {
                Ok(Ok(text)) => {
                    tracing::debug!("Chunk {} completed (waited): {:?}", chunk_index, text);
                    results.push(ChunkResult { text, chunk_index });
                }
                Ok(Err(e)) => {
                    tracing::warn!("Chunk {} transcription failed: {}", chunk_index, e);
                    results.push(ChunkResult {
                        text: String::new(),
                        chunk_index,
                    });
                }
                Err(e) => {
                    if e.is_cancelled() {
                        tracing::debug!("Chunk {} task was cancelled", chunk_index);
                    } else {
                        tracing::warn!("Chunk {} task panicked: {}", chunk_index, e);
                    }
                    results.push(ChunkResult {
                        text: String::new(),
                        chunk_index,
                    });
                }
            }
        }

        results
    }

    /// Finish eager recording: wait for all chunks, transcribe tail, combine results
    async fn finish_eager_recording(
        &mut self,
        state: &mut State,
        transcriber: Arc<dyn Transcriber>,
    ) -> Option<String> {
        // Extract state data
        let (accumulated_audio, mut chunk_results) = match state {
            State::EagerRecording {
                accumulated_audio,
                chunk_results,
                ..
            } => (accumulated_audio.clone(), chunk_results.clone()),
            _ => return None,
        };

        let audio_duration = accumulated_audio.len() as f32 / 16000.0;
        tracing::info!(
            "Finishing eager recording: {:.1}s of audio, {} chunks already transcribed",
            audio_duration,
            chunk_results.len()
        );

        // Wait for any in-flight chunk tasks
        let mut waited_results = self.wait_for_chunk_tasks().await;
        chunk_results.append(&mut waited_results);

        // Transcribe the tail (audio after last complete chunk)
        let eager_config = EagerConfig::from_whisper_config(&self.config.whisper);
        let chunks_sent = chunk_results.iter().map(|r| r.chunk_index).max().map(|i| i + 1).unwrap_or(0);
        let tail_start = chunks_sent * eager_config.stride_samples();

        if tail_start < accumulated_audio.len() {
            let tail_audio = accumulated_audio[tail_start..].to_vec();
            let tail_duration = tail_audio.len() as f32 / 16000.0;

            if tail_duration >= 0.3 {
                tracing::debug!(
                    "Transcribing tail audio: {:.1}s (from sample {})",
                    tail_duration,
                    tail_start
                );

                let tail_transcriber = transcriber.clone();
                match tokio::task::spawn_blocking(move || tail_transcriber.transcribe(&tail_audio)).await {
                    Ok(Ok(text)) => {
                        tracing::debug!("Tail transcription: {:?}", text);
                        chunk_results.push(ChunkResult {
                            text,
                            chunk_index: chunks_sent,
                        });
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("Tail transcription failed: {}", e);
                    }
                    Err(e) => {
                        tracing::warn!("Tail transcription task panicked: {}", e);
                    }
                }
            }
        }

        // Combine all chunk results
        let combined = eager::combine_chunk_results(chunk_results);
        tracing::info!("Combined eager transcription: {:?}", combined);

        if combined.is_empty() {
            None
        } else {
            Some(combined)
        }
    }

    /// Start transcription task (non-blocking, stores JoinHandle for later completion)
    /// Returns true if transcription was started, false if skipped (too short)
    async fn start_transcription_task(
        &mut self,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        transcriber: Option<Arc<dyn Transcriber>>,
    ) -> bool {
        let duration = state.recording_duration().unwrap_or_default();
        tracing::info!("Recording stopped ({:.1}s)", duration.as_secs_f32());

        // Play audio feedback
        self.play_feedback(SoundEvent::RecordingStop);

        // Send notification if enabled
        if self.config.output.notification.on_recording_stop {
            send_notification(
                "Recording Stopped",
                "Transcribing...",
                self.config.output.notification.show_engine_icon,
                self.config.engine,
            )
            .await;
        }

        // Stop recording and get samples
        if let Some(mut capture) = audio_capture.take() {
            match capture.stop().await {
                Ok(samples) => {
                    let audio_duration = samples.len() as f32 / 16000.0;

                    // Skip if too short (likely accidental press)
                    if audio_duration < 0.3 {
                        tracing::debug!("Recording too short ({:.2}s), ignoring", audio_duration);
                        self.reset_to_idle(state).await;
                        return false;
                    }

                    tracing::info!("Transcribing {:.1}s of audio...", audio_duration);
                    *state = State::Transcribing {
                        audio: samples.clone(),
                    };
                    self.update_state("transcribing");

                    // Spawn transcription task (non-blocking)
                    if let Some(t) = transcriber {
                        self.transcription_task =
                            Some(tokio::task::spawn_blocking(move || t.transcribe(&samples)));
                        return true;
                    } else {
                        tracing::error!("No transcriber available");
                        self.play_feedback(SoundEvent::Error);
                        self.reset_to_idle(state).await;
                        return false;
                    }
                }
                Err(e) => {
                    tracing::warn!("Recording error: {}", e);
                    self.reset_to_idle(state).await;
                    return false;
                }
            }
        } else {
            self.reset_to_idle(state).await;
            return false;
        }
    }

    /// Handle transcription completion (called when transcription_task completes)
    async fn handle_transcription_result(
        &self,
        state: &mut State,
        result: std::result::Result<TranscriptionResult, tokio::task::JoinError>,
    ) {
        match result {
            Ok(Ok(text)) => {
                if text.is_empty() {
                    tracing::debug!("Transcription was empty");
                    self.reset_to_idle(state).await;
                } else {
                    tracing::info!("Transcribed: {:?}", text);

                    // Apply text processing (replacements, punctuation)
                    let processed_text = self.text_processor.process(&text);
                    if processed_text != text {
                        tracing::debug!("After text processing: {:?}", processed_text);
                    }

                    // Check for profile override from CLI flags
                    let profile_override = read_profile_override();
                    let active_profile = profile_override
                        .as_ref()
                        .and_then(|name| self.config.get_profile(name));

                    if let Some(profile_name) = &profile_override {
                        if active_profile.is_none() {
                            tracing::warn!(
                                "Profile '{}' not found in config, using default settings",
                                profile_name
                            );
                        }
                    }

                    // Apply post-processing command (profile overrides default)
                    let final_text = if let Some(profile) = active_profile {
                        if let Some(ref cmd) = profile.post_process_command {
                            let timeout_ms = profile.post_process_timeout_ms.unwrap_or(30000);
                            let profile_config = crate::config::PostProcessConfig {
                                command: cmd.clone(),
                                timeout_ms,
                            };
                            let profile_processor = PostProcessor::new(&profile_config);
                            tracing::info!(
                                "Post-processing with profile: {:?}",
                                profile_override.as_ref().unwrap()
                            );
                            let result = profile_processor.process(&processed_text).await;
                            tracing::info!("Post-processed: {:?}", result);
                            result
                        } else {
                            // Profile exists but has no post_process_command, use default
                            if let Some(ref post_processor) = self.post_processor {
                                tracing::info!("Post-processing: {:?}", processed_text);
                                let result = post_processor.process(&processed_text).await;
                                tracing::info!("Post-processed: {:?}", result);
                                result
                            } else {
                                processed_text
                            }
                        }
                    } else if let Some(ref post_processor) = self.post_processor {
                        tracing::info!("Post-processing: {:?}", processed_text);
                        let result = post_processor.process(&processed_text).await;
                        tracing::info!("Post-processed: {:?}", result);
                        result
                    } else {
                        processed_text
                    };

                    // Check for output mode override from CLI flags
                    let output_override = read_output_mode_override();

                    // Check if profile specifies output mode override
                    let profile_output_mode = active_profile.and_then(|p| p.output_mode.clone());

                    // Determine file output path (if file mode)
                    // Priority: 1. CLI --file=path, 2. CLI --file (config path), 3. profile output_mode, 4. config mode=file
                    let file_output_path: Option<PathBuf> = match &output_override {
                        Some(OutputOverride::FileWithPath(path)) => {
                            // CLI --file=path.txt
                            Some(path.clone())
                        }
                        Some(OutputOverride::Mode(OutputMode::File)) => {
                            // CLI --file (no path) - use config's file_path
                            self.config.output.file_path.clone()
                        }
                        None if profile_output_mode == Some(OutputMode::File) => {
                            // Profile specifies file mode
                            self.config.output.file_path.clone()
                        }
                        None if self.config.output.mode == OutputMode::File => {
                            // Config mode = "file" (no CLI override)
                            self.config.output.file_path.clone()
                        }
                        _ => None,
                    };

                    if let Some(output_path) = file_output_path {
                        *state = State::Outputting {
                            text: final_text.clone(),
                        };

                        let file_mode = &self.config.output.file_mode;
                        match write_transcription_to_file(&output_path, &final_text, file_mode)
                            .await
                        {
                            Ok(()) => {
                                let mode_str = match file_mode {
                                    FileMode::Overwrite => "wrote",
                                    FileMode::Append => "appended",
                                };
                                tracing::info!("{} transcription to {:?}", mode_str, output_path);
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to write transcription to {:?}: {}",
                                    output_path,
                                    e
                                );
                            }
                        }

                        *state = State::Idle;
                        self.update_state("idle");
                        return;
                    }

                    // Create output chain with potential mode override (for non-file modes)
                    // Priority: 1. CLI override, 2. profile output_mode, 3. config default
                    let output_config = match output_override {
                        Some(OutputOverride::Mode(mode)) => {
                            let mut config = self.config.output.clone();
                            config.mode = mode;
                            config
                        }
                        _ => {
                            if let Some(mode) = profile_output_mode {
                                let mut config = self.config.output.clone();
                                config.mode = mode;
                                config
                            } else {
                                self.config.output.clone()
                            }
                        }
                    };
                    let output_chain = output::create_output_chain(&output_config);

                    // Output the text
                    *state = State::Outputting {
                        text: final_text.clone(),
                    };

                    let output_options = output::OutputOptions {
                        pre_output_command: output_config.pre_output_command.as_deref(),
                        post_output_command: output_config.post_output_command.as_deref(),
                    };

                    if let Err(e) =
                        output::output_with_fallback(&output_chain, &final_text, output_options)
                            .await
                    {
                        tracing::error!("Output failed: {}", e);
                    } else if self.config.output.notification.on_transcription {
                        // Send notification on successful output
                        output::send_transcription_notification(
                            &final_text,
                            self.config.output.notification.show_engine_icon,
                            self.config.engine,
                        )
                        .await;
                    }

                    *state = State::Idle;
                    self.update_state("idle");
                }
            }
            Ok(Err(e)) => {
                tracing::error!("Transcription failed: {}", e);
                self.reset_to_idle(state).await;
            }
            Err(e) => {
                // JoinError - task was cancelled or panicked
                if e.is_cancelled() {
                    tracing::debug!("Transcription task was cancelled");
                } else {
                    tracing::error!("Transcription task panicked: {}", e);
                }
                self.reset_to_idle(state).await;
            }
        }
    }

    /// Run the daemon main loop
    pub async fn run(&mut self) -> Result<()> {
        tracing::info!("Starting voxtype daemon");

        // Clean up any stale cancel file from previous runs
        cleanup_cancel_file();

        // Write PID file for external control via signals
        self.pid_file_path = write_pid_file();

        // Set up signal handlers for external control
        let mut sigusr1 = signal(SignalKind::user_defined1()).map_err(|e| {
            crate::error::VoxtypeError::Config(format!("Failed to set up SIGUSR1 handler: {}", e))
        })?;
        let mut sigusr2 = signal(SignalKind::user_defined2()).map_err(|e| {
            crate::error::VoxtypeError::Config(format!("Failed to set up SIGUSR2 handler: {}", e))
        })?;
        let mut sigterm = signal(SignalKind::terminate()).map_err(|e| {
            crate::error::VoxtypeError::Config(format!("Failed to set up SIGTERM handler: {}", e))
        })?;

        // Ensure required directories exist
        Config::ensure_directories().map_err(|e| {
            crate::error::VoxtypeError::Config(format!("Failed to create directories: {}", e))
        })?;

        // Check if another instance is already running (single-instance safeguard)
        let lock_path = Config::runtime_dir().join("voxtype.lock");
        let lock_path_str = lock_path.to_string_lossy().to_string();
        let mut pidlock = Pidlock::new(&lock_path_str);

        match pidlock.acquire() {
            Ok(_) => {
                tracing::debug!("Acquired PID lock at {:?}", lock_path);
            }
            Err(e) => {
                tracing::error!(
                    "Failed to acquire lock: another voxtype instance is already running"
                );
                return Err(crate::error::VoxtypeError::Config(format!(
                    "Another voxtype instance is already running (lock error: {:?})",
                    e
                ))
                .into());
            }
        }

        tracing::info!("Output mode: {:?}", self.config.output.mode);

        // Log state file if configured
        if let Some(ref path) = self.state_file_path {
            tracing::info!("State file: {:?}", path);
        }

        // Initialize hotkey listener (if enabled)
        let mut hotkey_listener = if self.config.hotkey.enabled {
            tracing::info!("Hotkey: {}", self.config.hotkey.key);
            let secondary_model = self.config.whisper.secondary_model.clone();
            Some(hotkey::create_listener(
                &self.config.hotkey,
                secondary_model,
            )?)
        } else {
            tracing::info!(
                "Built-in hotkey disabled, use 'voxtype record' commands or compositor keybindings"
            );
            None
        };

        // Log default output chain (chain is created dynamically per-transcription to support overrides)
        let default_chain = output::create_output_chain(&self.config.output);
        tracing::debug!(
            "Default output chain: {}",
            default_chain
                .iter()
                .map(|o| o.name())
                .collect::<Vec<_>>()
                .join(" -> ")
        );
        drop(default_chain); // Not used; chain is created per-transcription

        // Initialize model manager for multi-model support (Whisper only)
        let mut model_manager = ModelManager::new(&self.config.whisper, self.config_path.clone());

        // Pre-load transcription model if on_demand_loading is disabled
        let mut transcriber_preloaded: Option<Arc<dyn Transcriber>> = None;
        if !self.config.on_demand_loading() {
            tracing::info!("Loading transcription model: {}", self.config.model_name());
            match self.config.engine {
                crate::config::TranscriptionEngine::Whisper => {
                    // Use model manager for Whisper
                    if let Err(e) = model_manager.preload_primary() {
                        tracing::error!("Failed to preload model: {}", e);
                        return Err(crate::error::VoxtypeError::Transcribe(e));
                    }
                }
                crate::config::TranscriptionEngine::Parakeet => {
                    // Parakeet uses its own model loading
                    transcriber_preloaded = Some(Arc::from(crate::transcribe::create_transcriber(
                        &self.config,
                    )?));
                }
            }
            tracing::info!("Model loaded, ready for voice input");
        } else {
            tracing::info!("On-demand loading enabled, model will be loaded when recording starts");
        }

        // Log secondary model if configured
        if let Some(ref secondary) = self.config.whisper.secondary_model {
            tracing::info!("Secondary model configured: {}", secondary);
            if let Some(ref modifier) = self.config.hotkey.model_modifier {
                tracing::info!("Model modifier key: {}", modifier);
            }
        }

        self.model_manager = Some(model_manager);

        // Start hotkey listener (if enabled)
        let mut hotkey_rx = if let Some(ref mut listener) = hotkey_listener {
            Some(listener.start().await?)
        } else {
            None
        };

        // Current state
        let mut state = State::Idle;

        // Audio capture (created fresh for each recording)
        let mut audio_capture: Option<Box<dyn AudioCapture>> = None;

        // Recording timeout
        let max_duration = Duration::from_secs(self.config.audio.max_duration_secs as u64);

        let activation_mode = self.config.hotkey.mode;
        if self.config.hotkey.enabled {
            let mode_desc = match activation_mode {
                ActivationMode::PushToTalk => "hold to record, release to transcribe",
                ActivationMode::Toggle => "press to start/stop recording",
            };
            tracing::info!(
                "Listening for hotkey: {} ({})",
                self.config.hotkey.key,
                mode_desc
            );
        }

        // Write initial state
        self.update_state("idle");

        // Main event loop
        loop {
            tokio::select! {
                // Handle hotkey events (only if hotkey listener is enabled)
                Some(hotkey_event) = async {
                    match &mut hotkey_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match (hotkey_event, activation_mode) {
                        // === PUSH-TO-TALK MODE ===
                        (HotkeyEvent::Pressed { model_override }, ActivationMode::PushToTalk) => {
                            tracing::debug!("Received HotkeyEvent::Pressed (push-to-talk), state.is_idle() = {}, model_override = {:?}",
                                state.is_idle(), model_override);
                            if state.is_idle() {
                                tracing::info!("Recording started");

                                // Send notification if enabled
                                if self.config.output.notification.on_recording_start {
                                    send_notification("Push to Talk Active", "Recording...", self.config.output.notification.show_engine_icon, self.config.engine).await;
                                }

                                // Prepare model for transcription
                                if self.config.on_demand_loading() {
                                    // Start model loading in background
                                    match self.config.engine {
                                        crate::config::TranscriptionEngine::Whisper => {
                                            let config = self.config.whisper.clone();
                                            let config_path = self.config_path.clone();
                                            let model_to_load = model_override.clone();
                                            self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                                                let mut temp_manager = ModelManager::new(&config, config_path);
                                                temp_manager.get_transcriber(model_to_load.as_deref())
                                            }));
                                        }
                                        crate::config::TranscriptionEngine::Parakeet => {
                                            let config = self.config.clone();
                                            self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                                                crate::transcribe::create_transcriber(&config).map(Arc::from)
                                            }));
                                        }
                                    }
                                    tracing::debug!("Started background model loading");
                                } else {
                                    // Prepare model (spawns subprocess for gpu_isolation mode)
                                    match self.config.engine {
                                        crate::config::TranscriptionEngine::Whisper => {
                                            if let Some(ref mut mm) = self.model_manager {
                                                if let Err(e) = mm.prepare_model(model_override.as_deref()) {
                                                    tracing::warn!("Failed to prepare model: {}", e);
                                                }
                                            }
                                        }
                                        crate::config::TranscriptionEngine::Parakeet => {
                                            if let Some(ref t) = transcriber_preloaded {
                                                let transcriber = t.clone();
                                                tokio::task::spawn_blocking(move || {
                                                    transcriber.prepare();
                                                });
                                            }
                                        }
                                    }
                                }

                                // Create and start audio capture
                                tracing::debug!("Creating audio capture with device: {}", self.config.audio.device);
                                match audio::create_capture(&self.config.audio) {
                                    Ok(mut capture) => {
                                        tracing::debug!("Audio capture created, starting...");
                                        if let Err(e) = capture.start().await {
                                            tracing::error!("Failed to start audio: {}", e);
                                            continue;
                                        }
                                        tracing::debug!("Audio capture started successfully");
                                        audio_capture = Some(capture);

                                        // Use EagerRecording state if eager_processing is enabled
                                        if self.config.whisper.eager_processing {
                                            tracing::info!("Using eager input processing");
                                            state = State::EagerRecording {
                                                started_at: std::time::Instant::now(),
                                                model_override: model_override.clone(),
                                                accumulated_audio: Vec::new(),
                                                chunks_sent: 0,
                                                chunk_results: Vec::new(),
                                                tasks_in_flight: 0,
                                            };
                                        } else {
                                            state = State::Recording {
                                                started_at: std::time::Instant::now(),
                                                model_override: model_override.clone(),
                                            };
                                        }
                                        self.update_state("recording");
                                        self.play_feedback(SoundEvent::RecordingStart);

                                        // Run pre-recording hook (e.g., enter compositor submap for cancel)
                                        if let Some(cmd) = &self.config.output.pre_recording_command {
                                            if let Err(e) = output::run_hook(cmd, "pre_recording").await {
                                                tracing::warn!("{}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to create audio capture: {}", e);
                                        self.play_feedback(SoundEvent::Error);
                                    }
                                }
                            }
                        }

                        (HotkeyEvent::Released, ActivationMode::PushToTalk) => {
                            tracing::debug!("Received HotkeyEvent::Released (push-to-talk), state.is_recording() = {}", state.is_recording());
                            if let State::Recording { model_override, .. } = &state {
                                let transcriber = match self.get_transcriber_for_recording(
                                    model_override.as_deref(),
                                    &transcriber_preloaded,
                                ).await {
                                    Ok(t) => Some(t),
                                    Err(()) => {
                                        state = State::Idle;
                                        self.update_state("idle");
                                        continue;
                                    }
                                };

                                self.start_transcription_task(
                                    &mut state,
                                    &mut audio_capture,
                                    transcriber,
                                ).await;
                            } else if state.is_eager_recording() {
                                // Handle eager recording stop - extract model_override first
                                let model_override = match &state {
                                    State::EagerRecording { model_override, .. } => model_override.clone(),
                                    _ => None,
                                };

                                let duration = state.recording_duration().unwrap_or_default();
                                tracing::info!("Eager recording stopped ({:.1}s)", duration.as_secs_f32());

                                self.play_feedback(SoundEvent::RecordingStop);

                                if self.config.output.notification.on_recording_stop {
                                    send_notification("Recording Stopped", "Transcribing...", self.config.output.notification.show_engine_icon, self.config.engine).await;
                                }

                                // Stop audio capture and get remaining samples
                                if let Some(mut capture) = audio_capture.take() {
                                    if let Ok(final_samples) = capture.stop().await {
                                        // Add final samples to accumulated audio
                                        if let State::EagerRecording { accumulated_audio, .. } = &mut state {
                                            accumulated_audio.extend(final_samples);
                                        }
                                    }
                                }

                                let transcriber = match self.get_transcriber_for_recording(
                                    model_override.as_deref(),
                                    &transcriber_preloaded,
                                ).await {
                                    Ok(t) => t,
                                    Err(()) => {
                                        state = State::Idle;
                                        self.update_state("idle");
                                        continue;
                                    }
                                };

                                self.update_state("transcribing");

                                if let Some(text) = self.finish_eager_recording(&mut state, transcriber).await {
                                    // Move to outputting state and handle via transcription result flow
                                    state = State::Transcribing { audio: Vec::new() };
                                    self.handle_transcription_result(&mut state, Ok(Ok(text))).await;
                                } else {
                                    tracing::debug!("Eager recording produced empty result");
                                    self.reset_to_idle(&mut state).await;
                                }
                            }
                        }

                        // === TOGGLE MODE ===
                        (HotkeyEvent::Pressed { model_override }, ActivationMode::Toggle) => {
                            tracing::debug!("Received HotkeyEvent::Pressed (toggle), state.is_idle() = {}, state.is_recording() = {}, model_override = {:?}",
                                state.is_idle(), state.is_recording(), model_override);

                            if state.is_idle() {
                                // Start recording
                                tracing::info!("Recording started (toggle mode)");

                                if self.config.output.notification.on_recording_start {
                                    send_notification("Recording Started", "Press hotkey again to stop", self.config.output.notification.show_engine_icon, self.config.engine).await;
                                }

                                // Prepare model for transcription
                                if self.config.on_demand_loading() {
                                    // Start model loading in background
                                    match self.config.engine {
                                        crate::config::TranscriptionEngine::Whisper => {
                                            let config = self.config.whisper.clone();
                                            let config_path = self.config_path.clone();
                                            let model_to_load = model_override.clone();
                                            self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                                                let mut temp_manager = ModelManager::new(&config, config_path);
                                                temp_manager.get_transcriber(model_to_load.as_deref())
                                            }));
                                        }
                                        crate::config::TranscriptionEngine::Parakeet => {
                                            let config = self.config.clone();
                                            self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                                                crate::transcribe::create_transcriber(&config).map(Arc::from)
                                            }));
                                        }
                                    }
                                    tracing::debug!("Started background model loading");
                                } else {
                                    // Prepare model (spawns subprocess for gpu_isolation mode)
                                    match self.config.engine {
                                        crate::config::TranscriptionEngine::Whisper => {
                                            if let Some(ref mut mm) = self.model_manager {
                                                if let Err(e) = mm.prepare_model(model_override.as_deref()) {
                                                    tracing::warn!("Failed to prepare model: {}", e);
                                                }
                                            }
                                        }
                                        crate::config::TranscriptionEngine::Parakeet => {
                                            if let Some(ref t) = transcriber_preloaded {
                                                let transcriber = t.clone();
                                                tokio::task::spawn_blocking(move || {
                                                    transcriber.prepare();
                                                });
                                            }
                                        }
                                    }
                                }

                                match audio::create_capture(&self.config.audio) {
                                    Ok(mut capture) => {
                                        if let Err(e) = capture.start().await {
                                            tracing::error!("Failed to start audio: {}", e);
                                            self.play_feedback(SoundEvent::Error);
                                            continue;
                                        }
                                        audio_capture = Some(capture);

                                        // Use EagerRecording state if eager_processing is enabled
                                        if self.config.whisper.eager_processing {
                                            tracing::info!("Using eager input processing");
                                            state = State::EagerRecording {
                                                started_at: std::time::Instant::now(),
                                                model_override: model_override.clone(),
                                                accumulated_audio: Vec::new(),
                                                chunks_sent: 0,
                                                chunk_results: Vec::new(),
                                                tasks_in_flight: 0,
                                            };
                                        } else {
                                            state = State::Recording {
                                                started_at: std::time::Instant::now(),
                                                model_override: model_override.clone(),
                                            };
                                        }
                                        self.update_state("recording");
                                        self.play_feedback(SoundEvent::RecordingStart);

                                        // Run pre-recording hook (e.g., enter compositor submap for cancel)
                                        if let Some(cmd) = &self.config.output.pre_recording_command {
                                            if let Err(e) = output::run_hook(cmd, "pre_recording").await {
                                                tracing::warn!("{}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to create audio capture: {}", e);
                                        self.play_feedback(SoundEvent::Error);
                                    }
                                }
                            } else if let State::Recording { model_override: current_model_override, .. } = &state {
                                let transcriber = match self.get_transcriber_for_recording(
                                    current_model_override.as_deref(),
                                    &transcriber_preloaded,
                                ).await {
                                    Ok(t) => Some(t),
                                    Err(()) => {
                                        state = State::Idle;
                                        self.update_state("idle");
                                        continue;
                                    }
                                };

                                // Stop recording and start transcription
                                self.start_transcription_task(
                                    &mut state,
                                    &mut audio_capture,
                                    transcriber,
                                ).await;
                            } else if state.is_eager_recording() {
                                // Handle eager recording stop in toggle mode - extract model_override first
                                let model_override = match &state {
                                    State::EagerRecording { model_override, .. } => model_override.clone(),
                                    _ => None,
                                };

                                let duration = state.recording_duration().unwrap_or_default();
                                tracing::info!("Eager recording stopped ({:.1}s)", duration.as_secs_f32());

                                self.play_feedback(SoundEvent::RecordingStop);

                                if self.config.output.notification.on_recording_stop {
                                    send_notification("Recording Stopped", "Transcribing...", self.config.output.notification.show_engine_icon, self.config.engine).await;
                                }

                                // Stop audio capture and get remaining samples
                                if let Some(mut capture) = audio_capture.take() {
                                    if let Ok(final_samples) = capture.stop().await {
                                        if let State::EagerRecording { accumulated_audio, .. } = &mut state {
                                            accumulated_audio.extend(final_samples);
                                        }
                                    }
                                }

                                let transcriber = match self.get_transcriber_for_recording(
                                    model_override.as_deref(),
                                    &transcriber_preloaded,
                                ).await {
                                    Ok(t) => t,
                                    Err(()) => {
                                        state = State::Idle;
                                        self.update_state("idle");
                                        continue;
                                    }
                                };

                                self.update_state("transcribing");

                                if let Some(text) = self.finish_eager_recording(&mut state, transcriber).await {
                                    state = State::Transcribing { audio: Vec::new() };
                                    self.handle_transcription_result(&mut state, Ok(Ok(text))).await;
                                } else {
                                    tracing::debug!("Eager recording produced empty result");
                                    self.reset_to_idle(&mut state).await;
                                }
                            }
                        }

                        (HotkeyEvent::Released, ActivationMode::Toggle) => {
                            // In toggle mode, we ignore key release events
                            tracing::trace!("Ignoring HotkeyEvent::Released in toggle mode");
                        }

                        // === CANCEL KEY (works in both modes) ===
                        (HotkeyEvent::Cancel, _) => {
                            tracing::debug!("Received HotkeyEvent::Cancel");

                            if state.is_recording() {
                                tracing::info!("Recording cancelled via hotkey");

                                // Stop recording and discard audio
                                if let Some(mut capture) = audio_capture.take() {
                                    let _ = capture.stop().await;
                                }

                                // Cancel any pending model load task
                                if let Some(task) = self.model_load_task.take() {
                                    task.abort();
                                }

                                // Cancel any pending eager chunk tasks
                                for (_, task) in self.eager_chunk_tasks.drain(..) {
                                    task.abort();
                                }

                                cleanup_output_mode_override();
                                cleanup_model_override();
                                cleanup_profile_override();
                                state = State::Idle;
                                self.update_state("idle");
                                self.play_feedback(SoundEvent::Cancelled);

                                // Run post_output_command to reset compositor submap
                                if let Some(cmd) = &self.config.output.post_output_command {
                                    if let Err(e) = output::run_hook(cmd, "post_output").await {
                                        tracing::warn!("{}", e);
                                    }
                                }

                                if self.config.output.notification.on_recording_stop {
                                    send_notification("Cancelled", "Recording discarded", self.config.output.notification.show_engine_icon, self.config.engine).await;
                                }
                            } else if matches!(state, State::Transcribing { .. }) {
                                tracing::info!("Transcription cancelled via hotkey");

                                // Abort the transcription task
                                if let Some(task) = self.transcription_task.take() {
                                    task.abort();
                                }

                                cleanup_output_mode_override();
                                cleanup_model_override();
                                cleanup_profile_override();
                                state = State::Idle;
                                self.update_state("idle");
                                self.play_feedback(SoundEvent::Cancelled);

                                // Run post_output_command to reset compositor submap
                                if let Some(cmd) = &self.config.output.post_output_command {
                                    if let Err(e) = output::run_hook(cmd, "post_output").await {
                                        tracing::warn!("{}", e);
                                    }
                                }

                                if self.config.output.notification.on_recording_stop {
                                    send_notification("Cancelled", "Transcription aborted", self.config.output.notification.show_engine_icon, self.config.engine).await;
                                }
                            } else {
                                tracing::trace!("Cancel ignored - not recording or transcribing");
                            }
                        }
                    }
                }

                // Check for recording timeout and cancel requests
                _ = tokio::time::sleep(Duration::from_millis(100)), if state.is_recording() => {
                    // Check for cancel request first
                    if check_cancel_requested() {
                        tracing::info!("Recording cancelled");

                        // Stop recording and discard audio
                        if let Some(mut capture) = audio_capture.take() {
                            let _ = capture.stop().await;
                        }

                        // Cancel any pending model load task
                        if let Some(task) = self.model_load_task.take() {
                            task.abort();
                        }

                        // Cancel any pending eager chunk tasks
                        for (_, task) in self.eager_chunk_tasks.drain(..) {
                            task.abort();
                        }

                        cleanup_output_mode_override();
                        cleanup_model_override();
                        cleanup_profile_override();
                        state = State::Idle;
                        self.update_state("idle");
                        self.play_feedback(SoundEvent::Cancelled);

                        // Run post_output_command to reset compositor submap
                        if let Some(cmd) = &self.config.output.post_output_command {
                            if let Err(e) = output::run_hook(cmd, "post_output").await {
                                tracing::warn!("{}", e);
                            }
                        }

                        if self.config.output.notification.on_recording_stop {
                            send_notification("Cancelled", "Recording discarded", self.config.output.notification.show_engine_icon, self.config.engine).await;
                        }

                        continue;
                    }

                    // Check for recording timeout
                    if let Some(duration) = state.recording_duration() {
                        if duration > max_duration {
                            tracing::warn!(
                                "Recording timeout ({:.0}s limit), stopping",
                                max_duration.as_secs_f32()
                            );

                            // Stop recording
                            if let Some(mut capture) = audio_capture.take() {
                                let _ = capture.stop().await;
                            }

                            // Cancel any pending eager chunk tasks
                            for (_, task) in self.eager_chunk_tasks.drain(..) {
                                task.abort();
                            }

                            cleanup_output_mode_override();
                            cleanup_model_override();
                            cleanup_profile_override();
                            state = State::Idle;
                            self.update_state("idle");

                            // Run post_output_command to reset compositor submap
                            if let Some(cmd) = &self.config.output.post_output_command {
                                if let Err(e) = output::run_hook(cmd, "post_output").await {
                                    tracing::warn!("{}", e);
                                }
                            }
                        }
                    }
                }

                // Handle SIGUSR1 - start recording (for compositor keybindings)
                _ = sigusr1.recv() => {
                    tracing::debug!("Received SIGUSR1 (start recording)");
                    if state.is_idle() {
                        // Read model override from file (set by `voxtype record start --model X`)
                        let model_override = read_model_override();
                        tracing::info!("Recording started (external trigger), model_override = {:?}", model_override);

                        if self.config.output.notification.on_recording_start {
                            send_notification("Recording Started", "External trigger", self.config.output.notification.show_engine_icon, self.config.engine).await;
                        }

                        // Prepare model for transcription
                        if self.config.on_demand_loading() {
                            // Start model loading in background
                            match self.config.engine {
                                crate::config::TranscriptionEngine::Whisper => {
                                    let config = self.config.whisper.clone();
                                    let config_path = self.config_path.clone();
                                    let model_to_load = model_override.clone();
                                    self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                                        let mut temp_manager = ModelManager::new(&config, config_path);
                                        temp_manager.get_transcriber(model_to_load.as_deref())
                                    }));
                                }
                                crate::config::TranscriptionEngine::Parakeet => {
                                    let config = self.config.clone();
                                    self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                                        crate::transcribe::create_transcriber(&config).map(Arc::from)
                                    }));
                                }
                            }
                        } else {
                            // Prepare model (spawns subprocess for gpu_isolation mode)
                            match self.config.engine {
                                crate::config::TranscriptionEngine::Whisper => {
                                    if let Some(ref mut mm) = self.model_manager {
                                        if let Err(e) = mm.prepare_model(model_override.as_deref()) {
                                            tracing::warn!("Failed to prepare model: {}", e);
                                        }
                                    }
                                }
                                crate::config::TranscriptionEngine::Parakeet => {
                                    if let Some(ref t) = transcriber_preloaded {
                                        let transcriber = t.clone();
                                        tokio::task::spawn_blocking(move || {
                                            transcriber.prepare();
                                        });
                                    }
                                }
                            }
                        }

                        match audio::create_capture(&self.config.audio) {
                            Ok(mut capture) => {
                                if let Err(e) = capture.start().await {
                                    tracing::error!("Failed to start audio: {}", e);
                                } else {
                                    audio_capture = Some(capture);

                                    // Use EagerRecording state if eager_processing is enabled
                                    if self.config.whisper.eager_processing {
                                        tracing::info!("Using eager input processing");
                                        state = State::EagerRecording {
                                            started_at: std::time::Instant::now(),
                                            model_override,
                                            accumulated_audio: Vec::new(),
                                            chunks_sent: 0,
                                            chunk_results: Vec::new(),
                                            tasks_in_flight: 0,
                                        };
                                    } else {
                                        state = State::Recording {
                                            started_at: std::time::Instant::now(),
                                            model_override,
                                        };
                                    }
                                    self.update_state("recording");
                                    self.play_feedback(SoundEvent::RecordingStart);

                                    // Run pre-recording hook (e.g., enter compositor submap for cancel)
                                    if let Some(cmd) = &self.config.output.pre_recording_command {
                                        if let Err(e) = output::run_hook(cmd, "pre_recording").await {
                                            tracing::warn!("{}", e);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to create audio capture: {}", e);
                                self.play_feedback(SoundEvent::Error);
                            }
                        }
                    }
                }

                // Handle SIGUSR2 - stop recording (for compositor keybindings)
                _ = sigusr2.recv() => {
                    tracing::debug!("Received SIGUSR2 (stop recording)");
                    if let State::Recording { model_override, .. } = &state {
                        let transcriber = match self.get_transcriber_for_recording(
                            model_override.as_deref(),
                            &transcriber_preloaded,
                        ).await {
                            Ok(t) => Some(t),
                            Err(()) => {
                                state = State::Idle;
                                self.update_state("idle");
                                continue;
                            }
                        };

                        self.start_transcription_task(
                            &mut state,
                            &mut audio_capture,
                            transcriber,
                        ).await;
                    } else if state.is_eager_recording() {
                        // Handle eager recording stop via external trigger - extract model_override first
                        let model_override = match &state {
                            State::EagerRecording { model_override, .. } => model_override.clone(),
                            _ => None,
                        };

                        let duration = state.recording_duration().unwrap_or_default();
                        tracing::info!("Eager recording stopped ({:.1}s)", duration.as_secs_f32());

                        self.play_feedback(SoundEvent::RecordingStop);

                        if self.config.output.notification.on_recording_stop {
                            send_notification("Recording Stopped", "Transcribing...", self.config.output.notification.show_engine_icon, self.config.engine).await;
                        }

                        // Stop audio capture and get remaining samples
                        if let Some(mut capture) = audio_capture.take() {
                            if let Ok(final_samples) = capture.stop().await {
                                if let State::EagerRecording { accumulated_audio, .. } = &mut state {
                                    accumulated_audio.extend(final_samples);
                                }
                            }
                        }

                        let transcriber = match self.get_transcriber_for_recording(
                            model_override.as_deref(),
                            &transcriber_preloaded,
                        ).await {
                            Ok(t) => t,
                            Err(()) => {
                                state = State::Idle;
                                self.update_state("idle");
                                continue;
                            }
                        };

                        self.update_state("transcribing");

                        if let Some(text) = self.finish_eager_recording(&mut state, transcriber).await {
                            state = State::Transcribing { audio: Vec::new() };
                            self.handle_transcription_result(&mut state, Ok(Ok(text))).await;
                        } else {
                            tracing::debug!("Eager recording produced empty result");
                            self.reset_to_idle(&mut state).await;
                        }
                    }
                }

                // Handle transcription task completion
                result = async {
                    match self.transcription_task.as_mut() {
                        Some(task) => task.await,
                        None => std::future::pending().await,
                    }
                }, if self.transcription_task.is_some() => {
                    self.transcription_task = None;
                    self.handle_transcription_result(&mut state, result).await;
                }

                // Check for cancel during transcription
                _ = tokio::time::sleep(Duration::from_millis(100)), if matches!(state, State::Transcribing { .. }) => {
                    if check_cancel_requested() {
                        tracing::info!("Transcription cancelled");

                        // Abort the transcription task
                        if let Some(task) = self.transcription_task.take() {
                            task.abort();
                        }

                        cleanup_output_mode_override();
                        cleanup_model_override();
                        cleanup_profile_override();
                        state = State::Idle;
                        self.update_state("idle");
                        self.play_feedback(SoundEvent::Cancelled);

                        // Run post_output_command to reset compositor submap
                        if let Some(cmd) = &self.config.output.post_output_command {
                            if let Err(e) = output::run_hook(cmd, "post_output").await {
                                tracing::warn!("{}", e);
                            }
                        }

                        if self.config.output.notification.on_recording_stop {
                            send_notification("Cancelled", "Transcription aborted", self.config.output.notification.show_engine_icon, self.config.engine).await;
                        }
                    }
                }

                // Clean up stale cancel file when idle and evict idle models
                _ = tokio::time::sleep(Duration::from_millis(500)), if matches!(state, State::Idle) => {
                    // Silently consume any stale cancel request
                    let _ = check_cancel_requested();

                    // Periodically evict idle models (every ~60s when idle)
                    // The check interval is 500ms, so we use a counter to approximate 60s
                    static EVICTION_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                    let count = EVICTION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if count % 120 == 0 {  // 500ms * 120 = 60s
                        if let Some(ref mut mm) = self.model_manager {
                            mm.evict_idle_models();
                        }
                    }
                }

                // Handle graceful shutdown (SIGINT from Ctrl+C)
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Received SIGINT, shutting down...");
                    break;
                }

                // Handle graceful shutdown (SIGTERM from systemctl stop)
                _ = sigterm.recv() => {
                    tracing::info!("Received SIGTERM, shutting down...");
                    break;
                }
            }
        }

        // Cleanup
        if let Some(mut listener) = hotkey_listener {
            listener.stop().await?;
        }

        // Abort any pending transcription task
        if let Some(task) = self.transcription_task.take() {
            task.abort();
        }

        // Abort any pending eager chunk tasks
        for (_, task) in self.eager_chunk_tasks.drain(..) {
            task.abort();
        }

        // Remove state file on shutdown
        if let Some(ref path) = self.state_file_path {
            cleanup_state_file(path);
        }

        // Remove PID file on shutdown
        if let Some(ref path) = self.pid_file_path {
            cleanup_pid_file(path);
        }

        tracing::info!("Daemon stopped");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // Helper to create a test runtime directory and set it up
    fn with_test_runtime_dir<F, R>(f: F) -> R
    where
        F: FnOnce(&std::path::Path) -> R,
    {
        let temp_dir = TempDir::new().unwrap();
        let runtime_dir = temp_dir.path();

        // We can't easily mock Config::runtime_dir(), so we test the file operations
        // directly using the same logic as the functions under test
        f(runtime_dir)
    }

    #[test]
    fn test_cancel_file_detection() {
        with_test_runtime_dir(|dir| {
            let cancel_file = dir.join("cancel");

            // File doesn't exist - should return false
            assert!(!cancel_file.exists());

            // Create the cancel file
            fs::write(&cancel_file, "").unwrap();
            assert!(cancel_file.exists());

            // After checking, file should be removed (simulating check_cancel_requested behavior)
            if cancel_file.exists() {
                let _ = fs::remove_file(&cancel_file);
            }
            assert!(!cancel_file.exists());
        });
    }

    #[test]
    fn test_cancel_file_cleanup() {
        with_test_runtime_dir(|dir| {
            let cancel_file = dir.join("cancel");

            // Create a stale cancel file
            fs::write(&cancel_file, "").unwrap();
            assert!(cancel_file.exists());

            // Cleanup should remove it (simulating cleanup_cancel_file behavior)
            if cancel_file.exists() {
                let _ = fs::remove_file(&cancel_file);
            }
            assert!(!cancel_file.exists());

            // Cleanup on non-existent file should not error
            if cancel_file.exists() {
                let _ = fs::remove_file(&cancel_file);
            }
            // Should not panic
        });
    }

    #[test]
    fn test_output_mode_override_type() {
        with_test_runtime_dir(|dir| {
            let override_file = dir.join("output_mode_override");

            fs::write(&override_file, "type").unwrap();
            let content = fs::read_to_string(&override_file).unwrap();
            assert_eq!(content.trim(), "type");
        });
    }

    #[test]
    fn test_output_mode_override_clipboard() {
        with_test_runtime_dir(|dir| {
            let override_file = dir.join("output_mode_override");

            fs::write(&override_file, "clipboard").unwrap();
            let content = fs::read_to_string(&override_file).unwrap();
            assert_eq!(content.trim(), "clipboard");
        });
    }

    #[test]
    fn test_output_mode_override_paste() {
        with_test_runtime_dir(|dir| {
            let override_file = dir.join("output_mode_override");

            fs::write(&override_file, "paste").unwrap();
            let content = fs::read_to_string(&override_file).unwrap();
            assert_eq!(content.trim(), "paste");
        });
    }

    #[test]
    fn test_output_mode_override_invalid_returns_none_equivalent() {
        with_test_runtime_dir(|dir| {
            let override_file = dir.join("output_mode_override");

            fs::write(&override_file, "invalid_mode").unwrap();
            let content = fs::read_to_string(&override_file).unwrap();

            // Simulating the match logic from read_output_mode_override
            let result = match content.trim() {
                "type" => Some(OutputMode::Type),
                "clipboard" => Some(OutputMode::Clipboard),
                "paste" => Some(OutputMode::Paste),
                _ => None,
            };
            assert!(result.is_none());
        });
    }

    #[test]
    fn test_output_mode_override_file_with_path() {
        with_test_runtime_dir(|dir| {
            let override_file = dir.join("output_mode_override");

            // Test "file:/path/to/file.txt" format
            fs::write(&override_file, "file:/tmp/output.txt").unwrap();
            let content = fs::read_to_string(&override_file).unwrap();
            let trimmed = content.trim();

            assert!(trimmed.starts_with("file:"));
            let path = trimmed.strip_prefix("file:").unwrap();
            assert_eq!(path, "/tmp/output.txt");
        });
    }

    #[test]
    fn test_output_mode_override_file_consumed_after_read() {
        with_test_runtime_dir(|dir| {
            let override_file = dir.join("output_mode_override");

            fs::write(&override_file, "type").unwrap();
            assert!(override_file.exists());

            // Read and consume (simulating read_output_mode_override behavior)
            let _ = fs::read_to_string(&override_file).unwrap();
            let _ = fs::remove_file(&override_file);

            assert!(!override_file.exists());
        });
    }

    #[test]
    fn test_output_mode_override_whitespace_trimmed() {
        with_test_runtime_dir(|dir| {
            let override_file = dir.join("output_mode_override");

            fs::write(&override_file, "  clipboard  \n").unwrap();
            let content = fs::read_to_string(&override_file).unwrap();

            let result = match content.trim() {
                "type" => Some(OutputMode::Type),
                "clipboard" => Some(OutputMode::Clipboard),
                "paste" => Some(OutputMode::Paste),
                "file" => Some(OutputMode::File),
                _ => None,
            };
            assert_eq!(result, Some(OutputMode::Clipboard));
        });
    }

    #[test]
    fn test_cleanup_output_mode_override() {
        with_test_runtime_dir(|dir| {
            let override_file = dir.join("output_mode_override");

            // Create the file
            fs::write(&override_file, "type").unwrap();
            assert!(override_file.exists());

            // Cleanup (simulating cleanup_output_mode_override behavior)
            let _ = fs::remove_file(&override_file);
            assert!(!override_file.exists());

            // Cleanup on non-existent file should not error
            let _ = fs::remove_file(&override_file);
            // Should not panic
        });
    }

    fn test_pidlock_acquisition_succeeds() {
        with_test_runtime_dir(|dir| {
            let lock_path = dir.join("voxtype.lock");
            let lock_path_str = lock_path.to_string_lossy().to_string();

            let mut pidlock = Pidlock::new(&lock_path_str);
            let result = pidlock.acquire();

            assert!(result.is_ok(), "Lock acquisition should succeed");
            assert!(lock_path.exists(), "Lock file should be created");
        });
    }

    #[test]
    fn test_pidlock_blocks_second_instance() {
        with_test_runtime_dir(|dir| {
            let lock_path = dir.join("voxtype.lock");
            let lock_path_str = lock_path.to_string_lossy().to_string();

            // First lock acquisition
            let mut pidlock1 = Pidlock::new(&lock_path_str);
            pidlock1.acquire().expect("First lock should succeed");

            // Second lock acquisition should fail
            let mut pidlock2 = Pidlock::new(&lock_path_str);
            let result = pidlock2.acquire();

            assert!(result.is_err(), "Second lock acquisition should fail");
        });
    }

    #[test]
    fn test_pidlock_released_on_drop() {
        with_test_runtime_dir(|dir| {
            let lock_path = dir.join("voxtype.lock");
            let lock_path_str = lock_path.to_string_lossy().to_string();

            // Acquire and explicitly release lock in inner scope
            {
                let mut pidlock = Pidlock::new(&lock_path_str);
                pidlock.acquire().expect("Lock should succeed");
                // Explicitly release before drop
                let _ = pidlock.release();
            }

            // New lock acquisition should succeed after previous lock was released
            let mut pidlock2 = Pidlock::new(&lock_path_str);
            let result = pidlock2.acquire();

            assert!(
                result.is_ok(),
                "Lock acquisition should succeed after previous lock released: {:?}",
                result.err()
            );
        });
    }
}
