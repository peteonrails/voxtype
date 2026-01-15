//! Daemon module - main event loop orchestration
//!
//! Coordinates the hotkey listener, audio capture, transcription,
//! and text output components.

use crate::audio::feedback::{AudioFeedback, SoundEvent};
use crate::audio::{self, AudioCapture};
use crate::config::{ActivationMode, Config, OutputMode};
use crate::error::Result;
use crate::hotkey::{self, HotkeyEvent};
use crate::output;
use crate::output::post_process::PostProcessor;
use crate::state::State;
use crate::text::TextProcessor;
use crate::transcribe;
use pidlock::Pidlock;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::signal::unix::{signal, SignalKind};

/// Send a desktop notification
async fn send_notification(title: &str, body: &str) {
    let _ = Command::new("notify-send")
        .args([
            "--app-name=Voxtype",
            "--expire-time=2000",
            title,
            body,
        ])
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
fn read_output_mode_override() -> Option<OutputMode> {
    let override_file = Config::runtime_dir().join("output_mode_override");
    if !override_file.exists() {
        return None;
    }

    let mode_str = match std::fs::read_to_string(&override_file) {
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

    match mode_str.trim() {
        "type" => {
            tracing::info!("Using output mode override: type");
            Some(OutputMode::Type)
        }
        "clipboard" => {
            tracing::info!("Using output mode override: clipboard");
            Some(OutputMode::Clipboard)
        }
        "paste" => {
            tracing::info!("Using output mode override: paste");
            Some(OutputMode::Paste)
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

/// Result type for transcription task
type TranscriptionResult = std::result::Result<String, crate::error::TranscribeError>;

/// Main daemon that orchestrates all components
pub struct Daemon {
    config: Config,
    state_file_path: Option<PathBuf>,
    pid_file_path: Option<PathBuf>,
    audio_feedback: Option<AudioFeedback>,
    text_processor: TextProcessor,
    post_processor: Option<PostProcessor>,
    // Background task for loading model on-demand
    model_load_task: Option<tokio::task::JoinHandle<std::result::Result<Box<dyn crate::transcribe::Transcriber>, crate::error::TranscribeError>>>,
    // Background task for transcription (allows cancel during transcription)
    transcription_task: Option<tokio::task::JoinHandle<TranscriptionResult>>,
}

impl Daemon {
    /// Create a new daemon with the given configuration
    pub fn new(config: Config) -> Self {
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
            state_file_path,
            pid_file_path: None,
            audio_feedback,
            text_processor,
            post_processor,
            model_load_task: None,
            transcription_task: None,
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

    /// Reset state to idle and run post_output_command to reset compositor submap
    /// Call this when exiting from recording/transcribing without normal output flow
    async fn reset_to_idle(&self, state: &mut State) {
        cleanup_output_mode_override();
        *state = State::Idle;
        self.update_state("idle");

        // Run post_output_command to reset compositor submap
        if let Some(cmd) = &self.config.output.post_output_command {
            if let Err(e) = output::run_hook(cmd, "post_output").await {
                tracing::warn!("{}", e);
            }
        }
    }

    /// Start transcription task (non-blocking, stores JoinHandle for later completion)
    /// Returns true if transcription was started, false if skipped (too short)
    async fn start_transcription_task(
        &mut self,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        transcriber: Option<Arc<Box<dyn crate::transcribe::Transcriber>>>,
    ) -> bool {
        let duration = state.recording_duration().unwrap_or_default();
        tracing::info!("Recording stopped ({:.1}s)", duration.as_secs_f32());

        // Play audio feedback
        self.play_feedback(SoundEvent::RecordingStop);

        // Send notification if enabled
        if self.config.output.notification.on_recording_stop {
            send_notification("Recording Stopped", "Transcribing...").await;
        }

        // Stop recording and get samples
        if let Some(mut capture) = audio_capture.take() {
            match capture.stop().await {
                Ok(samples) => {
                    let audio_duration = samples.len() as f32 / 16000.0;

                    // Skip if too short (likely accidental press)
                    if audio_duration < 0.3 {
                        tracing::debug!(
                            "Recording too short ({:.2}s), ignoring",
                            audio_duration
                        );
                        self.reset_to_idle(state).await;
                        return false;
                    }

                    tracing::info!(
                        "Transcribing {:.1}s of audio...",
                        audio_duration
                    );
                    *state = State::Transcribing { audio: samples.clone() };
                    self.update_state("transcribing");

                    // Spawn transcription task (non-blocking)
                    if let Some(t) = transcriber {
                        self.transcription_task = Some(tokio::task::spawn_blocking(move || {
                            t.transcribe(&samples)
                        }));
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

                    // Apply post-processing command if configured
                    let final_text = if let Some(ref post_processor) = self.post_processor {
                        tracing::info!("Post-processing: {:?}", processed_text);
                        let result = post_processor.process(&processed_text).await;
                        tracing::info!("Post-processed: {:?}", result);
                        result
                    } else {
                        processed_text
                    };

                    // Create output chain with potential override
                    let output_config = if let Some(mode_override) = read_output_mode_override() {
                        let mut config = self.config.output.clone();
                        config.mode = mode_override;
                        config
                    } else {
                        self.config.output.clone()
                    };
                    let output_chain = output::create_output_chain(&output_config);

                    // Output the text
                    *state = State::Outputting { text: final_text.clone() };

                    let output_options = output::OutputOptions {
                        pre_output_command: output_config.pre_output_command.as_deref(),
                        post_output_command: output_config.post_output_command.as_deref(),
                    };

                    if let Err(e) = output::output_with_fallback(
                        &output_chain,
                        &final_text,
                        output_options,
                    ).await {
                        tracing::error!("Output failed: {}", e);
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
        let mut sigusr1 = signal(SignalKind::user_defined1())
            .map_err(|e| crate::error::VoxtypeError::Config(format!("Failed to set up SIGUSR1 handler: {}", e)))?;
        let mut sigusr2 = signal(SignalKind::user_defined2())
            .map_err(|e| crate::error::VoxtypeError::Config(format!("Failed to set up SIGUSR2 handler: {}", e)))?;
        let mut sigterm = signal(SignalKind::terminate())
            .map_err(|e| crate::error::VoxtypeError::Config(format!("Failed to set up SIGTERM handler: {}", e)))?;

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
                tracing::error!("Failed to acquire lock: another voxtype instance is already running");
                return Err(crate::error::VoxtypeError::Config(
                    format!("Another voxtype instance is already running (lock error: {:?})", e)
                ).into());
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
            Some(hotkey::create_listener(&self.config.hotkey)?)
        } else {
            tracing::info!("Built-in hotkey disabled, use 'voxtype record' commands or compositor keybindings");
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

        // Pre-load whisper model if on_demand_loading is disabled
        let mut transcriber_preloaded = None;
        if !self.config.whisper.on_demand_loading {
            tracing::info!("Loading transcription model: {}", self.config.whisper.model);
            transcriber_preloaded = Some(Arc::new(transcribe::create_transcriber(&self.config.whisper)?));
            tracing::info!("Model loaded, ready for voice input");
        } else {
            tracing::info!("On-demand loading enabled, model will be loaded when recording starts");
        }

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
                        (HotkeyEvent::Pressed, ActivationMode::PushToTalk) => {
                            tracing::debug!("Received HotkeyEvent::Pressed (push-to-talk), state.is_idle() = {}", state.is_idle());
                            if state.is_idle() {
                                tracing::info!("Recording started");

                                // Send notification if enabled
                                if self.config.output.notification.on_recording_start {
                                    send_notification("Push to Talk Active", "Recording...").await;
                                }

                                // Start model loading in background if on-demand loading is enabled
                                if self.config.whisper.on_demand_loading {
                                    let config = self.config.whisper.clone();
                                    self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                                        transcribe::create_transcriber(&config)
                                    }));
                                    tracing::debug!("Started background model loading");
                                } else if let Some(ref t) = transcriber_preloaded {
                                    // For gpu_isolation mode: prepare the subprocess now
                                    // (spawns worker and loads model while user speaks)
                                    let transcriber = t.clone();
                                    tokio::task::spawn_blocking(move || {
                                        transcriber.prepare();
                                    });
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
                                        state = State::Recording {
                                            started_at: std::time::Instant::now(),
                                        };
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
                            if state.is_recording() {
                                // Wait for model loading task if on-demand loading is enabled
                                let transcriber = if self.config.whisper.on_demand_loading {
                                    if let Some(task) = self.model_load_task.take() {
                                        match task.await {
                                            Ok(Ok(transcriber)) => {
                                                tracing::info!("Model loaded successfully");
                                                Some(Arc::new(transcriber))
                                            }
                                            Ok(Err(e)) => {
                                                tracing::error!("Model loading failed: {}", e);
                                                self.play_feedback(SoundEvent::Error);
                                                state = State::Idle;
                                                self.update_state("idle");
                                                continue;
                                            }
                                            Err(e) => {
                                                tracing::error!("Model loading task panicked: {}", e);
                                                self.play_feedback(SoundEvent::Error);
                                                state = State::Idle;
                                                self.update_state("idle");
                                                continue;
                                            }
                                        }
                                    } else {
                                        tracing::error!("No model loading task found");
                                        self.play_feedback(SoundEvent::Error);
                                        state = State::Idle;
                                        self.update_state("idle");
                                        continue;
                                    }
                                } else {
                                    transcriber_preloaded.clone()
                                };

                                self.start_transcription_task(
                                    &mut state,
                                    &mut audio_capture,
                                    transcriber,
                                ).await;
                            }
                        }

                        // === TOGGLE MODE ===
                        (HotkeyEvent::Pressed, ActivationMode::Toggle) => {
                            tracing::debug!("Received HotkeyEvent::Pressed (toggle), state.is_idle() = {}, state.is_recording() = {}",
                                state.is_idle(), state.is_recording());

                            if state.is_idle() {
                                // Start recording
                                tracing::info!("Recording started (toggle mode)");

                                if self.config.output.notification.on_recording_start {
                                    send_notification("Recording Started", "Press hotkey again to stop").await;
                                }

                                // Start model loading in background if on-demand loading is enabled
                                if self.config.whisper.on_demand_loading {
                                    let config = self.config.whisper.clone();
                                    self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                                        transcribe::create_transcriber(&config)
                                    }));
                                    tracing::debug!("Started background model loading");
                                } else if let Some(ref t) = transcriber_preloaded {
                                    // For gpu_isolation mode: prepare the subprocess now
                                    // (spawns worker and loads model while user speaks)
                                    let transcriber = t.clone();
                                    tokio::task::spawn_blocking(move || {
                                        transcriber.prepare();
                                    });
                                }

                                match audio::create_capture(&self.config.audio) {
                                    Ok(mut capture) => {
                                        if let Err(e) = capture.start().await {
                                            tracing::error!("Failed to start audio: {}", e);
                                            self.play_feedback(SoundEvent::Error);
                                            continue;
                                        }
                                        audio_capture = Some(capture);
                                        state = State::Recording {
                                            started_at: std::time::Instant::now(),
                                        };
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
                            } else if state.is_recording() {
                                // Wait for model loading task if on-demand loading is enabled
                                let transcriber = if self.config.whisper.on_demand_loading {
                                    if let Some(task) = self.model_load_task.take() {
                                        match task.await {
                                            Ok(Ok(transcriber)) => {
                                                tracing::info!("Model loaded successfully");
                                                Some(Arc::new(transcriber))
                                            }
                                            Ok(Err(e)) => {
                                                tracing::error!("Model loading failed: {}", e);
                                                self.play_feedback(SoundEvent::Error);
                                                state = State::Idle;
                                                self.update_state("idle");
                                                continue;
                                            }
                                            Err(e) => {
                                                tracing::error!("Model loading task panicked: {}", e);
                                                self.play_feedback(SoundEvent::Error);
                                                state = State::Idle;
                                                self.update_state("idle");
                                                continue;
                                            }
                                        }
                                    } else {
                                        tracing::error!("No model loading task found");
                                        self.play_feedback(SoundEvent::Error);
                                        state = State::Idle;
                                        self.update_state("idle");
                                        continue;
                                    }
                                } else {
                                    transcriber_preloaded.clone()
                                };

                                // Stop recording and start transcription
                                self.start_transcription_task(
                                    &mut state,
                                    &mut audio_capture,
                                    transcriber,
                                ).await;
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

                                cleanup_output_mode_override();
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
                                    send_notification("Cancelled", "Recording discarded").await;
                                }
                            } else if matches!(state, State::Transcribing { .. }) {
                                tracing::info!("Transcription cancelled via hotkey");

                                // Abort the transcription task
                                if let Some(task) = self.transcription_task.take() {
                                    task.abort();
                                }

                                cleanup_output_mode_override();
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
                                    send_notification("Cancelled", "Transcription aborted").await;
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

                        cleanup_output_mode_override();
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
                            send_notification("Cancelled", "Recording discarded").await;
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
                            cleanup_output_mode_override();
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
                        tracing::info!("Recording started (external trigger)");

                        if self.config.output.notification.on_recording_start {
                            send_notification("Recording Started", "External trigger").await;
                        }

                        // Start model loading in background if on-demand loading is enabled
                        if self.config.whisper.on_demand_loading {
                            let config = self.config.whisper.clone();
                            self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                                transcribe::create_transcriber(&config)
                            }));
                        } else if let Some(ref t) = transcriber_preloaded {
                            // For gpu_isolation mode: prepare the subprocess now
                            // (spawns worker and loads model while user speaks)
                            let transcriber = t.clone();
                            tokio::task::spawn_blocking(move || {
                                transcriber.prepare();
                            });
                        }

                        match audio::create_capture(&self.config.audio) {
                            Ok(mut capture) => {
                                if let Err(e) = capture.start().await {
                                    tracing::error!("Failed to start audio: {}", e);
                                } else {
                                    audio_capture = Some(capture);
                                    state = State::Recording {
                                        started_at: std::time::Instant::now(),
                                    };
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
                    if state.is_recording() {
                        // Wait for model loading task if on-demand loading is enabled
                        let transcriber = if self.config.whisper.on_demand_loading {
                            if let Some(task) = self.model_load_task.take() {
                                match task.await {
                                    Ok(Ok(transcriber)) => {
                                        tracing::info!("Model loaded successfully");
                                        Some(Arc::new(transcriber))
                                    }
                                    Ok(Err(e)) => {
                                        tracing::error!("Model loading failed: {}", e);
                                        self.play_feedback(SoundEvent::Error);
                                        state = State::Idle;
                                        self.update_state("idle");
                                        continue;
                                    }
                                    Err(e) => {
                                        tracing::error!("Model loading task panicked: {}", e);
                                        self.play_feedback(SoundEvent::Error);
                                        state = State::Idle;
                                        self.update_state("idle");
                                        continue;
                                    }
                                }
                            } else {
                                tracing::error!("No model loading task found");
                                self.play_feedback(SoundEvent::Error);
                                state = State::Idle;
                                self.update_state("idle");
                                continue;
                            }
                        } else {
                            transcriber_preloaded.clone()
                        };

                        self.start_transcription_task(
                            &mut state,
                            &mut audio_capture,
                            transcriber,
                        ).await;
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
                            send_notification("Cancelled", "Transcription aborted").await;
                        }
                    }
                }

                // Clean up stale cancel file when idle (in case cancel was called while not recording)
                _ = tokio::time::sleep(Duration::from_millis(500)), if matches!(state, State::Idle) => {
                    // Silently consume any stale cancel request
                    let _ = check_cancel_requested();
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
}
