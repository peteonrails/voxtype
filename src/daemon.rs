//! Daemon module - main event loop orchestration
//!
//! Coordinates the hotkey listener, audio capture, transcription,
//! and text output components.

use crate::audio::feedback::{AudioFeedback, SoundEvent};
use crate::audio::{self, AudioCapture};
use crate::config::{ActivationMode, Config};
use crate::error::Result;
use crate::hotkey::{self, HotkeyEvent};
use crate::output;
use crate::state::State;
use crate::text::TextProcessor;
use crate::transcribe;
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

/// Main daemon that orchestrates all components
pub struct Daemon {
    config: Config,
    state_file_path: Option<PathBuf>,
    pid_file_path: Option<PathBuf>,
    audio_feedback: Option<AudioFeedback>,
    text_processor: TextProcessor,
    // Background task for loading model on-demand
    model_load_task: Option<tokio::task::JoinHandle<std::result::Result<Box<dyn crate::transcribe::Transcriber>, crate::error::TranscribeError>>>,
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

        Self {
            config,
            state_file_path,
            pid_file_path: None,
            audio_feedback,
            text_processor,
            model_load_task: None,
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

    /// Stop recording and transcribe the audio
    async fn stop_and_transcribe(
        &self,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        transcriber: Option<Arc<Box<dyn crate::transcribe::Transcriber>>>,
        output_chain: &[Box<dyn output::TextOutput>],
    ) {
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
                        *state = State::Idle;
                        self.update_state("idle");
                        return;
                    }

                    tracing::info!(
                        "Transcribing {:.1}s of audio...",
                        audio_duration
                    );
                    *state = State::Transcribing { audio: samples.clone() };
                    self.update_state("transcribing");

                    // Run transcription in blocking task
                    let text_result = if let Some(t) = transcriber {
                        tokio::task::spawn_blocking(move || {
                            t.transcribe(&samples)
                        }).await
                    } else {
                        // This should not happen as we'll load the model on-demand
                        Ok(Err(crate::error::TranscribeError::InitFailed("No transcriber available".to_string())))
                    };

                    match text_result {
                        Ok(Ok(text)) => {
                            if text.is_empty() {
                                tracing::debug!("Transcription was empty");
                                *state = State::Idle;
                                self.update_state("idle");
                            } else {
                                tracing::info!("Transcribed: {:?}", text);

                                // Apply text processing (replacements, punctuation)
                                let processed_text = self.text_processor.process(&text);
                                if processed_text != text {
                                    tracing::debug!("After text processing: {:?}", processed_text);
                                }

                                // Output the text
                                *state = State::Outputting { text: processed_text.clone() };

                                if let Err(e) = output::output_with_fallback(
                                    output_chain,
                                    &processed_text
                                ).await {
                                    tracing::error!("Output failed: {}", e);
                                }

                                *state = State::Idle;
                                self.update_state("idle");
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::error!("Transcription failed: {}", e);
                            *state = State::Idle;
                            self.update_state("idle");
                        }
                        Err(e) => {
                            tracing::error!("Transcription task failed: {}", e);
                            *state = State::Idle;
                            self.update_state("idle");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Recording error: {}", e);
                    *state = State::Idle;
                    self.update_state("idle");
                }
            }
        } else {
            *state = State::Idle;
            self.update_state("idle");
        }
    }

    /// Run the daemon main loop
    pub async fn run(&mut self) -> Result<()> {
        tracing::info!("Starting voxtype daemon");

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

        // Initialize output chain
        let output_chain = output::create_output_chain(&self.config.output);
        tracing::debug!(
            "Output chain: {}",
            output_chain
                .iter()
                .map(|o| o.name())
                .collect::<Vec<_>>()
                .join(" -> ")
        );

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

                                self.stop_and_transcribe(
                                    &mut state,
                                    &mut audio_capture,
                                    transcriber,
                                    &output_chain,
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

                                // Stop recording and transcribe
                                self.stop_and_transcribe(
                                    &mut state,
                                    &mut audio_capture,
                                    transcriber,
                                    &output_chain,
                                ).await;
                            }
                        }

                        (HotkeyEvent::Released, ActivationMode::Toggle) => {
                            // In toggle mode, we ignore key release events
                            tracing::trace!("Ignoring HotkeyEvent::Released in toggle mode");
                        }
                    }
                }

                // Check for recording timeout
                _ = tokio::time::sleep(Duration::from_millis(100)), if state.is_recording() => {
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
                            state = State::Idle;
                            self.update_state("idle");
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

                        self.stop_and_transcribe(
                            &mut state,
                            &mut audio_capture,
                            transcriber,
                            &output_chain,
                        ).await;
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
