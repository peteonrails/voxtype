//! Daemon module - main event loop orchestration
//!
//! Coordinates the hotkey listener, audio capture, transcription,
//! and text output components.

use crate::audio::{self, AudioCapture};
use crate::config::Config;
use crate::error::Result;
use crate::hotkey::{self, HotkeyEvent};
use crate::output;
use crate::state::State;
use crate::transcribe;
use std::sync::Arc;
use std::time::Duration;

/// Main daemon that orchestrates all components
pub struct Daemon {
    config: Config,
}

impl Daemon {
    /// Create a new daemon with the given configuration
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Run the daemon main loop
    pub async fn run(&mut self) -> Result<()> {
        tracing::info!("Starting voxtype daemon");

        // Ensure required directories exist
        Config::ensure_directories().map_err(|e| {
            crate::error::VoxtypeError::Config(format!("Failed to create directories: {}", e))
        })?;

        tracing::info!("Hotkey: {}", self.config.hotkey.key);
        tracing::info!("Output mode: {:?}", self.config.output.mode);

        // Initialize hotkey listener
        let mut hotkey_listener = hotkey::create_listener(&self.config.hotkey)?;

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

        // Pre-load whisper model (can take a few seconds)
        tracing::info!("Loading transcription model: {}", self.config.whisper.model);
        let transcriber = Arc::new(transcribe::create_transcriber(&self.config.whisper)?);
        tracing::info!("Model loaded, ready for voice input");

        // Start hotkey listener
        let mut hotkey_rx = hotkey_listener.start().await?;

        // Current state
        let mut state = State::Idle;

        // Audio capture (created fresh for each recording)
        let mut audio_capture: Option<Box<dyn AudioCapture>> = None;

        // Recording timeout
        let max_duration = Duration::from_secs(self.config.audio.max_duration_secs as u64);

        tracing::info!(
            "Listening for hotkey: {} (hold to record, release to transcribe)",
            self.config.hotkey.key
        );

        // Main event loop
        loop {
            tokio::select! {
                // Handle hotkey events
                Some(hotkey_event) = hotkey_rx.recv() => {
                    match hotkey_event {
                        HotkeyEvent::Pressed => {
                            tracing::debug!("Received HotkeyEvent::Pressed, state.is_idle() = {}", state.is_idle());
                            if state.is_idle() {
                                tracing::info!("Recording started");

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
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to create audio capture: {}", e);
                                    }
                                }
                            }
                        }
                        
                        HotkeyEvent::Released => {
                            tracing::debug!("Received HotkeyEvent::Released, state.is_recording() = {}", state.is_recording());
                            if state.is_recording() {
                                let duration = state.recording_duration().unwrap_or_default();
                                tracing::info!("Recording stopped ({:.1}s)", duration.as_secs_f32());

                                // Stop recording and get samples
                                tracing::debug!("audio_capture.is_some() = {}", audio_capture.is_some());
                                if let Some(mut capture) = audio_capture.take() {
                                    tracing::debug!("Stopping audio capture...");
                                    match capture.stop().await {
                                        Ok(samples) => {
                                            tracing::debug!("Got {} samples from audio capture", samples.len());
                                            let audio_duration = samples.len() as f32 / 16000.0;
                                            
                                            // Skip if too short (likely accidental press)
                                            if audio_duration < 0.3 {
                                                tracing::debug!(
                                                    "Recording too short ({:.2}s), ignoring",
                                                    audio_duration
                                                );
                                                state = State::Idle;
                                                continue;
                                            }
                                            
                                            tracing::info!(
                                                "Transcribing {:.1}s of audio...",
                                                audio_duration
                                            );
                                            state = State::Transcribing { audio: samples.clone() };
                                            
                                            // Run transcription in blocking task
                                            let transcriber = transcriber.clone();
                                            let text_result = tokio::task::spawn_blocking(move || {
                                                transcriber.transcribe(&samples)
                                            })
                                            .await;
                                            
                                            match text_result {
                                                Ok(Ok(text)) => {
                                                    if text.is_empty() {
                                                        tracing::debug!("Transcription was empty");
                                                        state = State::Idle;
                                                    } else {
                                                        tracing::info!("Transcribed: {:?}", text);
                                                        
                                                        // Output the text
                                                        state = State::Outputting { text: text.clone() };
                                                        
                                                        if let Err(e) = output::output_with_fallback(
                                                            &output_chain,
                                                            &text
                                                        ).await {
                                                            tracing::error!("Output failed: {}", e);
                                                        }
                                                        
                                                        state = State::Idle;
                                                    }
                                                }
                                                Ok(Err(e)) => {
                                                    tracing::error!("Transcription failed: {}", e);
                                                    state = State::Idle;
                                                }
                                                Err(e) => {
                                                    tracing::error!("Transcription task failed: {}", e);
                                                    state = State::Idle;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("Recording error: {}", e);
                                            state = State::Idle;
                                        }
                                    }
                                } else {
                                    state = State::Idle;
                                }
                            }
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
                        }
                    }
                }
                
                // Handle graceful shutdown
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Received interrupt signal, shutting down...");
                    break;
                }
            }
        }

        // Cleanup
        hotkey_listener.stop().await?;
        tracing::info!("Daemon stopped");

        Ok(())
    }
}
