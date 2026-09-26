//! Daemon module - main event loop orchestration
//!
//! Coordinates the hotkey listener, audio capture, transcription,
//! and text output components.

mod deps;
#[cfg(test)]
mod harness;
mod media;
pub mod sidecar;

use deps::Deps;
use media::MediaSession;
pub use sidecar::result_sidecar_path;
use sidecar::{write_result_sidecar, write_transcription_to_file, TranscriptOutcome};

use crate::audio::feedback::{AudioFeedback, SoundEvent};
use crate::audio::{self, AudioCapture};
use crate::config::{ActivationMode, Config, FileMode, OutputMode};
use crate::eager::{self, EagerConfig};
use crate::error::Result;
#[cfg(target_os = "linux")]
use crate::hotkey::{self, HotkeyEvent};
#[cfg(target_os = "macos")]
use crate::hotkey_macos::{self as hotkey, HotkeyEvent};
use crate::meeting::{self, MeetingDaemon, MeetingEvent, StorageConfig};
use crate::model_manager::ModelManager;
use crate::notification::{self, Lifetime};
use crate::output;
use crate::output::post_process::PostProcessor;
use crate::output::streaming::StreamingSession;
use crate::output::TextOutput;
use crate::runtime_files::{OutputOverride, RuntimePaths};
use crate::state::{ChunkResult, State};
use crate::text::TextProcessor;
use crate::transcribe::{StreamHandle, StreamingEvent, Transcriber};
use pidlock::Pidlock;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal::unix::{signal, SignalKind};

/// Send a desktop notification with optional engine icon
async fn send_notification(
    title: &str,
    body: &str,
    show_engine_icon: bool,
    engine: crate::config::TranscriptionEngine,
    urgency: &str,
) {
    send_notification_with_lifetime(
        title,
        body,
        show_engine_icon,
        engine,
        urgency,
        Lifetime::Millis(2000),
    )
    .await;
}

/// As `send_notification`, but the caller decides how long it stays on screen.
async fn send_notification_with_lifetime(
    title: &str,
    body: &str,
    show_engine_icon: bool,
    engine: crate::config::TranscriptionEngine,
    urgency: &str,
    lifetime: Lifetime,
) {
    // On Linux, add emoji to title. On macOS, use content image instead.
    #[cfg(target_os = "linux")]
    let title = if show_engine_icon {
        format!("{} {}", crate::output::engine_icon(engine), title)
    } else {
        title.to_string()
    };
    #[cfg(not(target_os = "linux"))]
    let title = title.to_string();

    #[cfg(target_os = "linux")]
    {
        // Through the notification module rather than straight to
        // notify-send: that module owns the --replace-id bookkeeping that
        // keeps every Voxtype notification in a single slot. Posting from
        // here directly is why status notifications carried on stacking on
        // KDE after #532, which only fixed the module.
        notification::send_status(&title, body, urgency, lifetime).await;
    }

    #[cfg(target_os = "macos")]
    {
        // terminal-notifier has no urgency or lifetime concept; ignore both.
        let _ = (urgency, lifetime);
        let engine_for_icon = if show_engine_icon { Some(engine) } else { None };
        notification::send_with_engine(&title, body, engine_for_icon).await;
    }
}

/// Take down the recording banner now that the recording has ended.
///
/// With stop notifications enabled the stop message replaces the banner in
/// place, which is smoother than closing one and posting another. Without
/// them nothing else would ever take the banner down, so it is closed.
async fn end_recording_notification(
    title: &str,
    body: &str,
    notification_config: &crate::config::NotificationConfig,
    engine: crate::config::TranscriptionEngine,
) {
    if notification_config.on_recording_stop {
        send_notification(
            title,
            body,
            notification_config.show_engine_icon,
            engine,
            &notification_config.urgency,
        )
        .await;
    } else {
        notification::close_persistent().await;
    }
}

/// Whether a transcription task's `JoinError` means the engine that ran it
/// is suspect. A `JoinError` has exactly two causes: the daemon aborted the
/// task (a cancel — our own doing, the engine is fine) or the task panicked
/// (the engine's internal state is whatever the panic left behind). Only the
/// panic warrants discarding cached engine instances (#643).
fn join_error_poisons_engine(e: &tokio::task::JoinError) -> bool {
    !e.is_cancelled()
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

/// Check if lockfile is stale (PID no longer running) and remove it if so.
///
/// Liveness goes through `crate::daemon_status::is_running` so the daemon
/// agrees with every external caller (CLI, TUI) on what counts as a live
/// process. Previously this used a `kill(SIGCONT).is_ok() || kill(0).is_ok()`
/// pattern on Linux which delivered a real signal to whatever process held
/// the recycled PID; the unified helper uses signal 0 only.
#[cfg(unix)]
fn cleanup_stale_lockfile(lock_path: &std::path::Path) -> bool {
    if let Ok(contents) = std::fs::read_to_string(lock_path) {
        if let Ok(pid) = contents.trim().parse::<i32>() {
            // pid > 1 also rejects 0 (process-group), -1 (broadcast), and
            // init/systemd's PID 1 — none of which a user daemon could be.
            if pid > 1 && !crate::daemon_status::is_running(pid) {
                tracing::info!("Removing stale lockfile (PID {} is no longer running)", pid);
                if std::fs::remove_file(lock_path).is_ok() {
                    return true;
                }
            }
        }
    }
    false
}

/// Mark any active/paused meetings as completed on daemon startup.
/// This handles meetings orphaned by a crash or daemon restart.
fn cleanup_stale_meetings(paths: &RuntimePaths, config: &Config) {
    let storage_path = if config.meeting.storage_path == "auto" {
        Config::data_dir().join("meetings")
    } else {
        std::path::PathBuf::from(&config.meeting.storage_path)
    };

    let storage_config = StorageConfig {
        storage_path,
        retain_audio: config.meeting.retain_audio,
        max_meetings: 0,
    };

    match meeting::MeetingStorage::open(storage_config) {
        Ok(storage) => match storage.complete_stale_meetings() {
            Ok(count) if count > 0 => {
                tracing::info!("Marked {} orphaned meeting(s) as completed", count);
                // Reset meeting state file to idle
                let state_file = paths.meeting_state();
                let _ = std::fs::write(&state_file, "idle");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("Failed to clean up stale meetings: {}", e),
        },
        Err(e) => tracing::warn!("Failed to open meeting storage for cleanup: {}", e),
    }
}

/// Write meeting state file for external integrations
fn write_meeting_state_file(path: &PathBuf, state: &str, meeting_id: Option<&str>) {
    let content = if let Some(id) = meeting_id {
        format!("{}\n{}", state, id)
    } else {
        state.to_string()
    };

    if let Err(e) = std::fs::write(path, content) {
        tracing::warn!("Failed to write meeting state file: {}", e);
    }
}

/// Terminal outcome of a file-mode transcription, published beside the
/// transcript as `<transcript>.done`.
///
/// Result type for transcription task
type TranscriptionResult = std::result::Result<String, crate::error::TranscribeError>;

/// The state a live recording owns: what is capturing, what is streaming, and
/// what is draining.
///
/// These five were loop locals threaded through six methods as separate
/// `&mut Option<…>` parameters, which is why the three recording-start blocks
/// could not share a helper: each had to pass the whole set. One struct now
/// holds them, so a helper can take `&mut LiveState` instead.
#[derive(Default)]
struct LiveState {
    /// The capture for this recording, batch or streaming.
    audio_capture: Option<Box<dyn AudioCapture>>,
    /// The streaming backend's handle, while one is running.
    streaming_handle: Option<StreamHandle>,
    /// The accumulating streaming session, while one is running.
    streaming_session: Option<StreamingSession>,
    /// The output chain a streaming session types through.
    streaming_chain: Option<Vec<Box<dyn TextOutput>>>,
    /// A transcriber cached for the eager chunk pipeline.
    eager_transcriber: Option<Arc<dyn Transcriber>>,
}

/// Main daemon that orchestrates all components
pub struct Daemon {
    config: Config,
    config_path: Option<PathBuf>,
    state_file_path: Option<PathBuf>,
    paths: RuntimePaths,
    deps: Deps,
    audio_feedback: Option<AudioFeedback>,
    text_processor: TextProcessor,
    post_processor: Option<PostProcessor>,
    /// Last post-processed text and when it was produced, for context in subsequent dictations
    last_dictation: Option<(String, Instant)>,
    /// Audio level broadcaster for the OSD (None when disabled or bind failed)
    level_hub: Option<audio::levels::LevelHub>,
    /// Active per-recording level emitter task; aborted when recording stops
    level_emitter_task: Option<tokio::task::JoinHandle<()>>,
    /// Tracks time-since-last-speech for the active recording, when
    /// silence-based auto-stop is armed (external-trigger sessions only —
    /// see `new_speech_tracker`). `None` when idle or not armed for this
    /// session. Read by the periodic silence check in the main loop.
    silence_tracker: Option<audio::levels::SpeechTracker>,
    /// Whether the currently-active recording was started via an external
    /// trigger (SIGUSR1 / `record start`) rather than the hotkey. Set in
    /// `start_recording_capture`/`start_streaming_capture`, read (and
    /// cleared) by `stop_active_recording` to decide whether to run
    /// `external_trigger_stop_command`.
    is_external_trigger: bool,
    /// Synthetic zero-level publisher that keeps the OSD visible while a
    /// streaming session is draining server-side after the mic stopped.
    /// Aborted in `end_streaming`.
    streaming_drain_pump: Option<tokio::task::JoinHandle<()>>,
    /// OSD child supervisor task. Holds the JoinHandle so dropping it (on
    /// daemon shutdown) kill_on_drop's the spawned voxtype-osd process.
    osd_supervisor_task: Option<tokio::task::JoinHandle<()>>,
    // Model manager for multi-model support
    model_manager: Option<ModelManager>,
    // Background task for loading model on-demand
    model_load_task: Option<
        tokio::task::JoinHandle<
            std::result::Result<Arc<dyn Transcriber>, crate::error::TranscribeError>,
        >,
    >,
    // Background task that spawns and prepares the gpu_isolation subprocess
    // worker. Awaited before transcription so audio capture can start
    // immediately while the worker loads its model in parallel.
    whisper_prepare_task: Option<tokio::task::JoinHandle<()>>,
    // Background task for transcription (allows cancel during transcription)
    transcription_task: Option<tokio::task::JoinHandle<TranscriptionResult>>,
    // Transcriber Arc used for the in-flight transcription_task. Held so the
    // result handler can query language metadata (e.g. detected language for
    // keyboard-layout hints to eitype/dotool, see issue #180) after the task
    // completes. Cleared when transcription_task is taken.
    active_transcriber: Option<Arc<dyn Transcriber>>,
    // Engine instance preloaded at startup when on_demand_loading is off.
    // Whisper draws its transcriber from model_manager instead; every other
    // engine clones from here. A field rather than a `run()` local so the
    // panic recovery in handle_transcription_result can discard a poisoned
    // instance (#643) — get_transcriber_for_recording re-creates it on the
    // next recording when it finds this empty.
    transcriber_preloaded: Option<Arc<dyn Transcriber>>,
    // Background tasks for eager chunk transcriptions (chunk_index, task)
    eager_chunk_tasks: Vec<(
        usize,
        tokio::task::JoinHandle<std::result::Result<String, crate::error::TranscribeError>>,
    )>,
    // Voice Activity Detection (filters silence-only recordings)
    vad: Option<Box<dyn crate::vad::VoiceActivityDetector>>,
    // Meeting mode daemon (optional, created when meeting starts)
    meeting_daemon: Option<MeetingDaemon>,
    // Meeting state file path
    meeting_state_file_path: Option<PathBuf>,
    // Audio capture for meeting mode (dual: mic + loopback)
    meeting_audio_capture: Option<audio::DualCapture>,
    // Chunk buffers for meeting mode (separate mic and loopback)
    meeting_mic_buffer: Vec<f32>,
    meeting_loopback_buffer: Vec<f32>,
    // Meeting event receiver
    meeting_event_rx: Option<tokio::sync::mpsc::Receiver<MeetingEvent>>,
    // GTCRN speech enhancer for mic echo cancellation
    #[cfg(feature = "onnx-common")]
    speech_enhancer: Option<std::sync::Arc<audio::enhance::GtcrnEnhancer>>,
    // Media players that were paused when recording started (for resume on stop)
    /// The media this daemon has paused or ducked, with the fade in flight.
    media: MediaSession,
}

impl Daemon {
    /// Create a new daemon with the given configuration, resolving the runtime
    /// directory from the process environment.
    pub fn new(config: Config, config_path: Option<PathBuf>) -> Self {
        Self::with_deps(
            config,
            config_path,
            RuntimePaths::from_env(),
            Deps::production(),
        )
    }

    /// [`Daemon::new`] against a caller-supplied outside world.
    ///
    /// Every runtime file the daemon owns (the lock, the level socket, the
    /// published version, the override sentinels and the meeting triggers)
    /// resolves under `paths`; the audio device, the engine, the output drivers
    /// and the hotkey events come from `deps`. Production passes the process's
    /// own runtime directory and `Deps::production()`. A test passes a
    /// temporary directory and fakes, so a harness run never takes the running
    /// daemon's lock, consumes its sentinels, opens the microphone, loads a
    /// model or writes into the user's session.
    pub fn with_deps(
        config: Config,
        config_path: Option<PathBuf>,
        paths: RuntimePaths,
        deps: Deps,
    ) -> Self {
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
        // Pass the active engine's language so filler filtering can skip
        // words that are ordinary vocabulary there rather than disfluencies
        // (#566).
        let text_processor =
            TextProcessor::new_for_language(&config.text, config.active_language());
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

        // Initialize Voice Activity Detection if enabled
        let vad = match crate::vad::create_vad(&config) {
            Ok(Some(vad)) => {
                tracing::info!(
                    "Voice Activity Detection enabled (backend: {:?}, threshold: {:.2}, min_speech: {}ms)",
                    config.vad.backend,
                    config.vad.threshold,
                    config.vad.min_speech_duration_ms
                );
                Some(vad)
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("Failed to initialize VAD, continuing without: {}", e);
                None
            }
        };

        // Meeting state file path (separate from push-to-talk state)
        let meeting_state_file_path = if state_file_path.is_some() {
            Some(paths.meeting_state())
        } else {
            None
        };

        Self {
            config,
            config_path,
            state_file_path,
            paths,
            deps,
            audio_feedback,
            text_processor,
            post_processor,
            last_dictation: None,
            level_hub: None,
            level_emitter_task: None,
            silence_tracker: None,
            is_external_trigger: false,
            streaming_drain_pump: None,
            osd_supervisor_task: None,
            model_manager: None,
            model_load_task: None,
            whisper_prepare_task: None,
            transcription_task: None,
            active_transcriber: None,
            transcriber_preloaded: None,
            eager_chunk_tasks: Vec::new(),
            vad,
            meeting_daemon: None,
            meeting_state_file_path,
            meeting_audio_capture: None,
            meeting_mic_buffer: Vec::new(),
            meeting_loopback_buffer: Vec::new(),
            meeting_event_rx: None,
            #[cfg(feature = "onnx-common")]
            speech_enhancer: None,
            media: MediaSession::default(),
        }
    }

    /// Play audio feedback sound if enabled
    fn play_feedback(&self, event: SoundEvent) {
        if let Some(ref feedback) = self.audio_feedback {
            feedback.play(event);
        }
    }

    /// Suppress media before opening the microphone so playback cannot leak
    /// into the beginning of a recording.
    async fn suppress_recording_media(&mut self) {
        self.media.suppress(&self.config.audio).await;
    }

    /// Restore media as soon as microphone capture has stopped.
    fn restore_recording_media(&mut self) {
        self.media.restore(&self.config.audio);
    }

    /// Update the state file if configured
    fn update_state(&self, state_name: &str) {
        if let Some(ref path) = self.state_file_path {
            write_state_file(path, state_name);
        }

        // OSD suppression marker lifecycle. Consuming the sentinel here rather
        // than at output time is deliberate: the OSD appears when recording
        // starts, so the decision has to be made before the surface is drawn.
        // The marker survives the transcribing state and is cleared on the way
        // back to idle.
        match state_name {
            "recording" | "streaming" => {
                if self.paths.read_bool_override("no_osd").unwrap_or(false) {
                    self.paths.set_osd_suppressed(true);
                }
            }
            "idle" | "stopped" => self.paths.set_osd_suppressed(false),
            _ => {}
        }
    }

    /// Build a `SpeechTracker` for silence-based auto-stop, if `armed` and
    /// the feature is configured. `armed` should be `true` only for
    /// external-trigger (wake-word) sessions — see the doc comment on
    /// `silence_tracker` — never for hotkey-driven push-to-talk/toggle
    /// recordings, which already have an explicit user-driven stop.
    fn new_speech_tracker(&self, armed: bool) -> Option<audio::levels::SpeechTracker> {
        if !armed {
            return None;
        }
        self.config
            .audio
            .external_trigger_silence_timeout_secs
            .map(|timeout| {
                tracing::info!(
                    "Silence auto-stop armed for this session (timeout={:.1}s, threshold={:.1}dBFS)",
                    timeout,
                    self.config.audio.external_trigger_speech_threshold_dbfs,
                );
                audio::levels::SpeechTracker::new(
                    self.config.audio.external_trigger_speech_threshold_dbfs,
                )
            })
    }

    /// Start a batch (non-streaming) audio capture. `track_silence` arms
    /// silence-based auto-stop when the session is external-trigger and the
    /// feature is configured (see `new_speech_tracker`) — pass `false` for
    /// hotkey-driven recordings.
    async fn start_recording_capture(
        &mut self,
        track_silence: bool,
    ) -> std::result::Result<Box<dyn AudioCapture>, ()> {
        // A `record cancel` issued while idle leaves its trigger file behind,
        // and the idle-time sweep that was meant to consume it never runs:
        // its 500ms timer sits in a select! loop whose unconditional 100ms
        // poll arm recreates every timer each iteration, so the 500ms sleep
        // restarts forever. A stale trigger then kills this recording (and
        // each one after it) ~100-400ms in. Consume it here, at the single
        // point every recording path passes through, so a cancel can only
        // ever apply to a recording that was live when it was issued (#606).
        self.paths.cleanup_cancel_file();
        match self.deps.create_capture(&self.config.audio) {
            Ok(mut capture) => match capture.start().await {
                Ok(chunk_rx) => {
                    self.is_external_trigger = track_silence;
                    let speech_tracker = self.new_speech_tracker(track_silence);
                    self.silence_tracker = speech_tracker.clone();

                    // Cancel any prior emitter (defensive; should be idle).
                    if let Some(handle) = self.level_emitter_task.take() {
                        handle.abort();
                    }
                    let handle = if let Some(hub) = &self.level_hub {
                        Some(audio::levels::spawn_emitter_with_streaming_tap(
                            chunk_rx,
                            hub.frame_sink(),
                            None,
                            speech_tracker,
                        ))
                    } else {
                        // No OSD sink. If silence tracking is armed it still
                        // needs a consumer for chunk_rx — otherwise it'd
                        // just be dropped, matching the pre-tracking
                        // behaviour when tracking isn't armed either.
                        speech_tracker
                            .map(|tracker| audio::levels::spawn_silence_tracker(chunk_rx, tracker))
                    };
                    if let Some(handle) = handle {
                        self.level_emitter_task = Some(handle);
                    }
                    Ok(capture)
                }
                Err(e) => {
                    tracing::error!("Failed to start audio: {}", e);
                    self.play_feedback(SoundEvent::Error);
                    Err(())
                }
            },
            Err(e) => {
                tracing::error!("Failed to create audio capture: {}", e);
                self.play_feedback(SoundEvent::Error);
                Err(())
            }
        }
    }

    /// Stop the level emitter task (if running). The capture's chunk
    /// receiver will close when the capture itself is dropped, which would
    /// also end the emitter naturally — this just tightens the loop on
    /// state transitions.
    fn stop_level_emitter(&mut self) {
        if let Some(handle) = self.level_emitter_task.take() {
            handle.abort();
        }
    }

    /// Resolve the file-output target path for a recording, if any.
    ///
    /// Priority: 1. CLI `--file=path`, 2. CLI `--file` (config's
    /// `file_path`), 3. profile `output_mode = "file"`, 4. config
    /// `mode = "file"`. Shared by the classic (batch) transcription path
    /// and the streaming path so a `--file=` override behaves the same
    /// regardless of which one `[whisper] streaming` selects — before
    /// this was factored out, only the classic path consulted it, so a
    /// streaming session silently ignored `--file=` and typed at the
    /// cursor instead.
    fn resolve_file_output_path(
        &self,
        output_override: &Option<OutputOverride>,
        profile_output_mode: Option<OutputMode>,
    ) -> Option<PathBuf> {
        match output_override {
            // CLI --file=path.txt
            Some(OutputOverride::FileWithPath(path)) => Some(path.clone()),
            // CLI --file (no path) - use config's file_path
            Some(OutputOverride::Mode(OutputMode::File)) => self.config.output.file_path.clone(),
            // Profile specifies file mode
            None if profile_output_mode == Some(OutputMode::File) => {
                self.config.output.file_path.clone()
            }
            // Config mode = "file" (no CLI override)
            None if self.config.output.mode == OutputMode::File => {
                self.config.output.file_path.clone()
            }
            _ => None,
        }
    }

    /// Stop an eager recording: take the tail the capture still holds, hand the
    /// accumulated audio to the engine, and deliver whatever comes back.
    ///
    /// Push-to-talk and toggle share this exactly. The external path and the
    /// recording-timeout path do not: they stop the capture elsewhere, and the
    /// timeout one ends through `reset_to_idle` rather than a bare idle, so they
    /// keep their own code instead of growing flags here.
    async fn stop_eager_recording(&mut self, state: &mut State, live: &mut LiveState) {
        // Read the override before the state moves on.
        let model_override = match &*state {
            State::EagerRecording { model_override, .. } => model_override.clone(),
            _ => None,
        };

        let duration = state.recording_duration().unwrap_or_default();
        tracing::info!("Eager recording stopped ({:.1}s)", duration.as_secs_f32());

        // Stop audio capture and get the remaining samples.
        if let Some(mut capture) = live.audio_capture.take() {
            if let Ok(final_samples) = capture.stop().await {
                // Add the final samples to the accumulated audio.
                if let State::EagerRecording {
                    accumulated_audio, ..
                } = state
                {
                    accumulated_audio.extend(final_samples);
                }
            }
        }
        self.restore_recording_media();

        self.play_feedback(SoundEvent::RecordingStop);
        end_recording_notification(
            "Recording Stopped",
            "Transcribing...",
            &self.config.output.notification,
            self.config.engine,
        )
        .await;

        let transcriber = match self
            .get_transcriber_for_recording(model_override.as_deref())
            .await
        {
            Ok(t) => t,
            Err(()) => {
                // The cycle is over and it did open a capture, so end it the way
                // any other failed cycle ends: the post-output hook leaves the
                // compositor submap the pre-recording hook entered, and the
                // one-shot overrides written for this cycle are discarded
                // instead of surviving into the next recording.
                self.reset_to_idle(state).await;
                return;
            }
        };

        self.update_state("transcribing");

        if let Some(text) = self.finish_eager_recording(state, transcriber).await {
            // Move to the outputting state and handle it via the transcription
            // result flow.
            let next = state.into_transcribing(Vec::new());
            *state = next;
            self.handle_transcription_result(state, Ok(Ok(text))).await;
        } else {
            tracing::debug!("Eager recording produced empty result");
            self.reset_to_idle(state).await;
        }
        live.eager_transcriber = None;
    }

    /// Load or prepare the model for a recording that is about to start.
    ///
    /// With on-demand loading the load runs in the background, hidden behind
    /// the recording. Without it the engine is prepared up front, which for
    /// gpu isolation means spawning its worker so the first transcription does
    /// not pay for it.
    fn prepare_model_for_recording(&mut self, model_override: Option<&str>) {
        if self.config.on_demand_loading() {
            match self.config.engine {
                crate::config::TranscriptionEngine::Whisper => {
                    let config = self.config.whisper.clone();
                    let config_path = self.config_path.clone();
                    let model_to_load = model_override.map(str::to_string);
                    self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                        let mut temp_manager = ModelManager::new(&config, config_path);
                        temp_manager.get_transcriber(model_to_load.as_deref())
                    }));
                }
                // Every other engine builds its transcriber through `Deps`.
                _ => {
                    let config = self.config.clone();
                    let factories = self.deps.factories.clone();
                    self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                        factories.create_transcriber(&config).map(Arc::from)
                    }));
                }
            }
            tracing::debug!("Started background model loading");
        } else {
            match self.config.engine {
                crate::config::TranscriptionEngine::Whisper => {
                    if let Some(ref mut mm) = self.model_manager {
                        match mm.prepare_model(model_override) {
                            Ok(handle) => {
                                self.whisper_prepare_task = handle;
                            }
                            Err(e) => {
                                tracing::warn!("Failed to prepare model: {}", e);
                            }
                        }
                    }
                }
                // Every other engine builds its transcriber through `Deps`.
                _ => {
                    if let Some(ref t) = self.transcriber_preloaded {
                        let transcriber = t.clone();
                        tokio::task::spawn_blocking(move || {
                            transcriber.prepare();
                        });
                    }
                }
            }
        }
    }

    /// Begin a recording: prepare the model, open the capture or the stream, and
    /// put the state machine into the matching recording state.
    ///
    /// The three ways a recording starts — push-to-talk, toggle and the external
    /// trigger — differ only in `external`, which marks a session that tracks
    /// silence and ends through the stop hook, and in `started_by`, which is a
    /// log label. Everything else is this one sequence, so it is written once.
    async fn begin_recording(
        &mut self,
        state: &mut State,
        live: &mut LiveState,
        model_override: Option<String>,
        profile_override: Option<String>,
        external: bool,
        started_by: &str,
    ) {
        self.prepare_model_for_recording(model_override.as_deref());

        // Pause or duck playback before either capture path opens the
        // microphone.
        self.suppress_recording_media().await;

        // Try streaming first; fall through to batch if the engine doesn't
        // support streaming or setup fails.
        if self
            .try_start_streaming(state, live, model_override.clone(), external)
            .await
        {
            tracing::info!("Streaming session started ({})", started_by);
            return;
        }

        // Create and start audio capture
        tracing::debug!(
            "Creating audio capture with device: {}",
            self.config.audio.device
        );
        match self.start_recording_capture(external).await {
            Ok(capture) => {
                tracing::debug!("Audio capture started successfully");
                live.audio_capture = Some(capture);

                // Use EagerRecording state if eager_processing is enabled
                if self.config.whisper.eager_processing {
                    tracing::info!("Using eager input processing");
                    *state = State::EagerRecording {
                        started_at: std::time::Instant::now(),
                        model_override,
                        profile_override,
                        accumulated_audio: Vec::new(),
                        chunks_sent: 0,
                        chunk_results: Vec::new(),
                        tasks_in_flight: 0,
                    };
                } else {
                    *state = State::Recording {
                        started_at: std::time::Instant::now(),
                        model_override,
                        profile_override,
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
            Err(()) => {
                // Helper already logged and played the error sound.
                self.restore_recording_media();
                // The sentinel belongs to a session that never started. The
                // external path has consumed it by the time it gets here, so this
                // only ever removes a stale one; the two hotkey paths cleaned up
                // and the external one did not, a difference with no reason left
                // behind it since the CLI became the sentinel's only writer.
                self.paths.cleanup_profile_override();
            }
        }
    }

    /// Attempt to start a streaming transcription session.
    ///
    /// Returns `true` and populates the streaming locals on success. Returns
    /// `false` (and does nothing) when:
    /// - the preloaded transcriber is `None` (e.g., on_demand_loading without
    ///   a successful background load yet);
    /// - the preloaded transcriber's `as_streaming()` returns `None`;
    /// - audio capture or `start_stream` fail.
    ///
    /// On `false`, callers should fall through to the existing batch
    /// recording path.
    #[allow(clippy::too_many_arguments)]
    async fn try_start_streaming(
        &mut self,
        state: &mut State,
        live: &mut LiveState,
        model_override: Option<String>,
        track_silence: bool,
    ) -> bool {
        // Same stale-trigger hazard as start_recording_capture: Streaming is
        // an is_recording() state, so a leftover cancel file would kill the
        // session moments after it starts. See #606.
        self.paths.cleanup_cancel_file();
        // Clone out of the field so the borrow doesn't overlap the
        // `&mut self` capture start below.
        let Some(transcriber) = self.transcriber_preloaded.clone() else {
            return false;
        };
        if transcriber.as_streaming().is_none() {
            return false;
        }

        let (capture, samples_rx) = match self.start_streaming_capture(track_silence).await {
            Ok(v) => v,
            Err(()) => return false,
        };

        let streaming = transcriber.as_streaming().expect("checked above");
        let handle = match streaming.start_stream(samples_rx) {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("Failed to start streaming session: {}", e);
                self.play_feedback(SoundEvent::Error);
                // Drop the capture cleanly; ignore final samples.
                let mut c = capture;
                let _ = c.stop().await;
                return false;
            }
        };

        // A `--file=path` (or config/`mode = "file"`) override means this
        // session accumulates into `finalized_text` instead of typing —
        // see the Partial/Final/Replace arms in the event pump, gated on
        // `file_output_path`. The chain is still built normally; it's
        // simply unused for a file-output session.
        let output_override = self.paths.read_output_mode_override();
        let file_output_path = self.resolve_file_output_path(&output_override, None);

        live.audio_capture = Some(capture);
        live.streaming_handle = Some(handle);
        live.streaming_session = Some(StreamingSession::new());
        live.streaming_chain = Some(self.deps.create_output_chain(&self.config.output));
        *state = State::Streaming {
            started_at: std::time::Instant::now(),
            model_override,
            partial_buffer: String::new(),
            finalized_text: String::new(),
            typed_chars: 0,
            file_output_path,
        };
        self.update_state("streaming");
        self.play_feedback(SoundEvent::RecordingStart);

        if let Some(cmd) = &self.config.output.pre_recording_command {
            if let Err(e) = output::run_hook(cmd, "pre_recording").await {
                tracing::warn!("{}", e);
            }
        }

        if self.config.output.notification.on_recording_start {
            send_notification(
                "Streaming Active",
                "Listening...",
                self.config.output.notification.show_engine_icon,
                self.config.engine,
                &self.config.output.notification.urgency,
            )
            .await;
        }

        true
    }

    /// End an external-trigger (SIGUSR1 / `record start`) session that is
    /// being stopped on *any* path — the caller's own `record stop`, the
    /// silence timeout, the hard `max_duration_secs` cap, or a cancel — so
    /// the integration that started the recording is told it ended (it may
    /// not be the one that decided to stop it). Disarms the silence tracker
    /// and clears `is_external_trigger` unconditionally.
    ///
    /// `was_recording` gates whether the hook fires: pass `state.is_recording()`
    /// captured *before* mutating state. A stale `is_external_trigger` from an
    /// already-ended session (e.g. one cancelled outside
    /// `stop_active_recording`) must not fire the hook for an unrelated later
    /// SIGUSR2 that arrives while idle.
    async fn end_external_session(&mut self, was_recording: bool) {
        self.silence_tracker = None;
        let was_external = self.is_external_trigger && was_recording;
        self.is_external_trigger = false;
        if was_external {
            if let Some(cmd) = self.config.audio.external_trigger_stop_command.clone() {
                if let Err(e) = output::run_hook(&cmd, "external_trigger_stop").await {
                    tracing::warn!("{}", e);
                }
            }
        }
    }

    /// Stop whatever recording is currently active — streaming, batch
    /// (`State::Recording`), or eager — and hand it off to transcription.
    /// Shared by the SIGUSR2 handler (explicit `record stop` / compositor
    /// keybinding) and the silence-timeout auto-stop, which need identical
    /// per-state-variant stop behavior; the only difference between them is
    /// *why* the stop fired, not what happens next.
    async fn stop_active_recording(&mut self, state: &mut State, live: &mut LiveState) {
        // Notify an external-trigger caller that this session is ending,
        // regardless of why (stop, silence timeout, hard timeout) — it may
        // not be the one that decided to stop it — and disarm silence
        // tracking for the next session. Gated on the recording actually
        // being active: SIGUSR2 can arrive while idle, and a stale
        // `is_external_trigger` from an earlier session must not fire the
        // hook for an unrelated later signal.
        self.end_external_session(state.is_recording()).await;

        if state.is_streaming() {
            // File-output sessions (`--file=path`) have nowhere to leak
            // into — they accumulate text instead of typing it (see the
            // event pump) — so the disown-on-stop protection below
            // doesn't apply; disowning here would just silently drop
            // whatever the backend was still draining, and end_streaming
            // would then write an empty file. Let it drain normally so
            // end_streaming can write the real text.
            let file_output = matches!(
                &*state,
                State::Streaming {
                    file_output_path: Some(_),
                    ..
                }
            );
            if file_output {
                tracing::info!("Stopping streaming session (file output); closing capture");
            } else {
                tracing::info!("Stopping streaming session; closing capture and disowning session");
            }
            self.stop_streaming_capture(&mut *live).await;
            if !file_output {
                // Drop the typing surface synchronously so any
                // Final/Partial events the backend emits while
                // draining its internal buffer reach the event-pump
                // arm with `streaming_session = None` and get
                // discarded instead of typed into whatever window
                // has focus by then.
                live.streaming_session = None;
                live.streaming_chain = None;
            }
        } else if let State::Recording { model_override, .. } = &*state {
            let model_override = model_override.clone();

            self.start_transcription_task(state, live, model_override)
                .await;
        } else if state.is_eager_recording() {
            self.stop_eager_recording(state, live).await;
        }
    }

    /// End a streaming session gracefully (called when the backend emits
    /// `Ended`, or as a teardown after an error). Stops audio capture, awaits
    /// the backend task, and drops the session locals.
    /// Start the OSD "draining" pump that publishes silent frames at
    /// ~30 Hz to keep the visualizer on screen while the streaming
    /// backend is flushing pending finals after the mic has stopped.
    /// No-op if the pump is already running or the OSD level hub is
    /// disabled.
    fn start_streaming_drain_pump(&mut self) {
        if self.streaming_drain_pump.is_some() {
            return;
        }
        if let Some(hub) = &self.level_hub {
            self.streaming_drain_pump = Some(audio::levels::spawn_silence_pump(hub.frame_sink()));
        }
    }

    /// Cut audio flow to the streaming backend immediately. Aborts the
    /// chunk-rx → streaming_tx pump so any samples still in the audio
    /// capture's buffer never reach the backend — without this, the
    /// ~50–100ms of residual samples between the user's stop press and
    /// the actual mic shutdown leak in as low-level noise and cause
    /// hallucinated trailing tokens.
    fn cut_streaming_audio(&mut self) {
        if let Some(handle) = self.level_emitter_task.take() {
            handle.abort();
        }
    }

    /// Abort the OSD draining pump (if running) so the visualizer can
    /// fade out on its idle timer once the session is fully closed.
    fn stop_streaming_drain_pump(&mut self) {
        if let Some(h) = self.streaming_drain_pump.take() {
            h.abort();
        }
    }

    /// Early-stop the streaming capture: cut audio flow to the backend,
    /// start the OSD silence pump so the visualizer stays alive during
    /// drain, and stop the mic. Leaves `streaming_session`/`_chain` for
    /// the caller to disown (or keep, to receive trailing finals).
    async fn stop_streaming_capture(&mut self, live: &mut LiveState) {
        self.cut_streaming_audio();
        self.start_streaming_drain_pump();
        if let Some(mut c) = live.audio_capture.take() {
            let _ = c.stop().await;
        }
        self.restore_recording_media();
    }

    async fn end_streaming(&mut self, state: &mut State, live: &mut LiveState) {
        if let Some(mut c) = live.audio_capture.take() {
            let _ = c.stop().await;
        }
        self.restore_recording_media();
        if let Some(h) = live.streaming_handle.take() {
            // Don't error on join failure; the task may have already
            // completed. We drop events implicitly here.
            let _ = h.task.await;
        }
        self.stop_streaming_drain_pump();

        // File-output sessions (`--file=path`) never typed anything as
        // they went — see the event pump's `file_output` branch — so the
        // accumulated text only exists in the session. Write it out now,
        // before the session is dropped below. Mirrors the classic
        // (non-streaming) path's file handling, including skipping
        // post_output_command: file mode is a batch dump, not a
        // live-typing operation the hook is meant to wrap around.
        let file_output_path = match &state {
            State::Streaming {
                file_output_path, ..
            } => file_output_path.clone(),
            _ => None,
        };
        if let Some(output_path) = file_output_path {
            // Fold any leftover `partial` in first: the sliding-window
            // engine can confirm a whole short utterance as a single
            // Partial mid-session and leave nothing for `final_flush` to
            // send as Final, which would otherwise strand that text in
            // `partial` and write an empty file despite a correct
            // transcription. See `finalize_pending_partial`.
            if let Some(s) = live.streaming_session.as_mut() {
                s.finalize_pending_partial();
            }
            let final_text = live
                .streaming_session
                .as_ref()
                .map(|s| s.finalized_text().to_string())
                .unwrap_or_default();
            live.streaming_session = None;
            live.streaming_chain = None;

            // The session accumulated raw engine output (the event pump's
            // file_output branches deliberately skip per-segment
            // processing), so this is where the transcript becomes final:
            // apply replacements and spoken punctuation over the whole
            // text, same as the batch path does before writing (#669).
            let final_text = self.text_processor.process(&final_text);

            let file_mode = &self.config.output.file_mode;
            match write_transcription_to_file(&output_path, &final_text, file_mode).await {
                Ok(()) => {
                    let mode_str = match file_mode {
                        FileMode::Overwrite => "wrote",
                        FileMode::Append => "appended",
                    };
                    tracing::info!("{} streamed transcription to {:?}", mode_str, output_path);
                    self.play_feedback(SoundEvent::TranscriptionComplete);
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to write streamed transcription to {:?}: {}",
                        output_path,
                        e
                    );
                }
            }

            // The streaming file close: a dump to a file, so no post-output hook
            // and no notification; the cue above reports a successful write.
            // A streaming session delivers its segments as they finalize, so
            // nothing ever reads the one-shot boolean overrides; discard them
            // here, or `--auto-submit` written for this session leaks into the
            // next batch recording.
            self.discard_pending_overrides();
            *state = State::Idle;
            self.update_state("idle");
            return;
        }

        live.streaming_session = None;
        live.streaming_chain = None;

        self.play_feedback(SoundEvent::TranscriptionComplete);

        if let Some(cmd) = &self.config.output.post_output_command {
            if let Err(e) = output::run_hook(cmd, "post_output").await {
                tracing::warn!("{}", e);
            }
        }

        // The streaming typing close: the cue and the post-output hook run here
        // because no other path runs either for a streaming session. Nothing
        // here notifies; the only notification a streaming end can produce is
        // the caller's "Streaming Error", on the error arm.
        self.discard_pending_overrides();
        *state = State::Idle;
        self.update_state("idle");
    }

    /// Cancel an active streaming session: signal the backend, drop capture,
    /// rewind any typed text, and reset to idle (with cancel feedback +
    /// notification).
    #[allow(clippy::too_many_arguments)]
    async fn cancel_streaming_to_idle(
        &mut self,
        state: &mut State,
        live: &mut LiveState,
        notification_body: &str,
    ) {
        let backend_task = live.streaming_handle.take().map(|h| {
            let _ = h.cancel.send(());
            h.task
        });
        self.cut_streaming_audio();
        if let Some(mut c) = live.audio_capture.take() {
            let _ = c.stop().await;
        }
        self.restore_recording_media();
        if let Some(task) = backend_task {
            let _ = task.await;
        }
        if let Some(s) = live.streaming_session.as_mut() {
            if let Err(e) = s.rewind().await {
                tracing::warn!("Streaming rewind failed: {}", e);
            }
        }
        self.stop_streaming_drain_pump();
        live.streaming_session = None;
        live.streaming_chain = None;

        // A cancelled streaming session is still an ended session, so the
        // external stop hook has to run for it.
        self.close_cancelled_cycle(state, true).await;

        if self.config.output.notification.on_recording_stop {
            send_notification(
                "Cancelled",
                notification_body,
                self.config.output.notification.show_engine_icon,
                self.config.engine,
                &self.config.output.notification.urgency,
            )
            .await;
        }
    }

    /// Start a streaming-mode audio capture.
    ///
    /// Like [`start_recording_capture`] but additionally returns a receiver
    /// of audio chunks for the streaming transcription backend to consume.
    /// The OSD level emitter still runs and gets the same chunk stream
    /// (when `level_hub` is configured), so streaming and the audio-level
    /// OSD coexist without contention on the capture's mpsc.
    ///
    /// Returns `(capture, streaming_samples_rx)` on success.
    async fn start_streaming_capture(
        &mut self,
        track_silence: bool,
    ) -> std::result::Result<(Box<dyn AudioCapture>, tokio::sync::mpsc::Receiver<Vec<f32>>), ()>
    {
        match self.deps.create_capture(&self.config.audio) {
            Ok(mut capture) => match capture.start().await {
                Ok(chunk_rx) => {
                    // Bounded; backed-up streaming backend drops chunks
                    // rather than back-pressuring the capture.
                    let (streaming_tx, streaming_rx) = tokio::sync::mpsc::channel::<Vec<f32>>(64);

                    if let Some(handle) = self.level_emitter_task.take() {
                        handle.abort();
                    }
                    self.is_external_trigger = track_silence;
                    let speech_tracker = self.new_speech_tracker(track_silence);
                    self.silence_tracker = speech_tracker.clone();
                    let handle = if let Some(hub) = &self.level_hub {
                        audio::levels::spawn_emitter_with_streaming_tap(
                            chunk_rx,
                            hub.frame_sink(),
                            Some(streaming_tx),
                            speech_tracker,
                        )
                    } else {
                        // No OSD: still need to drive chunk_rx → streaming_tx,
                        // and optionally bucket for silence tracking too.
                        tokio::spawn(async move {
                            let mut rx = chunk_rx;
                            let mut bucketer = audio::levels::LevelBucketer::new();
                            let mut out = Vec::with_capacity(8);
                            while let Some(chunk) = rx.recv().await {
                                if let Some(tracker) = &speech_tracker {
                                    out.clear();
                                    bucketer.push(&chunk, &mut out);
                                    for frame in out.drain(..) {
                                        tracker.observe(frame.peak_dbfs).await;
                                    }
                                }
                                if let Err(e) = streaming_tx.try_send(chunk) {
                                    // Backend slow or gone; drop and keep going.
                                    tracing::trace!("streaming sample tap try_send failed: {}", e);
                                }
                            }
                        })
                    };
                    self.level_emitter_task = Some(handle);
                    Ok((capture, streaming_rx))
                }
                Err(e) => {
                    tracing::error!("Failed to start audio: {}", e);
                    self.play_feedback(SoundEvent::Error);
                    Err(())
                }
            },
            Err(e) => {
                tracing::error!("Failed to create audio capture: {}", e);
                self.play_feedback(SoundEvent::Error);
                Err(())
            }
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
                crate::config::TranscriptionEngine::Whisper => {
                    // Wait for the gpu_isolation worker to finish preparing
                    // (model load) before we hand the transcriber to the
                    // recording stop path. Otherwise transcribe() would race
                    // with the in-flight prepare and spawn a second worker.
                    if let Some(task) = self.whisper_prepare_task.take() {
                        if let Err(e) = task.await {
                            tracing::warn!("Whisper prepare task failed: {}", e);
                        }
                    }
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
                // Every other engine builds its transcriber through `Deps`.
                _ => {
                    if let Some(t) = self.transcriber_preloaded.clone() {
                        Ok(t)
                    } else {
                        // Empty on the non-on-demand path means the panic
                        // recovery discarded a poisoned instance (#643).
                        // Re-create rather than error so one bad recording
                        // doesn't disable every one after it.
                        tracing::info!("Transcriber not loaded; creating a fresh instance");
                        let config = self.config.clone();
                        let factories = self.deps.factories.clone();
                        let created = tokio::task::spawn_blocking(move || {
                            factories.create_transcriber(&config)
                        })
                        .await;
                        match created {
                            Ok(Ok(t)) => {
                                let t: Arc<dyn Transcriber> = Arc::from(t);
                                self.transcriber_preloaded = Some(t.clone());
                                Ok(t)
                            }
                            Ok(Err(e)) => {
                                tracing::error!("Failed to re-create transcriber: {}", e);
                                self.play_feedback(SoundEvent::Error);
                                Err(())
                            }
                            Err(e) => {
                                tracing::error!("Transcriber creation task panicked: {}", e);
                                self.play_feedback(SoundEvent::Error);
                                Err(())
                            }
                        }
                    }
                }
            }
        }
    }

    /// Update the meeting state file if configured
    fn update_meeting_state(&self, state_name: &str, meeting_id: Option<&str>) {
        if let Some(ref path) = self.meeting_state_file_path {
            write_meeting_state_file(path, state_name, meeting_id);
        }
    }

    /// Start a new meeting
    async fn start_meeting(
        &mut self,
        title: Option<String>,
        diarization_override: Option<String>,
    ) -> Result<()> {
        if self.meeting_daemon.is_some() {
            tracing::warn!("Meeting already in progress");
            return Ok(());
        }

        // CLI override (validated against ["simple", "ml"] by clap) wins over config.
        let backend = diarization_override
            .clone()
            .unwrap_or_else(|| self.config.meeting.diarization.backend.clone());

        // Create meeting config from main config
        tracing::debug!(
            "Diarization config: enabled={}, backend={} (override={:?})",
            self.config.meeting.diarization.enabled,
            backend,
            diarization_override
        );
        let diarization_config = if self.config.meeting.diarization.enabled {
            Some(meeting::diarization::DiarizationConfig {
                enabled: true,
                backend,
                max_speakers: self.config.meeting.diarization.max_speakers,
                min_segment_ms: self.config.meeting.diarization.min_segment_ms,
                model_path: self.config.meeting.diarization.model_path.clone(),
                similarity_threshold: self.config.meeting.diarization.similarity_threshold,
                vad_window_secs: self.config.meeting.diarization.vad_window_secs,
                vad_hop_secs: self.config.meeting.diarization.vad_hop_secs,
                vad_rms_floor: self.config.meeting.diarization.vad_rms_floor,
            })
        } else {
            None
        };

        let meeting_config = meeting::MeetingConfig {
            enabled: self.config.meeting.enabled,
            chunk_duration_secs: self.config.meeting.chunk_duration_secs,
            storage: StorageConfig {
                storage_path: if self.config.meeting.storage_path == "auto" {
                    Config::data_dir().join("meetings")
                } else {
                    PathBuf::from(&self.config.meeting.storage_path)
                },
                retain_audio: self.config.meeting.retain_audio,
                max_meetings: 0,
            },
            retain_audio: self.config.meeting.retain_audio,
            max_duration_mins: self.config.meeting.max_duration_mins,
            vad_threshold: self.config.meeting.audio.vad_threshold,
            diarization: diarization_config,
        };

        // Create event channel
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        self.meeting_event_rx = Some(rx);

        // Create meeting daemon
        match MeetingDaemon::new(meeting_config, &self.config, tx) {
            Ok(mut daemon) => {
                match daemon.start(title).await {
                    Ok(meeting_id) => {
                        let id_str = meeting_id.to_string();
                        self.update_meeting_state("recording", Some(&id_str));
                        tracing::info!("Meeting started: {}", meeting_id);

                        // Start dual audio capture for meeting (mic + loopback)
                        let loopback_device =
                            match self.config.meeting.audio.loopback_device.as_str() {
                                "disabled" | "" => None,
                                other => Some(other),
                            };
                        let mut meeting_audio_config = self.config.audio.clone();
                        let meeting_mic_device = self.config.meeting.audio.mic_device.as_str();
                        if !matches!(meeting_mic_device, "default" | "") {
                            tracing::info!(
                                "Meeting mic override: {} (dictation uses {})",
                                meeting_mic_device,
                                self.config.audio.device
                            );
                            meeting_audio_config.device =
                                self.config.meeting.audio.mic_device.clone();
                        }
                        match audio::DualCapture::new(&meeting_audio_config, loopback_device) {
                            Ok(mut capture) => {
                                if let Err(e) = capture.start().await {
                                    tracing::error!("Failed to start meeting audio: {}", e);
                                    let _ = daemon.stop().await;
                                    return Err(crate::error::VoxtypeError::Audio(e));
                                }
                                if capture.has_loopback() {
                                    tracing::info!("Dual audio capture: mic + loopback");
                                } else {
                                    tracing::info!("Single audio capture: mic only");
                                }
                                self.meeting_audio_capture = Some(capture);
                            }
                            Err(e) => {
                                tracing::error!("Failed to create meeting audio capture: {}", e);
                                let _ = daemon.stop().await;
                                return Err(crate::error::VoxtypeError::Audio(e));
                            }
                        }

                        // Load GTCRN speech enhancer for echo cancellation
                        #[cfg(feature = "onnx-common")]
                        if self.speech_enhancer.is_none()
                            && self.config.meeting.audio.echo_cancel != "disabled"
                        {
                            let model_path = Config::models_dir().join("gtcrn_simple.onnx");
                            if model_path.exists() {
                                match audio::enhance::GtcrnEnhancer::load(&model_path) {
                                    Ok(enhancer) => {
                                        self.speech_enhancer = Some(std::sync::Arc::new(enhancer));
                                        tracing::info!("GTCRN speech enhancer loaded for meeting echo cancellation");
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to load GTCRN enhancer, continuing without: {}",
                                            e
                                        );
                                    }
                                }
                            } else {
                                tracing::debug!(
                                    "GTCRN model not found at {:?}, skipping speech enhancement",
                                    model_path
                                );
                            }
                        }

                        self.meeting_daemon = Some(daemon);
                        self.meeting_mic_buffer.clear();
                        self.meeting_loopback_buffer.clear();

                        // Play feedback
                        self.play_feedback(SoundEvent::RecordingStart);

                        // Notification
                        if self.config.output.notification.on_recording_start {
                            send_notification(
                                "Meeting Started",
                                &format!("ID: {}", meeting_id),
                                false,
                                self.config.engine,
                                &self.config.output.notification.urgency,
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to start meeting: {}", e);
                        return Err(e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to create meeting daemon: {}", e);
                return Err(e);
            }
        }

        Ok(())
    }

    /// Stop the current meeting
    async fn stop_meeting(&mut self) -> Result<()> {
        if self.meeting_daemon.is_some() {
            // Stop audio capture and keep any samples that arrived since the last poll.
            if let Some(mut capture) = self.meeting_audio_capture.take() {
                match capture.stop().await {
                    Ok(dual_samples) => {
                        self.meeting_mic_buffer.extend(dual_samples.mic);
                        self.meeting_loopback_buffer.extend(dual_samples.loopback);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to stop meeting audio cleanly: {}", e);
                    }
                }
            }

            // Flush the final partial chunk so speech near stop is not dropped.
            self.process_buffered_meeting_audio(true).await;

            let mut daemon = self.meeting_daemon.take().expect("checked above");
            match daemon.stop().await {
                Ok(meeting_id) => {
                    self.update_meeting_state("idle", None);
                    tracing::info!("Meeting stopped: {}", meeting_id);

                    self.play_feedback(SoundEvent::RecordingStop);

                    if self.config.output.notification.on_recording_stop {
                        send_notification(
                            "Meeting Ended",
                            &format!("ID: {}", meeting_id),
                            false,
                            self.config.engine,
                            &self.config.output.notification.urgency,
                        )
                        .await;
                    }
                }
                Err(e) => {
                    tracing::error!("Error stopping meeting: {}", e);
                }
            }

            self.meeting_mic_buffer.clear();
            self.meeting_loopback_buffer.clear();
            self.meeting_event_rx = None;
        }

        Ok(())
    }

    /// Pause the current meeting
    async fn pause_meeting(&mut self) -> Result<()> {
        if let Some(ref mut daemon) = self.meeting_daemon {
            daemon.pause().await?;
            let meeting_id = daemon.current_meeting_id().map(|id| id.to_string());
            self.update_meeting_state("paused", meeting_id.as_deref());
            tracing::info!("Meeting paused");

            if self.config.output.notification.on_recording_stop {
                send_notification(
                    "Meeting Paused",
                    "Recording paused",
                    false,
                    self.config.engine,
                    &self.config.output.notification.urgency,
                )
                .await;
            }
        }
        Ok(())
    }

    /// Resume the current meeting
    async fn resume_meeting(&mut self) -> Result<()> {
        if let Some(ref mut daemon) = self.meeting_daemon {
            daemon.resume().await?;
            let meeting_id = daemon.current_meeting_id().map(|id| id.to_string());
            self.update_meeting_state("recording", meeting_id.as_deref());
            tracing::info!("Meeting resumed");

            if self.config.output.notification.on_recording_start {
                send_notification(
                    "Meeting Resumed",
                    "Recording resumed",
                    false,
                    self.config.engine,
                    &self.config.output.notification.urgency,
                )
                .await;
            }
        }
        Ok(())
    }

    /// Check if a meeting is in progress
    fn meeting_active(&self) -> bool {
        self.meeting_daemon
            .as_ref()
            .is_some_and(|d| d.state().is_active())
    }

    /// Get the chunk duration for meeting mode
    fn meeting_chunk_samples(&self) -> usize {
        // 16kHz sample rate * chunk duration in seconds
        16000 * self.config.meeting.chunk_duration_secs as usize
    }

    async fn process_meeting_audio_pair(&mut self, mic_chunk: Vec<f32>, loopback_chunk: Vec<f32>) {
        #[cfg_attr(not(feature = "onnx-common"), allow(unused_mut))]
        let mut mic_chunk = mic_chunk;

        // Enhance mic audio with GTCRN if available (removes echo/noise)
        #[cfg(feature = "onnx-common")]
        {
            if !mic_chunk.is_empty() {
                if let Some(ref enhancer) = self.speech_enhancer {
                    match enhancer.enhance(&mic_chunk) {
                        Ok(enhanced) => {
                            tracing::debug!(
                                "GTCRN enhanced mic chunk ({} samples)",
                                enhanced.len()
                            );
                            mic_chunk = enhanced;
                        }
                        Err(e) => {
                            tracing::warn!("GTCRN enhancement failed, using raw mic: {}", e);
                        }
                    }
                }
            }
        }

        if let Some(ref mut daemon) = self.meeting_daemon {
            let mut had_loopback = false;

            if !mic_chunk.is_empty() {
                match daemon
                    .process_chunk_with_source(mic_chunk, meeting::data::AudioSource::Microphone)
                    .await
                {
                    Ok(Some(segments)) => {
                        tracing::debug!("Processed mic chunk with {} segments", segments.len());
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!("Error processing mic chunk: {}", e);
                    }
                }
            }

            if !loopback_chunk.is_empty() {
                match daemon
                    .process_chunk_with_source(loopback_chunk, meeting::data::AudioSource::Loopback)
                    .await
                {
                    Ok(Some(segments)) => {
                        tracing::debug!(
                            "Processed loopback chunk with {} segments",
                            segments.len()
                        );
                        if !segments.is_empty() {
                            had_loopback = true;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!("Error processing loopback chunk: {}", e);
                    }
                }
            }

            // Reconcile per-source offsets so any source that received a short
            // or skipped chunk this iteration catches up to wall-clock before
            // the next one. Added in PR #330 to fix dual-source timestamp
            // inflation in meeting mode.
            daemon.sync_source_offsets();

            // Dedup bleed-through: strip echoed phrases from mic segments
            if had_loopback {
                if let Some(ref mut meeting) = daemon.current_meeting_mut() {
                    let removed = meeting.transcript.dedup_bleed_through();
                    if removed > 0 {
                        tracing::info!("Removed {} bleed-through word(s) via dedup", removed);
                    }
                }
            }
        }
    }

    async fn process_buffered_meeting_audio(&mut self, include_tail: bool) {
        let chunk_samples = self.meeting_chunk_samples();

        while self.meeting_mic_buffer.len() >= chunk_samples {
            let mic_chunk: Vec<f32> = self.meeting_mic_buffer.drain(..chunk_samples).collect();
            let loopback_len = self.meeting_loopback_buffer.len().min(chunk_samples);
            let loopback_chunk: Vec<f32> =
                self.meeting_loopback_buffer.drain(..loopback_len).collect();
            self.process_meeting_audio_pair(mic_chunk, loopback_chunk)
                .await;
        }

        if include_tail {
            let mic_tail = std::mem::take(&mut self.meeting_mic_buffer);
            let loopback_tail = std::mem::take(&mut self.meeting_loopback_buffer);
            if !mic_tail.is_empty() || !loopback_tail.is_empty() {
                tracing::debug!(
                    mic_samples = mic_tail.len(),
                    loopback_samples = loopback_tail.len(),
                    "Processing final meeting audio tail"
                );
                self.process_meeting_audio_pair(mic_tail, loopback_tail)
                    .await;
            }
        }
    }

    /// Tell a waiting file-mode client that this recording produced nothing.
    ///
    /// Only fires when the transcript would have gone to a file; interactive
    /// output modes have the OSD and sounds to say the same thing.
    fn publish_empty_outcome(&self) {
        if let Some(path) = self.paths.peek_file_output_path(&self.config) {
            write_result_sidecar(&path, &TranscriptOutcome::empty());
        }
    }

    /// Reset state to idle and run post_output_command to reset compositor submap
    /// Call this when exiting from recording/transcribing without normal output flow
    /// Discard the one-shot overrides a cancelled cycle will never consume.
    ///
    /// The boolean overrides are read when a transcript is *delivered*, so a
    /// cycle that ends without one has to clear them. Otherwise the
    /// `--auto-submit` or `--shift-enter` written for the recording the user
    /// just cancelled is applied to whatever unrelated recording is delivered
    /// next, which is the bug this helper exists to prevent.
    /// The shared end of a cancelled cycle, whichever way it was cancelled.
    ///
    /// What differs between cancels — which tasks are aborted, what the
    /// notification says, whether the session is external — stays at the call
    /// sites, because those are decisions. This is the part that is always the
    /// same: drop what the cycle will never consume, end an external session
    /// when there is one, publish idle, and reset a compositor submap.
    async fn close_cancelled_cycle(&mut self, state: &mut State, end_external: bool) {
        self.discard_pending_overrides();
        if end_external {
            self.end_external_session(state.is_recording()).await;
        }
        *state = State::Idle;
        self.update_state("idle");
        self.play_feedback(SoundEvent::Cancelled);

        if let Some(cmd) = &self.config.output.post_output_command {
            if let Err(e) = output::run_hook(cmd, "post_output").await {
                tracing::warn!("{}", e);
            }
        }
    }

    fn discard_pending_overrides(&self) {
        self.paths.cleanup_output_mode_override();
        self.paths.cleanup_model_override();
        self.paths.cleanup_profile_override();
        self.paths.cleanup_bool_override("auto_submit");
        self.paths.cleanup_bool_override("shift_enter");
        self.paths.cleanup_bool_override("smart_auto_submit");
    }

    async fn reset_to_idle(&mut self, state: &mut State) {
        self.discard_pending_overrides();

        // Release any model load this cycle never consumed. With
        // on_demand_loading the load starts when recording starts and is taken
        // by get_transcriber_for_recording on the way to transcription. A cycle
        // that ends before that point (recording too short, no speech detected,
        // capture failure) would otherwise leave the finished task parked in
        // this field, holding its Arc<dyn Transcriber> -- and with it the whole
        // model, hundreds of MiB -- until the next recording overwrote it.
        self.model_load_task = None;

        self.restore_recording_media();
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
            if let Some(chunk_audio) =
                eager::extract_chunk(accumulated_audio, *chunks_sent, &eager_config)
            {
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
        let chunks_sent = chunk_results
            .iter()
            .map(|r| r.chunk_index)
            .max()
            .map(|i| i + 1)
            .unwrap_or(0);
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
                match tokio::task::spawn_blocking(move || tail_transcriber.transcribe(&tail_audio))
                    .await
                {
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
                        if join_error_poisons_engine(&e) {
                            tracing::error!("Tail transcription task panicked: {}", e);
                            self.discard_poisoned_engine();
                        } else {
                            tracing::debug!("Tail transcription task was cancelled");
                        }
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
        live: &mut LiveState,
        model_override: Option<String>,
    ) -> bool {
        let duration = state.recording_duration().unwrap_or_default();
        tracing::info!("Recording stopped ({:.1}s)", duration.as_secs_f32());

        // Tear down the OSD audio-frame emitter for this session.
        self.stop_level_emitter();

        // Stop recording before waiting on model loading or doing any
        // transcription work, then restore media immediately.
        if let Some(mut capture) = live.audio_capture.take() {
            let stop_result = capture.stop().await;
            self.restore_recording_media();

            self.play_feedback(SoundEvent::RecordingStop);

            end_recording_notification(
                "Recording Stopped",
                "Transcribing...",
                &self.config.output.notification,
                self.config.engine,
            )
            .await;

            match stop_result {
                Ok(samples) => {
                    let audio_duration = samples.len() as f32 / 16000.0;

                    // Skip if too short (likely accidental press)
                    if audio_duration < 0.3 {
                        tracing::debug!("Recording too short ({:.2}s), ignoring", audio_duration);
                        self.publish_empty_outcome();
                        self.reset_to_idle(state).await;
                        return false;
                    }

                    // Voice Activity Detection: skip if no speech detected
                    if let Some(ref vad) = self.vad {
                        match vad.detect(&samples) {
                            Ok(result) if !result.has_speech => {
                                tracing::debug!(
                                    "No speech detected (speech={:.1}%, rms={:.4}), skipping transcription",
                                    result.speech_ratio * 100.0,
                                    result.rms_energy
                                );
                                self.play_feedback(SoundEvent::Cancelled);
                                self.publish_empty_outcome();
                                self.reset_to_idle(state).await;
                                return false;
                            }
                            Ok(result) => {
                                tracing::debug!(
                                    "Speech detected: {:.2}s ({:.1}%)",
                                    result.speech_duration_secs,
                                    result.speech_ratio * 100.0
                                );
                            }
                            Err(e) => {
                                // VAD failed, proceed with transcription anyway
                                tracing::warn!("VAD failed, proceeding anyway: {}", e);
                            }
                        }
                    }

                    tracing::info!("Transcribing {:.1}s of audio...", audio_duration);
                    let next = state.into_transcribing(samples.clone());
                    *state = next;
                    self.update_state("transcribing");

                    let transcriber = match self
                        .get_transcriber_for_recording(model_override.as_deref())
                        .await
                    {
                        Ok(transcriber) => transcriber,
                        Err(()) => {
                            self.reset_to_idle(state).await;
                            return false;
                        }
                    };

                    // Spawn transcription task (non-blocking)
                    // Hold an Arc clone so the result handler can query
                    // post-transcription metadata (e.g. detected language
                    // for layout hints, issue #180) without re-fetching
                    // the transcriber.
                    self.active_transcriber = Some(transcriber.clone());
                    self.transcription_task = Some(tokio::task::spawn_blocking(move || {
                        transcriber.transcribe(&samples)
                    }));
                    true
                }
                Err(e) => {
                    tracing::warn!("Recording error: {}", e);
                    self.reset_to_idle(state).await;
                    false
                }
            }
        } else {
            self.restore_recording_media();
            self.reset_to_idle(state).await;
            false
        }
    }

    /// Handle transcription completion (called when transcription_task completes)
    async fn handle_transcription_result(
        &mut self,
        state: &mut State,
        result: std::result::Result<TranscriptionResult, tokio::task::JoinError>,
    ) {
        // Take ownership of the transcriber Arc we cloned at spawn time so it
        // is dropped on every exit path (success, transcription error, or
        // task error). The Ok(Ok(_)) branch consults it for the language
        // layout hint before letting it drop.
        let active_transcriber = self.active_transcriber.take();
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

                    // Smart auto-submit: detect "submit" trigger word at end
                    // CLI override (--smart-auto-submit / --no-smart-auto-submit) takes priority
                    let smart_auto_submit_cli = self.paths.read_bool_override("smart_auto_submit");
                    let (processed_text, smart_submit) = self
                        .text_processor
                        .detect_submit(&processed_text, smart_auto_submit_cli);
                    if smart_submit {
                        tracing::debug!(
                            "Smart auto-submit triggered, stripped text: {:?}",
                            processed_text
                        );
                    }

                    // The profile this cycle was started with, carried in the
                    // state rather than laundered through a runtime file.
                    let profile_override = state.profile_override().map(str::to_string);
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

                    // Get context from last dictation if within 60 seconds
                    let recent_context = self.last_dictation.as_ref().and_then(|(text, when)| {
                        if when.elapsed() < Duration::from_secs(60) {
                            Some(text.clone())
                        } else {
                            None
                        }
                    });
                    // Apply post-processing command (profile overrides default)
                    let final_text = if let Some(profile) = active_profile {
                        if let Some(ref cmd) = profile.post_process_command {
                            let timeout_ms = profile.post_process_timeout_ms.unwrap_or(30000);
                            let profile_config = crate::config::PostProcessConfig {
                                command: cmd.clone(),
                                timeout_ms,
                                trim: true,
                                fallback_on_empty: true,
                            };
                            let profile_processor = PostProcessor::new(&profile_config);
                            tracing::info!(
                                "Post-processing with profile: {:?}, has_context: {}",
                                profile_override.as_ref().unwrap(),
                                recent_context.is_some()
                            );
                            tracing::debug!("Post-processing context: {:?}", recent_context);
                            let result = profile_processor
                                .process_with_context(&processed_text, recent_context.as_deref())
                                .await;
                            tracing::info!("Post-processed: changed: {}", result != processed_text);
                            tracing::debug!("Post-processed result: {:?}", result);
                            result
                        } else {
                            // Profile exists but has no post_process_command, use default
                            if let Some(ref post_processor) = self.post_processor {
                                tracing::info!(
                                    "Post-processing, has_context: {}",
                                    recent_context.is_some()
                                );
                                tracing::debug!(
                                    "Post-processing input: {:?}, context: {:?}",
                                    processed_text,
                                    recent_context
                                );
                                let result = post_processor
                                    .process_with_context(
                                        &processed_text,
                                        recent_context.as_deref(),
                                    )
                                    .await;
                                tracing::info!(
                                    "Post-processed: changed: {}",
                                    result != processed_text
                                );
                                tracing::debug!("Post-processed result: {:?}", result);
                                result
                            } else {
                                processed_text
                            }
                        }
                    } else if let Some(ref post_processor) = self.post_processor {
                        tracing::info!(
                            "Post-processing, has_context: {}",
                            recent_context.is_some()
                        );
                        tracing::debug!(
                            "Post-processing input: {:?}, context: {:?}",
                            processed_text,
                            recent_context
                        );
                        let result = post_processor
                            .process_with_context(&processed_text, recent_context.as_deref())
                            .await;
                        tracing::info!("Post-processed: changed: {}", result != processed_text);
                        tracing::debug!("Post-processed result: {:?}", result);
                        result
                    } else {
                        processed_text
                    };

                    // Track last dictation for context in subsequent post-processing
                    self.last_dictation = Some((final_text.clone(), Instant::now()));

                    if smart_submit {
                        tracing::debug!(
                            "Smart auto-submit: final text after post-processing: {:?}",
                            final_text
                        );
                    }

                    // Check for output mode override from CLI flags
                    let output_override = self.paths.read_output_mode_override();

                    // Check if profile specifies output mode override
                    let profile_output_mode = active_profile.and_then(|p| p.output_mode.clone());

                    // Determine file output path (if file mode)
                    let file_output_path = self
                        .resolve_file_output_path(&output_override, profile_output_mode.clone());

                    // Consume the per-recording boolean overrides before any
                    // early return below. File output returns without building
                    // an output chain, and these sentinels used to survive it:
                    // the next recording (typically the user's own hotkey, in
                    // type mode) then picked them up. Any client that passes
                    // --no-auto-submit with --file hit this on every dictation.
                    let auto_submit_override = self.paths.read_bool_override("auto_submit");
                    let shift_enter_override = self.paths.read_bool_override("shift_enter");

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
                                write_result_sidecar(
                                    &output_path,
                                    &TranscriptOutcome::ok(final_text.chars().count()),
                                );
                                self.play_feedback(SoundEvent::TranscriptionComplete);
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to write transcription to {:?}: {}",
                                    output_path,
                                    e
                                );
                                write_result_sidecar(
                                    &output_path,
                                    &TranscriptOutcome::error(&e.to_string()),
                                );
                            }
                        }

                        // The batch file close: the same file dump as the
                        // streaming file close, so no post-output hook and no
                        // notification either. The one-shot overrides were
                        // consumed near the top of this function, before the
                        // file branch could return early.
                        *state = State::Idle;
                        self.update_state("idle");
                        return;
                    }

                    // Create output chain with potential mode override (for non-file modes)
                    // Priority: 1. CLI override, 2. profile output_mode, 3. config default
                    let mut output_config = match output_override {
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

                    // Apply per-recording boolean overrides
                    if let Some(auto_submit) = auto_submit_override {
                        output_config.auto_submit = auto_submit;
                    }
                    if let Some(shift_enter) = shift_enter_override {
                        output_config.shift_enter_newlines = shift_enter;
                    }

                    // If smart auto-submit triggered, enable auto_submit for this cycle
                    if smart_submit {
                        output_config.auto_submit = true;
                    }

                    // Inject keyboard layout/variant hints derived from the
                    // transcriber's detected language (issue #180). Skipped
                    // per field when the user has already set explicit
                    // `eitype_xkb_*` / `dotool_xkb_*` values, so static
                    // configuration wins over auto-detection.
                    if let Some(ref transcriber) = active_transcriber {
                        if let Some(lang) = transcriber.last_detected_language() {
                            let applied = output_config.apply_language_xkb_hint(&lang);
                            if applied.is_empty() {
                                tracing::debug!(
                                    "No XKB mapping for detected language '{}'; \
                                     not setting a layout or variant hint",
                                    lang
                                );
                            } else {
                                if applied.eitype_layout_applied {
                                    if let Some(ref layout) = applied.layout {
                                        tracing::debug!(
                                            "Auto layout for eitype: language='{}' -> layout='{}'",
                                            lang,
                                            layout
                                        );
                                    }
                                }
                                if applied.dotool_layout_applied {
                                    if let Some(ref layout) = applied.layout {
                                        tracing::debug!(
                                            "Auto layout for dotool: language='{}' -> layout='{}'",
                                            lang,
                                            layout
                                        );
                                    }
                                }
                                if applied.eitype_variant_applied {
                                    if let Some(ref variant) = applied.variant {
                                        tracing::debug!(
                                            "Auto variant for eitype: language='{}' -> variant='{}'",
                                            lang,
                                            variant
                                        );
                                    }
                                }
                                if applied.dotool_variant_applied {
                                    if let Some(ref variant) = applied.variant {
                                        tracing::debug!(
                                            "Auto variant for dotool: language='{}' -> variant='{}'",
                                            lang,
                                            variant
                                        );
                                    }
                                }
                            }
                        }
                    }

                    let output_chain = self.deps.create_output_chain(&output_config);

                    // Output the text
                    *state = State::Outputting {
                        text: final_text.clone(),
                    };

                    let output_options = output::OutputOptions {
                        pre_output_command: output_config.pre_output_command.as_deref(),
                        post_output_command: output_config.post_output_command.as_deref(),
                        wait_for_modifier_release: output_config.wait_for_modifier_release,
                        modifier_release_timeout: std::time::Duration::from_millis(
                            output_config.modifier_release_timeout_ms,
                        ),
                    };

                    if let Err(e) =
                        output::output_with_fallback(&output_chain, &final_text, output_options)
                            .await
                    {
                        tracing::error!("Output failed: {}", e);
                    } else {
                        self.play_feedback(SoundEvent::TranscriptionComplete);

                        if self.config.output.notification.on_transcription {
                            // Send notification on successful output
                            output::send_transcription_notification(
                                &final_text,
                                self.config.output.notification.show_engine_icon,
                                self.config.engine,
                                &self.config.output.notification.urgency,
                            )
                            .await;
                        }
                    }

                    // The batch typing close: the cue and the notification both
                    // report a successful output, and the post-output hook ran
                    // inside the output chain above rather than here.
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
                if !join_error_poisons_engine(&e) {
                    tracing::debug!("Transcription task was cancelled");
                } else {
                    tracing::error!("Transcription task panicked: {}", e);
                    self.discard_poisoned_engine();
                }
                self.reset_to_idle(state).await;
            }
        }
    }

    /// Discard a cached engine whose task panicked.
    ///
    /// A panic inside `spawn_blocking` leaves the engine's internal state
    /// whatever the panic left behind, and `spawn_blocking` already kept the
    /// panic out of the daemon, so this is not about survival: it is about not
    /// reusing that engine. The next recording loads or re-creates a clean one
    /// (#643).
    ///
    /// Only for a real panic: a cancellation is our own doing and leaves the
    /// engine usable.
    fn discard_poisoned_engine(&mut self) {
        if let Some(ref mut mm) = self.model_manager {
            let dropped = mm.drop_loaded_models();
            if dropped > 0 {
                tracing::warn!(
                    "Dropped {} cached model(s) after the panic; \
                     the next recording will reload",
                    dropped
                );
            }
        }
        // model_manager only covers Whisper. Every other engine clones from
        // transcriber_preloaded, so a poisoned instance there has to go too;
        // get_transcriber_for_recording re-creates it on the next recording.
        if self.transcriber_preloaded.take().is_some() {
            tracing::warn!(
                "Dropped the preloaded transcriber after the panic; \
                 the next recording will re-create it"
            );
        }
    }

    /// Fire a desktop notification when the running binary can't service
    /// the configured engine (e.g. `engine = "parakeet"` but the wrapper
    /// dispatches to a CPU Whisper variant — the Ryan case from #450).
    /// Logged at WARN regardless, so journalctl users see it too.
    fn warn_on_variant_mismatch(&self) {
        let inventory = crate::setup::binary::inventory();
        let Some(mismatch) = crate::setup::variant_check::detect_mismatch(&self.config, &inventory)
        else {
            return;
        };

        let active = mismatch
            .active_variant_name
            .as_deref()
            .unwrap_or("the running binary");
        let title = format!("Voxtype: {} unavailable", mismatch.configured_engine);
        let body = match &mismatch.remediation {
            crate::setup::variant_check::Remediation::SwitchToVariant { target } => format!(
                "{} was built without --features {}. \
                 Run `sudo voxtype setup onnx --enable` (or open `voxtype configure` and press F2) \
                 to switch to {}.",
                active,
                mismatch.required_feature,
                target.binary_name(),
            ),
            crate::setup::variant_check::Remediation::Rebuild { feature } => format!(
                "This source build was compiled without --features {}. \
                 Rebuild voxtype with that feature to enable the {} engine.",
                feature, mismatch.configured_engine,
            ),
        };

        tracing::warn!(
            engine = mismatch.configured_engine,
            feature = mismatch.required_feature,
            active = active,
            "Variant mismatch at daemon startup: {}",
            body
        );
        crate::notification::send_sync(&title, &body);
    }

    /// Run the daemon main loop
    pub async fn run(&mut self) -> Result<()> {
        tracing::info!("Starting voxtype daemon");

        // Engine-vs-binary mismatch check at startup so users see a desktop
        // notification before the first transcription attempt would fail.
        // create_transcriber() below will surface the same error in logs,
        // but logs go to journald and most users never see them. A
        // notify-send pops up where the user is actually looking. See
        // #450 — the silent v0.6.x to v0.7.0 wrapper-flip incident.
        self.warn_on_variant_mismatch();

        // Streaming dictation types characters at the cursor while the user is
        // still holding the PTT key. On Wayland compositors backed by libinput
        // (Hyprland, Sway, River) those synthetic key events clobber the held-
        // key state tracker, so the physical key release never fires bindrd and
        // the daemon gets stuck in streaming. Force toggle activation when
        // streaming is enabled. The user's config file is left untouched; this
        // override only applies to the running daemon.
        if self.config.streaming_active()
            && self.config.hotkey.mode == crate::config::ActivationMode::PushToTalk
        {
            tracing::warn!(
                "Streaming transcription requires toggle activation, not push-to-talk. \
                 Auto-promoting [hotkey] mode from push_to_talk to toggle for this session. \
                 Streaming output types characters at the cursor while you dictate; if your \
                 PTT key is held during typing, libinput-based compositors (Hyprland, Sway, \
                 River) lose track of the held-key state and the release event never fires. \
                 Update your config to set [hotkey] mode = \"toggle\" to silence this warning."
            );
            self.config.hotkey.mode = crate::config::ActivationMode::Toggle;
        }

        // Clean up any stale cancel and profile override files from previous runs
        self.paths.cleanup_cancel_file();
        self.paths.cleanup_profile_override();

        // Clean up any stale meeting command files
        self.paths.cleanup_meeting_files();

        // Mark any orphaned active meetings as completed
        cleanup_stale_meetings(&self.paths, &self.config);

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

        // Start the audio-level broadcaster for the OSD. Failure to bind
        // the socket is not fatal: the daemon still runs without an OSD
        // feed, and downstream code treats `level_hub == None` as "no OSD".
        let level_socket = self.paths.level_socket();
        match audio::levels::LevelHub::start(level_socket.clone()).await {
            Ok(hub) => {
                tracing::info!("OSD audio level socket: {:?}", hub.socket_path());
                self.level_hub = Some(hub);
            }
            Err(e) => {
                tracing::warn!(
                    "Could not start OSD audio level socket at {:?}: {}",
                    level_socket,
                    e
                );
            }
        }

        // Spawn the OSD child if enabled and the level socket bound. Without
        // the socket the frontend has nothing to render, so skip the spawn
        // rather than burning a slot in the launcher's restart logic.
        if self.config.osd.enabled && self.level_hub.is_some() {
            self.osd_supervisor_task = Some(crate::osd::supervisor::spawn());
        }

        // Check if another instance is already running (single-instance safeguard)
        let lock_path = self.paths.lock();
        let lock_path_str = lock_path.to_string_lossy().to_string();
        let mut pidlock = Pidlock::new(&lock_path_str);

        match pidlock.acquire() {
            Ok(_) => {
                tracing::debug!("Acquired PID lock at {:?}", lock_path);
            }
            Err(_) => {
                // Check if the lock is stale (previous daemon crashed)
                #[cfg(unix)]
                if cleanup_stale_lockfile(&lock_path) {
                    // Try again after removing stale lock
                    pidlock = Pidlock::new(&lock_path_str);
                    if let Err(e) = pidlock.acquire() {
                        tracing::error!("Failed to acquire lock after stale cleanup: {:?}", e);
                        return Err(crate::error::VoxtypeError::Config(format!(
                            "Another voxtype instance is already running (lock error: {:?})",
                            e
                        )));
                    }
                    tracing::debug!("Acquired PID lock at {:?} (after stale cleanup)", lock_path);
                } else {
                    tracing::error!(
                        "Failed to acquire lock: another voxtype instance is already running"
                    );
                    return Err(crate::error::VoxtypeError::Config(
                        "Another voxtype instance is already running".to_string(),
                    ));
                }
                #[cfg(not(unix))]
                {
                    tracing::error!(
                        "Failed to acquire lock: another voxtype instance is already running"
                    );
                    return Err(crate::error::VoxtypeError::Config(
                        "Another voxtype instance is already running".to_string(),
                    )
                    .into());
                }
            }
        }

        // Only now that the lock is ours: a refused second instance must not
        // overwrite the running daemon's answer with its own version.
        crate::daemon_status::publish_version_in(self.paths.dir());

        tracing::info!("Output mode: {:?}", self.config.output.mode);

        // Log state file if configured
        if let Some(ref path) = self.state_file_path {
            tracing::info!("State file: {:?}", path);
        }

        // Warn about profile modifiers that reference undefined profiles. Runs
        // before either platform's hotkey listener is created so the warning
        // surfaces regardless of evdev/rdev backend.
        if self.config.hotkey.enabled {
            for (key_name, profile_name) in &self.config.hotkey.profile_modifiers {
                if self.config.get_profile(profile_name).is_none() {
                    tracing::warn!(
                        "Profile modifier {} references undefined profile '{}' — \
                         add a [profiles.{}] section to your config",
                        key_name,
                        profile_name,
                        profile_name
                    );
                }
            }
        }

        // Initialize hotkey listener (Linux: evdev, macOS: rdev)
        #[cfg(target_os = "linux")]
        let mut hotkey_listener: Option<Box<dyn hotkey::HotkeyListener>> =
            if self.deps.hotkey_events.is_some() {
                // An injected event source replaces the listener, so nothing
                // opens the input device.
                None
            } else if self.config.hotkey.enabled {
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

        #[cfg(target_os = "macos")]
        let mut hotkey_listener: Option<Box<dyn hotkey::HotkeyListener>> = if self
            .deps
            .hotkey_events
            .is_some()
        {
            // An injected event source replaces the listener, so nothing
            // opens the input device.
            None
        } else if self.config.hotkey.enabled {
            tracing::info!("Hotkey: {}", self.config.hotkey.key);
            let secondary_model = self.config.whisper.secondary_model.clone();
            match hotkey::create_listener(&self.config.hotkey, secondary_model) {
                Ok(listener) => Some(listener),
                Err(e) => {
                    tracing::warn!("Failed to create hotkey listener: {}. Use 'voxtype record' commands instead.", e);
                    None
                }
            }
        } else {
            tracing::info!(
                "Built-in hotkey disabled, use 'voxtype record' commands or compositor keybindings"
            );
            None
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let hotkey_listener: Option<()> = {
            if self.config.hotkey.enabled {
                tracing::warn!(
                    "Built-in hotkey not supported on this platform, use 'voxtype record' commands"
                );
            }
            None
        };

        // Log default output chain (chain is created dynamically per-transcription to support overrides)
        let default_chain = self.deps.create_output_chain(&self.config.output);
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
        if !self.config.on_demand_loading() {
            tracing::info!("Loading transcription model: {}", self.config.model_name());
            match self.config.engine {
                crate::config::TranscriptionEngine::Whisper => {
                    if self.config.whisper.streaming {
                        // Streaming needs the transcriber in `transcriber_preloaded`
                        // so try_start_streaming can find it. The factory returns
                        // the sliding-window wrapper when [whisper] streaming = true.
                        if self.config.whisper.on_demand_loading {
                            tracing::warn!(
                                "[whisper] streaming requires on_demand_loading = false; \
                                 streaming will be unavailable"
                            );
                        }
                        self.transcriber_preloaded =
                            Some(Arc::from(self.deps.create_transcriber(&self.config)?));
                    } else {
                        // Use model manager for Whisper
                        if let Err(e) = model_manager.preload_primary() {
                            tracing::error!("Failed to preload model: {}", e);
                            return Err(crate::error::VoxtypeError::Transcribe(e));
                        }
                    }
                }
                // Every other engine builds its transcriber through `Deps`.
                _ => {
                    // Non-Whisper engines do their own setup; Soniox just validates
                    // API key + endpoint at construction (no model to download).
                    self.transcriber_preloaded =
                        Some(Arc::from(self.deps.create_transcriber(&self.config)?));
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
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let mut hotkey_rx = match self.deps.hotkey_events.take() {
            // An injected source drives the loop directly: the test keeps the
            // sender and no input device is opened.
            Some(rx) => Some(rx),
            None => match &mut hotkey_listener {
                Some(listener) => match listener.start() {
                    Ok(rx) => Some(rx),
                    Err(e) => {
                        tracing::warn!("Failed to start hotkey listener: {}. Use 'voxtype record' commands instead.", e);
                        None
                    }
                },
                None => None,
            },
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let mut hotkey_rx: Option<tokio::sync::mpsc::Receiver<HotkeyEvent>> = None;

        // Current state
        let mut state = State::Idle;

        // The capture, streaming handle and session, and the eager transcriber
        // for the recording in flight: one bundle, because every recording path
        // hands the whole set around.
        let mut live = LiveState::default();

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
                        (HotkeyEvent::Pressed { model_override, profile_override }, ActivationMode::PushToTalk) => {
                            tracing::debug!("Received HotkeyEvent::Pressed (push-to-talk), state.is_idle() = {}, model_override = {:?}, profile_override = {:?}",
                                state.is_idle(), model_override, profile_override);
                            if state.is_idle() {
                                // A profile sentinel is written by `voxtype
                                // record start --profile` for an external
                                // start, which reads it. A hotkey cycle carries
                                // its profile in the state, so a sentinel still
                                // on disk is stale: it must not survive to
                                // post-process a later recording.
                                self.paths.cleanup_profile_override();

                                tracing::info!("Recording started");

                                // Send notification if enabled
                                if self.config.output.notification.on_recording_start {
                                    send_notification_with_lifetime("Push to Talk Active", "Recording...", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency, Lifetime::UntilClosed).await;
                                }

                                self.begin_recording(
                                    &mut state,
                                    &mut live,
                                    model_override,
                                    profile_override,
                                    false,
                                    "push-to-talk",
                                )
                                .await;
                            }
                        }

                        (HotkeyEvent::Released, ActivationMode::PushToTalk) => {
                            tracing::debug!("Received HotkeyEvent::Released (push-to-talk), state.is_recording() = {}", state.is_recording());
                            if state.is_streaming() {
                                tracing::debug!("Streaming push-to-talk released; closing audio capture and disowning session");
                                self.stop_streaming_capture(&mut live).await;
                                // Drop session/chain so the backend's
                                // post-stop flush emission is dropped at
                                // the event pump instead of typed.
                                // Matches the SIGUSR2 stop path.
                                live.streaming_session = None;
                                live.streaming_chain = None;
                            } else if let State::Recording { model_override, .. } = &state {
                                let model_override = model_override.clone();

                                self.start_transcription_task(
                                    &mut state,
                                    &mut live,
                                    model_override,
                                ).await;
                            } else if state.is_eager_recording() {
                                self.stop_eager_recording(&mut state, &mut live).await;
                            }
                        }

                        // === TOGGLE MODE ===
                        (HotkeyEvent::Pressed { model_override, profile_override }, ActivationMode::Toggle) => {
                            tracing::debug!("Received HotkeyEvent::Pressed (toggle), state.is_idle() = {}, state.is_recording() = {}, model_override = {:?}, profile_override = {:?}",
                                state.is_idle(), state.is_recording(), model_override, profile_override);

                            if state.is_idle() {
                                // A profile sentinel is written by `voxtype
                                // record start --profile` for an external
                                // start, which reads it. A hotkey cycle carries
                                // its profile in the state, so a sentinel still
                                // on disk is stale: it must not survive to
                                // post-process a later recording.
                                self.paths.cleanup_profile_override();

                                // Start recording
                                tracing::info!("Recording started (toggle mode)");

                                if self.config.output.notification.on_recording_start {
                                    send_notification_with_lifetime("Recording Started", "Press hotkey again to stop", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency, Lifetime::UntilClosed).await;
                                }

                                self.begin_recording(
                                    &mut state,
                                    &mut live,
                                    model_override,
                                    profile_override,
                                    false,
                                    "toggle",
                                )
                                .await;
                            } else if state.is_streaming() {
                                tracing::info!("Toggle stop while streaming; closing capture");
                                self.stop_streaming_capture(&mut live).await;
                            } else if let State::Recording { model_override: current_model_override, .. } = &state {
                                let model_override = current_model_override.clone();

                                // Stop recording and start transcription
                                self.start_transcription_task(
                                    &mut state,
                                    &mut live,
                                    model_override,
                                ).await;
                            } else if state.is_eager_recording() {
                                self.stop_eager_recording(&mut state, &mut live).await;
                            }
                        }

                        (HotkeyEvent::Released, ActivationMode::Toggle) => {
                            // In toggle mode, we ignore key release events
                            tracing::trace!("Ignoring HotkeyEvent::Released in toggle mode");
                        }

                        // === CANCEL KEY (works in both modes) ===
                        (HotkeyEvent::Cancel, _) => {
                            tracing::debug!("Received HotkeyEvent::Cancel");

                            if state.is_streaming() {
                                tracing::info!("Streaming cancelled via hotkey");
                                self.cancel_streaming_to_idle(
                                    &mut state,
                                    &mut live,
                                    "Recording discarded",
                                ).await;
                            } else if state.is_recording() {
                                tracing::info!("Recording cancelled via hotkey");

                                // A cancelled external-trigger session is
                                // still an ended session — tell the caller
                                // and disarm tracking.
                                self.end_external_session(state.is_recording()).await;

                                // Stop recording and discard audio
                                if let Some(mut capture) = live.audio_capture.take() {
                                    let _ = capture.stop().await;
                                }
                                self.restore_recording_media();

                                // Cancel any pending model load task
                                if let Some(task) = self.model_load_task.take() {
                                    task.abort();
                                }

                                // Cancel any pending eager chunk tasks
                                for (_, task) in self.eager_chunk_tasks.drain(..) {
                                    task.abort();
                                }

                                self.close_cancelled_cycle(&mut state, false).await;

                                end_recording_notification("Cancelled", "Recording discarded", &self.config.output.notification, self.config.engine).await;
                            } else if matches!(state, State::Transcribing { .. }) {
                                tracing::info!("Transcription cancelled via hotkey");

                                // Abort the transcription task
                                if let Some(task) = self.transcription_task.take() {
                                    task.abort();
                                }
                                // Drop the cloned transcriber Arc so it isn't
                                // held until the next transcription.
                                self.active_transcriber = None;

                                self.close_cancelled_cycle(&mut state, false).await;

                                end_recording_notification("Cancelled", "Transcription aborted", &self.config.output.notification, self.config.engine).await;
                            } else {
                                tracing::trace!("Cancel ignored - not recording or transcribing");
                            }
                        }
                    }
                }

                // Check for recording timeout and cancel requests
                _ = tokio::time::sleep(Duration::from_millis(100)), if state.is_recording() => {
                    // Check for cancel request first
                    if self.paths.check_cancel_requested() {
                        tracing::info!("Recording cancelled");

                        // Stop recording and discard audio
                        if let Some(mut capture) = live.audio_capture.take() {
                            let _ = capture.stop().await;
                        }
                        self.restore_recording_media();

                        // Cancel any pending model load task
                        if let Some(task) = self.model_load_task.take() {
                            task.abort();
                        }

                        // Cancel any pending eager chunk tasks
                        for (_, task) in self.eager_chunk_tasks.drain(..) {
                            task.abort();
                        }

                        if let State::EagerRecording {
                            accumulated_audio,
                            chunk_results,
                            chunks_sent,
                            tasks_in_flight,
                            ..
                        } = &mut state
                        {
                            accumulated_audio.clear();
                            chunk_results.clear();
                            *chunks_sent = 0;
                            *tasks_in_flight = 0;
                        }

                        // A cancelled external-trigger session is still an
                        // ended session: tell the caller and disarm tracking.
                        self.close_cancelled_cycle(&mut state, true).await;
                        live.eager_transcriber = None;

                        end_recording_notification("Cancelled", "Recording discarded", &self.config.output.notification, self.config.engine).await;

                        continue;
                    }

                    // Populate eager transcriber cache on first poll
                    if live.eager_transcriber.is_none() && state.is_eager_recording() {
                        let model_override = match &state {
                            State::EagerRecording { model_override, .. } => model_override.as_deref(),
                            _ => None,
                        };
                        live.eager_transcriber = self.transcriber_preloaded.clone();
                        if live.eager_transcriber.is_none()
                            && self.config.engine
                                == crate::config::TranscriptionEngine::Whisper
                        {
                            // Whisper engine: get from model manager
                            if let Some(ref mut mm) = self.model_manager {
                                match mm.get_prepared_transcriber(model_override) {
                                    Ok(t) => {
                                        tracing::debug!("Created eager transcriber for chunk dispatch");
                                        live.eager_transcriber = Some(t);
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to create eager transcriber: {}", e);
                                    }
                                }
                            }
                        }
                    }

                    if let State::EagerRecording {
                        accumulated_audio,
                        chunks_sent,
                        chunk_results,
                        tasks_in_flight,
                        ..
                    } = &mut state
                    {
                        if let Some(ref mut capture) = live.audio_capture {
                            let new_samples = capture.get_samples().await;
                            if !new_samples.is_empty() {
                                accumulated_audio.extend(new_samples);
                            }
                        }

                        if let Some(ref transcriber) = live.eager_transcriber {
                            let transcriber = transcriber.clone();
                            self.process_eager_chunks(
                                accumulated_audio,
                                chunks_sent,
                                tasks_in_flight,
                                &transcriber,
                            );
                        }

                        let completed = self.poll_chunk_tasks().await;
                        if !completed.is_empty() {
                            *tasks_in_flight = tasks_in_flight.saturating_sub(completed.len());
                            chunk_results.extend(completed);
                        }
                    }

                    // Silence-based auto-stop for external-trigger
                    // (wake-word) sessions. Checked from this tick rather
                    // than a standalone sleep arm: `loop { select! }` rebuilds
                    // every arm future each iteration, so a 300 ms sleep arm
                    // here was permanently starved by this same 100 ms tick
                    // and never fired. 100 ms granularity is plenty for a
                    // multi-second threshold.
                    if self.is_external_trigger && state.is_recording() {
                        if let Some(tracker) = &self.silence_tracker {
                            if let Some(timeout_secs) =
                                self.config.audio.external_trigger_silence_timeout_secs
                            {
                                let elapsed = tracker.silence_elapsed().await;
                                if elapsed.as_secs_f32() >= timeout_secs {
                                    tracing::info!(
                                        "Silence timeout ({:.1}s >= {:.1}s), auto-stopping",
                                        elapsed.as_secs_f32(),
                                        timeout_secs
                                    );
                                    self.stop_active_recording(
                                        &mut state,
                                        &mut live,
                                    )
                                    .await;
                                    continue;
                                }
                            }
                        }
                    }

                    // Check for recording timeout. Skip when audio_capture is
                    // already gone so we don't re-fire cleanup on every 100ms
                    // tick while the streaming session drains server-side
                    // (state stays Streaming until Ended arrives).
                    let timeout_fired = live.audio_capture.is_some()
                        && state.recording_duration().is_some_and(|d| d > max_duration);
                    if timeout_fired {
                        // A hard-capped session is ending on voxtype's own
                        // initiative (not the caller's `record stop`) — end
                        // it as an external-trigger session so the caller is
                        // told, and the silence tracker is disarmed.
                        self.end_external_session(state.is_recording()).await;

                        // Streaming has its own clean stop path: skip the
                        // batch_transcribe branch below to avoid opening a
                        // second WS session for audio already being processed
                        // by the active streaming one.
                        if state.is_streaming() {
                            tracing::warn!(
                                "Recording timeout ({:.0}s limit) while streaming; closing capture",
                                max_duration.as_secs_f32()
                            );
                            self.stop_streaming_capture(&mut live).await;
                            continue;
                        }

                        tracing::warn!(
                            "Recording timeout ({:.0}s limit), transcribing captured audio",
                            max_duration.as_secs_f32()
                        );

                        // A subset on purpose. This cycle still delivers a
                        // transcript, so the flags the user asked for
                        // (auto-submit, shift-enter) must survive to be
                        // applied; only the smart-submit decision is dropped,
                        // because the recording ran to its limit rather than
                        // ending where the user meant it to.
                        self.paths.cleanup_output_mode_override();
                        self.paths.cleanup_model_override();
                        self.paths.cleanup_profile_override();
                        self.paths.cleanup_bool_override("smart_auto_submit");

                        let model_override = match &state {
                            State::Recording { model_override, .. } => model_override.clone(),
                            State::EagerRecording { model_override, .. } => model_override.clone(),
                            _ => None,
                        };

                        if state.is_eager_recording() {
                            if let Some(mut capture) = live.audio_capture.take() {
                                if let Ok(final_samples) = capture.stop().await {
                                    if let State::EagerRecording { accumulated_audio, .. } = &mut state {
                                        accumulated_audio.extend(final_samples);
                                    }
                                }
                            }
                            self.restore_recording_media();

                            let transcriber = match self.get_transcriber_for_recording(
                                model_override.as_deref(),
                            ).await {
                                Ok(transcriber) => transcriber,
                                Err(()) => {
                                    self.reset_to_idle(&mut state).await;
                                    continue;
                                }
                            };

                            self.update_state("transcribing");

                            if let Some(text) = self.finish_eager_recording(&mut state, transcriber).await {
                                let next = state.into_transcribing(Vec::new());
                                state = next;
                                self.handle_transcription_result(&mut state, Ok(Ok(text))).await;
                            } else {
                                tracing::debug!("Eager recording timeout produced empty result");
                                self.reset_to_idle(&mut state).await;
                            }
                            live.eager_transcriber = None;
                        } else {
                            for (_, task) in self.eager_chunk_tasks.drain(..) {
                                task.abort();
                            }

                            self.start_transcription_task(
                                &mut state,
                                &mut live,
                                model_override,
                            ).await;
                        }
                    }
                }

                // Handle SIGUSR1 - start recording (for compositor keybindings).
                // A test can inject the same trigger instead of signalling the
                // process, which would reach every daemon in the test binary.
                _ = async {
                    match self.deps.external_start.as_mut() {
                        Some(rx) => {
                            let _ = rx.recv().await;
                        }
                        None => {
                            let _ = sigusr1.recv().await;
                        }
                    }
                } => {
                    tracing::debug!("Received SIGUSR1 (start recording)");
                    if state.is_idle() {
                        // Read model override from file (set by `voxtype record start --model X`)
                        let model_override = self.paths.read_model_override();
                        // `voxtype record start --profile X` writes this file;
                        // reading it here hands it to the cycle, which carries
                        // it in state from this point on.
                        let profile_override = self.paths.read_profile_override();
                        tracing::info!("Recording started (external trigger), model_override = {:?}", model_override);

                        if self.config.output.notification.on_recording_start {
                            send_notification_with_lifetime("Recording Started", "External trigger", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency, Lifetime::UntilClosed).await;
                        }

                        self.begin_recording(
                            &mut state,
                            &mut live,
                            model_override,
                            profile_override,
                            true,
                            "SIGUSR1",
                        )
                        .await;
                    }
                }

                // Handle SIGUSR2 - stop recording (for compositor keybindings),
                // or an injected stop when a test supplies one.
                _ = async {
                    match self.deps.external_stop.as_mut() {
                        Some(rx) => {
                            let _ = rx.recv().await;
                        }
                        None => {
                            let _ = sigusr2.recv().await;
                        }
                    }
                } => {
                    tracing::debug!("Received SIGUSR2 (stop recording)");
                    self.stop_active_recording(
                        &mut state,
                        &mut live,
                    ).await;
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

                // Streaming event pump (active only while State::Streaming).
                event = async {
                    match live.streaming_handle.as_mut() {
                        Some(h) => h.events.recv().await,
                        None => std::future::pending().await,
                    }
                }, if state.is_streaming() && live.streaming_handle.is_some() => {
                    // File-output sessions (`--file=path`) accumulate into
                    // finalized_text via the `_silent` session methods
                    // instead of typing through `chain` — there's no
                    // cursor/focused window to type into, and doing so
                    // anyway is exactly the leak this branch exists to
                    // avoid (see the SIGUSR2 handler below).
                    let file_output = matches!(
                        &state,
                        State::Streaming { file_output_path: Some(_), .. }
                    );
                    match event {
                        Some(StreamingEvent::Partial { text, .. }) => {
                            if let Some(s) = live.streaming_session.as_mut() {
                                if file_output {
                                    s.observe_partial_delta(&text);
                                } else if let Some(chain) = live.streaming_chain.as_ref() {
                                    if let Err(e) = s.type_partial_delta(
                                        chain,
                                        text,
                                        self.config.output.pre_output_command.as_deref(),
                                        self.config.output.post_output_command.as_deref(),
                                    ).await {
                                        tracing::warn!("Streaming partial delta type failed: {}", e);
                                    }
                                }
                                if let State::Streaming { typed_chars, .. } = &mut state {
                                    *typed_chars = s.typed_chars();
                                }
                            }
                        }
                        Some(StreamingEvent::Final { text, .. }) => {
                            if let Some(s) = live.streaming_session.as_mut() {
                                if file_output {
                                    // Raw on purpose: file-mode text is
                                    // processed once, whole, at write time
                                    // in end_streaming — that also catches
                                    // matches spanning segment boundaries.
                                    s.commit_segment_silent(&text);
                                } else if let Some(chain) = live.streaming_chain.as_ref() {
                                    if let Err(e) = s.commit_segment(
                                        chain,
                                        &text,
                                        Some(&self.text_processor),
                                        self.config.output.pre_output_command.as_deref(),
                                        self.config.output.post_output_command.as_deref(),
                                    ).await {
                                        tracing::error!("Streaming commit_segment failed: {}", e);
                                    }
                                }
                                // Mirror typed_chars onto the state for cancel-rewind.
                                if let State::Streaming { typed_chars, finalized_text, .. } = &mut state {
                                    *typed_chars = s.typed_chars();
                                    finalized_text.clear();
                                    finalized_text.push_str(s.finalized_text());
                                }
                            }
                        }
                        Some(StreamingEvent::Replace { backspace, text, .. }) => {
                            if let Some(s) = live.streaming_session.as_mut() {
                                if file_output {
                                    // Raw on purpose — see the Final arm.
                                    s.replace_and_commit_silent(backspace, &text);
                                } else if let Some(chain) = live.streaming_chain.as_ref() {
                                    if let Err(e) = s.replace_and_commit(
                                        chain,
                                        backspace,
                                        &text,
                                        Some(&self.text_processor),
                                        self.config.output.pre_output_command.as_deref(),
                                        self.config.output.post_output_command.as_deref(),
                                    ).await {
                                        tracing::error!("Streaming replace_and_commit failed: {}", e);
                                    }
                                }
                                if let State::Streaming { typed_chars, finalized_text, .. } = &mut state {
                                    *typed_chars = s.typed_chars();
                                    finalized_text.clear();
                                    finalized_text.push_str(s.finalized_text());
                                }
                            }
                        }
                        Some(StreamingEvent::Error(err)) => {
                            tracing::error!("Streaming backend error: {}", err);
                            send_notification(
                                "Streaming Error",
                                &err.to_string(),
                                self.config.output.notification.show_engine_icon,
                                self.config.engine,
                                "critical",
                            ).await;
                            // The backend ended its own session, so no stop ran
                            // for it: the caller that started the session still
                            // has to be told, or a compositor that entered a
                            // submap stays in it, and the silence tracker is
                            // disarmed for a session that is already over.
                            self.end_external_session(state.is_recording()).await;
                            self.end_streaming(
                                &mut state,
                                &mut live,
                            ).await;
                        }
                        Some(StreamingEvent::Ended) | None => {
                            // Same as the error arm: `Ended` and a closed events
                            // channel both mean the backend ended the session
                            // itself. No-ops when a stop already ended it, since
                            // `end_external_session` clears its own flag.
                            self.end_external_session(state.is_recording()).await;
                            self.end_streaming(
                                &mut state,
                                &mut live,
                            ).await;
                        }
                    }
                }

                // Check for cancel during transcription
                _ = tokio::time::sleep(Duration::from_millis(100)), if matches!(state, State::Transcribing { .. }) => {
                    if self.paths.check_cancel_requested() {
                        tracing::info!("Transcription cancelled");

                        // Abort the transcription task
                        if let Some(task) = self.transcription_task.take() {
                            task.abort();
                        }
                        // Drop the cloned transcriber Arc so it isn't held
                        // until the next transcription.
                        self.active_transcriber = None;

                        self.close_cancelled_cycle(&mut state, false).await;

                        end_recording_notification("Cancelled", "Transcription aborted", &self.config.output.notification, self.config.engine).await;
                    }
                }

                // === MEETING MODE HANDLERS ===

                // Poll for meeting commands (file-based IPC), and carry the
                // idle model eviction that used to live on the 500ms idle arm.
                //
                // That arm never ran: select! drops and recreates its
                // un-completed timer futures each iteration, so this
                // unconditional 100ms sleep restarted the 500ms sleep before
                // it could fire. #606 fixed the cancel-trigger half of that
                // starvation; eviction was the other half, and it meant a
                // daemon that loaded a model on demand never released it
                // (#644).
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    // Evict roughly every 60s, and only while idle — unloading
                    // a model out from under a recording would be worse than
                    // holding it.
                    static EVICTION_COUNTER: std::sync::atomic::AtomicU32 =
                        std::sync::atomic::AtomicU32::new(0);
                    let count =
                        EVICTION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if count.is_multiple_of(600) && matches!(state, State::Idle) {
                        if let Some(ref mut mm) = self.model_manager {
                            mm.evict_idle_models();
                        }
                    }

                    // Check for meeting start command
                    if let Some(trigger) = self.paths.check_meeting_start() {
                        if self.config.meeting.enabled && self.meeting_daemon.is_none() {
                            tracing::debug!("Meeting start requested via file trigger");
                            if let Err(e) = self.start_meeting(trigger.title, trigger.diarization).await {
                                tracing::error!("Failed to start meeting: {}", e);
                            }
                        } else if !self.config.meeting.enabled {
                            tracing::warn!("Meeting mode is disabled in config");
                        } else {
                            tracing::warn!("Meeting already in progress");
                        }
                    }

                    // Check for meeting stop command
                    if self.paths.check_meeting_stop()
                        && self.meeting_daemon.is_some() {
                            tracing::debug!("Meeting stop requested via file trigger");
                            if let Err(e) = self.stop_meeting().await {
                                tracing::error!("Failed to stop meeting: {}", e);
                            }
                        }

                    // Check for meeting pause command
                    if self.paths.check_meeting_pause()
                        && self.meeting_active() {
                            tracing::debug!("Meeting pause requested via file trigger");
                            if let Err(e) = self.pause_meeting().await {
                                tracing::error!("Failed to pause meeting: {}", e);
                            }
                        }

                    // Check for meeting resume command
                    if self.paths.check_meeting_resume()
                        && self.meeting_daemon.as_ref().is_some_and(|d| d.state().is_paused()) {
                            tracing::debug!("Meeting resume requested via file trigger");
                            if let Err(e) = self.resume_meeting().await {
                                tracing::error!("Failed to resume meeting: {}", e);
                            }
                        }
                }

                // Process meeting audio chunks
                _ = tokio::time::sleep(Duration::from_millis(50)), if self.meeting_active() => {
                    // Check for meeting stop/pause/resume while active
                    // (the 100ms polling branch is starved by this faster 50ms branch)
                    if self.paths.check_meeting_stop() && self.meeting_daemon.is_some() {
                        tracing::debug!("Meeting stop requested via file trigger");
                        if let Err(e) = self.stop_meeting().await {
                            tracing::error!("Failed to stop meeting: {}", e);
                        }
                        continue;
                    }
                    if self.paths.check_meeting_pause() && self.meeting_active() {
                        tracing::debug!("Meeting pause requested via file trigger");
                        if let Err(e) = self.pause_meeting().await {
                            tracing::error!("Failed to pause meeting: {}", e);
                        }
                        continue;
                    }
                    if self.paths.check_meeting_resume()
                        && self.meeting_daemon.as_ref().is_some_and(|d| d.state().is_paused())
                    {
                        tracing::debug!("Meeting resume requested via file trigger");
                        if let Err(e) = self.resume_meeting().await {
                            tracing::error!("Failed to resume meeting: {}", e);
                        }
                        continue;
                    }

                    // Get samples from dual audio capture
                    if let Some(ref mut capture) = self.meeting_audio_capture {
                        let dual_samples = capture.get_samples().await;
                        self.meeting_mic_buffer.extend(dual_samples.mic);
                        self.meeting_loopback_buffer.extend(dual_samples.loopback);

                        self.process_buffered_meeting_audio(false).await;
                    }

                    // Check meeting timeout
                    if self.config.meeting.max_duration_mins > 0 {
                        if let Some(ref daemon) = self.meeting_daemon {
                            if let Some(duration) = daemon.state().elapsed() {
                                let max_duration = Duration::from_secs(
                                    self.config.meeting.max_duration_mins as u64 * 60
                                );
                                if duration > max_duration {
                                    tracing::warn!("Meeting timeout ({} min limit), stopping",
                                        self.config.meeting.max_duration_mins);
                                    if let Err(e) = self.stop_meeting().await {
                                        tracing::error!("Failed to stop meeting after timeout: {}", e);
                                    }
                                }
                            }
                        }
                    }
                }

                // Handle meeting events
                event = async {
                    match self.meeting_event_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                }, if self.meeting_event_rx.is_some() => {
                    match event {
                        Some(MeetingEvent::Started { meeting_id }) => {
                            tracing::info!("Meeting event: started {}", meeting_id);
                        }
                        Some(MeetingEvent::ChunkProcessed { chunk_id, segments }) => {
                            tracing::debug!("Meeting event: chunk {} processed with {} segments",
                                chunk_id, segments.len());
                        }
                        Some(MeetingEvent::Paused) => {
                            tracing::info!("Meeting event: paused");
                        }
                        Some(MeetingEvent::Resumed) => {
                            tracing::info!("Meeting event: resumed");
                        }
                        Some(MeetingEvent::Stopped { meeting_id }) => {
                            tracing::info!("Meeting event: stopped {}", meeting_id);
                        }
                        Some(MeetingEvent::Error(msg)) => {
                            tracing::error!("Meeting error: {}", msg);
                        }
                        None => {
                            // Channel closed
                            tracing::debug!("Meeting event channel closed");
                            self.meeting_event_rx = None;
                        }
                    }
                }

                // Injected stop, standing in for SIGINT/SIGTERM. Production
                // leaves `deps.shutdown` empty, so this arm never fires.
                _ = async {
                    match self.deps.shutdown.as_mut() {
                        Some(rx) => {
                            let _ = rx.await;
                        }
                        None => std::future::pending().await,
                    }
                } => {
                    tracing::info!("Shutdown requested");
                    break;
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

        // Stop any active dictation capture before shutting down and always
        // restore media that this daemon suppressed for the session.
        let streaming_task = live.streaming_handle.take().map(|handle| {
            let _ = handle.cancel.send(());
            handle.task
        });
        self.cut_streaming_audio();
        if let Some(mut capture) = live.audio_capture.take() {
            let _ = capture.stop().await;
        }
        self.restore_recording_media();
        notification::close_persistent().await;
        if let Some(task) = streaming_task {
            let _ = task.await;
        }

        // Cleanup hotkey listener
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(mut listener) = hotkey_listener {
            let _ = listener.stop(); // Best effort cleanup
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let _ = hotkey_listener; // Silence unused variable warning

        // Abort any pending transcription task
        if let Some(task) = self.transcription_task.take() {
            task.abort();
        }
        self.active_transcriber = None;

        // Abort any pending eager chunk tasks
        for (_, task) in self.eager_chunk_tasks.drain(..) {
            task.abort();
        }

        // Stop any active meeting
        if self.meeting_daemon.is_some() {
            tracing::info!("Stopping active meeting on shutdown");
            let _ = self.stop_meeting().await;
        }

        // Remove override files on shutdown
        self.paths.cleanup_profile_override();

        // Remove state file on shutdown
        if let Some(ref path) = self.state_file_path {
            cleanup_state_file(path);
        }

        // Remove meeting state file on shutdown
        if let Some(ref path) = self.meeting_state_file_path {
            cleanup_state_file(path);
        }

        // Remove the OSD audio level socket so a stale path doesn't
        // confuse the next daemon start.
        if let Some(ref hub) = self.level_hub {
            hub.cleanup();
        }

        tracing::info!("Daemon stopped");

        // A test drives `run` to completion in its own process, so it clears
        // this and takes the ordinary return path instead.
        if !self.deps.exit_process_on_shutdown {
            return Ok(());
        }

        // Exit without unwinding. Everything this daemon owns is already
        // released above: profile override, state file, meeting state file,
        // PID file and the OSD level socket. What remains between here and
        // `main` returning is tokio teardown plus `_dl_fini` running the
        // static destructors of ONNX Runtime, ROCm/MIGraphX and PipeWire, and
        // that stretch is actively hostile:
        //
        //   * The ORT/MIGraphX stack releases a shared_ptr it has already
        //     freed, decrementing a refcount inside a chunk parked in glibc's
        //     448-byte bin. Nothing faults at the time. glibc aborts on the
        //     next free landing in that size class, which at exit is
        //     PipeWire's `pw_log_topic_unregister`. PipeWire is the detector,
        //     not the cause; any 448-class free would do it. Each abort costs
        //     a ~1.9 GB core dump, a desktop crash notification, and a unit
        //     recorded as `Failed with result 'core-dump'`.
        //   * ROCm's AsyncEventsLoop threads park indefinitely in KFD ioctls
        //     and MIGraphX's embedded LLVM thread pool never joins, so the
        //     same teardown can hang instead of aborting.
        //
        // ANY CLEANUP THIS DAEMON NEEDS MUST GO ABOVE THIS LINE. Code added
        // below it will never run.
        //
        // `_exit` skips stdio flushing. Rust's stderr is unbuffered so the
        // tracing output above is already out; anything that starts buffering
        // daemon output has to flush before reaching here.
        unsafe { libc::_exit(0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// #643: the panic-recovery arm in handle_transcription_result must fire
    /// only for a real panic. Both JoinError flavors are constructed for real
    /// here — a task that panics and a task that gets aborted — because the
    /// two are indistinguishable by type and only differ in what
    /// `is_cancelled`/`is_panic` report.
    #[tokio::test]
    async fn join_error_distinguishes_panic_from_abort() {
        // Keep the spawned panic from printing a backtrace into test output.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let panic_err = tokio::spawn(async { panic!("engine blew up") })
            .await
            .expect_err("a panicking task must yield a JoinError");
        std::panic::set_hook(prev_hook);
        assert!(panic_err.is_panic());
        assert!(join_error_poisons_engine(&panic_err));

        let aborted = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        });
        aborted.abort();
        let abort_err = aborted
            .await
            .expect_err("an aborted task must yield a JoinError");
        assert!(abort_err.is_cancelled());
        assert!(!join_error_poisons_engine(&abort_err));
    }

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

    #[test]
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

    #[test]
    fn test_stale_lockfile_cleanup() {
        with_test_runtime_dir(|dir| {
            let lock_path = dir.join("voxtype.lock");

            // Write a stale lockfile with a PID that doesn't exist
            // PID 99999999 is very unlikely to exist
            std::fs::write(&lock_path, "99999999").expect("Failed to write stale lockfile");
            assert!(lock_path.exists(), "Stale lockfile should exist");

            // cleanup_stale_lockfile should detect and remove it
            let cleaned = cleanup_stale_lockfile(&lock_path);
            assert!(cleaned, "Stale lockfile should be cleaned up");
            assert!(!lock_path.exists(), "Stale lockfile should be removed");
        });
    }

    #[test]
    fn test_stale_lockfile_not_cleaned_if_pid_running() {
        with_test_runtime_dir(|dir| {
            let lock_path = dir.join("voxtype.lock");

            // Write a lockfile with our own PID (which is running)
            let our_pid = std::process::id();
            std::fs::write(&lock_path, our_pid.to_string()).expect("Failed to write lockfile");

            // cleanup_stale_lockfile should NOT remove it (PID is running)
            let cleaned = cleanup_stale_lockfile(&lock_path);
            assert!(!cleaned, "Lockfile with running PID should not be cleaned");
            assert!(lock_path.exists(), "Lockfile should still exist");
        });
    }
}
