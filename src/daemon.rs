//! Daemon module - main event loop orchestration
//!
//! Coordinates the hotkey listener, audio capture, transcription,
//! and text output components.

use crate::audio::feedback::{AudioFeedback, SoundEvent};
use crate::audio::{self, AudioCapture};
use crate::config::{ActivationMode, Config, FileMode, OutputConfig, OutputMode};
use crate::eager::{self, EagerConfig};
use crate::error::Result;
#[cfg(target_os = "linux")]
use crate::hotkey::{self, HotkeyEvent};
#[cfg(target_os = "macos")]
use crate::hotkey_macos::{self as hotkey, HotkeyEvent};
use crate::meeting::{self, MeetingDaemon, MeetingEvent, StorageConfig};
use crate::model_catalog;
use crate::model_catalog::override_config;
use crate::model_manager::ModelManager;
use crate::notification::{self, Lifetime};
use crate::osd;
use crate::output;
use crate::output::post_process::PostProcessor;
use crate::output::streaming::StreamingSession;
use crate::output::TextOutput;
use crate::state::{ChunkResult, State};
use crate::text::vocabulary::Vocabulary;
use crate::text::TextProcessor;
use crate::transcribe::{StreamHandle, StreamingEvent, Transcriber};
use pidlock::Pidlock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal::unix::{signal, SignalKind};

/// Runtime-directory marker holding the last model-load failure notified at
/// startup, so a systemd restart loop doesn't repeat the same notice.
const STARTUP_FAILURE_MARKER: &str = "startup_failure_notice";

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

/// Time left before `max_duration` stops a recording, when it is close
/// enough to warn about: 30s ahead for caps of two minutes or more, 10s for
/// shorter ones, and never for caps under 30s, where a warning would arrive
/// within seconds of starting to speak.
fn cap_warning_due(elapsed: Duration, max_duration: Duration) -> Option<Duration> {
    let lead = if max_duration >= Duration::from_secs(120) {
        Duration::from_secs(30)
    } else if max_duration >= Duration::from_secs(30) {
        Duration::from_secs(10)
    } else {
        return None;
    };
    let left = max_duration.saturating_sub(elapsed);
    (left <= lead).then_some(left)
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

    // Write to a sibling and rename, rather than writing in place.
    // std::fs::write truncates before it writes, so a reader polling this
    // file - waybar runs `voxtype status --follow` against it - can catch the
    // empty window and read "" as the state. rename(2) is atomic within a
    // filesystem, so a reader sees either the old contents or the new.
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, state) {
        tracing::warn!("Failed to write state file: {}", e);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        tracing::warn!("Failed to replace state file: {}", e);
        let _ = std::fs::remove_file(&tmp);
    } else {
        tracing::trace!("State file updated: {}", state);
    }
}

/// Remove state file on shutdown
/// Path of the marker that tells OSD frontends to stay hidden for the
/// recording in flight. Written when a recording starts with `--no-osd`,
/// removed when the daemon returns to idle.
///
/// A marker file rather than a state-file value on purpose: the status JSON
/// contract went stable in 1.0.0, and Waybar, `status --follow`, and every
/// other consumer must keep seeing the real state. Only the OSD frontends
/// read this.
fn osd_suppressed_path() -> PathBuf {
    Config::runtime_dir().join("osd_suppressed")
}

fn set_osd_suppressed(suppressed: bool) {
    set_osd_suppressed_at(&osd_suppressed_path(), suppressed);
}

/// Path-taking half of `set_osd_suppressed`, so the marker lifecycle is
/// testable without mocking `Config::runtime_dir()`.
fn set_osd_suppressed_at(path: &Path, suppressed: bool) {
    if suppressed {
        if let Err(e) = std::fs::write(path, "1") {
            tracing::warn!("Failed to write OSD suppression marker: {}", e);
        }
    } else if path.exists() {
        if let Err(e) = std::fs::remove_file(path) {
            tracing::warn!("Failed to clear OSD suppression marker: {}", e);
        }
    }
}

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

/// Take the single-instance lock in `runtime_dir`, recovering a stale one
/// left by a daemon that crashed.
///
/// This must be the daemon's first shared-state action. Everything else it
/// touches at startup (PID file, OSD audio socket, cancel/override files,
/// meeting state) belongs to whichever instance is already running, so a
/// refused second instance has to leave before touching any of it.
fn acquire_instance_lock(runtime_dir: &std::path::Path) -> Result<Pidlock> {
    std::fs::create_dir_all(runtime_dir).map_err(|e| {
        crate::error::VoxtypeError::Config(format!(
            "Failed to create runtime directory {:?}: {}",
            runtime_dir, e
        ))
    })?;

    let lock_path = runtime_dir.join("voxtype.lock");
    let lock_path_str = lock_path.to_string_lossy().to_string();
    let mut pidlock = Pidlock::new(&lock_path_str);

    if pidlock.acquire().is_ok() {
        tracing::debug!("Acquired PID lock at {:?}", lock_path);
        return Ok(pidlock);
    }

    #[cfg(unix)]
    if cleanup_stale_lockfile(&lock_path) {
        pidlock = Pidlock::new(&lock_path_str);
        if let Err(e) = pidlock.acquire() {
            tracing::error!("Failed to acquire lock after stale cleanup: {:?}", e);
            return Err(crate::error::VoxtypeError::Config(format!(
                "Another voxtype instance is already running (lock error: {:?})",
                e
            )));
        }
        tracing::debug!("Acquired PID lock at {:?} (after stale cleanup)", lock_path);
        return Ok(pidlock);
    }

    tracing::error!("Failed to acquire lock: another voxtype instance is already running");
    Err(crate::error::VoxtypeError::Config(
        "Another voxtype instance is already running".to_string(),
    ))
}

/// Return heap memory freed by a model unload to the kernel.
///
/// glibc keeps freed heap memory for reuse rather than giving it back, so an
/// on-demand model left its full footprint resident after it was dropped.
/// Measured with on-demand Parakeet on a CPU build: 1976 MB resident after two
/// transcriptions, 52 MB after trimming. Memory a GPU runtime keeps for the
/// life of the process (ROCm/MIGraphX libraries and staging buffers) is not
/// heap and is unaffected; only a separate transcription process frees that,
/// which today is Whisper's `gpu_isolation`.
fn release_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only releases free heap pages; it has no
    // preconditions and does not touch live allocations.
    unsafe {
        libc::malloc_trim(0);
    }
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

/// Whether a pending `record cancel` has a session to act on. Idle and
/// outputting states are left alone: a trigger written while idle is
/// consumed when the next capture starts (#606), not by this path.
fn cancel_applies_to(state: &State) -> bool {
    state.is_recording() || matches!(state, State::Transcribing { .. })
}

/// Clean up any stale cancel file on startup
fn cleanup_cancel_file() {
    let cancel_file = Config::runtime_dir().join("cancel");
    if cancel_file.exists() {
        let _ = std::fs::remove_file(&cancel_file);
    }
}

/// Written by `voxtype record start` (and a toggle that starts) just before
/// it signals the daemon, stamped with [`trigger_stamp`].
pub const START_REQUEST_FILE: &str = "start_requested";

/// Capacity, in chunks, of the channel feeding captured audio to a streaming
/// backend. The sliding-window engine only drains it between inference
/// passes, so it must hold at least one pass worth of audio or the overflow
/// is dropped by `try_send` and never reaches the transcript. Chunk size is
/// set by the audio backend, not by us: through PipeWire's ALSA plugin a
/// chunk can be ~2-3 ms, so the previous 64 held only ~0.15 s against a
/// ~0.6 s Whisper pass. 8192 chunks is ~20 s at that size (more with larger
/// chunks), a few MB at most, and recordings are capped by
/// `max_duration_secs` anyway.
const STREAMING_TAP_CAPACITY: usize = 8192;

/// Wall-clock nanoseconds, the content of the `cancel` and
/// [`START_REQUEST_FILE`] triggers. Both are written by separate CLI
/// processes, so only the realtime clock orders them.
pub fn trigger_stamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .to_string()
}

fn read_trigger_stamp(path: &std::path::Path) -> Option<u128> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The start-request marker for the signal being handled. It stays on disk
/// until this is dropped, after the start has run: while it exists a
/// `record cancel` knows a start is in flight even though the state file
/// does not say "recording" yet.
struct StartRequest {
    issued_at: Option<u128>,
}

impl StartRequest {
    fn read() -> Self {
        Self {
            issued_at: read_trigger_stamp(&Config::runtime_dir().join(START_REQUEST_FILE)),
        }
    }
}

impl Drop for StartRequest {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(Config::runtime_dir().join(START_REQUEST_FILE));
    }
}

/// Whether a cancel stamped `cancel_at` was issued for the start requested at
/// `start_at`. A cancel from before the request, or one carrying no stamp,
/// is a leftover from an idle `record cancel` and must not touch the new
/// recording (#606).
fn cancel_targets_start(start_at: Option<u128>, cancel_at: Option<u128>) -> bool {
    matches!((start_at, cancel_at), (Some(start), Some(cancel)) if cancel >= start)
}

/// Drop a leftover cancel trigger before a recording starts, keeping one
/// issued after this recording was requested. `record start; record cancel`
/// sent back to back can put the cancel on disk before the daemon has acted
/// on the start signal; the recording poll then applies it once capture is
/// live.
fn discard_stale_cancel(start_at: Option<u128>) {
    let cancel_file = Config::runtime_dir().join("cancel");
    if !cancel_file.exists() {
        return;
    }
    if cancel_targets_start(start_at, read_trigger_stamp(&cancel_file)) {
        tracing::debug!("Cancel was issued after this start request; applying it");
        return;
    }
    let _ = std::fs::remove_file(&cancel_file);
}

/// Read and consume the output mode override file
/// Returns the override mode if the file exists and is valid, None otherwise
/// Output mode override result, which may include a file path for file mode
#[derive(Debug, PartialEq)]
enum OutputOverride {
    Mode(OutputMode),
    FileWithPath(PathBuf),
}

/// Where this recording's transcript would land, without consuming anything.
///
/// The output override is only read once transcription succeeds, so the paths
/// that bail out earlier (too short, no speech) do not know the transcript
/// path and cannot report an outcome. This peeks the pending override so those
/// paths can still publish a completion sidecar, leaving the sentinel for the
/// normal consuming read.
fn peek_file_output_path(config: &Config) -> Option<PathBuf> {
    let override_file = Config::runtime_dir().join("output_mode_override");
    let pending = std::fs::read_to_string(&override_file).ok();
    match pending.as_deref().map(str::trim) {
        Some(value) => {
            if let Some(path) = value.strip_prefix("file:") {
                let path = path.trim();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
                return config.output.file_path.clone();
            }
            // A non-file override wins over the configured mode.
            None
        }
        None => {
            if config.output.mode == OutputMode::File {
                config.output.file_path.clone()
            } else {
                None
            }
        }
    }
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

/// Fold a per-recording output override into the configuration used to build
/// a streaming session's output chain.
fn apply_output_override(
    base: &OutputConfig,
    override_mode: Option<OutputOverride>,
) -> OutputConfig {
    let mut config = base.clone();
    match override_mode {
        Some(OutputOverride::FileWithPath(path)) => {
            config.mode = OutputMode::File;
            config.file_path = Some(path);
        }
        Some(OutputOverride::Mode(mode)) => {
            config.mode = mode;
        }
        None => {}
    }
    config
}

/// Every override file a single recording can carry, in the runtime dir.
const RECORDING_OVERRIDE_FILES: [&str; 6] = [
    "output_mode_override",
    "model_override",
    "profile_override",
    "auto_submit_override",
    "shift_enter_override",
    "smart_auto_submit_override",
];

fn discard_recording_overrides_in(runtime_dir: &std::path::Path) {
    for name in RECORDING_OVERRIDE_FILES {
        let _ = std::fs::remove_file(runtime_dir.join(name));
    }
}

/// Read and consume the profile override file
/// Returns the profile name if the file exists and is valid, None otherwise.
/// Only `Daemon::recording_profile` and `prepare_for_recording` call this:
/// the daemon keeps the name for the rest of the recording, because the
/// file is gone after the first read (#1097).
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

/// Write a profile override file so the daemon uses the named profile for post-processing.
/// Same mechanism as `voxtype record start --profile <name>`.
fn write_profile_override(profile_name: &str) {
    let profile_file = Config::runtime_dir().join("profile_override");
    if let Err(e) = std::fs::write(&profile_file, profile_name) {
        tracing::warn!("Failed to write profile override: {}", e);
    } else {
        tracing::info!("Profile modifier activated: {}", profile_name);
    }
}

/// Read and consume a boolean override file from the runtime directory.
/// Returns Some(true) or Some(false) if the file exists and is valid, None otherwise.
fn read_bool_override(name: &str) -> Option<bool> {
    let override_file = Config::runtime_dir().join(format!("{}_override", name));
    if !override_file.exists() {
        return None;
    }

    let content = match std::fs::read_to_string(&override_file) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to read {} override file: {}", name, e);
            return None;
        }
    };

    if let Err(e) = std::fs::remove_file(&override_file) {
        tracing::warn!("Failed to remove {} override file: {}", name, e);
    }

    match content.trim() {
        "true" => {
            tracing::info!("Using {} override: true", name);
            Some(true)
        }
        "false" => {
            tracing::info!("Using {} override: false", name);
            Some(false)
        }
        other => {
            tracing::warn!("Invalid {} override value: {:?}", name, other);
            None
        }
    }
}

// === Meeting Mode IPC ===

/// A pending meeting-start trigger with optional title, diarization override
/// and the engine, model, language and URL choices.
struct MeetingStartTrigger {
    title: Option<String>,
    diarization: Option<String>,
    options: crate::config::MeetingStartOptions,
}

/// Read a file and return its trimmed contents, or None if missing or empty.
///
/// Logs read failures so transient FS / permission errors aren't silent: a
/// trigger file existing but being unreadable previously looked identical to
/// "no file" and would then be consumed by the caller's remove_file.
fn read_trimmed_nonempty(path: &std::path::Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let trimmed = contents.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "Failed to read IPC trigger file");
            None
        }
    }
}

/// Allowed diarization backend override values from the CLI handler. Kept in
/// sync with the `value_parser` list on `MeetingAction::Start::diarization`.
const ALLOWED_DIARIZATION_OVERRIDES: &[&str] = &["simple", "ml", "hybrid"];

/// Validate a diarization backend override against the allowlist.
///
/// The CLI's clap `value_parser` already rejects bad values at parse time, but
/// the daemon reads the trigger from a runtime file written by an arbitrary
/// process and shouldn't propagate unknown values. Returns `None` and logs a
/// warning for anything outside the allowlist; defense-in-depth against stale
/// trigger files, partial writes from older voxtype versions, or a malicious
/// writer with access to the user's `$XDG_RUNTIME_DIR`.
fn validate_diarization_override(value: String) -> Option<String> {
    if ALLOWED_DIARIZATION_OVERRIDES.contains(&value.as_str()) {
        Some(value)
    } else {
        tracing::warn!(
            value = %value,
            "Ignoring unknown diarization override; expected one of {:?}",
            ALLOWED_DIARIZATION_OVERRIDES
        );
        None
    }
}

/// Check for meeting start command (via file trigger)
fn check_meeting_start() -> Option<MeetingStartTrigger> {
    let runtime_dir = Config::runtime_dir();
    let start_file = runtime_dir.join("meeting_start");
    if !start_file.exists() {
        return None;
    }

    let title = read_trimmed_nonempty(&start_file);

    // Diarization override is written by the CLI handler before the start
    // trigger. Re-validate against the allowlist (see
    // `validate_diarization_override` for the rationale).
    let diarization_file = runtime_dir.join("meeting_start_diarization");
    let diarization =
        read_trimmed_nonempty(&diarization_file).and_then(validate_diarization_override);
    let _ = std::fs::remove_file(&diarization_file);

    // `meeting start --engine/--model/--language/--url`, written the same way.
    let options_file = runtime_dir.join("meeting_start_options");
    let options = read_trimmed_nonempty(&options_file)
        .and_then(|json| match serde_json::from_str(&json) {
            Ok(options) => Some(options),
            Err(e) => {
                tracing::warn!("Ignoring unreadable meeting start options: {}", e);
                None
            }
        })
        .unwrap_or_default();
    let _ = std::fs::remove_file(&options_file);

    // The start trigger stays on disk until `ack_meeting_start`, once the
    // meeting is recording or refused. While it exists a second
    // `voxtype meeting start` sees a start in flight and refuses, which it
    // cannot tell from the state file until the meeting is up.
    Some(MeetingStartTrigger {
        title,
        diarization,
        options,
    })
}

/// Acknowledge a meeting start trigger read by `check_meeting_start`.
fn ack_meeting_start() {
    let _ = std::fs::remove_file(Config::runtime_dir().join("meeting_start"));
}

/// Check for meeting stop command (via file trigger)
fn check_meeting_stop() -> bool {
    let stop_file = Config::runtime_dir().join("meeting_stop");
    if stop_file.exists() {
        let _ = std::fs::remove_file(&stop_file);
        true
    } else {
        false
    }
}

/// Check for meeting pause command (via file trigger)
fn check_meeting_pause() -> bool {
    let pause_file = Config::runtime_dir().join("meeting_pause");
    if pause_file.exists() {
        let _ = std::fs::remove_file(&pause_file);
        true
    } else {
        false
    }
}

/// Check for meeting resume command (via file trigger)
fn check_meeting_resume() -> bool {
    let resume_file = Config::runtime_dir().join("meeting_resume");
    if resume_file.exists() {
        let _ = std::fs::remove_file(&resume_file);
        true
    } else {
        false
    }
}

/// Check for a `meeting reassign` or `meeting annotate` (via file trigger).
/// The CLI has already stored the change; the daemon applies it to the
/// running meeting.
fn check_meeting_edit() -> bool {
    take_trigger("meeting_edit")
}

/// Check for a change to the running meeting's notes (via file trigger).
fn check_meeting_notes() -> bool {
    take_trigger("meeting_notes")
}

/// A `voxtype meeting soundcheck` request, consumed.
fn take_soundcheck_trigger() -> Option<meeting::soundcheck::Request> {
    let file = Config::runtime_dir().join(meeting::soundcheck::TRIGGER);
    let text = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    Some(meeting::soundcheck::Request::parse(&text))
}

/// A sound check the daemon is running.
struct Soundcheck {
    /// The meeting's capture, canceller and liveness guard included.
    opened: meeting::soundcheck::MeetingCapture,
    levels: Option<tokio::task::JoinHandle<()>>,
    deadline: std::time::Instant,
    /// The `running` event, re-sent when the check is extended.
    running: meeting::soundcheck::SoundcheckEvent,
}

/// The meeting named by a `meeting_summary_updated` trigger, taken.
fn take_summary_updated() -> Option<String> {
    let file = Config::runtime_dir().join(meeting::SUMMARY_UPDATED_TRIGGER);
    let id = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    meeting::MeetingId::parse(id.trim())
        .ok()
        .map(|id| id.to_string())
}

fn take_trigger(name: &str) -> bool {
    let file = Config::runtime_dir().join(name);
    if file.exists() {
        let _ = std::fs::remove_file(&file);
        true
    } else {
        false
    }
}

/// Clean up any stale meeting command files on startup
fn cleanup_meeting_files() {
    let runtime_dir = Config::runtime_dir();
    for name in &[
        "meeting_start",
        "meeting_start_diarization",
        "meeting_start_options",
        "meeting_stop",
        "meeting_pause",
        "meeting_resume",
        "meeting_edit",
        "meeting_notes",
        meeting::retitle::TRIGGER,
        meeting::notice::ACK_TRIGGER,
        meeting::SUMMARY_UPDATED_TRIGGER,
    ] {
        let file = runtime_dir.join(name);
        if file.exists() {
            let _ = std::fs::remove_file(&file);
        }
    }
}

/// Where meetings are stored, per `[meeting] storage_path`.
fn meeting_storage_config(config: &Config) -> StorageConfig {
    let storage_path = if config.meeting.storage_path == "auto" {
        Config::data_dir().join("meetings")
    } else {
        std::path::PathBuf::from(&config.meeting.storage_path)
    };
    StorageConfig {
        storage_path,
        retain_audio: config.meeting.retain_audio,
        max_meetings: 0,
        my_name: config.meeting.my_name.clone(),
    }
}

/// Mark meetings still active or paused as interrupted on daemon startup:
/// the daemon that recorded them crashed or was killed, so none of them
/// finished. `voxtype meeting recover` finishes or discards them.
fn cleanup_stale_meetings(config: &Config) {
    match meeting::MeetingStorage::open(meeting_storage_config(config)) {
        Ok(storage) => match storage.interrupt_stale_meetings() {
            Ok(count) if count > 0 => {
                tracing::warn!(
                    "{} meeting(s) never finished (the daemon stopped while recording); see voxtype meeting recover",
                    count
                );
                // Reset meeting state file to idle
                let state_file = Config::runtime_dir().join("meeting_state");
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
/// The daemon's control surface is fire-and-forget: a client that asked for
/// file output has no way to learn that the recording finished, so it has to
/// poll the transcript until its own deadline expires. When no speech is
/// detected nothing is ever written and that deadline is the only thing that
/// ends the wait, reported to the user as a timeout, which it is not. This
/// sidecar is the missing completion signal: exactly one is written per
/// file-mode recording, and `voxtype record stop --wait` blocks on it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TranscriptOutcome {
    /// `ok`, `empty`, or `error`.
    pub status: String,
    /// Characters written. Zero for `empty` and `error`.
    pub chars: usize,
    /// Present only for `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl TranscriptOutcome {
    pub fn ok(chars: usize) -> Self {
        Self {
            status: "ok".to_string(),
            chars,
            message: None,
        }
    }

    /// No speech survived voice-activity detection, so nothing was transcribed.
    pub fn empty() -> Self {
        Self {
            status: "empty".to_string(),
            chars: 0,
            message: None,
        }
    }

    pub fn error(message: &str) -> Self {
        Self {
            status: "error".to_string(),
            chars: 0,
            message: Some(message.to_string()),
        }
    }
}

/// Path of the completion sidecar for a transcript.
pub fn result_sidecar_path(transcript: &std::path::Path) -> std::path::PathBuf {
    let mut sidecar = transcript.as_os_str().to_os_string();
    sidecar.push(".done");
    std::path::PathBuf::from(sidecar)
}

/// Publish `outcome` beside `transcript`, atomically and last.
///
/// Written after the transcript itself so a client that sees the sidecar can
/// read a complete transcript. Failure to write it is logged and otherwise
/// ignored: the transcription already succeeded, and a client that misses the
/// signal falls back to its own timeout.
fn write_result_sidecar(transcript: &std::path::Path, outcome: &TranscriptOutcome) {
    let sidecar = result_sidecar_path(transcript);
    let body = match serde_json::to_string(outcome) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!("Failed to encode transcript outcome: {}", e);
            return;
        }
    };
    let staged = temp_sibling(&sidecar);
    if let Err(e) = std::fs::write(&staged, format!("{}\n", body)) {
        tracing::warn!("Failed to stage transcript outcome {:?}: {}", staged, e);
        return;
    }
    if let Err(e) = std::fs::rename(&staged, &sidecar) {
        tracing::warn!("Failed to publish transcript outcome {:?}: {}", sidecar, e);
        let _ = std::fs::remove_file(&staged);
    }
}

/// Sibling temporary path used to stage an atomic transcript write.
///
/// Kept in the same directory as the target so the rename stays within one
/// filesystem. The pid keeps two daemons from colliding on it.
fn temp_sibling(path: &std::path::Path) -> std::path::PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "transcription".to_string());
    let mut staged = path.to_path_buf();
    staged.set_file_name(format!(".{}.{}.tmp", name, std::process::id()));
    staged
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

    // An empty transcript (post-processing returned nothing with
    // fallback_on_empty off) writes nothing: overwrite still replaces the
    // previous transcript, with an empty file, so a reader can't take the old
    // text for this result, and append adds no blank line.
    let output_text = if text.is_empty() || text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{}\n", text)
    };

    match file_mode {
        FileMode::Overwrite => {
            // Write through a sibling temporary file and rename, so a reader
            // polling for a non-empty transcript can never observe a partial
            // one. Programmatic consumers (OmaPilot, agent harnesses) return
            // the first non-empty read they get; a truncate-then-write would
            // hand them a half-written transcript.
            let temporary = temp_sibling(path);
            tokio::fs::write(&temporary, output_text).await?;
            if let Err(e) = tokio::fs::rename(&temporary, path).await {
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(e);
            }
        }
        FileMode::Append if output_text.is_empty() => {}
        FileMode::Append => {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await?;
            file.write_all(output_text.as_bytes()).await?;
            // tokio completes the write on a background thread; without the
            // flush a reader right after this returns can still see the old
            // file, and dropping the handle doesn't wait for it.
            file.flush().await?;
        }
    }

    Ok(())
}

/// Where a file-mode transcript ended up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDelivery {
    Written,
    /// The write failed (error text) and the fallback chain took the text.
    FellBack(String),
    /// The write failed (error text) and nothing else could take the text.
    Lost(String),
}

/// Deliver a transcript to `path`, wrapped in the pre/post output hooks like
/// every other output mode, so a compositor submap entered for the recording
/// is always reset. When the write fails, `fallback` (the clipboard chain,
/// empty when `fallback_to_clipboard` is off) gets the text instead.
#[cfg(test)]
async fn deliver_to_file(
    path: &Path,
    text: &str,
    cfg: &crate::config::OutputConfig,
    fallback: &[Box<dyn TextOutput>],
) -> FileDelivery {
    deliver_to_file_observed(path, text, cfg, fallback, None).await
}

pub async fn deliver_to_file_observed(
    path: &Path,
    text: &str,
    cfg: &crate::config::OutputConfig,
    fallback: &[Box<dyn TextOutput>],
    history_context: Option<&crate::history::recorder::DeliveryContext>,
) -> FileDelivery {
    if let Some(cmd) = &cfg.pre_output_command {
        if let Err(e) = output::run_hook(cmd, "pre_output").await {
            tracing::warn!("{}", e);
        }
    }

    let delivery = match write_transcription_to_file(path, text, &cfg.file_mode).await {
        Ok(()) => FileDelivery::Written,
        Err(e) => {
            let err = e.to_string();
            tracing::error!("Failed to write transcription to {:?}: {}", path, err);
            let rescued = !fallback.is_empty()
                && output::output_with_fallback(
                    fallback,
                    text,
                    output::OutputOptions {
                        history_context,
                        pre_output_command: None,
                        post_output_command: None,
                        wait_for_modifier_release: false,
                        modifier_release_timeout: Duration::ZERO,
                    },
                )
                .await
                .is_ok();
            if rescued {
                tracing::warn!(
                    "Transcription for {:?} copied to the clipboard instead",
                    path
                );
                FileDelivery::FellBack(err)
            } else {
                FileDelivery::Lost(err)
            }
        }
    };

    // Always, even when nothing was delivered, matching output_with_fallback.
    if let Some(cmd) = &cfg.post_output_command {
        if let Err(e) = output::run_hook(cmd, "post_output").await {
            tracing::warn!("{}", e);
        }
    }

    delivery
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
        tracing::info!("Model override requested: {}", model);
        Some(model)
    }
}

/// Result type for transcription task
type TranscriptionResult = std::result::Result<String, crate::error::TranscribeError>;

/// Background construction of a transcriber.
type TranscriberLoadTask = tokio::task::JoinHandle<
    std::result::Result<Arc<dyn Transcriber>, crate::error::TranscribeError>,
>;

/// How a streaming session ended, for `end_streaming`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StreamingEndReason {
    /// The backend sent `Ended` after delivering what it had.
    Completed,
    /// The backend failed, closed without `Ended`, or stalled past the stop
    /// watchdog. Carries the error to record and show, when there is one.
    Failed(Option<String>),
}

/// A finished dictation held back while the hotkey is down for the next one.
struct DeferredOutput {
    config: crate::config::OutputConfig,
    text: String,
    /// The transcription before text processing, for `show_edits`
    original: String,
    /// Its history entry, committed once the text is delivered
    history: Option<crate::history::recorder::Pending>,
}

/// Main daemon that orchestrates all components
#[cfg(test)]
type HistoryTestEagerLoop = (
    State,
    Arc<dyn Transcriber>,
    Arc<dyn Transcriber>,
    tokio::sync::mpsc::Receiver<HotkeyEvent>,
);

pub struct Daemon {
    config: Config,
    pending_language: Option<Config>,
    /// When the `record start` being handled was issued, while that start
    /// runs. Lets a cancel racing the start signal survive the #606
    /// stale-cancel sweep.
    start_request: Option<u128>,
    config_path: Option<PathBuf>,
    state_file_path: Option<PathBuf>,
    pid_file_path: Option<PathBuf>,
    audio_feedback: Option<AudioFeedback>,
    text_processor: TextProcessor,
    /// False while the current recording's `--model` override produces text
    /// that is not known to be English, so English-only cleanup is skipped
    /// for it. See `override_text_is_english`.
    session_text_is_english: bool,
    /// Vocabulary matchers prepared once at startup, keyed by profile name.
    profile_vocabularies: HashMap<String, Vocabulary>,
    /// The `--profile` of the current recording, kept from its override
    /// file for every stage of the dictation. See `recording_profile`.
    recording_profile: Option<String>,
    post_processor: Option<PostProcessor>,
    /// Last post-processed text and when it was produced, for context in subsequent dictations
    last_dictation: Option<(String, Instant)>,
    /// Audio level broadcaster for the OSD (None when disabled or bind failed)
    level_hub: Option<audio::levels::LevelHub>,
    /// Meeting transcript/status broadcaster for the meeting HUD (None if bind failed)
    meeting_event_hub: Option<meeting::MeetingEventHub>,
    /// The daemon's current view of the active (or most recently ended)
    /// meeting, mirrored into `meeting_event_hub`'s `hello` snapshot on every
    /// transition. Source of truth for the HUD; independent of the meeting
    /// state *file* (`write_meeting_state_file`), which exists for Waybar-
    /// style consumers that only want the plain-text status.
    meeting_hud: meeting::MeetingHudState,
    /// Active per-recording level emitter task; aborted when recording stops
    level_emitter_task: Option<tokio::task::JoinHandle<()>>,
    /// Tracks time-since-last-speech for the active recording, when
    /// silence-based auto-stop is armed (external-trigger sessions only —
    /// see `new_speech_tracker`). `None` when idle or not armed for this
    /// session. Read by the periodic silence check in the main loop.
    silence_tracker: Option<audio::levels::SpeechTracker>,
    /// Receiver for the one dead-input report produced by the active capture.
    dead_input_rx: Option<tokio::sync::mpsc::Receiver<audio::input_guard::DeadInputReport>>,
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
    /// Complete capture retained for a streaming session. AudioCapture keeps
    /// this independently of the backend tap so a failed stream can fall back
    /// to ordinary batch transcription.
    streaming_recovery_audio: Vec<f32>,
    /// Deadline armed when the microphone stops while the backend drains.
    /// An absolute deadline survives unrelated select! iterations.
    streaming_stop_deadline: Option<tokio::time::Instant>,
    /// File target selected when the streaming session began. Kept across the
    /// recovery handoff because the runtime override is consumed at start.
    streaming_recovery_file_output_path: Option<PathBuf>,

    /// OSD child supervisor task. Holds the JoinHandle so dropping it (on
    /// daemon shutdown) kill_on_drop's the spawned voxtype-osd process.
    osd_supervisor_task: Option<tokio::task::JoinHandle<()>>,
    #[cfg(target_os = "linux")]
    tray: Option<crate::tray::TrayRuntime>,
    // Model manager for multi-model support
    model_manager: Option<ModelManager>,
    // Rate-limits the notification for a microphone that won't open, so a
    // hotkey pressed again and again doesn't stack identical notices.
    capture_notices: crate::failure_notice::NoticeGate,
    // Same for a model that won't load.
    model_notices: crate::failure_notice::NoticeGate,
    // Background task for loading model on-demand
    model_load_task: Option<
        tokio::task::JoinHandle<
            std::result::Result<Arc<dyn Transcriber>, crate::error::TranscribeError>,
        >,
    >,
    // Transcriber for the most recent `--model` override the active engine
    // can't serve (see `model_catalog::OverrideRoute::Separate`), keyed by
    // the override name. One slot: the default model stays loaded separately.
    override_cache: Option<(String, Arc<dyn Transcriber>)>,
    // Load of an override transcriber started when its recording began.
    override_load_task: Option<(String, TranscriberLoadTask)>,
    // Background task that spawns and prepares the gpu_isolation subprocess
    // worker. Awaited before transcription so audio capture can start
    // immediately while the worker loads its model in parallel.
    whisper_prepare_task: Option<tokio::task::JoinHandle<()>>,
    // Background task for transcription (allows cancel during transcription)
    transcription_task: Option<tokio::task::JoinHandle<TranscriptionResult>>,
    // Push-to-talk pipelining. The microphone keeps running from the end of
    // a recording until its transcription finishes, so a press in that
    // window records from where it left off instead of being dropped. Text
    // that finishes while the hotkey is held for the next dictation waits in
    // `deferred_output` until the release, so it doesn't land mid-sentence.
    standby_capture: Option<Box<dyn AudioCapture>>,
    deferred_output: Option<DeferredOutput>,
    hotkey_down: bool,
    // Transcriber Arc used for the in-flight transcription_task. Held so the
    // result handler can query language metadata (e.g. detected language for
    // keyboard-layout hints to eitype/dotool, see issue #180) after the task
    // completes. Cleared when transcription_task is taken.
    active_transcriber: Option<Arc<dyn Transcriber>>,
    // Exercise streaming startup/tap without opening an audio device.
    #[cfg(test)]
    history_test_capture: Option<Box<dyn AudioCapture>>,
    /// Stands in for the configured output methods in one batch delivery.
    #[cfg(test)]
    history_test_output_chain: Option<Vec<Box<dyn TextOutput>>>,
    #[cfg(test)]
    history_test_eager_loop: Option<HistoryTestEagerLoop>,
    #[cfg(test)]
    history_test_next_capture: Option<Box<dyn AudioCapture>>,
    #[cfg(test)]
    history_test_completion_observer: Option<Arc<std::sync::atomic::AtomicBool>>,
    // Engine serving the current recording, after any `--model` override,
    // for the "Transcribed" log line.
    recording_engine: String,
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
    // A fresh Arc fences late blocking work from a cancelled eager recording.
    eager_decoded: Arc<std::sync::atomic::AtomicBool>,
    // Voice Activity Detection (filters silence-only recordings)
    vad: Option<Arc<dyn crate::vad::VoiceActivityDetector>>,
    // Meeting mode daemon (optional, created when meeting starts)
    meeting_daemon: Option<MeetingDaemon>,
    // Meeting state file path
    meeting_state_file_path: Option<PathBuf>,
    // Audio capture for meeting mode (dual: mic + loopback)
    meeting_audio_capture: Option<audio::DualCapture>,
    /// A running `voxtype meeting soundcheck`.
    meeting_soundcheck: Option<Soundcheck>,
    /// The meeting.sock level publisher and its "recording, not paused" flag.
    meeting_levels: Option<(
        tokio::task::JoinHandle<()>,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
    // Chunk buffers for meeting mode (separate mic and loopback)
    meeting_mic_buffer: Vec<f32>,
    meeting_loopback_buffer: Vec<f32>,
    // Meeting event receiver
    meeting_event_rx: Option<meeting::MeetingEventReceiver>,
    // The recording announcement's progress, while it runs
    meeting_announce_rx: Option<tokio::sync::mpsc::Receiver<meeting::announce::Update>>,
    meeting_announce_task: Option<tokio::task::JoinHandle<meeting::announce::Status>>,
    // When the meeting's capture started: zero on its transcript timeline
    meeting_capture_started: Option<std::time::Instant>,
    // Missing-model notices already shown this run, so each pops up once
    meeting_notices_shown: std::collections::HashSet<String>,
    // Whether this meeting's first transcription error was already notified
    meeting_error_notified: bool,
    // GTCRN speech enhancer for mic echo cancellation
    #[cfg(feature = "onnx-common")]
    speech_enhancer: Option<std::sync::Arc<audio::enhance::GtcrnEnhancer>>,
    // Whether the running meeting's mic goes through GTCRN. The enhancer
    // stays loaded between meetings; a meeting on the echo canceller skips it.
    #[cfg_attr(not(feature = "onnx-common"), allow(dead_code))]
    meeting_gtcrn: bool,
    // PipeWire's echo canceller, while a meeting on speakers runs
    meeting_echo_canceller: Option<audio::echo_cancel::EchoCanceller>,
    // Watches the first seconds of the cancelled mic for a canceller that
    // delivers nothing; set while that is still open
    meeting_aec_liveness: Option<audio::echo_cancel::LivenessCheck>,
    // Media players that were paused when recording started (for resume on stop)
    paused_media_players: Vec<String>,
    // Audio streams that were ducked when recording started (for restore on recording stop)
    ducked_media_streams: Vec<audio::media::DuckedMediaStream>,
    // In-flight media volume fade, down at recording start or up at stop. Held
    // so the next duck/restore can serialize against it; see
    // `duck_media_streams` for why capturing originals mid-fade is unsafe.
    media_fade_task: Option<tokio::task::JoinHandle<()>>,
    // Set when `max_duration_secs` ends the session in flight, so the
    // outcome published for the OSD can say the limit stopped it.
    osd_limit_reached: bool,
    osd_session_started_at_ms: u64,
    osd_session_suppressed: std::sync::atomic::AtomicBool,
    /// When the daemon last went idle after being busy, as a trigger stamp
    /// (nanoseconds since the epoch; 0 while it has been idle since
    /// startup). A start request issued before this was issued while the
    /// daemon was busy, so it is ignored like one handled while busy
    /// (#1004). Without it, a `record toggle` sent during output waits in
    /// the signal queue and starts a recording once output finishes.
    idle_since: std::sync::atomic::AtomicU64,
    /// Whether the last state written was idle, so a repeated idle write
    /// does not move `idle_since` forward.
    last_state_idle: std::sync::atomic::AtomicBool,
    // Whether this session's outcome has been published. The first one
    // wins: some paths report a specific outcome (no speech) and then
    // fall through a generic one (empty result) on the way to idle.
    osd_outcome_sent: std::sync::atomic::AtomicBool,
    // An accepted model upgrade still waiting for its first successful
    // transcription, after which `setup migrate --cleanup` may delete the
    // previous model. Read once at startup so the hot path does no I/O.
    model_upgrade_pending: bool,
    // Dictation history (#209): settings, paths, and the dictation in
    // flight. Filled in as the session goes, written once it is back to
    // idle, so recording it never delays output.
    history: Arc<std::sync::Mutex<crate::history::recorder::Recorder>>,
    dictation: std::sync::Mutex<Option<crate::history::recorder::Pending>>,
    history_maintenance: Option<tokio::task::JoinHandle<()>>,
}

impl Daemon {
    /// Create a new daemon with the given configuration
    /// Send the last dictation again, for the paste-last key. This runs
    /// `voxtype paste-last`, the command a compositor binding runs: one code
    /// path for both kinds of hotkey, including its notification when
    /// history is off or empty.
    fn paste_last_dictation(&self) {
        let Ok(exe) = std::env::current_exe() else {
            tracing::warn!("Could not find the voxtype binary to paste the last dictation");
            return;
        };
        let mut command = tokio::process::Command::new(exe);
        if let Some(path) = &self.config_path {
            command.arg("--config").arg(path);
        }
        command
            .arg("paste-last")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        match command.spawn() {
            Ok(mut child) => {
                tokio::spawn(async move {
                    match child.wait().await {
                        Ok(status) if !status.success() => {
                            tracing::warn!("Pasting the last dictation failed ({status})")
                        }
                        Err(e) => tracing::warn!("Pasting the last dictation failed: {e}"),
                        _ => {}
                    }
                });
            }
            Err(e) => tracing::warn!("Could not paste the last dictation: {e}"),
        }
    }

    pub fn new(config: Config, config_path: Option<PathBuf>) -> Self {
        let state_file_path = config.resolve_state_file();
        let history = Arc::new(std::sync::Mutex::new(
            crate::history::recorder::Recorder::new(&config, config_path.clone()),
        ));

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
        // Pass the language of the text so filler filtering can skip words
        // that are ordinary vocabulary there rather than disfluencies (#566)
        // and English-only cleanup runs only on English (#696).
        let text_processor =
            TextProcessor::new_for_config(&config.text, config.text_language(), &config);
        let profile_vocabularies = config
            .profiles
            .iter()
            .filter(|(_, profile)| {
                !profile.vocabulary.terms.is_empty() || profile.vocabulary.terms_file.is_some()
            })
            .filter_map(
                |(name, profile)| match Vocabulary::load(&profile.vocabulary) {
                    Ok(vocabulary) => Some((name.clone(), vocabulary)),
                    Err(error) => {
                        tracing::warn!(profile = %name, %error, "Profile vocabulary disabled");
                        None
                    }
                },
            )
            .collect();
        if config.text.spoken_punctuation {
            tracing::info!("Spoken punctuation enabled");
        }
        if !config.text.replacements.is_empty() {
            tracing::info!(
                "Word replacements configured: {} rules",
                config.text.replacements.len()
            );
        }

        if config.streaming_enabled() && config.output.post_process.is_some() {
            // [text] rules no longer belong in this warning: since the #669
            // fix they are applied when a streaming segment is committed, and
            // warning that a working setting is ignored is worse than not
            // warning at all. The LLM post_process hook is the one completed-
            // transcript stage streaming still skips (see the note in
            // output::streaming): incremental output would re-run it per
            // segment and let it rewrite already-typed text.
            tracing::warn!(
                "Streaming is enabled, so output.post_process will NOT run. \
                 The post-process command needs a completed transcript, which \
                 streaming never produces. Disable streaming to use it."
            );
        }

        // Initialize post-processor if configured
        let post_processor = config.output.active_post_process().map(|cfg| {
            match cfg.llm() {
                Some(llm) => tracing::warn!(
                    "Post-processing enabled through the {} model backend, timeout={}ms. {}",
                    llm.provider.as_str(),
                    cfg.timeout_ms,
                    crate::output::post_process::LATENCY_WARNING
                ),
                None => tracing::info!(
                    "Post-processing enabled: command={:?}, timeout={}ms",
                    cfg.command,
                    cfg.timeout_ms
                ),
            }
            PostProcessor::new(cfg)
        });

        // Initialize Voice Activity Detection if enabled
        let vad = match crate::vad::create_vad(&config) {
            Ok(Some(init)) => {
                tracing::info!(
                    "Voice Activity Detection enabled (backend: {:?}, configured: {:?}, threshold: {:.2}, min_speech: {}ms)",
                    init.backend,
                    config.vad.backend,
                    config.vad.threshold,
                    config.vad.min_speech_duration_ms
                );
                Some(Arc::from(init.detector))
            }
            Ok(None) => None,
            Err(e) => {
                // Without VAD, silence reaches the model and can be typed as
                // hallucinated text, so the user has to hear about it.
                let (title, body) = crate::vad::unavailable_notice(&config.vad, &e);
                tracing::warn!("Failed to initialize VAD, continuing without: {}", e);
                crate::notification::send_sync(title, &body);
                None
            }
        };

        // Meeting state file path (separate from push-to-talk state)
        let meeting_state_file_path = if state_file_path.is_some() {
            Some(Config::runtime_dir().join("meeting_state"))
        } else {
            None
        };

        let model_upgrade_pending = crate::model_upgrade::pending_first_success(
            &crate::model_upgrade::state_path(),
            config.model_name(),
        );
        let recording_engine = config.engine.to_string();

        Self {
            start_request: None,
            config,
            config_path,
            state_file_path,
            pid_file_path: None,
            audio_feedback,
            text_processor,
            session_text_is_english: true,
            profile_vocabularies,
            recording_profile: None,
            post_processor,
            last_dictation: None,
            level_hub: None,
            meeting_event_hub: None,
            meeting_hud: meeting::MeetingHudState::idle(),
            level_emitter_task: None,
            silence_tracker: None,
            dead_input_rx: None,
            is_external_trigger: false,
            streaming_drain_pump: None,
            streaming_recovery_audio: Vec::new(),
            streaming_stop_deadline: None,
            streaming_recovery_file_output_path: None,

            osd_supervisor_task: None,
            #[cfg(target_os = "linux")]
            tray: None,
            pending_language: None,
            model_manager: None,
            capture_notices: Default::default(),
            model_notices: Default::default(),
            model_load_task: None,
            override_cache: None,
            override_load_task: None,
            whisper_prepare_task: None,
            transcription_task: None,
            standby_capture: None,
            deferred_output: None,
            hotkey_down: false,
            active_transcriber: None,
            #[cfg(test)]
            history_test_capture: None,
            #[cfg(test)]
            history_test_output_chain: None,
            #[cfg(test)]
            history_test_eager_loop: None,
            #[cfg(test)]
            history_test_next_capture: None,
            #[cfg(test)]
            history_test_completion_observer: None,
            recording_engine,
            transcriber_preloaded: None,
            eager_chunk_tasks: Vec::new(),
            eager_decoded: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            vad,
            meeting_daemon: None,
            meeting_state_file_path,
            meeting_audio_capture: None,
            meeting_levels: None,
            meeting_soundcheck: None,
            meeting_mic_buffer: Vec::new(),
            meeting_loopback_buffer: Vec::new(),
            meeting_event_rx: None,
            meeting_announce_rx: None,
            meeting_announce_task: None,
            meeting_capture_started: None,
            meeting_notices_shown: std::collections::HashSet::new(),
            meeting_error_notified: false,
            #[cfg(feature = "onnx-common")]
            speech_enhancer: None,
            meeting_gtcrn: false,
            meeting_echo_canceller: None,
            meeting_aec_liveness: None,
            paused_media_players: Vec::new(),
            ducked_media_streams: Vec::new(),
            media_fade_task: None,
            osd_limit_reached: false,
            osd_session_started_at_ms: 0,
            osd_session_suppressed: std::sync::atomic::AtomicBool::new(false),
            idle_since: std::sync::atomic::AtomicU64::new(0),
            last_state_idle: std::sync::atomic::AtomicBool::new(true),
            osd_outcome_sent: std::sync::atomic::AtomicBool::new(false),
            model_upgrade_pending,
            history,
            dictation: std::sync::Mutex::new(None),
            history_maintenance: None,
        }
    }

    /// Add to the dictation in flight, when history is recording it.
    fn history_note(&self, note: impl FnOnce(&mut crate::history::recorder::Pending)) {
        if let Some(pending) = self
            .dictation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            note(pending);
        }
    }

    /// Start recording a dictation to history, if it is on and not paused.
    fn history_context(&self) -> Option<crate::history::recorder::DeliveryContext> {
        self.dictation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|pending| pending.delivery_context())
    }

    /// Copy facts reported by the engine selected for this recording.
    fn history_transcriber_metadata(&self, engine: &dyn Transcriber) {
        let metadata = engine.runtime_metadata();
        let language = engine.last_detected_language();
        self.history_note(|pending| {
            if let Some(model) = metadata.model {
                pending.entry.model = model;
            }
            pending.entry.backend = metadata.backend;
            pending.entry.model_load_ms = metadata.model_load_ms;
            if language.is_some() {
                pending.entry.language = language;
            }
        });
    }

    /// Start recording a dictation to history, if it is on and not paused.
    fn history_eager_transcriber_metadata(&self, engine: &dyn Transcriber) {
        if self
            .eager_decoded
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.history_transcriber_metadata(engine);
        } else {
            let loaded = engine.runtime_metadata();
            self.history_note(|pending| {
                if let Some(model) = loaded.model {
                    pending.entry.model = model;
                }
                pending.entry.model_load_ms = loaded.model_load_ms;
                pending.entry.backend = None;
                pending.entry.language = None;
            });
        }
    }

    fn history_begin(&self, state: &State) {
        use crate::history::Trigger;
        let pending = self
            .history
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .begin();
        let pending = pending.map(|mut p| {
            let model_override = match state {
                State::Recording { model_override, .. }
                | State::EagerRecording { model_override, .. }
                | State::Streaming { model_override, .. } => model_override.as_deref(),
                _ => None,
            };
            let (engine, model) = self.recording_engine_and_model(model_override);
            p.entry.engine = engine;
            p.entry.model = model;
            p.entry.language = self.config.active_language().map(str::to_string);
            p.entry.streaming = state.is_streaming();
            p.entry.trigger = if self.is_external_trigger {
                Trigger::Record
            } else {
                Trigger::Hotkey
            };
            let device = self.config.audio.device.trim();
            p.entry.input_device = (!device.is_empty()).then(|| device.to_string());
            let (variant, _) = crate::history::recorder::binary_variant_and_backend();
            p.entry.variant = variant;
            p
        });
        *self.dictation.lock().unwrap_or_else(|e| e.into_inner()) = pending;
    }

    /// The dictation is over: write it in the background.
    fn history_commit(&self) {
        if let Some(pending) = self.history_take() {
            self.history_commit_pending(pending);
        }
    }

    /// Take the dictation's history entry out of the recorder, for a
    /// dictation whose text is delivered after the next one starts.
    fn history_take(&self) -> Option<crate::history::recorder::Pending> {
        let mut pending = self
            .dictation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()?;
        if self.osd_limit_reached && pending.entry.ended_by == crate::history::EndedBy::Released {
            pending.entry.ended_by = crate::history::EndedBy::Timeout;
        }
        Some(pending)
    }

    fn history_swap(
        &self,
        pending: Option<crate::history::recorder::Pending>,
    ) -> Option<crate::history::recorder::Pending> {
        std::mem::replace(
            &mut *self.dictation.lock().unwrap_or_else(|e| e.into_inner()),
            pending,
        )
    }

    fn history_commit_pending(&self, pending: crate::history::recorder::Pending) {
        let _ = self
            .history
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .commit(pending);
    }

    /// Play audio feedback sound if enabled
    fn play_feedback(&self, event: SoundEvent) {
        if let Some(ref feedback) = self.audio_feedback {
            feedback.play(event);
        }
    }

    /// Pause MPRIS media players if configured, storing which ones were paused
    async fn pause_media_players(&mut self) {
        if self.config.audio.pause_media {
            self.paused_media_players =
                audio::media::pause_playing_players(&self.config.audio.pause_media_ignored_players)
                    .await;
        }
    }

    /// Duck active audio streams if configured, storing original volumes
    async fn duck_media_streams(&mut self) {
        if self.config.audio.duck_media {
            // Wait out any restore still fading up. Its final write is what
            // puts the streams back at their true original volumes, and
            // enumerating before that lands would capture intermediate values
            // as the new originals — every fast toggle cycle would then store
            // a quieter baseline and media would drift down permanently.
            // Normally already finished, so this costs nothing.
            if let Some(task) = self.media_fade_task.take() {
                let _ = task.await;
            }
            let (streams, fade) = audio::media::duck_playing_audio(
                self.config.audio.duck_media_volume_percent,
                self.config.audio.duck_media_fade_ms,
            )
            .await;
            self.ducked_media_streams = streams;
            self.media_fade_task = fade;
        }
    }

    /// Restore any audio streams that were ducked at recording start
    fn restore_ducked_media_streams(&mut self) {
        if !self.ducked_media_streams.is_empty() {
            // Abort rather than await a fade still on its way down: this path
            // is synchronous, and the restore we are about to spawn ends by
            // writing the stored originals, so an interrupted duck ramp is
            // corrected either way.
            if let Some(task) = self.media_fade_task.take() {
                task.abort();
            }
            let streams = std::mem::take(&mut self.ducked_media_streams);
            self.media_fade_task = Some(tokio::spawn(audio::media::restore_ducked_audio(
                streams,
                self.config.audio.duck_media_volume_percent,
                self.config.audio.duck_media_fade_ms,
            )));
        }
    }

    /// Resume any MPRIS media players that were paused at recording start
    fn resume_media_players(&mut self) {
        if !self.paused_media_players.is_empty() {
            let players = std::mem::take(&mut self.paused_media_players);
            tokio::spawn(audio::media::resume_players(players));
        }
    }

    /// Suppress media before opening the microphone so playback cannot leak
    /// into the beginning of a recording.
    async fn suppress_recording_media(&mut self) {
        self.pause_media_players().await;
        self.duck_media_streams().await;
    }

    /// Restore media as soon as microphone capture has stopped. Transcription
    /// and text output may continue after this point without keeping playback
    /// paused or ducked.
    fn restore_recording_media(&mut self) {
        self.restore_ducked_media_streams();
        self.resume_media_players();
    }

    /// Leave the failure visible until the next recording/state change, without
    /// changing the public state-file protocol or requiring audio feedback.
    fn queue_language(&mut self, language: &str) -> std::result::Result<(), String> {
        let path = self
            .config_path
            .clone()
            .or_else(Config::resolve_existing_path)
            .ok_or("No configuration file is available")?;
        self.pending_language = Some(crate::language_selection::persist(
            &self.config,
            &path,
            language,
        )?);
        Ok(())
    }

    fn apply_pending_language(&mut self, idle: bool) -> bool {
        if !idle {
            return false;
        }
        let Some(next) = self.pending_language.take() else {
            return false;
        };
        // No active capture/transcription is touched. All future construction,
        // including resident workers and per-record overrides, uses this snapshot.
        if let Some(task) = self.model_load_task.take() {
            task.abort();
        }
        if let Some((_, task)) = self.override_load_task.take() {
            task.abort();
        }
        if let Some(task) = self.whisper_prepare_task.take() {
            task.abort();
        }
        self.transcriber_preloaded = None;
        self.override_cache = None;
        self.config = next;
        self.model_manager = (self.config.engine == crate::config::TranscriptionEngine::Whisper)
            .then(|| ModelManager::new(&self.config.whisper, self.config_path.clone()));
        self.text_processor = TextProcessor::new_for_config(
            &self.config.text,
            self.config.text_language(),
            &self.config,
        );
        tracing::info!(language = ?self.config.active_language(), "Tray language applied for next dictation");
        true
    }

    fn show_tray_error(&self) {
        #[cfg(target_os = "linux")]
        if let Some(tray) = &self.tray {
            tray.update(crate::tray::TrayState::Error);
        }
    }

    /// Update the state file if configured
    fn update_state(&self, state_name: &str) {
        self.note_idle_transition(state_name);
        #[cfg(target_os = "linux")]
        if let Some(tray) = &self.tray {
            tray.update(crate::tray::TrayState::from_state_name(state_name));
        }
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
                let suppressed = read_bool_override("no_osd").unwrap_or(false);
                self.osd_session_suppressed
                    .store(suppressed, std::sync::atomic::Ordering::Relaxed);
                set_osd_suppressed(suppressed);
            }
            "idle" | "stopped" => {
                self.history_commit();
                set_osd_suppressed(false);
                if self.state_file_path.is_some() {
                    osd::session::clear_session(&Config::runtime_dir());
                }
            }
            _ => {}
        }
    }

    /// Resolve `[osd] frontend` (logging the choice and why, once) and
    /// start the OSD supervisor. Returns the active frontend's name for
    /// `voxtype status`, or "none" when nothing can run.
    fn spawn_osd(&mut self) -> &'static str {
        use osd::config::OsdFrontend;
        let configured = osd::frontend::effective_configured(self.config.osd.frontend);
        let resolution = osd::frontend::resolve(configured, &osd::frontend::SystemProbe);
        // Tell the user once per start when the OSD they asked for by name
        // won't be the one shown. The default `auto` never notifies, only
        // logs below. Respawns don't repeat it. Off the runtime threads,
        // since the notification is sent synchronously.
        if let Some((title, body)) = osd::frontend::fallback_notice(&resolution) {
            tokio::task::spawn_blocking(move || crate::notification::send_sync(&title, &body));
        }
        let Some(active) = resolution.active else {
            tracing::warn!(
                "OSD frontend: none ({}). Run `voxtype setup osd --status` for details, \
                 or set [osd] enabled = false.",
                resolution.reason
            );
            return "none";
        };
        tracing::info!("OSD frontend: {} ({})", active.as_str(), resolution.reason);
        // Explicit values keep the launcher's own resolution, unchanged.
        let forced = (configured == OsdFrontend::Auto).then_some(active);
        self.osd_supervisor_task = Some(osd::supervisor::spawn(forced));
        active.as_str()
    }

    /// Publish `recording.json` for the session `state` just started, so OSD
    /// frontends can show elapsed time, the remaining time before
    /// `max_duration_secs`, and which engine and model this recording uses.
    /// Skipped when the state file is disabled, like every runtime file the
    /// OSD reads.
    fn begin_osd_session(&mut self, state: &State) {
        self.active_transcriber = None;
        self.session_text_is_english = match state {
            State::Recording { model_override, .. }
            | State::EagerRecording { model_override, .. } => {
                self.override_text_is_english(model_override.as_deref())
            }
            // Streaming always runs the configured model.
            _ => true,
        };
        self.eager_decoded = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.history_begin(state);
        self.osd_limit_reached = false;
        self.osd_outcome_sent
            .store(false, std::sync::atomic::Ordering::Relaxed);
        if self.state_file_path.is_none() {
            return;
        }
        let (mode, model_override) = match state {
            State::Recording { model_override, .. }
            | State::EagerRecording { model_override, .. } => ("recording", model_override),
            State::Streaming { model_override, .. } => ("streaming", model_override),
            _ => return,
        };
        let (engine, model) = self.recording_engine_and_model(model_override.as_deref());
        self.osd_session_started_at_ms = osd::session::now_unix_ms();
        osd::session::write_session(
            &Config::runtime_dir(),
            &osd::session::OsdSession {
                mode: mode.to_string(),
                started_at_ms: self.osd_session_started_at_ms,
                max_duration_secs: u64::from(self.config.audio.max_duration_secs),
                engine,
                model,
            },
        );
    }

    /// Whether English-only cleanup may run on a recording made with
    /// `model_override`.
    ///
    /// The text processor is built for the configured engine and model. A
    /// `--model` override swaps in another model, even another engine, for
    /// one recording, and what made the configured one English (an
    /// English-only model, most of all) may not hold for it. So the same
    /// rule is asked of the configuration that recording runs with. Without
    /// an override the processor's own language gate is the answer.
    fn override_text_is_english(&self, model_override: Option<&str>) -> bool {
        let Some(name) = model_override else {
            return true;
        };
        match model_catalog::override_route(&self.config, name) {
            Ok(route) => model_catalog::override_text_is_english(&self.config, name, &route),
            // Refused before it records anything.
            Err(_) => true,
        }
    }

    /// The engine and model that will serve a recording, after any
    /// per-recording `--model` override.
    fn recording_engine_and_model(&self, model_override: Option<&str>) -> (String, String) {
        let engine = self.config.engine.to_string();
        let Some(name) = model_override else {
            return (engine, self.config.model_name().to_string());
        };
        match crate::model_catalog::override_route(&self.config, name) {
            Ok(crate::model_catalog::OverrideRoute::Separate { engine, model }) => {
                (engine.to_string(), model)
            }
            _ => (engine, name.to_string()),
        }
    }

    /// Publish the outcome of a session that delivered `text`.
    fn publish_osd_text_outcome(&self, text: &str) {
        let chars = text.chars().count();
        let kind = if chars == 0 {
            osd::session::OutcomeKind::Empty
        } else {
            osd::session::OutcomeKind::Ok
        };
        self.publish_osd_outcome(kind, chars);
    }

    /// Publish `outcome.json` saying how the session just ended, for OSD
    /// frontends to confirm the result.
    fn publish_osd_outcome(&self, outcome: osd::session::OutcomeKind, chars: usize) {
        {
            use crate::history::Outcome;
            use osd::session::OutcomeKind;
            let decided = match outcome {
                OutcomeKind::Ok => Outcome::Delivered,
                OutcomeKind::NoSpeech => Outcome::NoSpeech,
                OutcomeKind::TooShort => Outcome::TooShort,
                OutcomeKind::Empty => Outcome::Empty,
                OutcomeKind::Cancelled => Outcome::Cancelled,
                // A silent microphone is the history's "heard no speech",
                // seen before transcription instead of after it.
                OutcomeKind::NoInput => Outcome::NoSpeech,
                OutcomeKind::Error => Outcome::TranscriptionFailed,
                OutcomeKind::OutputFailed => Outcome::OutputFailed,
            };
            self.history_note(|p| p.outcome(decided, None));
        }
        if self.state_file_path.is_none()
            || self
                .osd_outcome_sent
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        osd::session::write_outcome(
            &Config::runtime_dir(),
            &osd::session::OsdOutcome {
                outcome,
                session_started_at_ms: self.osd_session_started_at_ms,
                suppressed: self
                    .osd_session_suppressed
                    .load(std::sync::atomic::Ordering::Relaxed),
                chars,
                limit_reached: self.osd_limit_reached,
                at_ms: osd::session::now_unix_ms(),
            },
        );
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

    /// Arm the dead-input guard for the capture about to start.
    fn start_dead_input_guard(&mut self) -> Option<audio::input_guard::DeadInputGuard> {
        self.dead_input_rx = None;
        if !self.config.audio.dead_input.enabled {
            return None;
        }

        let (guard, report_rx) = audio::input_guard::DeadInputGuard::new(
            self.config.audio.sample_rate,
            &self.config.audio.dead_input,
        );
        self.dead_input_rx = Some(report_rx);
        Some(guard)
    }

    fn disarm_dead_input_guard(&mut self) {
        self.dead_input_rx = None;
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
        // ever apply to a recording that was live, or already requested,
        // when it was issued (#606).
        discard_stale_cancel(self.start_request);
        #[cfg(test)]
        let capture = match self.history_test_capture.take() {
            Some(capture) => Ok(capture),
            None => audio::create_capture(&self.config.audio),
        };
        #[cfg(not(test))]
        let capture = audio::create_capture(&self.config.audio);
        match capture {
            Ok(mut capture) => match capture.start().await {
                Ok(chunk_rx) => {
                    self.is_external_trigger = track_silence;
                    let speech_tracker = self.new_speech_tracker(track_silence);
                    self.silence_tracker = speech_tracker.clone();
                    let guard = self.start_dead_input_guard();
                    let sink = self.level_hub.as_ref().map(|hub| hub.frame_sink());

                    // Cancel any prior emitter (defensive; should be idle).
                    if let Some(handle) = self.level_emitter_task.take() {
                        handle.abort();
                    }
                    // With no consumer, chunk_rx is dropped, as it was before
                    // silence tracking and the dead-input guard existed.
                    if audio::levels::needs_chunk_pump(
                        sink.is_some(),
                        speech_tracker.is_some(),
                        guard.is_some(),
                    ) {
                        self.level_emitter_task = Some(audio::levels::spawn_chunk_pump(
                            chunk_rx,
                            sink,
                            None,
                            speech_tracker,
                            guard,
                        ));
                    }
                    self.capture_notices.clear();
                    Ok(capture)
                }
                Err(e) => {
                    tracing::error!("Failed to start audio: {}", e);
                    self.show_tray_error();
                    self.report_capture_failure(&e).await;
                    Err(())
                }
            },
            Err(e) => {
                tracing::error!("Failed to create audio capture: {}", e);
                self.show_tray_error();
                self.report_capture_failure(&e).await;
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

    /// File-mode delivery with the daemon's clipboard fallback. Tells the
    /// user when the transcript didn't land at the path they asked for.
    async fn deliver_file_output(&self, path: &Path, text: &str) -> FileDelivery {
        let cfg = &self.config.output;
        let fallback = if cfg.fallback_to_clipboard {
            let mut clipboard = cfg.clone();
            clipboard.mode = OutputMode::Clipboard;
            output::create_output_chain(&clipboard)
        } else {
            Vec::new()
        };
        let context = self.history_context();
        let delivery = deliver_to_file_observed(path, text, cfg, &fallback, context.as_ref()).await;
        match &delivery {
            FileDelivery::Written => {}
            FileDelivery::FellBack(err) => {
                notification::send(
                    "Voxtype",
                    &format!(
                        "Could not write {}: {}. Transcription copied to the clipboard instead.",
                        path.display(),
                        err
                    ),
                )
                .await
            }
            FileDelivery::Lost(err) => {
                notification::send(
                    "Voxtype",
                    &format!(
                        "Could not write {}: {}. The transcription was not saved.",
                        path.display(),
                        err
                    ),
                )
                .await
            }
        }
        delivery
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
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_handle: &mut Option<StreamHandle>,
        streaming_session: &mut Option<StreamingSession>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
        model_override: Option<String>,
        track_silence: bool,
    ) -> bool {
        // Same stale-trigger hazard as start_recording_capture: Streaming is
        // an is_recording() state, so a leftover cancel file would kill the
        // session moments after it starts. See #606.
        discard_stale_cancel(self.start_request);
        // An override model has its own transcriber, which records in batch
        // mode; streaming would run the default model instead.
        if let Some(name) = model_override.as_deref() {
            if matches!(
                model_catalog::override_route(&self.config, name),
                Ok(model_catalog::OverrideRoute::Separate { .. })
            ) {
                return false;
            }
        }
        // Clone out of the field so the borrow doesn't overlap the
        // `&mut self` capture start below.
        let Some(transcriber) = self.transcriber_preloaded.clone() else {
            return false;
        };
        if transcriber.as_streaming().is_none() {
            return false;
        }
        self.streaming_recovery_audio.clear();
        self.streaming_stop_deadline = None;
        self.streaming_recovery_file_output_path = None;

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

        // Batch transcription reads the output override when it has a final
        // transcript. A streaming session needs the same override before it
        // builds its chain, otherwise --clipboard, --type, and --paste all
        // silently use the configured default. A `--file=path` (or
        // `mode = "file"`) session accumulates into `finalized_text` instead
        // of typing, gated on `file_output_path`; its chain goes unused.
        let output_override = read_output_mode_override();
        let file_output_path = self.resolve_file_output_path(&output_override, None);
        let output_config = apply_output_override(&self.config.output, output_override);

        *audio_capture = Some(capture);
        *streaming_handle = Some(handle);
        *streaming_chain = Some(output::create_output_chain(&output_config));
        *state = State::Streaming {
            started_at: std::time::Instant::now(),
            model_override,
            partial_buffer: String::new(),
            finalized_text: String::new(),
            typed_chars: 0,
            file_output_path,
        };
        self.update_state("streaming");
        self.begin_osd_session(state);
        self.active_transcriber = Some(transcriber);
        self.history_note(|pending| pending.transcribing(0));
        *streaming_session =
            Some(StreamingSession::new().with_history_context(self.history_context()));
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
    async fn stop_active_recording(
        &mut self,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
        eager_transcriber: &mut Option<Arc<dyn Transcriber>>,
    ) {
        self.disarm_dead_input_guard();

        // Notify an external-trigger caller that this session is ending,
        // regardless of why (stop, silence timeout, hard timeout) — it may
        // not be the one that decided to stop it — and disarm silence
        // tracking for the next session. Gated on the recording actually
        // being active: SIGUSR2 can arrive while idle, and a stale
        // `is_external_trigger` from an earlier session must not fire the
        // hook for an unrelated later signal.
        self.end_external_session(state.is_recording()).await;

        if state.is_streaming() {
            tracing::info!("Stopping streaming session; closing capture and disowning output");
            self.request_streaming_stop(audio_capture, streaming_chain, true)
                .await;
        } else if let State::Recording { model_override, .. } = &*state {
            let model_override = model_override.clone();

            self.start_transcription_task(state, audio_capture, model_override)
                .await;
        } else if state.is_eager_recording() {
            // Handle eager recording stop via external trigger - extract model_override first
            let model_override = match &*state {
                State::EagerRecording { model_override, .. } => model_override.clone(),
                _ => None,
            };

            let duration = state.recording_duration().unwrap_or_default();
            tracing::info!("Eager recording stopped ({:.1}s)", duration.as_secs_f32());

            // Stop audio capture and get remaining samples
            if let Some(mut capture) = audio_capture.take() {
                if let Ok(final_samples) = capture.stop().await {
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

            let transcriber = if let Some(selected) = eager_transcriber.as_ref() {
                selected.clone()
            } else {
                match self
                    .get_transcriber_for_recording(model_override.as_deref())
                    .await
                {
                    Ok(t) => t,
                    Err(()) => {
                        *state = State::Idle;
                        self.update_state("idle");
                        return;
                    }
                }
            };

            self.update_state("transcribing");

            if let Some(text) = self.finish_eager_recording(state, transcriber).await {
                *state = State::Transcribing { audio: Vec::new() };
                self.handle_transcription_result(state, Ok(Ok(text))).await;
            } else {
                tracing::debug!("Eager recording produced empty result");
                self.active_transcriber = None;
                self.publish_osd_outcome(osd::session::OutcomeKind::Empty, 0);
                self.reset_to_idle(state).await;
            }
            *eager_transcriber = None;
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
    /// retain the capture's complete sample buffer for recovery, start the
    /// OSD silence pump, and stop the mic.
    async fn stop_streaming_capture(&mut self, audio_capture: &mut Option<Box<dyn AudioCapture>>) {
        self.disarm_dead_input_guard();
        self.cut_streaming_audio();
        self.start_streaming_drain_pump();
        if let Some(mut capture) = audio_capture.take() {
            match capture.stop().await {
                Ok(samples) => {
                    self.history_note(|pending| pending.entry.audio_ms = samples.len() as u64 / 16);
                    // Kept for `end_streaming`, which recovers the dictation
                    // from it if the backend fails before typing anything.
                    self.streaming_recovery_audio = samples;
                }
                Err(error) => tracing::warn!("Streaming capture stop failed: {}", error),
            }
        }
        self.restore_recording_media();
    }

    // The event pump and private completion regressions share this exact error
    // branch. Keep it separate from capture startup, which opens real devices.
    async fn end_streaming_with_error(
        &mut self,
        error: crate::error::TranscribeError,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_handle: &mut Option<StreamHandle>,
        streaming_session: &mut Option<StreamingSession>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
    ) {
        tracing::error!("Streaming backend error: {}", error);
        self.end_external_session(state.is_recording()).await;
        send_notification(
            "Streaming Error",
            &error.to_string(),
            self.config.output.notification.show_engine_icon,
            self.config.engine,
            "critical",
        )
        .await;
        self.end_streaming(
            state,
            audio_capture,
            streaming_handle,
            streaming_session,
            streaming_chain,
            StreamingEndReason::Failed(Some(error.to_string())),
        )
        .await;
    }

    /// Stop the microphone for a streaming session and arm the stop
    /// watchdog. With `disown_output`, drop the typing surface so anything
    /// the backend still sends lands in the session silently instead of
    /// typing into whatever window has focus by then; `end_streaming`
    /// delivers it through the batch output path if nothing was typed.
    async fn request_streaming_stop(
        &mut self,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
        disown_output: bool,
    ) {
        self.stop_streaming_capture(audio_capture).await;
        self.arm_streaming_stop_watchdog();
        if disown_output {
            *streaming_chain = None;
        }
    }

    fn streaming_recovery_enabled(&self) -> bool {
        self.config.streaming_recovery.unwrap_or_default().enabled
    }

    fn arm_streaming_stop_watchdog(&mut self) {
        // The bound runs from the first stop request. A repeated stop while
        // the backend drains must not push the deadline out again.
        if self.streaming_stop_deadline.is_some() {
            return;
        }
        let timeout_secs = self
            .config
            .streaming_recovery
            .unwrap_or_default()
            .stop_timeout_secs
            .clamp(1, 120);
        self.streaming_stop_deadline =
            Some(tokio::time::Instant::now() + Duration::from_secs(timeout_secs));
    }

    /// End a streaming session: after the backend's `Ended`, after an
    /// error, or when the stop watchdog fires. Never waits on the backend
    /// task; it is cancelled and aborted, so a backend stuck on a dead
    /// socket cannot hold the daemon.
    ///
    /// A session that typed text finishes as it always has. One that typed
    /// nothing is not thrown away: after a failure its retained audio goes
    /// through batch transcription, and text that arrived after the stop
    /// disowned the typing surface goes through the batch output path.
    async fn end_streaming(
        &mut self,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_handle: &mut Option<StreamHandle>,
        streaming_session: &mut Option<StreamingSession>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
        reason: StreamingEndReason,
    ) {
        self.stop_streaming_capture(audio_capture).await;
        if let Some(handle) = streaming_handle.take() {
            let _ = handle.cancel.send(());
            handle.task.abort();
        }
        self.stop_streaming_drain_pump();
        self.streaming_stop_deadline = None;
        let samples = std::mem::take(&mut self.streaming_recovery_audio);

        // Fold any leftover `partial` in first: the sliding-window engine
        // can confirm a whole short utterance as a single Partial and leave
        // nothing for `final_flush` to send as Final. See
        // `finalize_pending_partial`.
        if let Some(session) = streaming_session.as_mut() {
            session.finalize_pending_partial();
        }
        let typed_chars = streaming_session
            .as_ref()
            .map_or(0, StreamingSession::typed_chars);
        let buffered = streaming_session
            .as_ref()
            .map(|s| s.finalized_text().to_string())
            .unwrap_or_default();
        let file_output_path = match &*state {
            State::Streaming {
                file_output_path, ..
            } => file_output_path.clone(),
            _ => None,
        };

        let failure = match &reason {
            StreamingEndReason::Failed(error) => Some(error.clone()),
            StreamingEndReason::Completed => None,
        };
        if typed_chars == 0 {
            if failure.is_some() && self.streaming_recovery_enabled() && !samples.is_empty() {
                *streaming_session = None;
                *streaming_chain = None;
                self.recover_streaming_audio(state, file_output_path, samples)
                    .await;
                return;
            }
            // Only text the session held silently because the stop
            // disowned the typing surface. With the chain still attached,
            // the text already went through it, delivered or failed, and
            // must not be sent again.
            if file_output_path.is_none()
                && streaming_chain.is_none()
                && !buffered.trim().is_empty()
            {
                *streaming_session = None;
                *streaming_chain = None;
                self.deliver_buffered_streaming_text(state, buffered);
                return;
            }
        }
        if let Some(Some(error)) = failure {
            self.history_note(|pending| {
                pending.outcome(crate::history::Outcome::TranscriptionFailed, Some(error))
            });
            self.publish_osd_outcome(osd::session::OutcomeKind::Error, 0);
        }

        if let Some(engine) = self.active_transcriber.take() {
            self.history_transcriber_metadata(engine.as_ref());
        }
        self.history_note(|pending| pending.transcribed());

        // File-output sessions (`--file=path`) never typed anything as
        // they went — see the event pump's `file_output` branch — so the
        // accumulated text only exists in the session. Write it out now,
        // before the session is dropped below. Mirrors the classic
        // (non-streaming) path's file handling: the output hooks wrap the
        // write and a failed write falls back to the clipboard.
        if let Some(output_path) = file_output_path {
            *streaming_session = None;
            *streaming_chain = None;

            // The session accumulated raw engine output (the event pump's
            // file_output branches deliberately skip per-segment
            // processing), so this is where the transcript becomes final:
            // apply replacements and spoken punctuation over the whole
            // text, same as the batch path does before writing (#669).
            let profile = self.recording_profile();
            let vocabulary = profile
                .as_ref()
                .and_then(|name| self.profile_vocabularies.get(name));
            let raw_text = buffered;
            let final_text = self
                .text_processor
                .process_with_vocabulary(&raw_text, vocabulary);
            self.history_note(|p| p.texts(&raw_text, &final_text));

            let delivery = self.deliver_file_output(&output_path, &final_text).await;
            {
                let (fell_back, error) = match &delivery {
                    FileDelivery::Written => (false, None),
                    FileDelivery::FellBack(_) => (true, None),
                    FileDelivery::Lost(err) => (false, Some(err.to_string())),
                };
                let fallback_text =
                    fell_back.then(|| output::delivered_form(&final_text).into_owned());
                self.history_note(|p| {
                    if let Some(text) = &fallback_text {
                        p.delivered_text(text);
                    }
                    p.entry.output = Some(crate::history::OutputInfo {
                        mode: "file".into(),
                        driver: Some(if fell_back { "clipboard" } else { "file" }.into()),
                        fell_back,
                    });
                    if let Some(err) = error {
                        p.outcome(crate::history::Outcome::OutputFailed, Some(err));
                    }
                });
            }
            match delivery {
                FileDelivery::Written => {
                    let mode_str = match self.config.output.file_mode {
                        FileMode::Overwrite => "wrote",
                        FileMode::Append => "appended",
                    };
                    tracing::info!("{} streamed transcription to {:?}", mode_str, output_path);
                    self.publish_osd_text_outcome(&final_text);
                    self.play_feedback(SoundEvent::TranscriptionComplete);
                }
                FileDelivery::FellBack(_) => {
                    self.publish_osd_text_outcome(&final_text);
                    self.play_feedback(SoundEvent::TranscriptionComplete);
                }
                FileDelivery::Lost(_) => {
                    self.publish_osd_outcome(osd::session::OutcomeKind::OutputFailed, 0);
                    self.play_feedback(SoundEvent::Error);
                }
            }
            output::deferred_notice::flush().await;

            *state = State::Idle;
            self.update_state("idle");
            self.discard_recording_overrides();
            return;
        }

        // Each piece went through the output chain, which changes one
        // character for another and never the count, so the whole text in
        // its delivered form is the pieces as they were delivered.
        let streamed = streaming_session
            .as_ref()
            .map(|s| output::delivered_form(s.finalized_text()).into_owned())
            .unwrap_or_default();
        let raw = streaming_session
            .as_ref()
            .map(StreamingSession::history_raw_text)
            .unwrap_or_default();
        let output_error = streaming_session
            .as_ref()
            .and_then(|session| session.output_error())
            .map(str::to_owned);
        let delivery = streaming_session
            .as_ref()
            .and_then(|session| session.last_delivery())
            .cloned();
        {
            let mode = format!("{:?}", self.config.output.mode).to_lowercase();
            self.history_note(|p| {
                p.texts(&raw, &streamed);
                p.entry.output = Some(crate::history::OutputInfo {
                    mode,
                    driver: delivery.as_ref().map(|delivery| delivery.driver.clone()),
                    fell_back: delivery.as_ref().is_some_and(|delivery| delivery.fell_back),
                });
                if let Some(error) = output_error {
                    p.outcome(crate::history::Outcome::OutputFailed, Some(error));
                }
            });
        }
        self.publish_osd_text_outcome(&streamed);
        *streaming_session = None;
        *streaming_chain = None;

        self.play_feedback(SoundEvent::TranscriptionComplete);

        if let Some(cmd) = &self.config.output.post_output_command {
            if let Err(e) = output::run_hook(cmd, "post_output").await {
                tracing::warn!("{}", e);
            }
        }
        output::deferred_notice::flush().await;

        *state = State::Idle;
        self.update_state("idle");
        self.discard_recording_overrides();
    }

    /// Deliver text a streaming session buffered but never typed (it
    /// arrived after the stop disowned the typing surface) through the
    /// ordinary batch result path: text processing, history and the
    /// output chain, exactly like a batch dictation.
    fn deliver_buffered_streaming_text(&mut self, state: &mut State, text: String) {
        tracing::info!("Delivering buffered streaming text through normal output");
        self.history_note(|p| p.transcribing(0));
        *state = State::Transcribing { audio: Vec::new() };
        self.update_state("transcribing");
        self.active_transcriber = None;
        self.transcription_task = Some(tokio::task::spawn_blocking(move || Ok(text)));
    }

    /// Recover a streaming dictation that failed before it typed anything:
    /// run the capture's retained audio through batch transcription and
    /// the ordinary batch result path.
    ///
    /// A cloud engine (Soniox) that just lost its connection is the least
    /// likely thing to transcribe this audio, so it recovers with the
    /// configured `[whisper]` backend first. A local engine recovers with
    /// its own batch path, which is already loaded. Either way the other
    /// one is the fallback, because any transcript beats losing it.
    async fn recover_streaming_audio(
        &mut self,
        state: &mut State,
        file_output_path: Option<PathBuf>,
        samples: Vec<f32>,
    ) {
        // Batch output resolves its file target from one-shot overrides
        // that the streaming start already consumed; carry it across.
        self.streaming_recovery_file_output_path = file_output_path;
        self.history_note(|p| p.transcribing(samples.len()));

        let audio_duration = samples.len() as f32 / 16000.0;
        if audio_duration < 0.3 {
            tracing::debug!(
                "Recovered recording too short ({:.2}s), ignoring",
                audio_duration
            );
            self.publish_empty_outcome();
            self.publish_osd_outcome(osd::session::OutcomeKind::TooShort, 0);
            self.reset_to_idle(state).await;
            return;
        }
        if let Some(vad) = &self.vad {
            match vad.detect(&samples) {
                Ok(result) if !result.has_speech => {
                    self.report_no_speech(&result).await;
                    self.publish_empty_outcome();
                    self.reset_to_idle(state).await;
                    return;
                }
                Ok(result) => tracing::debug!(
                    "Speech detected in recovered audio: {:.2}s ({:.1}%)",
                    result.speech_duration_secs,
                    result.speech_ratio * 100.0
                ),
                Err(error) => tracing::warn!(
                    "VAD failed for recovered audio, proceeding anyway: {}",
                    error
                ),
            }
        }

        let prefer_whisper = self.config.engine == crate::config::TranscriptionEngine::Soniox;
        let fallback = if prefer_whisper { "Whisper" } else { "batch" };
        tracing::warn!(
            "Streaming session ended without text; recovering {:.1}s with {} transcription",
            audio_duration,
            fallback
        );
        *state = State::Transcribing {
            audio: samples.clone(),
        };
        self.update_state("transcribing");

        let preloaded = self.transcriber_preloaded.clone();
        let running_config = self.config.clone();
        let mut whisper_config = self.config.clone();
        whisper_config.engine = crate::config::TranscriptionEngine::Whisper;
        self.active_transcriber = None;
        self.transcription_task = Some(tokio::task::spawn_blocking(move || {
            let build = |whisper: bool| -> std::result::Result<
                Arc<dyn Transcriber>,
                crate::error::TranscribeError,
            > {
                match (whisper, preloaded.clone()) {
                    (true, _) => {
                        crate::transcribe::create_transcriber(&whisper_config).map(Arc::from)
                    }
                    (false, Some(running)) => Ok(running),
                    (false, None) => {
                        crate::transcribe::create_transcriber(&running_config).map(Arc::from)
                    }
                }
            };
            match build(prefer_whisper) {
                Ok(transcriber) => transcriber.transcribe(&samples),
                Err(error) => {
                    tracing::warn!(
                        "Recovery transcriber unavailable ({error}); trying the other one"
                    );
                    build(!prefer_whisper)?.transcribe(&samples)
                }
            }
        }));
        send_notification(
            "Dictation Recovered",
            &format!(
                "Streaming ended before it typed anything. Transcribing the recording with {} instead.",
                if prefer_whisper { "Whisper" } else { "batch transcription" }
            ),
            self.config.output.notification.show_engine_icon,
            self.config.engine,
            "normal",
        )
        .await;
    }

    /// Carry out a `voxtype record cancel` trigger against whatever session is
    /// live: discard a recording (batch, eager, or streaming) or abort a
    /// transcription. The caller has already consumed the trigger file.
    #[allow(clippy::too_many_arguments)]
    async fn cancel_requested_session(
        &mut self,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_handle: &mut Option<StreamHandle>,
        streaming_session: &mut Option<StreamingSession>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
        eager_transcriber: &mut Option<Arc<dyn Transcriber>>,
    ) {
        self.disarm_dead_input_guard();

        if state.is_streaming() {
            tracing::info!("Recording cancelled");
            self.end_external_session(true).await;
            self.cancel_streaming_to_idle(
                state,
                audio_capture,
                streaming_handle,
                streaming_session,
                streaming_chain,
                "Recording discarded",
            )
            .await;
            return;
        }

        if state.is_recording() {
            tracing::info!("Recording cancelled");

            // Stop recording and discard audio
            if let Some(mut capture) = audio_capture.take() {
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

            if let Some(engine) = self.active_transcriber.take() {
                if state.is_eager_recording() {
                    self.history_eager_transcriber_metadata(engine.as_ref());
                } else {
                    self.history_transcriber_metadata(engine.as_ref());
                }
            } else if let Some(engine) = eager_transcriber.as_ref() {
                // Loaded does not mean this recording reached a decode.
                let loaded = engine.runtime_metadata();
                self.history_note(|pending| {
                    if let Some(model) = loaded.model {
                        pending.entry.model = model;
                    }
                    pending.entry.model_load_ms = loaded.model_load_ms;
                });
            }
            self.history_note(|pending| pending.transcribed());

            if let State::EagerRecording {
                accumulated_audio,
                chunk_results,
                chunks_sent,
                tasks_in_flight,
                ..
            } = state
            {
                accumulated_audio.clear();
                chunk_results.clear();
                *chunks_sent = 0;
                *tasks_in_flight = 0;
            }

            self.discard_recording_overrides();
            // A cancelled external-trigger session is still an ended
            // session — tell the caller and disarm tracking.
            self.end_external_session(true).await;
            *state = State::Idle;
            *eager_transcriber = None;
            self.publish_osd_outcome(osd::session::OutcomeKind::Cancelled, 0);
            self.update_state("idle");
            self.play_feedback(SoundEvent::Cancelled);

            // Run post_output_command to reset compositor submap
            if let Some(cmd) = &self.config.output.post_output_command {
                if let Err(e) = output::run_hook(cmd, "post_output").await {
                    tracing::warn!("{}", e);
                }
            }

            end_recording_notification(
                "Cancelled",
                "Recording discarded",
                &self.config.output.notification,
                self.config.engine,
            )
            .await;
            return;
        }

        if matches!(state, State::Transcribing { .. }) {
            tracing::info!("Transcription cancelled");

            // Abort the transcription task
            if let Some(task) = self.transcription_task.take() {
                task.abort();
            }
            // Drop the cloned transcriber Arc so it isn't held until the
            // next transcription.
            if let Some(engine) = self.active_transcriber.as_ref() {
                self.history_transcriber_metadata(engine.as_ref());
            }
            self.history_note(|pending| pending.transcribed());
            self.active_transcriber = None;
            // A cancelled streaming recovery must not hand its file target
            // to the next batch dictation.
            self.streaming_recovery_file_output_path = None;

            self.discard_recording_overrides();
            *state = State::Idle;
            self.publish_osd_outcome(osd::session::OutcomeKind::Cancelled, 0);
            self.update_state("idle");
            self.play_feedback(SoundEvent::Cancelled);

            // Run post_output_command to reset compositor submap
            if let Some(cmd) = &self.config.output.post_output_command {
                if let Err(e) = output::run_hook(cmd, "post_output").await {
                    tracing::warn!("{}", e);
                }
            }

            end_recording_notification(
                "Cancelled",
                "Transcription aborted",
                &self.config.output.notification,
                self.config.engine,
            )
            .await;
        }
    }

    /// Tear down an active streaming session: signal the backend, drop
    /// capture, rewind any typed text, and reset to idle. Callers choose the
    /// feedback and notification.
    #[allow(clippy::too_many_arguments)]
    async fn teardown_streaming_to_idle(
        &mut self,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_handle: &mut Option<StreamHandle>,
        streaming_session: &mut Option<StreamingSession>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
        outcome: osd::session::OutcomeKind,
    ) {
        if let Some(handle) = streaming_handle.take() {
            let _ = handle.cancel.send(());
            handle.task.abort();
        }
        self.cut_streaming_audio();
        if let Some(mut capture) = audio_capture.take() {
            let _ = capture.stop().await;
        }
        self.streaming_recovery_audio.clear();
        self.streaming_stop_deadline = None;
        self.streaming_recovery_file_output_path = None;

        self.restore_recording_media();
        if streaming_chain.is_some() {
            if let Some(session) = streaming_session.as_mut() {
                if let Err(error) = session.rewind().await {
                    tracing::warn!("Streaming rewind failed: {}", error);
                }
            }
        }
        self.stop_streaming_drain_pump();
        *streaming_session = None;
        *streaming_chain = None;

        if let Some(engine) = self.active_transcriber.take() {
            self.history_transcriber_metadata(engine.as_ref());
        }
        self.history_note(|pending| pending.transcribed());

        self.discard_recording_overrides();
        *state = State::Idle;
        self.publish_osd_outcome(outcome, 0);
        self.update_state("idle");
        self.disarm_dead_input_guard();
    }

    /// Cancel an active streaming session with cancellation feedback.
    #[allow(clippy::too_many_arguments)]
    async fn cancel_streaming_to_idle(
        &mut self,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_handle: &mut Option<StreamHandle>,
        streaming_session: &mut Option<StreamingSession>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
        notification_body: &str,
    ) {
        self.teardown_streaming_to_idle(
            state,
            audio_capture,
            streaming_handle,
            streaming_session,
            streaming_chain,
            osd::session::OutcomeKind::Cancelled,
        )
        .await;
        self.play_feedback(SoundEvent::Cancelled);

        if let Some(cmd) = &self.config.output.post_output_command {
            if let Err(e) = output::run_hook(cmd, "post_output").await {
                tracing::warn!("{}", e);
            }
        }

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

    /// Stop a capture that delivered no audible input and explain why.
    #[allow(clippy::too_many_arguments)]
    async fn abort_recording_dead_input(
        &mut self,
        report: audio::input_guard::DeadInputReport,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_handle: &mut Option<StreamHandle>,
        streaming_session: &mut Option<StreamingSession>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
        eager_transcriber: &mut Option<Arc<dyn Transcriber>>,
    ) {
        self.end_external_session(state.is_recording()).await;
        if let Some(path) = peek_file_output_path(&self.config) {
            write_result_sidecar(&path, &TranscriptOutcome::error("Microphone silent"));
        }

        if state.is_streaming() {
            self.teardown_streaming_to_idle(
                state,
                audio_capture,
                streaming_handle,
                streaming_session,
                streaming_chain,
                osd::session::OutcomeKind::NoInput,
            )
            .await;
            // Leave the compositor submap that pre_recording_command entered,
            // as every other streaming exit does.
            if let Some(cmd) = &self.config.output.post_output_command {
                if let Err(error) = output::run_hook(cmd, "post_output").await {
                    tracing::warn!(%error, "post-output hook failed");
                }
            }
        } else {
            self.stop_level_emitter();
            if let Some(mut capture) = audio_capture.take() {
                let _ = capture.stop().await;
            }
            if let Some(task) = self.model_load_task.take() {
                task.abort();
            }
            for (_, task) in self.eager_chunk_tasks.drain(..) {
                task.abort();
            }
            if let State::EagerRecording {
                accumulated_audio,
                chunk_results,
                chunks_sent,
                tasks_in_flight,
                ..
            } = state
            {
                accumulated_audio.clear();
                chunk_results.clear();
                *chunks_sent = 0;
                *tasks_in_flight = 0;
            }
            *eager_transcriber = None;
            self.publish_osd_outcome(osd::session::OutcomeKind::NoInput, 0);
            self.reset_to_idle(state).await;
        }

        self.show_tray_error();

        self.play_feedback(SoundEvent::Error);
        notification::close_persistent().await;
        let source = audio::input_guard::source_label(&self.config.audio.device).await;
        tracing::warn!(
            source = %source,
            peak_dbfs = report.peak_dbfs,
            threshold_dbfs = report.threshold_dbfs,
            "microphone delivered no audible input; recording stopped"
        );
        notification::send_alert(
            "Microphone silent",
            &format!(
                "No sound from {} in the first {:.1} seconds. \
                 Check that the microphone is powered on and unmuted.",
                source,
                report.window_ms as f32 / 1_000.0,
            ),
        )
        .await;
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
        #[cfg(test)]
        let capture = match self.history_test_capture.take() {
            Some(capture) => Ok(capture),
            None => audio::create_capture(&self.config.audio),
        };
        #[cfg(not(test))]
        let capture = audio::create_capture(&self.config.audio);
        match capture {
            Ok(mut capture) => match capture.start().await {
                Ok(chunk_rx) => {
                    // Bounded; backed-up streaming backend drops chunks
                    // rather than back-pressuring the capture.
                    let (streaming_tx, streaming_rx) =
                        tokio::sync::mpsc::channel::<Vec<f32>>(STREAMING_TAP_CAPACITY);

                    if let Some(handle) = self.level_emitter_task.take() {
                        handle.abort();
                    }
                    self.is_external_trigger = track_silence;
                    let speech_tracker = self.new_speech_tracker(track_silence);
                    self.silence_tracker = speech_tracker.clone();
                    let guard = self.start_dead_input_guard();
                    let sink = self.level_hub.as_ref().map(|hub| hub.frame_sink());
                    self.level_emitter_task = Some(audio::levels::spawn_chunk_pump(
                        chunk_rx,
                        sink,
                        Some(streaming_tx),
                        speech_tracker,
                        guard,
                    ));
                    self.capture_notices.clear();
                    Ok((capture, streaming_rx))
                }
                Err(e) => {
                    tracing::error!("Failed to start audio: {}", e);
                    self.show_tray_error();
                    self.report_capture_failure(&e).await;
                    Err(())
                }
            },
            Err(e) => {
                tracing::error!("Failed to create audio capture: {}", e);
                self.show_tray_error();
                self.report_capture_failure(&e).await;
                Err(())
            }
        }
    }

    /// The profile of the current recording. Recording start, transcriber
    /// prep, post-processing, output and each streamed segment all ask, and
    /// the override file only survives the first read, so the name is kept
    /// here (#1097). A file written since (`record stop --profile`) replaces
    /// it, so a profile named only at stop still applies.
    fn recording_profile(&mut self) -> Option<String> {
        if let Some(name) = read_profile_override() {
            self.recording_profile = Some(name);
        }
        self.recording_profile.clone()
    }

    /// Remove every per-recording override (`voxtype record start --file`,
    /// `--model`, `--profile`, `--auto-submit`, ...) when a recording is
    /// discarded or finished, and forget the profile kept from them. A normal
    /// stop, including a max-duration timeout, must leave them for the result
    /// handler, which reads them when it delivers the text.
    fn discard_recording_overrides(&mut self) {
        discard_recording_overrides_in(&Config::runtime_dir());
        self.recording_profile = None;
    }

    /// Forget the profile of a recording whose capture failed to start.
    fn discard_recording_profile(&mut self) {
        cleanup_profile_override();
        self.recording_profile = None;
    }

    fn active_profile_vocabulary_terms(&mut self) -> Vec<String> {
        self.recording_profile()
            .as_ref()
            .and_then(|name| self.profile_vocabularies.get(name))
            .map(|vocabulary| vocabulary.terms().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Get the model ready while the user speaks: start an on-demand load,
    /// or warm the preloaded transcriber (spawning the gpu_isolation worker).
    /// An override the active transcriber can't serve loads its own instead.
    fn prepare_for_recording(&mut self, model_override: Option<&str>) {
        // A new recording carries only the profile its own override names.
        self.recording_profile = read_profile_override();
        let vocabulary = self.active_profile_vocabulary_terms();
        if let Some(name) = model_override {
            if let Ok(model_catalog::OverrideRoute::Separate { engine, model }) =
                model_catalog::override_route(&self.config, name)
            {
                self.start_override_load(name, engine, &model, vocabulary);
                return;
            }
        }

        if self.config.on_demand_loading() {
            // Start model loading in background
            match self.config.engine {
                crate::config::TranscriptionEngine::Whisper => {
                    let config = self.config.whisper.clone();
                    let config_path = self.config_path.clone();
                    let model_to_load = model_override.map(str::to_string);
                    let vocabulary = vocabulary.clone();
                    self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                        let mut temp_manager = ModelManager::new(&config, config_path);
                        let transcriber = temp_manager.get_transcriber(model_to_load.as_deref())?;
                        transcriber.set_vocabulary(&vocabulary);
                        Ok(transcriber)
                    }));
                }
                crate::config::TranscriptionEngine::Parakeet
                | crate::config::TranscriptionEngine::Moonshine
                | crate::config::TranscriptionEngine::SenseVoice
                | crate::config::TranscriptionEngine::Paraformer
                | crate::config::TranscriptionEngine::Dolphin
                | crate::config::TranscriptionEngine::Omnilingual
                | crate::config::TranscriptionEngine::Cohere
                | crate::config::TranscriptionEngine::Gguf
                | crate::config::TranscriptionEngine::OpenVino
                | crate::config::TranscriptionEngine::Soniox => {
                    let config = self.config.clone();
                    let vocabulary = vocabulary.clone();
                    self.model_load_task = Some(tokio::task::spawn_blocking(move || {
                        let transcriber: Arc<dyn Transcriber> =
                            Arc::from(crate::transcribe::create_transcriber(&config)?);
                        transcriber.set_vocabulary(&vocabulary);
                        Ok(transcriber)
                    }));
                }
            }
            tracing::debug!("Started background model loading");
        } else {
            // Prepare model (spawns subprocess for gpu_isolation mode)
            match self.config.engine {
                crate::config::TranscriptionEngine::Whisper => {
                    if let Some(ref mut mm) = self.model_manager {
                        match mm.prepare_model(model_override, &vocabulary) {
                            Ok(handle) => {
                                self.whisper_prepare_task = handle;
                            }
                            Err(e) => {
                                tracing::warn!("Failed to prepare model: {}", e);
                            }
                        }
                    }
                }
                crate::config::TranscriptionEngine::Parakeet
                | crate::config::TranscriptionEngine::Moonshine
                | crate::config::TranscriptionEngine::SenseVoice
                | crate::config::TranscriptionEngine::Paraformer
                | crate::config::TranscriptionEngine::Dolphin
                | crate::config::TranscriptionEngine::Omnilingual
                | crate::config::TranscriptionEngine::Cohere
                | crate::config::TranscriptionEngine::Gguf
                | crate::config::TranscriptionEngine::OpenVino
                | crate::config::TranscriptionEngine::Soniox => {
                    if let Some(ref t) = self.transcriber_preloaded {
                        let transcriber = t.clone();
                        transcriber.set_vocabulary(&vocabulary);
                        tokio::task::spawn_blocking(move || {
                            transcriber.prepare();
                        });
                    }
                }
            }
        }
    }

    /// Reject an unusable `--model` override before recording starts, so the
    /// user hears the error now rather than after speaking.
    async fn accept_model_override(&self, model_override: Option<&str>) -> bool {
        let Some(name) = model_override else {
            return true;
        };
        match model_catalog::override_route(&self.config, name) {
            Ok(_) => true,
            Err(problem) => {
                self.report_override_problem(&problem).await;
                false
            }
        }
    }

    /// Tell the user the recording will stop soon because of the cap.
    async fn warn_recording_cap(&self, left: Duration, max_duration: Duration) {
        tracing::info!(
            "Recording reaches the {:.0}s limit in {:.0}s",
            max_duration.as_secs_f32(),
            left.as_secs_f32()
        );
        send_notification_with_lifetime(
            "Recording Limit Approaching",
            &format!(
                "Recording stops in {:.0}s (limit: audio.max_duration_secs = {}).",
                left.as_secs_f32().ceil(),
                max_duration.as_secs()
            ),
            self.config.output.notification.show_engine_icon,
            self.config.engine,
            &self.config.output.notification.urgency,
            Lifetime::Millis(5000),
        )
        .await;
    }

    /// Tell the user the recording was cut off by the cap, not by them, and
    /// how to raise it. Without this a long dictation just stopped.
    async fn notify_recording_capped(&self, max_duration: Duration) {
        send_notification_with_lifetime(
            "Recording Limit Reached",
            &format!(
                "Stopped at {}s and transcribing what was captured. To record longer: \
                 voxtype config set audio.max_duration_secs <SECS> (or --max-duration).",
                max_duration.as_secs()
            ),
            self.config.output.notification.show_engine_icon,
            self.config.engine,
            "critical",
            Lifetime::Millis(10000),
        )
        .await;
    }

    /// Save the audio of a recording whose transcription failed, so the
    /// failure costs a retry instead of the whole dictation, and tell the
    /// user where it is and how to retry it. With `[audio]
    /// keep_failed_recordings` off nothing is written, and the notification
    /// says so instead of naming a file.
    async fn keep_failed_recording(&self, state: &State) {
        let State::Transcribing { audio } = state else {
            return;
        };
        if audio.is_empty() {
            return;
        }
        let audio = audio.clone();
        let keep = self.config.audio.keep_failed_recordings;
        let saved =
            tokio::task::spawn_blocking(move || crate::failed_recording::keep(keep, &audio))
                .await
                .map_err(anyhow::Error::from)
                .and_then(|r| r);
        match saved {
            Ok(saved) => {
                // The retry command stays up until dismissed so it can be
                // copied. With nothing to retry, the notice just times out.
                let lifetime = match &saved {
                    Some(path) => {
                        tracing::error!(
                            "Recording saved to {}. Retry with: voxtype transcribe {}",
                            path.display(),
                            path.display()
                        );
                        Lifetime::UntilClosed
                    }
                    None => {
                        tracing::info!("Recording not kept: [audio] keep_failed_recordings is off");
                        Lifetime::Millis(10000)
                    }
                };
                send_notification_with_lifetime(
                    "Transcription Failed",
                    &crate::failed_recording::notice_body(saved.as_deref()),
                    self.config.output.notification.show_engine_icon,
                    self.config.engine,
                    "critical",
                    lifetime,
                )
                .await;
            }
            Err(e) => tracing::error!("Could not save the failed recording: {}", e),
        }
    }

    /// Tell the user a recording was discarded because VAD heard no speech.
    /// Logged at info and notified (unless `on_no_speech` is off): at debug
    /// it read as a hang, since nothing else happens after the stop.
    async fn report_no_speech(&self, result: &crate::vad::VadResult) {
        tracing::info!(
            "No speech detected ({:.2}s above threshold, {:.1}% of the recording, avg rms {:.4}); \
             discarding the recording",
            result.speech_duration_secs,
            result.speech_ratio * 100.0,
            result.rms_energy
        );
        self.play_feedback(SoundEvent::Cancelled);
        if self.config.output.notification.on_no_speech {
            send_notification(
                "Voxtype: No speech detected",
                "Nothing was transcribed. If you were speaking, your microphone level may be too low.",
                self.config.output.notification.show_engine_icon,
                self.config.engine,
                &self.config.output.notification.urgency,
            )
            .await;
        }
        self.publish_osd_outcome(osd::session::OutcomeKind::NoSpeech, 0);
    }

    async fn report_override_problem(&self, problem: &str) {
        tracing::error!("Model override failed: {}", problem);
        self.play_feedback(SoundEvent::Error);
        send_notification(
            "Voxtype: Model Override Failed",
            problem,
            self.config.output.notification.show_engine_icon,
            self.config.engine,
            &self.config.output.notification.urgency,
        )
        .await;
    }

    /// Post a failure notice for ten seconds: long enough to read the fix.
    async fn post_failure_notice(&self, notice: &crate::failure_notice::Notice) {
        send_notification_with_lifetime(
            &notice.title,
            &notice.body,
            self.config.output.notification.show_engine_icon,
            self.config.engine,
            &self.config.output.notification.urgency,
            Lifetime::Millis(10_000),
        )
        .await;
    }

    /// The microphone could not be opened. The error beep alone goes
    /// through the same audio stack that may be broken, so say what failed
    /// and how to fix it, once per distinct failure.
    async fn report_capture_failure(&mut self, err: &crate::error::AudioError) {
        self.play_feedback(SoundEvent::Error);
        let device = self.config.audio.device.clone();
        if self
            .capture_notices
            .should_notify(&format!("{device}|{err}"), Instant::now())
        {
            let notice = crate::failure_notice::capture_failed(&device, err);
            self.post_failure_notice(&notice).await;
        }
    }

    /// The model for this recording could not be loaded.
    async fn report_model_failure(
        &mut self,
        model_override: Option<&str>,
        err: &crate::error::TranscribeError,
    ) {
        self.play_feedback(SoundEvent::Error);
        let model = model_override
            .unwrap_or(self.config.model_name())
            .to_string();
        if self
            .model_notices
            .should_notify(&format!("{model}|{err}"), Instant::now())
        {
            let notice = crate::failure_notice::model_load_failed(&self.config, &model, err, false);
            self.post_failure_notice(&notice).await;
        }
    }

    /// Start building the override's transcriber in the background so it's
    /// ready by the time the recording stops.
    fn start_override_load(
        &mut self,
        name: &str,
        engine: crate::config::TranscriptionEngine,
        model: &str,
        vocabulary: Vec<String>,
    ) {
        if let Some((cached, transcriber)) = &self.override_cache {
            if cached == name {
                let transcriber = transcriber.clone();
                transcriber.set_vocabulary(&vocabulary);
                tokio::task::spawn_blocking(move || transcriber.prepare());
                return;
            }
        }
        if matches!(&self.override_load_task, Some((pending, _)) if pending == name) {
            return;
        }
        tracing::info!("Loading {:?} model '{}' for this recording", engine, model);
        let config = override_config(&self.config, engine, model);
        self.override_load_task = Some((
            name.to_string(),
            tokio::task::spawn_blocking(move || {
                let transcriber: Arc<dyn Transcriber> =
                    Arc::from(crate::transcribe::create_transcriber(&config)?);
                transcriber.set_vocabulary(&vocabulary);
                transcriber.prepare();
                Ok(transcriber)
            }),
        ));
    }

    /// The override transcriber once it's loaded, without waiting for it.
    /// Eager chunking polls this during recording.
    async fn ready_override(&mut self, name: &str) -> Option<Arc<dyn Transcriber>> {
        if let Some((cached, transcriber)) = &self.override_cache {
            if cached == name {
                return Some(transcriber.clone());
            }
        }
        let finished = matches!(
            &self.override_load_task,
            Some((pending, task)) if pending == name && task.is_finished()
        );
        if !finished {
            return None;
        }
        let (_, task) = self.override_load_task.take()?;
        match task.await {
            Ok(Ok(transcriber)) => {
                self.override_cache = Some((name.to_string(), transcriber.clone()));
                Some(transcriber)
            }
            // Recording stop retries the load and reports the failure once.
            Ok(Err(e)) => {
                tracing::debug!("Override model '{}' failed to load: {}", name, e);
                None
            }
            Err(e) => {
                tracing::debug!("Override model '{}' load task panicked: {}", name, e);
                None
            }
        }
    }

    /// The transcriber for an override the active engine can't serve: the
    /// cached one, the load started at recording start, or a fresh build.
    async fn override_transcriber(
        &mut self,
        name: &str,
        engine: crate::config::TranscriptionEngine,
        model: &str,
    ) -> std::result::Result<Arc<dyn Transcriber>, ()> {
        let on_demand = self.config.on_demand_loading();
        if matches!(&self.override_cache, Some((cached, _)) if cached == name) {
            let (key, transcriber) = self.override_cache.take().expect("matched above");
            if !on_demand {
                self.override_cache = Some((key, transcriber.clone()));
            }
            return Ok(transcriber);
        }
        let task = match self.override_load_task.take() {
            Some((pending, task)) if pending == name => task,
            _ => {
                let config = override_config(&self.config, engine, model);
                tokio::task::spawn_blocking(move || {
                    crate::transcribe::create_transcriber(&config).map(Arc::from)
                })
            }
        };
        match task.await {
            Ok(Ok(transcriber)) => {
                // On-demand loading frees models after use; honor that here.
                if !on_demand {
                    self.override_cache = Some((name.to_string(), transcriber.clone()));
                }
                Ok(transcriber)
            }
            Ok(Err(e)) => {
                self.report_override_problem(&format!("Could not load model '{name}': {e}"))
                    .await;
                Err(())
            }
            Err(e) => {
                self.report_override_problem(&format!("Loading model '{name}' panicked: {e}"))
                    .await;
                Err(())
            }
        }
    }

    /// Get the transcriber for the current recording session
    ///
    /// For on-demand loading: waits for the background model load task to complete
    /// For preloaded models: returns the preloaded transcriber (Parakeet) or gets from model manager (Whisper)
    /// A `--model` override the active transcriber can't serve gets its own.
    ///
    /// Returns Ok(transcriber) on success, Err(()) if an error occurred and caller should skip to next iteration
    /// `get_transcriber_for_recording_inner`, publishing an error outcome
    /// for the OSD when no transcriber can serve the recording.
    async fn get_transcriber_for_recording(
        &mut self,
        model_override: Option<&str>,
    ) -> std::result::Result<Arc<dyn Transcriber>, ()> {
        let result = self
            .get_transcriber_for_recording_inner(model_override)
            .await;
        if result.is_ok() {
            self.recording_engine = self.recording_engine_and_model(model_override).0;
            self.model_notices.clear();
        } else {
            self.publish_osd_outcome(osd::session::OutcomeKind::Error, 0);
        }
        result
    }

    async fn get_transcriber_for_recording_inner(
        &mut self,
        model_override: Option<&str>,
    ) -> std::result::Result<Arc<dyn Transcriber>, ()> {
        let vocabulary = self.active_profile_vocabulary_terms();
        if let Some(name) = model_override {
            match model_catalog::override_route(&self.config, name) {
                Ok(model_catalog::OverrideRoute::Separate { engine, model }) => {
                    let transcriber = self.override_transcriber(name, engine, &model).await?;
                    tracing::info!(
                        "Transcribing with override model '{}' ({:?} engine)",
                        name,
                        engine
                    );
                    transcriber.set_vocabulary(&vocabulary);
                    return Ok(transcriber);
                }
                Ok(_) => {}
                Err(problem) => {
                    self.report_override_problem(&problem).await;
                    return Err(());
                }
            }
        }
        if self.config.on_demand_loading() {
            // Wait for background model load task
            if let Some(task) = self.model_load_task.take() {
                match task.await {
                    Ok(Ok(transcriber)) => {
                        tracing::info!("Model loaded successfully");
                        transcriber.set_vocabulary(&vocabulary);
                        Ok(transcriber)
                    }
                    Ok(Err(e)) => {
                        tracing::error!("Model loading failed: {}", e);
                        self.report_model_failure(model_override, &e).await;
                        Err(())
                    }
                    Err(e) => {
                        tracing::error!("Model loading task panicked: {}", e);
                        let e = crate::error::TranscribeError::InitFailed(format!(
                            "model loading panicked: {e}"
                        ));
                        self.report_model_failure(model_override, &e).await;
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
                crate::config::TranscriptionEngine::Parakeet
                | crate::config::TranscriptionEngine::Moonshine
                | crate::config::TranscriptionEngine::SenseVoice
                | crate::config::TranscriptionEngine::Paraformer
                | crate::config::TranscriptionEngine::Dolphin
                | crate::config::TranscriptionEngine::Omnilingual
                | crate::config::TranscriptionEngine::Cohere
                | crate::config::TranscriptionEngine::Gguf
                | crate::config::TranscriptionEngine::OpenVino
                | crate::config::TranscriptionEngine::Soniox => {
                    if let Some(t) = self.transcriber_preloaded.clone() {
                        t.set_vocabulary(&vocabulary);
                        Ok(t)
                    } else {
                        // Empty on the non-on-demand path means the panic
                        // recovery discarded a poisoned instance (#643).
                        // Re-create rather than error so one bad recording
                        // doesn't disable every one after it.
                        tracing::info!("Transcriber not loaded; creating a fresh instance");
                        let config = self.config.clone();
                        let created = tokio::task::spawn_blocking(move || {
                            crate::transcribe::create_transcriber(&config)
                        })
                        .await;
                        match created {
                            Ok(Ok(t)) => {
                                let t: Arc<dyn Transcriber> = Arc::from(t);
                                t.set_vocabulary(&vocabulary);
                                self.transcriber_preloaded = Some(t.clone());
                                Ok(t)
                            }
                            Ok(Err(e)) => {
                                tracing::error!("Failed to re-create transcriber: {}", e);
                                self.report_model_failure(model_override, &e).await;
                                Err(())
                            }
                            Err(e) => {
                                tracing::error!("Transcriber creation task panicked: {}", e);
                                let e = crate::error::TranscribeError::InitFailed(format!(
                                    "model loading panicked: {e}"
                                ));
                                self.report_model_failure(model_override, &e).await;
                                Err(())
                            }
                        }
                    }
                }
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
                            Ok(t) => {
                                t.set_vocabulary(&vocabulary);
                                Ok(t)
                            }
                            Err(e) => {
                                tracing::error!("Failed to get transcriber: {}", e);
                                self.report_model_failure(model_override, &e).await;
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

    /// Update the meeting state file if configured
    fn update_meeting_state(&mut self, state_name: &str, meeting_id: Option<&str>) {
        if let Some(ref path) = self.meeting_state_file_path {
            write_meeting_state_file(path, state_name, meeting_id);
        }
        #[cfg(target_os = "linux")]
        if let Some(tray) = &self.tray {
            tray.update_meeting(crate::tray::MeetingTrayState::from_state_name(state_name));
        }

        // Mirror into the meeting HUD snapshot. "idle" always means the
        // meeting fully ended (or none is running): reset everything so a
        // HUD connecting afterwards doesn't see a stale title/transcript
        // from the meeting that just finished.
        if state_name == "idle" {
            self.meeting_hud = meeting::MeetingHudState::idle();
            meeting::silent_track::write_status(&Config::runtime_dir(), &[]);
        } else {
            self.meeting_hud.status = state_name.to_string();
            if let Some(id) = meeting_id {
                self.meeting_hud.meeting_id = Some(id.to_string());
            }
        }
        // Read on every change, so views see an acknowledgement made from
        // anywhere by the next status at the latest.
        self.meeting_hud.recording_notice = meeting::notice::pending();

        if let Some(ref hub) = self.meeting_event_hub {
            hub.update_snapshot(&self.meeting_hud);
            hub.publish_status(state_name);
        }
    }

    /// A meeting was retitled (`voxtype meeting title`, or from the
    /// calendar), its storage already updated: bring the running meeting,
    /// its speaker names and the meeting view up to date.
    async fn apply_retitle(&mut self, retitle: meeting::retitle::Retitle) {
        let live = self
            .meeting_daemon
            .as_ref()
            .and_then(|d| d.current_meeting_id())
            .is_some_and(|id| id.to_string() == retitle.meeting_id);
        if live {
            if let Some(ref mut daemon) = self.meeting_daemon {
                daemon.retitle(&retitle.title, retitle.url.as_deref()).await;
            }
        } else {
            // Stopped since: its last save may have kept the old title.
            let storage = match self.meeting_daemon.as_ref() {
                Some(d) => d.storage().config().clone(),
                None => StorageConfig {
                    storage_path: if self.config.meeting.storage_path == "auto" {
                        Config::data_dir().join("meetings")
                    } else {
                        PathBuf::from(&self.config.meeting.storage_path)
                    },
                    retain_audio: self.config.meeting.retain_audio,
                    max_meetings: 0,
                    my_name: self.config.meeting.my_name.clone(),
                },
            };
            let reapplied = Ok::<_, String>(storage).and_then(|config| {
                let storage = meeting::MeetingStorage::open(config).map_err(|e| e.to_string())?;
                let id = storage
                    .resolve_meeting_id(&retitle.meeting_id)
                    .map_err(|e| e.to_string())?;
                meeting::retitle::retitle_stored(
                    &storage,
                    &id,
                    &retitle.title,
                    retitle.url.as_deref(),
                    true,
                )
                .map_err(|e| e.to_string())
            });
            if let Err(e) = reapplied {
                tracing::debug!("Retitle of a meeting no longer running: {}", e);
            }
        }
        if self.meeting_hud.meeting_id.as_deref() == Some(retitle.meeting_id.as_str()) {
            self.meeting_hud.title = Some(retitle.title.clone());
        }
        if let Some(ref hub) = self.meeting_event_hub {
            hub.update_snapshot(&self.meeting_hud);
            hub.publish_title_changed(&retitle.meeting_id, &retitle.title);
        }
    }

    /// Ask the calendar, off the event loop, what the meeting that just
    /// started is called. A title found is stored and sent back through the
    /// retitle trigger, unless the meeting got a real title meanwhile. Any
    /// failure leaves the title as it is.
    fn title_from_calendar(
        &self,
        meeting_id: meeting::MeetingId,
        storage: StorageConfig,
        started_title: Option<String>,
    ) {
        let lookup = meeting::calendar::Lookup::current(&self.config.meeting.calendar);
        tokio::spawn(async move {
            let Some(found) = lookup.title().await else {
                return;
            };
            let source = found.source;
            let stored = tokio::task::spawn_blocking(move || {
                let storage = meeting::MeetingStorage::open(storage).map_err(|e| e.to_string())?;
                let current = storage
                    .get_meeting(&meeting_id)
                    .map_err(|e| e.to_string())?
                    .ok_or("the meeting is gone")?;
                // Retitled by hand meanwhile: that title stays.
                if current.title != started_title {
                    return Err(format!(
                        "it was titled {:?} meanwhile",
                        current.title.unwrap_or_default()
                    ));
                }
                meeting::retitle::retitle_stored(
                    &storage,
                    &meeting_id,
                    &found.title,
                    found.url.as_deref(),
                    false,
                )
                .map_err(|e| e.to_string())
            })
            .await;
            match stored {
                Ok(Ok(done)) => {
                    tracing::info!(
                        "Meeting {} titled {:?} from {}",
                        meeting_id,
                        done.title,
                        source
                    );
                    let trigger = meeting::retitle::Retitle {
                        meeting_id: meeting_id.to_string(),
                        title: done.title,
                        url: done.url,
                    };
                    if let Err(e) =
                        meeting::retitle::write_trigger(&Config::runtime_dir(), &trigger)
                    {
                        tracing::warn!("Could not hand the calendar title to the meeting: {}", e);
                    }
                }
                Ok(Err(e)) => tracing::debug!("Calendar title not used: {}", e),
                Err(e) => tracing::debug!("Calendar title not used: {}", e),
            }
        });
    }

    /// Add a processed chunk to the HUD snapshot and stream it on meeting.sock.
    fn publish_meeting_chunk(&mut self, chunk_id: u32, segments: &[meeting::TranscriptSegment]) {
        // The live view shows [meeting] my_name for "You", like the saved
        // transcript does; a display label only, never saved.
        let mut shown = meeting::Transcript {
            segments: segments.to_vec(),
            total_chunks: 0,
            audio_ms: 0,
        };
        if let Some(name) = self.config.meeting.my_name.as_deref() {
            shown.apply_my_name(name);
        }
        let segments = &shown.segments[..];
        self.meeting_hud.push_segments(segments);
        self.meeting_hud.chunk_count = chunk_id + 1;
        if let Some(ref hub) = self.meeting_event_hub {
            hub.publish_segments(chunk_id, segments);
            hub.update_snapshot(&self.meeting_hud);
        }
    }

    /// Rename a speaker in the HUD snapshot and tell meeting.sock subscribers.
    fn publish_meeting_rename(&mut self, from: &str, to: &str) {
        self.meeting_hud.rename_speaker(from, to);
        if let Some(ref hub) = self.meeting_event_hub {
            hub.publish_speaker_renamed(from, to);
            hub.update_snapshot(&self.meeting_hud);
        }
    }

    /// Replace a segment in the HUD snapshot and tell meeting.sock subscribers.
    fn publish_segment_updated(&mut self, segment: &meeting::TranscriptSegment) {
        self.meeting_hud.update_segment(segment);
        if let Some(ref hub) = self.meeting_event_hub {
            hub.publish_segment_updated(segment);
            hub.update_snapshot(&self.meeting_hud);
        }
    }

    /// Dictation's loaded transcriber, when a meeting configured as `meeting`
    /// can use it instead of loading its own copy.
    fn resident_transcriber_for(&mut self, meeting: &Config) -> Option<Arc<dyn Transcriber>> {
        if !meeting::start::shares_dictation_transcriber(&self.config, meeting) {
            return None;
        }
        if self.config.engine == crate::config::TranscriptionEngine::Whisper {
            self.model_manager.as_mut()?.get_transcriber(None).ok()
        } else {
            self.transcriber_preloaded.clone()
        }
    }

    /// Drop a deleted segment from the HUD snapshot and tell meeting.sock
    /// subscribers.
    fn publish_segment_deleted(&mut self, segment_id: u32) {
        self.meeting_hud.remove_segment(segment_id);
        if let Some(ref hub) = self.meeting_event_hub {
            let meeting_id = self.meeting_hud.meeting_id.clone().unwrap_or_default();
            hub.publish_segment_deleted(&meeting_id, segment_id);
            hub.update_snapshot(&self.meeting_hud);
        }
    }

    /// Start a new meeting
    async fn start_meeting(
        &mut self,
        title: Option<String>,
        diarization_override: Option<String>,
        options: crate::config::MeetingStartOptions,
    ) -> Result<()> {
        if self.meeting_daemon.is_some() {
            tracing::warn!("Meeting already in progress");
            return Ok(());
        }

        // The CLI checked these before writing the trigger; check again, as
        // anything can write the trigger file.
        let meeting_app_config =
            meeting::start::meeting_transcription_config(&self.config, &options).map_err(|e| {
                tracing::error!("Refusing to start the meeting: {}", e);
                crate::error::VoxtypeError::Config(e)
            })?;
        let announce_clip = meeting::announce::Clip::for_meeting(&self.config.meeting, &options)
            .map(|clip| {
                let volume = meeting::announce::volume_for_meeting(&self.config.meeting, &options);
                (clip, volume)
            });
        let announces = announce_clip.is_some();

        // CLI override (validated against DIARIZATION_BACKENDS by clap) wins over config.
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

        // Captured for the HUD snapshot before `diarization_config` and
        // `title` are moved into `meeting_config` / `daemon.start()` below.
        let (diar_enabled_for_hud, diar_backend_for_hud) = match &diarization_config {
            Some(cfg) => (cfg.enabled, Some(cfg.backend.clone())),
            None => (false, None),
        };
        let title_for_hud = title.clone();

        // A title from the call window, a detection placeholder or none at
        // all gives way to the calendar's; one the user typed never does.
        let calendar_title_wanted =
            options.title_from_window || meeting::calendar::is_replaceable(title.as_deref());
        let started_title = title.clone();
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
                my_name: self.config.meeting.my_name.clone(),
            },
            retain_audio: self.config.meeting.retain_audio,
            max_duration_mins: self.config.meeting.max_duration_mins,
            vad_threshold: self.config.meeting.audio.vad_threshold,
            diarization: diarization_config,
            echo_rules: (&self.config.meeting.audio).into(),
            auto_label_from_title: self.config.meeting.auto_label_from_title,
        };

        let storage_for_calendar = meeting_config.storage.clone();

        // Start capturing before anything loads. Building the meeting daemon
        // loads the transcription model (a GGUF worker, seconds on a CPU) and
        // the diarizer, and `meeting start` has already returned, so anyone
        // who starts talking then lost the opening of the meeting. The
        // capture buffers what arrives meanwhile, and the first chunk holds it.
        // A sound check holds the same devices; it gives way.
        self.stop_soundcheck("meeting").await;
        let opened = meeting::soundcheck::open_capture(&self.config)
            .await
            .map_err(crate::error::VoxtypeError::Audio)?;
        let mut capture = opened.capture;
        let echo_path = opened.echo_path;
        let mut echo_canceller = opened.canceller;
        let liveness = opened.liveness;
        // Zero on the transcript's timeline: the first sample captured.
        let capture_started = std::time::Instant::now();
        if capture.has_loopback() {
            tracing::info!("Dual audio capture: mic + loopback");
        } else {
            tracing::info!("Single audio capture: mic only");
        }

        // The meeting's events, read by the event loop. Sending never
        // waits: the meeting runs on the loop's own task.
        let (tx, rx) = meeting::event_channel();
        self.meeting_event_rx = Some(rx);

        // Create meeting daemon, on dictation's loaded model when it is the
        // same one (#808: loading a second copy took most of a slow start).
        let resident = self.resident_transcriber_for(&meeting_app_config);
        match MeetingDaemon::new(meeting_config, &meeting_app_config, tx, resident) {
            Ok(mut daemon) => {
                // What this meeting will be missing, told to the user in one
                // notification once it is recording.
                let mut notices: Vec<String> = Vec::new();
                if let Some(notice) = daemon.diarization_notice() {
                    if self.meeting_notices_shown.insert(notice.to_string()) {
                        notices.push(notice.to_string());
                    }
                }
                // Every meeting, not once: each one loses the remote side of
                // the conversation.
                if let Some(issue) = capture.loopback_issue() {
                    notices.push(issue.message());
                }
                match daemon.start(title, options.url).await {
                    Ok(meeting_id) => {
                        let id_str = meeting_id.to_string();
                        self.meeting_hud = meeting::MeetingHudState {
                            status: "recording".to_string(),
                            meeting_id: Some(id_str.clone()),
                            title: title_for_hud,
                            started_at: Some(chrono::Utc::now()),
                            chunk_count: 0,
                            diarization: meeting::DiarizationInfo {
                                enabled: diar_enabled_for_hud,
                                backend: diar_backend_for_hud,
                            },
                            // Set once GTCRN has loaded or not, below.
                            echo_cancel: None,
                            segments: Vec::new(),
                            // Set from the notice's state in update_meeting_state.
                            recording_notice: false,
                            announcement: None,
                            announces,
                        };
                        self.update_meeting_state("recording", Some(&id_str));
                        tracing::info!("Meeting started: {}", meeting_id);
                        self.start_meeting_levels(capture.meters(), &id_str);
                        if calendar_title_wanted {
                            self.title_from_calendar(
                                meeting_id,
                                storage_for_calendar.clone(),
                                started_title.clone(),
                            );
                        }
                        self.meeting_audio_capture = Some(capture);

                        let (used, echo_notice) = self.meeting_echo_path_in_use(echo_path);
                        if let Some(notice) = echo_notice {
                            notices.push(notice);
                        }
                        self.meeting_gtcrn = used == audio::echo_cancel::EchoPath::Gtcrn;
                        self.meeting_aec_liveness = liveness;
                        self.meeting_echo_canceller = echo_canceller.take();
                        tracing::info!("Meeting {} echo handling: {}", meeting_id, used.as_str());
                        daemon.record_echo_cancel(used.as_str());
                        self.meeting_hud.echo_cancel = Some(used.as_str().to_string());
                        if let Some(ref hub) = self.meeting_event_hub {
                            hub.update_snapshot(&self.meeting_hud);
                        }

                        self.meeting_daemon = Some(daemon);
                        self.meeting_mic_buffer.clear();
                        self.meeting_loopback_buffer.clear();
                        self.meeting_error_notified = false;
                        // One a crashed daemon left behind is not this meeting's.
                        meeting::silent_track::write_status(&Config::runtime_dir(), &[]);

                        // Play feedback
                        self.play_feedback(SoundEvent::RecordingStart);

                        // After the start cue, with capture up: the
                        // announcement goes into the call once there is one.
                        self.meeting_capture_started = Some(capture_started);
                        if let Some((clip, volume)) = announce_clip {
                            self.start_announcement(clip, volume);
                        }

                        // Degradations replace the usual "started" notice
                        // and are shown whatever on_recording_start or a
                        // visible meeting view says: the user has to act on
                        // them.
                        if !notices.is_empty() {
                            send_notification_with_lifetime(
                                "Meeting started with limitations",
                                &notices.join("\n\n"),
                                false,
                                self.config.engine,
                                "normal",
                                Lifetime::Millis(15000),
                            )
                            .await;
                        } else {
                            let title = meeting::status_notice::display_title(
                                self.meeting_hud.title.as_deref(),
                                self.meeting_hud.started_at,
                            );
                            self.meeting_status_notice(
                                meeting::status_notice::MeetingNotice::Started { title },
                                self.config.output.notification.on_recording_start,
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to start meeting: {}", e);
                        let _ = capture.stop().await;
                        return Err(e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to create meeting daemon: {}", e);
                let _ = capture.stop().await;
                return Err(e);
            }
        }

        Ok(())
    }

    /// Publish the meeting's per-track levels on meeting.sock about ten
    /// times a second, while a client is connected. The publisher only reads
    /// the meters; the capture feeds them and never waits on it.
    fn start_meeting_levels(&mut self, meters: meeting::levels::MeetingMeters, meeting_id: &str) {
        self.stop_meeting_levels();
        let Some(hub) = self.meeting_event_hub.clone() else {
            return;
        };
        let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let clients = hub.clone();
        let task = tokio::spawn(meeting::levels::run_publisher(
            meters,
            Some(meeting_id.to_string()),
            active.clone(),
            move || clients.has_subscribers(),
            move |levels| hub.publish_levels(levels),
        ));
        self.meeting_levels = Some((task, active));
    }

    /// Start a sound check: the meeting's capture, opened as a meeting
    /// would, its levels on meeting.sock, nothing recorded. Asking again
    /// while one runs gives it `seconds` more.
    async fn start_soundcheck(&mut self, seconds: u64) {
        use meeting::soundcheck::SoundcheckEvent;
        if self.meeting_daemon.is_some() {
            self.publish_soundcheck(SoundcheckEvent::stopped(
                "meeting",
                Some("A meeting is recording; its levels are already on meeting.sock.".into()),
            ));
            return;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
        if let Some(sc) = self.meeting_soundcheck.as_mut() {
            sc.deadline = deadline;
            sc.running.seconds = Some(seconds);
            let event = sc.running.clone();
            self.publish_soundcheck(event);
            return;
        }
        match meeting::soundcheck::open_capture(&self.config).await {
            Ok(opened) => {
                let running = SoundcheckEvent::running(&opened, seconds);
                tracing::info!(
                    "Sound check for {} s: mic {}, echo path {}, loopback {}",
                    seconds,
                    opened.mic,
                    running.echo_cancel.as_deref().unwrap_or("?"),
                    opened.capture.has_loopback()
                );
                let levels = self.meeting_event_hub.clone().map(|hub| {
                    let clients = hub.clone();
                    tokio::spawn(meeting::levels::run_publisher(
                        opened.capture.meters(),
                        None,
                        Arc::new(std::sync::atomic::AtomicBool::new(true)),
                        move || clients.has_subscribers(),
                        move |l| hub.publish_levels(l),
                    ))
                });
                self.meeting_soundcheck = Some(Soundcheck {
                    opened,
                    levels,
                    deadline,
                    running: running.clone(),
                });
                self.publish_soundcheck(running);
            }
            Err(e) => {
                tracing::warn!("Sound check could not open the meeting audio: {}", e);
                self.publish_soundcheck(SoundcheckEvent::stopped("error", Some(e.to_string())));
            }
        }
    }

    /// End a running sound check, saying why.
    async fn stop_soundcheck(&mut self, reason: &str) {
        let Some(mut sc) = self.meeting_soundcheck.take() else {
            return;
        };
        if let Some(task) = sc.levels.take() {
            task.abort();
        }
        let _ = sc.opened.capture.stop().await;
        drop(sc);
        tracing::info!("Sound check ended ({})", reason);
        self.publish_soundcheck(meeting::soundcheck::SoundcheckEvent::stopped(reason, None));
    }

    /// Keep a sound check's capture from piling up audio nobody records,
    /// and end it when its time is up.
    async fn tick_soundcheck(&mut self) {
        let Some(sc) = self.meeting_soundcheck.as_mut() else {
            return;
        };
        // Nothing records this audio; the liveness guard still watches it,
        // so the check moves off a silent canceller as a meeting would.
        let (_, moved) = sc.opened.samples(&self.config).await;
        if moved {
            let left = sc
                .deadline
                .saturating_duration_since(std::time::Instant::now());
            sc.running =
                meeting::soundcheck::SoundcheckEvent::running(&sc.opened, left.as_secs().max(1));
            let event = sc.running.clone();
            self.publish_soundcheck(event);
        }
        let Some(sc) = self.meeting_soundcheck.as_ref() else {
            return;
        };
        if std::time::Instant::now() >= sc.deadline {
            self.stop_soundcheck("done").await;
        }
    }

    fn publish_soundcheck(&self, event: meeting::soundcheck::SoundcheckEvent) {
        if let Some(hub) = &self.meeting_event_hub {
            hub.publish_soundcheck(event);
        }
    }

    fn stop_meeting_levels(&mut self) {
        if let Some((task, _)) = self.meeting_levels.take() {
            task.abort();
        }
    }

    /// Hold level events back while the meeting is paused.
    fn set_meeting_levels_active(&self, on: bool) {
        if let Some((_, active)) = &self.meeting_levels {
            active.store(on, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Run the recording announcement beside the meeting that just started.
    fn start_announcement(&mut self, clip: meeting::announce::Clip, volume: f32) {
        let runtime_dir = Config::runtime_dir();
        let _ = std::fs::remove_file(runtime_dir.join(meeting::announce::STATUS_FILE));
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let mut runner =
            meeting::announce::Runner::new(clip, self.config.meeting.detect.clone(), runtime_dir);
        runner.volume = volume;
        self.meeting_announce_rx = Some(rx);
        self.meeting_announce_task = Some(tokio::spawn(runner.run(tx)));
    }

    /// Stop an announcement still waiting for the call or playing.
    fn end_announcement(&mut self) {
        if let Some(task) = self.meeting_announce_task.take() {
            // Dropping its players kills them, links and all.
            task.abort();
        }
        self.meeting_announce_rx = None;
        self.meeting_capture_started = None;
        let _ = std::fs::remove_file(Config::runtime_dir().join(meeting::announce::STATUS_FILE));
    }

    /// The announcement's progress: keep it out of the transcript once it
    /// plays, and tell `meeting status` and the views where it got to.
    fn on_announcement_update(&mut self, update: meeting::announce::Update) {
        use meeting::announce::{Update, Window};
        let on_timeline = |at: std::time::Instant, started: Option<std::time::Instant>| {
            started
                .map(|s| at.saturating_duration_since(s).as_millis() as u64)
                .unwrap_or(0)
        };
        let status = match update {
            Update::Waiting => meeting::announce::Status::waiting(),
            Update::Started {
                at,
                expected,
                phrase,
            } => {
                let start_ms = on_timeline(at, self.meeting_capture_started);
                if let Some(ref mut daemon) = self.meeting_daemon {
                    daemon.set_announcement(Window {
                        start_ms,
                        end_ms: start_ms + expected.as_millis() as u64,
                        phrase,
                    });
                }
                return;
            }
            Update::Finished { status, ended_at } => {
                let end_ms = on_timeline(ended_at, self.meeting_capture_started);
                if let Some(ref mut daemon) = self.meeting_daemon {
                    if let Some(mut window) = daemon.announcement().cloned() {
                        window.end_ms = window.end_ms.max(end_ms);
                        daemon.set_announcement(window);
                    }
                }
                self.meeting_announce_rx = None;
                self.meeting_announce_task = None;
                status
            }
        };
        let path = Config::runtime_dir().join(meeting::announce::STATUS_FILE);
        if let Err(e) = std::fs::write(&path, status.to_json().to_string()) {
            tracing::warn!("Cannot write {}: {}", path.display(), e);
        }
        self.meeting_hud.announcement = Some(status.clone());
        if let Some(ref hub) = self.meeting_event_hub {
            hub.update_snapshot(&self.meeting_hud);
            hub.publish_announcement(self.meeting_hud.meeting_id.as_deref(), &status);
        }
    }

    /// Stop the current meeting
    async fn stop_meeting(&mut self) -> Result<()> {
        self.stop_meeting_levels();
        if self.meeting_daemon.is_some() {
            self.end_announcement();
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
            if let Some(mut canceller) = self.meeting_echo_canceller.take() {
                canceller.stop();
            }
            self.meeting_aec_liveness = None;
            self.meeting_gtcrn = false;

            // The tail flush below is where real wall-clock time is spent
            // (transcribing the last partial chunk), yet the external state
            // previously jumped straight from "recording" to "idle" with no
            // wind-down in between. Surface "finalizing" first so a HUD (or
            // any other consumer) doesn't read the daemon as hung, or
            // disappear before the transcript is actually complete.
            let meeting_id = self
                .meeting_daemon
                .as_ref()
                .and_then(|d| d.current_meeting_id())
                .map(|id| id.to_string());
            self.update_meeting_state("finalizing", meeting_id.as_deref());

            // Flush the final partial chunk so speech near stop is not dropped.
            self.process_buffered_meeting_audio(true).await;

            // The event channel is dropped below, so publish the tail chunk's
            // segments now or meeting.sock never sees them.
            let pending: Vec<MeetingEvent> = match self.meeting_event_rx.as_mut() {
                Some(rx) => std::iter::from_fn(|| rx.try_recv().ok()).collect(),
                None => Vec::new(),
            };
            for event in pending {
                match event {
                    MeetingEvent::ChunkProcessed { chunk_id, segments } => {
                        self.publish_meeting_chunk(chunk_id, &segments);
                    }
                    MeetingEvent::SpeakerRenamed { from, to } => {
                        self.publish_meeting_rename(&from, &to);
                    }
                    MeetingEvent::SegmentUpdated { segment } => {
                        self.publish_segment_updated(&segment);
                    }
                    MeetingEvent::SegmentDeleted { segment_id } => {
                        self.publish_segment_deleted(segment_id);
                    }
                    MeetingEvent::SummaryUpdated { meeting_id } => {
                        if let Some(ref hub) = self.meeting_event_hub {
                            hub.publish_summary_updated(&meeting_id.to_string());
                        }
                    }
                    _ => {}
                }
            }

            // Read before "idle" resets the HUD snapshot they come from.
            let title = meeting::status_notice::display_title(
                self.meeting_hud.title.as_deref(),
                self.meeting_hud.started_at,
            );
            let elapsed = self
                .meeting_hud
                .started_at
                .and_then(|t| (chrono::Utc::now() - t).to_std().ok())
                .unwrap_or_default();

            let mut daemon = self.meeting_daemon.take().expect("checked above");
            match daemon.stop().await {
                Ok(meeting_id) => {
                    self.update_meeting_state("idle", None);
                    tracing::info!("Meeting stopped: {}", meeting_id);

                    // Summarize, then rewrite meeting.json and run
                    // post_meeting_command, in the background: the meeting
                    // is already saved, and a slow or failing agent or
                    // command must not hold up the daemon or the next
                    // meeting. The command runs after the summary so it
                    // sees summary.md, and the manifest lists it.
                    let summary = daemon.auto_summary(meeting_id);
                    let post = daemon.post_meeting(meeting_id);
                    let hub = self.meeting_event_hub.clone();
                    tokio::spawn(async move {
                        if let Some(job) = summary {
                            job.run(move |event| {
                                if let Some(ref hub) = hub {
                                    hub.publish_summary_event(event);
                                }
                            })
                            .await;
                        }
                        post.run().await;
                    });

                    self.play_feedback(SoundEvent::RecordingStop);

                    self.meeting_status_notice(
                        meeting::status_notice::MeetingNotice::Ended { title, elapsed },
                        self.config.output.notification.on_recording_stop,
                    )
                    .await;
                }
                Err(e) => {
                    tracing::error!("Error stopping meeting: {}", e);
                    send_notification(
                        "Meeting Error",
                        &format!("Could not stop the meeting cleanly: {}", e),
                        false,
                        self.config.engine,
                        "critical",
                    )
                    .await;
                }
            }

            self.meeting_mic_buffer.clear();
            self.meeting_loopback_buffer.clear();
            self.meeting_event_rx = None;
        }

        Ok(())
    }

    /// Post a meeting status notification, unless `[meeting]
    /// status_notifications` says not to or (in auto mode) a meeting view is
    /// already showing the same thing. Errors never come through here.
    async fn meeting_status_notice(
        &self,
        notice: meeting::status_notice::MeetingNotice,
        enabled: bool,
    ) {
        use meeting::status_notice::{should_notify, StatusMode};
        let mode = StatusMode::parse(&self.config.meeting.status_notifications);
        if !should_notify(mode, enabled, meeting::view_presence::view_visible) {
            tracing::debug!(?notice, "Meeting status notification skipped");
            return;
        }
        let (summary, body) = notice.text();
        send_notification(
            summary,
            &body,
            false,
            self.config.engine,
            &self.config.output.notification.urgency,
        )
        .await;
    }

    /// Pause the current meeting
    async fn pause_meeting(&mut self) -> Result<()> {
        if let Some(ref mut daemon) = self.meeting_daemon {
            daemon.pause().await?;
            let meeting_id = daemon.current_meeting_id().map(|id| id.to_string());
            self.set_meeting_levels_active(false);
            self.update_meeting_state("paused", meeting_id.as_deref());
            tracing::info!("Meeting paused");

            let title = meeting::status_notice::display_title(
                self.meeting_hud.title.as_deref(),
                self.meeting_hud.started_at,
            );
            self.meeting_status_notice(
                meeting::status_notice::MeetingNotice::Paused { title },
                self.config.output.notification.on_recording_stop,
            )
            .await;
        }
        Ok(())
    }

    /// Resume the current meeting
    async fn resume_meeting(&mut self) -> Result<()> {
        if let Some(ref mut daemon) = self.meeting_daemon {
            daemon.resume().await?;
            let meeting_id = daemon.current_meeting_id().map(|id| id.to_string());
            self.set_meeting_levels_active(true);
            self.update_meeting_state("recording", meeting_id.as_deref());
            tracing::info!("Meeting resumed");

            let title = meeting::status_notice::display_title(
                self.meeting_hud.title.as_deref(),
                self.meeting_hud.started_at,
            );
            self.meeting_status_notice(
                meeting::status_notice::MeetingNotice::Resumed { title },
                self.config.output.notification.on_recording_start,
            )
            .await;
        }
        Ok(())
    }

    /// Act on pending meeting file triggers. Returns whether the meeting
    /// changed state, so the audio branch can skip this tick's samples.
    ///
    /// Every trigger is consumed whether or not it applies, so a request the
    /// daemon refused cannot fire later against a different meeting.
    /// Tell meeting.sock which meetings never finished, for the recovery
    /// card: at startup, and again when `meeting recover` finished or
    /// discarded one.
    fn announce_recoverable_meetings(&self) {
        let Some(ref hub) = self.meeting_event_hub else {
            return;
        };
        let running = self
            .meeting_daemon
            .as_ref()
            .and_then(|d| d.current_meeting_id());
        match meeting::MeetingStorage::open(meeting_storage_config(&self.config)).and_then(
            |storage| {
                meeting::recover::recoverable(
                    &storage,
                    running.as_ref(),
                    self.config.meeting.chunk_duration_secs,
                )
            },
        ) {
            Ok(meetings) => {
                if !meetings.is_empty() {
                    tracing::info!("{} meeting(s) can be recovered", meetings.len());
                }
                hub.publish_recoverable(&meetings);
            }
            Err(e) => tracing::warn!("Could not list meetings to recover: {}", e),
        }
    }

    /// At startup, a desktop notification for each meeting that never
    /// finished and was not announced before, under the same
    /// `status_notifications` rule as the other meeting notices. Its Finish
    /// or Open button runs `voxtype meeting recover --finish` or
    /// `voxtype meeting view`.
    fn notify_interrupted_meetings(&self) {
        use meeting::recover::notify;
        use meeting::status_notice::{should_notify, StatusMode};
        let current = match meeting::MeetingStorage::open(meeting_storage_config(&self.config))
            .and_then(|storage| {
                meeting::recover::recoverable(
                    &storage,
                    None,
                    self.config.meeting.chunk_duration_secs,
                )
            }) {
            Ok(current) => current,
            Err(e) => {
                tracing::warn!("Could not list meetings to recover: {}", e);
                return;
            }
        };
        // Recorded as announced whether or not it shows, so a later
        // restart never brings the same meeting up again.
        let new = notify::unannounced(&notify::state_path(), &current);
        if new.is_empty() {
            return;
        }
        let mode = StatusMode::parse(&self.config.meeting.status_notifications);
        if !should_notify(mode, true, meeting::view_presence::view_visible) {
            tracing::debug!("Interrupted meeting notification skipped");
            return;
        }
        // Only Linux has the freedesktop notification action protocol. Other
        // platforms use the existing native notifier and name the recovery CLI.
        #[cfg(not(target_os = "linux"))]
        tokio::spawn(async move {
            let (summary, mut body, _) = notify::wording(&new);
            body.push_str("\nRun: voxtype meeting recover");
            crate::notification::send(&summary, &body).await;
        });
        #[cfg(target_os = "linux")]
        tokio::spawn(async move {
            let choice = match zbus::Connection::session().await {
                Ok(conn) => notify::announce(&conn, &new).await,
                Err(e) => Err(e.into()),
            };
            let choice = match choice {
                Ok(Some(choice)) => choice,
                Ok(None) => return,
                Err(e) => {
                    tracing::debug!("Interrupted meeting notification unavailable: {}", e);
                    return;
                }
            };
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("voxtype"));
            match std::process::Command::new(exe)
                .args(choice.args())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    let _ = tokio::task::spawn_blocking(move || child.wait()).await;
                }
                Err(e) => tracing::warn!("Could not run {:?}: {}", choice.args(), e),
            }
        });
    }

    async fn handle_meeting_triggers(&mut self) -> bool {
        let mut changed = false;

        let recover_trigger = Config::runtime_dir().join(meeting::recover::TRIGGER);
        if recover_trigger.exists() {
            let _ = std::fs::remove_file(&recover_trigger);
            self.announce_recoverable_meetings();
        }
        if let Some(request) = take_soundcheck_trigger() {
            match request {
                meeting::soundcheck::Request::Start { seconds } => {
                    self.start_soundcheck(seconds).await
                }
                meeting::soundcheck::Request::Stop => self.stop_soundcheck("stopped").await,
            }
        }
        self.tick_soundcheck().await;

        // `meeting notice --ack` ran: tell every view now, not at the next
        // state change.
        let ack = Config::runtime_dir().join(meeting::notice::ACK_TRIGGER);
        if ack.exists() {
            let _ = std::fs::remove_file(&ack);
            self.meeting_hud.recording_notice = meeting::notice::pending();
            if let Some(ref hub) = self.meeting_event_hub {
                hub.update_snapshot(&self.meeting_hud);
                hub.publish_status(&self.meeting_hud.status);
            }
        }

        if let Some(trigger) = check_meeting_start() {
            if !self.config.meeting.enabled {
                tracing::warn!("Meeting mode is disabled in config");
            } else if self.meeting_daemon.is_some() {
                tracing::warn!("Meeting already in progress; ignoring start request");
            } else {
                tracing::debug!("Meeting start requested via file trigger");
                if let Err(e) = self
                    .start_meeting(trigger.title, trigger.diarization, trigger.options)
                    .await
                {
                    tracing::error!("Failed to start meeting: {}", e);
                    send_notification(
                        "Meeting Error",
                        &format!("The meeting could not start: {}", e),
                        false,
                        self.config.engine,
                        "critical",
                    )
                    .await;
                }
                changed = true;
            }
            ack_meeting_start();
        }

        if check_meeting_stop() && self.meeting_daemon.is_some() {
            tracing::debug!("Meeting stop requested via file trigger");
            if let Err(e) = self.stop_meeting().await {
                tracing::error!("Failed to stop meeting: {}", e);
            }
            changed = true;
        }

        if check_meeting_pause() && self.meeting_active() {
            tracing::debug!("Meeting pause requested via file trigger");
            if let Err(e) = self.pause_meeting().await {
                tracing::error!("Failed to pause meeting: {}", e);
            }
            changed = true;
        }

        if check_meeting_edit() {
            tracing::debug!("Meeting transcript edit requested via file trigger");
            self.apply_meeting_edits().await;
        }

        if let Some(retitle) = meeting::retitle::take_trigger(&Config::runtime_dir()) {
            self.apply_retitle(retitle).await;
        }

        // A summary changed in storage (`meeting label` moved its action
        // items' owners): tell the views showing that meeting.
        if let Some(id) = take_summary_updated() {
            if let Some(ref hub) = self.meeting_event_hub {
                hub.publish_summary_updated(&id);
            }
        }

        if check_meeting_notes() {
            let notes = self.meeting_daemon.as_ref().and_then(|daemon| {
                let id = daemon.current_meeting_id()?;
                daemon.storage().load_notes(&id).ok()
            });
            if let (Some(notes), Some(ref hub)) = (notes, &self.meeting_event_hub) {
                hub.publish_notes_updated(&notes);
            }
        }

        // `meeting import` or `meeting retranscribe` got a chunk further.
        if take_trigger(meeting::import::PROGRESS_TRIGGER) {
            if let (Some(progress), Some(ref hub)) = (
                meeting::import::read_progress(&Config::runtime_dir()),
                &self.meeting_event_hub,
            ) {
                hub.publish_import_progress(&progress);
            }
        }

        // `meeting play` started or stopped, or a playback ran to its end.
        if take_trigger(meeting::playback::TRIGGER) {
            if let Some(ref hub) = self.meeting_event_hub {
                hub.publish_playback(&meeting::playback::status(&Config::runtime_dir()));
            }
        }

        if check_meeting_resume()
            && self
                .meeting_daemon
                .as_ref()
                .is_some_and(|d| d.state().is_paused())
        {
            tracing::debug!("Meeting resume requested via file trigger");
            if let Err(e) = self.resume_meeting().await {
                tracing::error!("Failed to resume meeting: {}", e);
            }
            changed = true;
        }

        changed
    }

    /// Bring the running meeting up to date with what the user changed in
    /// its transcript (`meeting label`, `reassign`, `annotate`, `edit`,
    /// `delete-line`). The changed lines reach the views as meeting events,
    /// once the event loop is back to reading them.
    async fn apply_meeting_edits(&mut self) {
        if let Some(ref mut daemon) = self.meeting_daemon {
            daemon.apply_segment_edits().await;
        }
    }

    /// Tell the user a meeting is losing transcript, once per meeting: a
    /// broken engine fails every chunk, and one notification per chunk would
    /// bury the desktop. The log keeps every occurrence.
    async fn notify_meeting_error(&mut self, error: &str) {
        if std::mem::replace(&mut self.meeting_error_notified, true) {
            return;
        }
        send_notification_with_lifetime(
            "Meeting transcription failed",
            &format!(
                "{}\n\nThe meeting is still recording, but this part is missing from the \
                 transcript. Later errors are only logged: journalctl --user -u voxtype",
                error
            ),
            false,
            self.config.engine,
            "critical",
            Lifetime::Millis(15000),
        )
        .await;
    }

    /// Check if a meeting is in progress
    fn meeting_active(&self) -> bool {
        self.meeting_daemon
            .as_ref()
            .is_some_and(|d| d.state().is_active())
    }

    /// The echo handling a meeting gets for `path`, loading GTCRN when it
    /// is the one: none if its model is missing or this build lacks it.
    /// The second value says why GTCRN is off, the first time it is.
    fn meeting_echo_path_in_use(
        &mut self,
        path: audio::echo_cancel::EchoPath,
    ) -> (audio::echo_cancel::EchoPath, Option<String>) {
        // Why GTCRN was wanted but is off, once per daemon run: the model
        // stays missing until the user fetches it.
        #[cfg_attr(not(feature = "onnx-common"), allow(unused_mut))]
        let mut notice: Option<String> = None;

        // Load GTCRN speech enhancer for echo cancellation
        #[cfg(feature = "onnx-common")]
        if self.speech_enhancer.is_none() && path == audio::echo_cancel::EchoPath::Gtcrn {
            let model_path = Config::models_dir().join(audio::echo_cancel::GTCRN_MODEL_FILE);
            if model_path.exists() {
                match audio::enhance::GtcrnEnhancer::load(&model_path) {
                    Ok(enhancer) => {
                        self.speech_enhancer = Some(std::sync::Arc::new(enhancer));
                        tracing::info!(
                            "GTCRN speech enhancer loaded for meeting echo cancellation"
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load GTCRN enhancer, continuing without: {}", e);
                        notice = Some(format!(
                            "Echo cancellation is off: the GTCRN model failed to load ({}). \
                             Delete it and download it again with: {}",
                            e,
                            crate::setup::model::gtcrn_download_command()
                        ));
                    }
                }
            } else {
                tracing::warn!(
                    "GTCRN model not found at {:?}, skipping speech enhancement",
                    model_path
                );
                notice = Some(format!(
                    "Echo cancellation is off: the GTCRN model is missing. Download it \
                     with: {} (or set [meeting.audio] echo_cancel = \"disabled\")",
                    crate::setup::model::gtcrn_download_command()
                ));
            }
        }
        let notice = notice.filter(|n| self.meeting_notices_shown.insert(n.clone()));

        // What the meeting really uses: GTCRN only if its
        // model loaded (and this build has it).
        #[cfg(feature = "onnx-common")]
        let gtcrn_loaded = self.speech_enhancer.is_some();
        #[cfg(not(feature = "onnx-common"))]
        let gtcrn_loaded = false;
        (path.effective(gtcrn_loaded), notice)
    }

    /// A track of the meeting has sound but no lines, or got its lines
    /// back. `meeting status` always follows it. The first time a track
    /// goes missing in a meeting, the meeting views get it as an error and
    /// the user a notification, once per track however long it lasts.
    async fn on_track_changes(&mut self, changes: Vec<meeting::silent_track::Change>) {
        use meeting::silent_track::{self, Change};
        let Some(daemon) = self.meeting_daemon.as_ref() else {
            return;
        };
        silent_track::write_status(&Config::runtime_dir(), &daemon.silent_tracks());
        let audio_kept = daemon.keeps_audio();
        let meeting_id = daemon.current_meeting_id().map(|id| id.to_string());
        // Both sides raised by the same pair go out together: voxtype's
        // notifications share a slot, and the views' banner holds one error.
        let announced: Vec<silent_track::SilentTrack> = changes
            .into_iter()
            .filter_map(|change| match change {
                Change::Raised {
                    track,
                    announce: true,
                } => Some(track),
                _ => None,
            })
            .collect();
        let Some((title, body)) =
            silent_track::notification(&announced, audio_kept, meeting_id.as_deref())
        else {
            return;
        };
        if let Some(ref hub) = self.meeting_event_hub {
            hub.publish_track_notice(&announced);
        }
        // Whatever `[meeting] status_notifications` says and whether or not
        // a meeting view is on screen, like the other meeting errors: this
        // is transcript being lost, and a view minimized to its pill shows
        // no banner.
        send_notification_with_lifetime(
            title,
            &body,
            false,
            self.config.engine,
            "normal",
            Lifetime::Millis(15000),
        )
        .await;
    }

    /// The echo canceller has delivered only digital silence since the
    /// meeting started: it is not hearing the mic, and every line of yours
    /// would be lost. Record the mic directly instead, with GTCRN, as a
    /// meeting does when the canceller cannot start.
    async fn leave_silent_echo_canceller(&mut self) {
        let rms = self
            .meeting_aec_liveness
            .take()
            .map(|check| check.rms())
            .unwrap_or_default();
        let Some(capture) = self.meeting_audio_capture.as_mut() else {
            return;
        };
        let Some(rest) = meeting::soundcheck::leave_silent_canceller(
            &self.config,
            capture,
            &mut self.meeting_echo_canceller,
            rms,
        )
        .await
        else {
            return;
        };
        self.meeting_mic_buffer.extend(rest.mic);
        self.meeting_loopback_buffer.extend(rest.loopback);
        let (used, echo_notice) =
            self.meeting_echo_path_in_use(audio::echo_cancel::EchoPath::Gtcrn);
        if let Some(notice) = echo_notice {
            send_notification_with_lifetime(
                "Meeting continues with limitations",
                &notice,
                false,
                self.config.engine,
                "normal",
                Lifetime::Millis(15000),
            )
            .await;
        }
        self.meeting_gtcrn = used == audio::echo_cancel::EchoPath::Gtcrn;
        tracing::info!("Meeting echo handling: {}", used.as_str());
        if let Some(ref mut daemon) = self.meeting_daemon {
            daemon.record_echo_cancel(used.as_str());
        }
        self.meeting_hud.echo_cancel = Some(used.as_str().to_string());
        if let Some(ref hub) = self.meeting_event_hub {
            hub.update_snapshot(&self.meeting_hud);
        }
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
                if let Some(enhancer) = self.speech_enhancer.as_ref().filter(|_| self.meeting_gtcrn)
                {
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

        let (chunk_error, track_changes) = match self.meeting_daemon {
            Some(ref mut daemon) => (
                daemon.process_pair(mic_chunk, loopback_chunk).await,
                daemon.take_track_changes(),
            ),
            None => (None, Vec::new()),
        };
        if let Some(error) = chunk_error {
            self.notify_meeting_error(&error).await;
        }
        if !track_changes.is_empty() {
            self.on_track_changes(track_changes).await;
        }
    }

    async fn process_buffered_meeting_audio(&mut self, include_tail: bool) {
        let chunk_samples = self.meeting_chunk_samples();

        while self.meeting_mic_buffer.len() >= chunk_samples {
            let cut = meeting::chunk::chunk_cut_point(
                &self.meeting_mic_buffer,
                &self.meeting_loopback_buffer,
                chunk_samples,
                16000,
                self.config.meeting.audio.vad_threshold,
            );
            let mic_chunk: Vec<f32> = self.meeting_mic_buffer.drain(..cut).collect();
            let loopback_len = self.meeting_loopback_buffer.len().min(cut);
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
        if let Some(path) = peek_file_output_path(&self.config) {
            write_result_sidecar(&path, &TranscriptOutcome::empty());
        }
    }

    /// Reset state to idle and run post_output_command to reset compositor submap
    /// Call this when exiting from recording/transcribing without normal output flow
    async fn reset_to_idle(&mut self, state: &mut State) {
        self.disarm_dead_input_guard();

        self.discard_recording_overrides();
        self.restore_recording_media();
        self.streaming_recovery_file_output_path = None;

        *state = State::Idle;
        self.update_state("idle");

        // Run post_output_command to reset compositor submap
        if let Some(cmd) = &self.config.output.post_output_command {
            if let Err(e) = output::run_hook(cmd, "post_output").await {
                tracing::warn!("{}", e);
            }
        }
        output::deferred_notice::flush().await;
    }

    /// Spawn a transcription task for a single chunk (eager processing)
    fn spawn_chunk_transcription(
        &mut self,
        chunk_index: usize,
        chunk_audio: Vec<f32>,
        transcriber: Arc<dyn Transcriber>,
    ) {
        self.active_transcriber = Some(transcriber.clone());
        self.history_note(|pending| pending.transcribing(0));
        tracing::debug!(
            "Spawning eager transcription for chunk {} ({:.1}s)",
            chunk_index,
            chunk_audio.len() as f32 / 16000.0
        );

        // A chunk with no speech is skipped rather than transcribed: engines
        // like Whisper invent text for silence, and a silent chunk mid-way
        // through a recording would otherwise land in the output.
        let vad = self.vad.clone();
        let decoded = self.eager_decoded.clone();
        let task = tokio::task::spawn_blocking(move || {
            if let Some(vad) = vad {
                match vad.detect(&chunk_audio) {
                    Ok(result) if !result.has_speech => {
                        tracing::debug!("Chunk {} has no speech, skipping", chunk_index);
                        return Ok(String::new());
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("VAD failed on chunk {}: {}", chunk_index, e),
                }
            }
            let result = transcriber.transcribe(&chunk_audio);
            decoded.store(true, std::sync::atomic::Ordering::Release);
            result
        });

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
        let boundaries = eager::chunk_boundaries(accumulated_audio, &eager_config);

        let mut spawned = 0;
        while let Some((start, end)) = eager::chunk_range(&boundaries, *chunks_sent) {
            let chunk_audio = accumulated_audio[start..end].to_vec();
            self.spawn_chunk_transcription(*chunks_sent, chunk_audio, transcriber.clone());
            *chunks_sent += 1;
            *tasks_in_flight += 1;
            spawned += 1;
        }

        spawned
    }

    /// Poll for completed chunk transcription tasks and collect results
    /// Returns any completed results
    async fn poll_chunk_tasks(&mut self) -> Vec<ChunkResult> {
        let mut completed = Vec::new();
        let mut remaining_tasks = Vec::new();
        let mut failure = None;

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
                        failure = Some(e.to_string());
                        completed.push(ChunkResult {
                            text: String::new(),
                            chunk_index,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Chunk {} task panicked: {}", chunk_index, e);
                        failure = Some(e.to_string());
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
        if let Some(error) = failure {
            self.history_note(|pending| {
                pending.outcome(crate::history::Outcome::TranscriptionFailed, Some(error))
            });
        }
        completed
    }

    /// Wait for all remaining chunk tasks to complete
    async fn wait_for_chunk_tasks(&mut self) -> Vec<ChunkResult> {
        let mut results = Vec::new();
        let mut failure = None;

        for (chunk_index, task) in self.eager_chunk_tasks.drain(..) {
            match task.await {
                Ok(Ok(text)) => {
                    tracing::debug!("Chunk {} completed (waited): {:?}", chunk_index, text);
                    results.push(ChunkResult { text, chunk_index });
                }
                Ok(Err(e)) => {
                    tracing::warn!("Chunk {} transcription failed: {}", chunk_index, e);
                    failure = Some(e.to_string());
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
                    failure = Some(e.to_string());
                    results.push(ChunkResult {
                        text: String::new(),
                        chunk_index,
                    });
                }
            }
        }

        if let Some(error) = failure {
            self.history_note(|pending| {
                pending.outcome(crate::history::Outcome::TranscriptionFailed, Some(error))
            });
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
        self.active_transcriber = Some(transcriber.clone());
        self.history_note(|p| p.transcribing(accumulated_audio.len()));
        tracing::info!(
            "Finishing eager recording: {:.1}s of audio, {} chunks already transcribed",
            audio_duration,
            chunk_results.len()
        );

        // Whole-recording VAD, the same gate the batch path applies. It runs
        // alongside the in-flight chunks and the tail so it adds no wait.
        let vad_task = self.vad.clone().map(|vad| {
            let audio = accumulated_audio.clone();
            tokio::task::spawn_blocking(move || vad.detect(&audio))
        });

        // Wait for any in-flight chunk tasks
        let mut waited_results = self.wait_for_chunk_tasks().await;
        chunk_results.append(&mut waited_results);

        // Transcribe the tail (audio after last complete chunk). Chunk
        // boundaries depend only on the audio before them, so recomputing
        // them over the final audio reproduces the bounds of every chunk sent.
        let eager_config = EagerConfig::from_whisper_config(&self.config.whisper);
        let boundaries = eager::chunk_boundaries(&accumulated_audio, &eager_config);
        let chunks_sent = chunk_results
            .iter()
            .map(|r| r.chunk_index)
            .max()
            .map(|i| i + 1)
            .unwrap_or(0);
        let tail_start = eager::tail_start(&boundaries, chunks_sent);

        let tail_task = (accumulated_audio.len().saturating_sub(tail_start) >= 4800).then(|| {
            let tail_audio = accumulated_audio[tail_start..].to_vec();
            tracing::debug!(
                "Transcribing tail audio: {:.1}s (from sample {})",
                tail_audio.len() as f32 / 16000.0,
                tail_start
            );
            let tail_transcriber = transcriber.clone();
            let decoded = self.eager_decoded.clone();
            tokio::task::spawn_blocking(move || {
                let result = tail_transcriber.transcribe(&tail_audio);
                decoded.store(true, std::sync::atomic::Ordering::Release);
                result
            })
        });

        if let Some(task) = vad_task {
            match task.await {
                Ok(Ok(result)) if !result.has_speech => {
                    if let Some(task) = tail_task {
                        task.abort();
                    }
                    self.report_no_speech(&result).await;
                    self.history_eager_transcriber_metadata(transcriber.as_ref());
                    self.history_note(|pending| pending.transcribed());
                    self.publish_empty_outcome();
                    return None;
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!("VAD failed, proceeding anyway: {}", e),
                Err(e) => tracing::warn!("VAD task failed, proceeding anyway: {}", e),
            }
        }

        if let Some(task) = tail_task {
            match task.await {
                Ok(Ok(text)) => {
                    tracing::debug!("Tail transcription: {:?}", text);
                    chunk_results.push(ChunkResult {
                        text,
                        chunk_index: chunks_sent,
                    });
                }
                Ok(Err(e)) => {
                    tracing::warn!("Tail transcription failed: {}", e);
                    self.history_note(|pending| {
                        pending.outcome(
                            crate::history::Outcome::TranscriptionFailed,
                            Some(e.to_string()),
                        )
                    });
                }
                Err(e) => {
                    tracing::warn!("Tail transcription task panicked: {}", e);
                    self.history_note(|pending| {
                        pending.outcome(
                            crate::history::Outcome::TranscriptionFailed,
                            Some(e.to_string()),
                        )
                    });
                }
            }
        }

        // Combine all chunk results
        self.history_eager_transcriber_metadata(transcriber.as_ref());
        self.history_note(|pending| pending.transcribed());
        let chunk_count = chunk_results.len();
        let combined = eager::combine_chunk_results(chunk_results);
        // The words themselves only at debug: see the "Transcribed" line.
        tracing::info!(
            "Combined eager transcription: {} chars from {} chunk(s)",
            combined.chars().count(),
            chunk_count
        );
        tracing::debug!("Combined eager transcription: {:?}", combined);

        if combined.is_empty() {
            None
        } else {
            Some(combined)
        }
    }

    /// Whether to keep the microphone open while a push-to-talk dictation
    /// transcribes, ready for the next press.
    fn pipelines_dictation(&self) -> bool {
        self.config.hotkey.enabled && self.config.hotkey.mode == ActivationMode::PushToTalk
    }

    async fn stop_standby_capture(&mut self) {
        if let Some(mut capture) = self.standby_capture.take() {
            tracing::debug!("Closing the standby capture");
            let _ = capture.stop().await;
            self.stop_level_emitter();
        }
    }

    /// Output a dictation that was held back while the hotkey was down.
    async fn flush_deferred_output(&mut self) {
        let Some(deferred) = self.deferred_output.take() else {
            return;
        };
        tracing::info!("Hotkey released; outputting the held-back dictation");
        // Its history entry stands in for the recording's in progress while
        // it is delivered.
        let current = self.history_swap(deferred.history);
        self.deliver_dictation(&deferred.config, &deferred.text, &deferred.original, false)
            .await;
        if let Some(finished) = self.history_swap(current) {
            self.history_commit_pending(finished);
        }
    }

    /// Type or paste a finished dictation and record how it went. `osd` is
    /// false for a dictation held back while the next one records, whose
    /// outcome would land on the new recording's OSD.
    async fn deliver_dictation(
        &mut self,
        output_config: &crate::config::OutputConfig,
        final_text: &str,
        original: &str,
        osd: bool,
    ) {
        #[cfg(test)]
        let output_chain = match self.history_test_output_chain.take() {
            Some(chain) => chain,
            None => output::create_output_chain(output_config),
        };
        #[cfg(not(test))]
        let output_chain = output::create_output_chain(output_config);

        let history_context = self.history_context();
        let output_options = output::OutputOptions {
            history_context: history_context.as_ref(),
            pre_output_command: output_config.pre_output_command.as_deref(),
            post_output_command: output_config.post_output_command.as_deref(),
            wait_for_modifier_release: output_config.wait_for_modifier_release,
            modifier_release_timeout: std::time::Duration::from_millis(
                output_config.modifier_release_timeout_ms,
            ),
        };

        let (report, delivery) =
            output::output_with_fallback_report(&output_chain, final_text, output_options).await;
        {
            use crate::history::{Outcome, OutputInfo};
            let mode = format!("{:?}", output_config.mode).to_lowercase();
            let auto_submit = output_config.auto_submit;
            let (driver, fell_back, error) = match &delivery {
                Ok(d) => (Some(d.driver.clone()), d.fell_back, None),
                Err(e) => (None, false, Some(e.to_string())),
            };
            let delivered_text = delivery.as_ref().ok().map(|d| d.text.clone());
            self.history_note(|p| {
                if let Some(text) = &delivered_text {
                    p.delivered_text(text);
                }
                p.entry.output = Some(OutputInfo {
                    mode,
                    driver,
                    fell_back,
                });
                p.entry.auto_submit = auto_submit;
                if let Some(err) = error {
                    p.outcome(Outcome::OutputFailed, Some(err));
                }
            });
        }
        if let Err(e) = delivery {
            tracing::error!("Output failed: {}", e);
            if osd {
                self.publish_osd_outcome(osd::session::OutcomeKind::OutputFailed, 0);
            }
            // Nothing typed or copied: keep the text recoverable,
            // unless [output] keep_undelivered_text is off.
            output::fallback::notify_failed(
                &report,
                &e,
                final_text,
                self.config.output.keep_undelivered_text,
            )
            .await;
        } else {
            if osd {
                self.publish_osd_text_outcome(final_text);
            }
            self.play_feedback(SoundEvent::TranscriptionComplete);
            // Says "copied to clipboard" (and why, once) when
            // typing fell back to the clipboard.
            output::fallback::notify_delivered(
                &report,
                final_text,
                self.config
                    .output
                    .notification
                    .show_edits
                    .then_some(original),
                &self.config.output.notification,
                self.config.engine,
            )
            .await;
        }
        // After the "Transcribed" notification, which would
        // otherwise replace a post-process or hook failure notice.
        output::deferred_notice::flush().await;
    }

    /// Start transcription task (non-blocking, stores JoinHandle for later completion)
    /// Returns true if transcription was started, false if skipped (too short)
    async fn start_transcription_task(
        &mut self,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        model_override: Option<String>,
    ) -> bool {
        let duration = state.recording_duration().unwrap_or_default();
        tracing::info!("Recording stopped ({:.1}s)", duration.as_secs_f32());

        // Tear down the OSD audio-frame emitter for this session.
        self.stop_level_emitter();

        // Stop recording before waiting on model loading or doing any
        // transcription work, then restore media immediately.
        if let Some(mut capture) = audio_capture.take() {
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
                    self.history_note(|p| p.transcribing(samples.len()));

                    // Skip if too short (likely accidental press)
                    if audio_duration < 0.3 {
                        tracing::debug!("Recording too short ({:.2}s), ignoring", audio_duration);
                        self.publish_empty_outcome();
                        self.publish_osd_outcome(osd::session::OutcomeKind::TooShort, 0);
                        self.reset_to_idle(state).await;
                        return false;
                    }

                    // Voice Activity Detection: skip if no speech detected
                    if let Some(ref vad) = self.vad {
                        match vad.detect(&samples) {
                            Ok(result) if !result.has_speech => {
                                self.report_no_speech(&result).await;
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
                    *state = State::Transcribing {
                        audio: samples.clone(),
                    };
                    self.update_state("transcribing");

                    let transcriber = match self
                        .get_transcriber_for_recording(model_override.as_deref())
                        .await
                    {
                        Ok(transcriber) => transcriber,
                        Err(()) => {
                            self.history_note(|p| {
                                p.outcome(
                                    crate::history::Outcome::TranscriptionFailed,
                                    Some("the model could not be loaded".into()),
                                )
                            });
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
                    if self.pipelines_dictation() && self.standby_capture.is_none() {
                        match self.start_recording_capture(false).await {
                            Ok(capture) => self.standby_capture = Some(capture),
                            Err(()) => tracing::debug!("Standby capture didn't start"),
                        }
                    }
                    true
                }
                Err(e) => {
                    tracing::warn!("Recording error: {}", e);
                    self.history_note(|p| {
                        p.outcome(
                            crate::history::Outcome::TranscriptionFailed,
                            Some(format!("recording error: {e}")),
                        )
                    });
                    self.publish_osd_outcome(osd::session::OutcomeKind::Error, 0);
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

    /// Mark an accepted model upgrade as proven by a successful transcription.
    fn record_model_upgrade_success(&mut self) {
        if !self.model_upgrade_pending {
            return;
        }
        self.model_upgrade_pending = false;
        match crate::model_upgrade::record_first_success(
            &crate::model_upgrade::state_path(),
            self.config.model_name(),
        ) {
            Ok(true) => {
                tracing::info!(
                    "First transcription with the upgraded model succeeded; the previous model can be removed with `voxtype setup migrate --cleanup`"
                );
                // The tray reads offers once at startup; push the cleanup
                // offer now so "Remove old model files" appears without a
                // daemon restart.
                self.refresh_tray_upgrade();
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "Could not record the model upgrade success"),
        }
    }

    fn refresh_tray_upgrade(&self) {
        #[cfg(target_os = "linux")]
        if let Some(tray) = &self.tray {
            match serde_json::to_value(crate::model_upgrade::status_for_config(&self.config)) {
                Ok(value) => {
                    tray.update_upgrade(crate::tray::UpgradeMenu::from_status_value(&value))
                }
                Err(error) => {
                    tracing::warn!(%error, "Could not refresh the tray's model upgrade menu")
                }
            }
        }
    }

    /// Handle transcription completion (called when transcription_task completes)
    async fn handle_transcription_result(
        &mut self,
        state: &mut State,
        result: std::result::Result<TranscriptionResult, tokio::task::JoinError>,
    ) {
        let failed = match &result {
            Ok(Err(_)) => true,
            Err(error) => !error.is_cancelled(),
            _ => false,
        };
        self.handle_transcription_result_inner(state, result).await;
        // The dictation is over whichever way it ended.
        self.recording_profile = None;
        if failed {
            self.show_tray_error();
        }
        // The inner handler owns the last reference to an on-demand
        // transcriber, so by here the model has been dropped.
        if self.config.on_demand_loading() {
            release_freed_memory();
        }
    }

    async fn handle_transcription_result_inner(
        &mut self,
        state: &mut State,
        result: std::result::Result<TranscriptionResult, tokio::task::JoinError>,
    ) {
        // Take ownership of the transcriber Arc we cloned at spawn time so it
        // is dropped on every exit path (success, transcription error, or
        // task error). The Ok(Ok(_)) branch consults it for the language
        // layout hint before letting it drop.
        let active_transcriber = self.active_transcriber.take();
        if let Some(engine) = active_transcriber.as_ref() {
            self.history_transcriber_metadata(engine.as_ref());
        }
        self.history_note(|pending| pending.transcribed());
        match result {
            Ok(Ok(text)) => {
                if text.is_empty() {
                    tracing::debug!("Transcription was empty");
                    self.publish_osd_outcome(osd::session::OutcomeKind::Empty, 0);
                    self.reset_to_idle(state).await;
                } else {
                    // The length and the engine at info, the words only at
                    // debug. The service logs to the journal at info, and the
                    // journal keeps what it is given: logging the text there
                    // would retain every dictation whatever the user chose
                    // for history, failed recordings and undelivered text.
                    tracing::info!(
                        "Transcribed {} chars ({} engine)",
                        text.chars().count(),
                        self.recording_engine
                    );
                    tracing::debug!("Transcribed: {:?}", text);
                    self.record_model_upgrade_success();

                    // Resolve the profile before cleanup because vocabulary is
                    // the first profile-aware stage in the cleanup pipeline.
                    let profile_override = self.recording_profile();
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

                    // Apply text processing (replacements, punctuation)
                    let vocabulary = profile_override
                        .as_ref()
                        .and_then(|name| self.profile_vocabularies.get(name));
                    let processed_text = self.text_processor.process_recording(
                        &text,
                        vocabulary,
                        self.session_text_is_english,
                    );
                    if processed_text != text {
                        tracing::debug!("After text processing: {:?}", processed_text);
                    }

                    // Smart auto-submit: detect "submit" trigger word at end
                    // CLI override (--smart-auto-submit / --no-smart-auto-submit) takes priority
                    let smart_auto_submit_cli = read_bool_override("smart_auto_submit");
                    let (processed_text, smart_submit) = self
                        .text_processor
                        .detect_submit(&processed_text, smart_auto_submit_cli);
                    if smart_submit {
                        tracing::debug!(
                            "Smart auto-submit triggered, stripped text: {:?}",
                            processed_text
                        );
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
                                edit_list: self
                                    .config
                                    .output
                                    .post_process
                                    .as_ref()
                                    .is_some_and(|config| config.edit_list),
                                ..Default::default()
                            };
                            let profile_name = profile_override.as_deref().unwrap_or_default();
                            let profile_processor =
                                PostProcessor::new(&profile_config).for_profile(profile_name);
                            tracing::info!(
                                "Post-processing with profile: {:?}, has_context: {}",
                                profile_name,
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

                    // Check for output mode override from CLI flags.
                    let output_override = read_output_mode_override();

                    // Check if profile specifies output mode override
                    let profile_output_mode = active_profile.and_then(|p| p.output_mode.clone());

                    // A streaming recovery keeps the resolved file target from
                    // session start, after consuming its one-shot override.
                    let file_output_path =
                        self.streaming_recovery_file_output_path.take().or_else(|| {
                            self.resolve_file_output_path(
                                &output_override,
                                profile_output_mode.clone(),
                            )
                        });

                    // Consume the per-recording boolean overrides before any
                    // early return below. File output returns without building
                    // an output chain, and these sentinels used to survive it:
                    // the next recording (typically the user's own hotkey, in
                    // type mode) then picked them up. Any client that passes
                    // --no-auto-submit with --file hit this on every dictation.
                    let auto_submit_override = read_bool_override("auto_submit");
                    let shift_enter_override = read_bool_override("shift_enter");

                    {
                        let language = active_transcriber
                            .as_ref()
                            .and_then(|t| t.last_detected_language());
                        let profile = profile_override.clone();
                        self.history_note(|p| {
                            p.texts(&text, &final_text);
                            if language.is_some() {
                                p.entry.language = language;
                            }
                            p.entry.profile = profile;
                        });
                    }

                    if let Some(output_path) = file_output_path {
                        *state = State::Outputting {
                            text: final_text.clone(),
                        };

                        let delivery = self.deliver_file_output(&output_path, &final_text).await;
                        {
                            use crate::history::{Outcome, OutputInfo};
                            let (fell_back, error) = match &delivery {
                                FileDelivery::Written => (false, None),
                                FileDelivery::FellBack(_) => (true, None),
                                FileDelivery::Lost(err) => (false, Some(err.to_string())),
                            };
                            let fallback_text =
                                fell_back.then(|| output::delivered_form(&final_text).into_owned());
                            self.history_note(|p| {
                                // The file gets the text as is; the clipboard
                                // fallback gets it through the output chain.
                                if let Some(text) = &fallback_text {
                                    p.delivered_text(text);
                                }
                                p.entry.output = Some(OutputInfo {
                                    mode: "file".into(),
                                    driver: Some(
                                        if fell_back { "clipboard" } else { "file" }.into(),
                                    ),
                                    fell_back,
                                });
                                if let Some(err) = error {
                                    p.outcome(Outcome::OutputFailed, Some(err));
                                }
                            });
                        }
                        match delivery {
                            FileDelivery::Written => {
                                let mode_str = match self.config.output.file_mode {
                                    FileMode::Overwrite => "wrote",
                                    FileMode::Append => "appended",
                                };
                                tracing::info!("{} transcription to {:?}", mode_str, output_path);
                                let outcome = if final_text.is_empty() {
                                    TranscriptOutcome::empty()
                                } else {
                                    TranscriptOutcome::ok(final_text.chars().count())
                                };
                                write_result_sidecar(&output_path, &outcome);
                                self.publish_osd_text_outcome(&final_text);
                                self.play_feedback(SoundEvent::TranscriptionComplete);
                            }
                            FileDelivery::FellBack(err) => {
                                write_result_sidecar(
                                    &output_path,
                                    &TranscriptOutcome::error(&format!(
                                        "{err}; copied to the clipboard instead"
                                    )),
                                );
                                self.publish_osd_text_outcome(&final_text);
                                self.play_feedback(SoundEvent::TranscriptionComplete);
                            }
                            FileDelivery::Lost(err) => {
                                write_result_sidecar(&output_path, &TranscriptOutcome::error(&err));
                                self.publish_osd_outcome(
                                    osd::session::OutcomeKind::OutputFailed,
                                    0,
                                );
                                self.play_feedback(SoundEvent::Error);
                            }
                        }
                        output::deferred_notice::flush().await;

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

                    if self.pipelines_dictation() && self.hotkey_down {
                        tracing::info!(
                            "Hotkey held for the next dictation; output waits for the release"
                        );
                        let history = self.history_take();
                        self.deferred_output = Some(DeferredOutput {
                            config: output_config,
                            text: final_text,
                            original: text.clone(),
                            history,
                        });
                        *state = State::Idle;
                        self.update_state("idle");
                        return;
                    }

                    *state = State::Outputting {
                        text: final_text.clone(),
                    };
                    self.deliver_dictation(&output_config, &final_text, &text, true)
                        .await;

                    *state = State::Idle;
                    self.update_state("idle");
                }
            }
            Ok(Err(e)) => {
                tracing::error!("Transcription failed: {}", e);
                self.history_note(|p| {
                    p.outcome(
                        crate::history::Outcome::TranscriptionFailed,
                        Some(e.to_string()),
                    )
                });
                self.publish_osd_outcome(osd::session::OutcomeKind::Error, 0);
                self.keep_failed_recording(state).await;
                self.reset_to_idle(state).await;
            }
            Err(e) => {
                // JoinError - task was cancelled or panicked
                if !join_error_poisons_engine(&e) {
                    tracing::debug!("Transcription task was cancelled");
                    self.history_note(|p| p.outcome(crate::history::Outcome::Cancelled, None));
                } else {
                    tracing::error!("Transcription task panicked: {}", e);
                    self.history_note(|p| {
                        p.outcome(
                            crate::history::Outcome::TranscriptionFailed,
                            Some("the transcription engine crashed".into()),
                        )
                    });
                    self.publish_osd_outcome(osd::session::OutcomeKind::Error, 0);
                    self.keep_failed_recording(state).await;

                    // spawn_blocking already kept the panic from reaching the
                    // daemon, so this is not about survival. It is about not
                    // reusing an engine whose internal state is whatever the
                    // panic left behind: drop the cached model so the next
                    // recording loads a clean one (#643).
                    //
                    // Only on a real panic. A cancellation is our own doing
                    // and leaves the engine perfectly usable.
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
                    // model_manager only covers Whisper. Every other engine
                    // clones from transcriber_preloaded, so a poisoned
                    // instance there has to be discarded the same way;
                    // get_transcriber_for_recording re-creates it on the
                    // next recording.
                    if self.transcriber_preloaded.take().is_some() {
                        tracing::warn!(
                            "Dropped the preloaded transcriber after the panic; \
                             the next recording will re-create it"
                        );
                    }
                }
                self.reset_to_idle(state).await;
            }
        }
    }

    fn note_idle_transition(&self, state_name: &str) {
        use std::sync::atomic::Ordering;
        let idle = state_name == "idle";
        let was_idle = self.last_state_idle.swap(idle, Ordering::Relaxed);
        if idle && !was_idle {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            self.idle_since.store(now, Ordering::Relaxed);
        }
    }

    /// Whether a start request stamped `issued_at` was issued while the
    /// daemon was still busy with the previous dictation. Requests with no
    /// stamp (a bare SIGUSR1) are taken as current.
    fn start_request_predates_idle(&self, issued_at: Option<u128>) -> bool {
        let idle_since = self.idle_since.load(std::sync::atomic::Ordering::Relaxed);
        matches!(issued_at, Some(issued) if idle_since > 0 && issued < u128::from(idle_since))
    }

    /// Load the transcription model at startup (on_demand_loading off).
    fn preload_transcriber(
        &mut self,
        model_manager: &mut ModelManager,
    ) -> std::result::Result<(), crate::error::TranscribeError> {
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
                    self.transcriber_preloaded = Some(Arc::from(
                        crate::transcribe::create_transcriber(&self.config)?,
                    ));
                } else {
                    // Use model manager for Whisper
                    model_manager.preload_primary()?;
                }
            }
            crate::config::TranscriptionEngine::Parakeet
            | crate::config::TranscriptionEngine::Moonshine
            | crate::config::TranscriptionEngine::SenseVoice
            | crate::config::TranscriptionEngine::Paraformer
            | crate::config::TranscriptionEngine::Dolphin
            | crate::config::TranscriptionEngine::Omnilingual
            | crate::config::TranscriptionEngine::Cohere
            | crate::config::TranscriptionEngine::Gguf
            | crate::config::TranscriptionEngine::OpenVino
            | crate::config::TranscriptionEngine::Soniox => {
                // Non-Whisper engines do their own setup; Soniox just validates
                // API key + endpoint at construction (no model to download).
                self.transcriber_preloaded = Some(Arc::from(
                    crate::transcribe::create_transcriber(&self.config)?,
                ));
            }
        }
        Ok(())
    }

    /// The model failed to load at startup and the daemon is about to exit.
    /// Under systemd it is restarted every few seconds and fails the same
    /// way, so the notice is posted once per distinct failure: a marker in
    /// the runtime directory remembers the last one until a start succeeds.
    async fn notify_startup_model_failure(&self, err: &crate::error::TranscribeError) {
        let model = self.config.model_name();
        let key = format!("{}|{}|{}", self.config.engine.name(), model, err);
        let marker = Config::runtime_dir().join(STARTUP_FAILURE_MARKER);
        if std::fs::read_to_string(&marker).ok().as_deref() == Some(key.as_str()) {
            tracing::debug!("Startup failure already notified; not repeating it");
            return;
        }
        let _ = std::fs::write(&marker, &key);
        let notice = crate::failure_notice::model_load_failed(&self.config, model, err, true);
        self.post_failure_notice(&notice).await;
    }

    /// Fire a desktop notification when the running binary can't service
    /// the configured engine (e.g. `engine = "parakeet"` but the wrapper
    /// dispatches to a CPU Whisper variant — the Ryan case from #450).
    /// Logged at WARN regardless, so journalctl users see it too.
    /// Returns whether it notified.
    fn warn_on_variant_mismatch(&self) -> bool {
        let inventory = crate::setup::binary::inventory();
        let Some(mismatch) = crate::setup::variant_check::detect_mismatch(&self.config, &inventory)
        else {
            return false;
        };

        let active = mismatch
            .active_variant_name
            .as_deref()
            .unwrap_or("the running binary");
        let title = format!("Voxtype: {} unavailable", mismatch.configured_engine);
        let body = mismatch.explanation();

        tracing::warn!(
            engine = mismatch.configured_engine,
            feature = mismatch.required_feature,
            active = active,
            "Variant mismatch at daemon startup: {}",
            body
        );
        crate::notification::send_sync(&title, &body);
        true
    }

    /// Fire a desktop notification when the daemon has no WAYLAND_DISPLAY or
    /// DISPLAY, which is how a systemd service started before the compositor
    /// imported its environment looks: every typing and clipboard tool fails
    /// and dictation goes nowhere. Not needed when output goes to a file.
    #[cfg(target_os = "linux")]
    fn warn_on_missing_display_env(&self) {
        let out = &self.config.output;
        if out.mode == crate::config::OutputMode::File && out.file_path.is_some() {
            return;
        }
        if !output::session::display_env_missing() {
            return;
        }
        let under_systemd = std::env::var_os("INVOCATION_ID").is_some();
        let (title, body) = output::session::display_env_notice(under_systemd);
        tracing::warn!("{}: {}", title, body.replace('\n', " "));
        crate::notification::send_sync(&title, &body);
    }

    /// Say loudly, once per start, when this is a GPU build whose execution
    /// provider cannot load (e.g. cuDNN missing for voxtype-onnx-cuda).
    /// ONNX Runtime would otherwise fall back to CPU with only a raw dlopen
    /// error in the journal, and dictation would just be slow for no
    /// visible reason. Dictation keeps working on CPU either way.
    fn warn_on_unloadable_gpu_provider(&self) {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Some(missing) = crate::setup::gpu::unresolved_provider_deps_at(&exe) else {
            return;
        };
        if missing.is_empty() {
            return;
        }
        let binary = exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fix = crate::setup::gpu::provider_fix_lines(&binary, &missing);
        tracing::error!(
            "GPU acceleration unavailable: {} cannot load its GPU execution provider \
             (missing {}), falling back to CPU. {}",
            binary,
            missing.join(", "),
            fix.join(" ")
        );
        crate::notification::send_sync(
            "Voxtype: GPU unavailable, using CPU",
            &format!("Missing {}.\n{}", missing.join(", "), fix.join("\n")),
        );
    }

    /// Run the daemon main loop
    pub async fn run(&mut self) -> Result<()> {
        tracing::info!("Starting voxtype daemon");

        // Single-instance lock before anything else. Pidlock has no Drop
        // impl, so holding the binding for run()'s lifetime keeps it; a
        // crashed daemon's leftover lockfile is recovered as stale on the
        // next start.
        let _instance_lock = acquire_instance_lock(&Config::runtime_dir())?;

        // Engine-vs-binary mismatch check at startup so users see a desktop
        // notification before the first transcription attempt would fail.
        // create_transcriber() below will surface the same error in logs,
        // but logs go to journald and most users never see them. A
        // notify-send pops up where the user is actually looking. See
        // #450 — the silent v0.6.x to v0.7.0 wrapper-flip incident.
        let variant_mismatch_notified = self.warn_on_variant_mismatch();
        #[cfg(target_os = "linux")]
        self.warn_on_missing_display_env();
        self.warn_on_unloadable_gpu_provider();

        // On Omarchy, link the packaged Settings panel on first start and
        // rescan it after a package upgrade. Runs on its own thread; it never
        // blocks startup or fails the daemon.
        crate::setup::omarchy::spawn_auto_link(&self.config);

        // Everything else the config depends on (model, VAD model, output
        // tools, display env, OSD, meeting tools): one grouped notification
        // for errors, warnings to the log. Stats, PATH lookups and env reads
        // only, and before model loading so a missing model is reported
        // even though the load below then fails. `voxtype doctor` runs the
        // full set.
        crate::setup::doctor::startup_summary(&self.config).await;

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
        cleanup_cancel_file();
        // The GGUF worker rewrites this once it has bound a backend; a report
        // from a previous daemon must not describe this one.
        let _ =
            std::fs::remove_file(Config::runtime_dir().join(crate::status_json::GGUF_STATUS_FILE));
        let _ = std::fs::remove_file(Config::runtime_dir().join(START_REQUEST_FILE));
        cleanup_profile_override();

        // Clean up any stale meeting command files
        cleanup_meeting_files();

        // Mark any orphaned active meetings as completed
        cleanup_stale_meetings(&self.config);

        // A predecessor's acceleration state describes a model this daemon
        // has not loaded yet.
        crate::setup::accel::clear();

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

        // Start the audio-level broadcaster for the OSD. Failure to bind
        // the socket is not fatal: the daemon still runs without an OSD
        // feed, and downstream code treats `level_hub == None` as "no OSD".
        let level_socket = audio::levels::default_socket_path();
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
        let osd_active = if self.config.osd.enabled && self.level_hub.is_some() {
            self.spawn_osd()
        } else {
            "none"
        };
        osd::frontend::publish_active(&Config::runtime_dir(), osd_active);

        // History retention and compression: now, then once a day. Runs
        // whether or not history is on, so turning it off never leaves old
        // days past their retention.
        if self.history_maintenance.is_none() {
            self.history_maintenance = Some(crate::history::recorder::spawn_daily_maintenance(
                self.history.clone(),
            ));
        }

        // The single-instance lock is already held. Never register a duplicate tray.
        #[cfg(target_os = "linux")]
        if self.tray.is_none() {
            self.tray =
                crate::tray::TrayRuntime::start_for_daemon(&self.config, self.config_path.clone());
        }

        // Start the meeting-event broadcaster for the meeting HUD. Bound
        // unconditionally (like the audio level socket) regardless of
        // whether meeting mode is enabled in config, so a HUD can always
        // connect and simply see the idle snapshot when no meeting is
        // running. Failure to bind is not fatal.
        let meeting_socket = meeting::events::default_socket_path();
        match meeting::MeetingEventHub::start(meeting_socket.clone()).await {
            Ok(hub) => {
                tracing::info!("Meeting event socket: {:?}", hub.socket_path());
                self.meeting_hud.recording_notice = meeting::notice::pending();
                hub.update_snapshot(&self.meeting_hud);
                self.meeting_event_hub = Some(hub);
                self.announce_recoverable_meetings();
                self.notify_interrupted_meetings();
            }
            Err(e) => {
                tracing::warn!(
                    "Could not start meeting event socket at {:?}: {}",
                    meeting_socket,
                    e
                );
            }
        }

        // Offer to record calls as they start ([meeting.detect]). Held for
        // run()'s lifetime; dropping it stops the watcher.
        #[cfg(target_os = "linux")]
        let _meeting_detector =
            meeting::detect::runner::spawn_in_daemon(&self.config, self.config_path.clone());

        // Keep the calendar link's events fresh for meeting titles. It does
        // nothing until `voxtype meeting calendar set-ics` sets a link.
        let calendar_refresh = self.config.meeting.enabled.then(|| {
            tokio::spawn(meeting::calendar::refresh_forever(
                meeting::calendar::source::Store::new(
                    meeting::calendar::source::Store::default_dir(),
                ),
            ))
        });
        let _calendar_refresh = AbortOnDrop(calendar_refresh);

        crate::daemon_status::publish_version();

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
            if self.config.hotkey.enabled {
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
            .config
            .hotkey
            .enabled
        {
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
        if !self.config.on_demand_loading() {
            tracing::info!("Loading transcription model: {}", self.config.model_name());
            if let Err(e) = self.preload_transcriber(&mut model_manager) {
                tracing::error!("Failed to preload model: {}", e);
                // A variant mismatch was already explained; this failure is
                // its consequence, and a second notice would replace the fix.
                if !variant_mismatch_notified {
                    self.notify_startup_model_failure(&e).await;
                }
                return Err(crate::error::VoxtypeError::Transcribe(e));
            }
            tracing::info!("Model loaded, ready for voice input");
        } else {
            tracing::info!("On-demand loading enabled, model will be loaded when recording starts");
        }
        let _ = std::fs::remove_file(Config::runtime_dir().join(STARTUP_FAILURE_MARKER));

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
        let mut hotkey_rx = if let Some(ref mut listener) = hotkey_listener {
            match listener.start() {
                Ok(rx) => {
                    // A keyboard opened: a later failure is news again.
                    #[cfg(target_os = "linux")]
                    hotkey::access::clear_notice(&hotkey::access::notice_marker_path());
                    Some(rx)
                }
                Err(e) => {
                    tracing::warn!("Failed to start hotkey listener: {}\n  Until then, use 'voxtype record' commands.", e);
                    // The daemon keeps running for `voxtype record`, so
                    // without this the built-in hotkey just does nothing.
                    // Notified once per install, not on every start: people
                    // who use compositor keybindings never needed it. The
                    // warning above is logged every time.
                    #[cfg(target_os = "linux")]
                    if let Some((title, body)) =
                        hotkey::access::notify_once(&e, &hotkey::access::notice_marker_path())
                    {
                        notification::send_status(title, &body, "normal", Lifetime::Millis(15000))
                            .await;
                    }
                    None
                }
            }
        } else {
            None
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let mut hotkey_rx: Option<tokio::sync::mpsc::Receiver<HotkeyEvent>> = None;

        // Current state
        let mut state = State::Idle;

        // Audio capture (created fresh for each recording)
        let mut audio_capture: Option<Box<dyn AudioCapture>> = None;

        // Recording timeout
        let max_duration = Duration::from_secs(self.config.audio.max_duration_secs as u64);

        let activation_mode = self.config.hotkey.mode;
        if hotkey_rx.is_some() {
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
        // Cached transcriber for eager chunk processing during recording
        let mut eager_transcriber: Option<Arc<dyn Transcriber>> = None;

        // Streaming session locals (Some only while State::Streaming).
        let mut streaming_handle: Option<StreamHandle> = None;
        let mut streaming_session: Option<StreamingSession> = None;
        let mut streaming_chain: Option<Vec<Box<dyn TextOutput>>> = None;

        // Private synthetic session injection exercises the real select! stop
        // callers without an input device, capture device or provider factory.
        #[cfg(test)]
        if let Some((initial, selected, replacement, events)) = self.history_test_eager_loop.take()
        {
            state = initial;
            hotkey_rx = Some(events);
            audio_capture = self.history_test_capture.take();
            self.begin_osd_session(&state);
            self.history_test_capture = self.history_test_next_capture.take();
            if let Some(flag) = self.history_test_completion_observer.take() {
                self.eager_decoded = flag;
            }
            self.config.engine = crate::config::TranscriptionEngine::Soniox;
            self.transcriber_preloaded = Some(replacement);
            self.spawn_chunk_transcription(0, vec![0.2; 16000], selected.clone());
            eager_transcriber = Some(selected);
        }

        // Persistent poll timers for the recording and transcribing arms. A
        // `sleep()` arm is rebuilt every loop iteration, so it competes with
        // the unconditional 100ms poll below: both are due on the same tick,
        // select! picks one at random, and the loser starts over. That turned
        // the cancel check into a coin flip every 100ms (observed 0.1-0.5s
        // before a `record cancel` took effect). An Interval keeps its
        // deadline across iterations, so a missed tick fires on the next one.
        let mut recording_poll = tokio::time::interval(Duration::from_millis(100));
        recording_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Whether the current recording was already warned that it is nearing
        // max_duration_secs. Re-arms whenever a recording is further than the
        // warning lead from the cap, so it needs no per-session reset.
        let mut cap_warned = false;
        let mut transcribing_poll = tokio::time::interval(Duration::from_millis(100));
        transcribing_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // Push-to-talk events that arrived while a dictation was
        // transcribing, and the same events queued to run once it's done.
        let mut held_events: Vec<HotkeyEvent> = Vec::new();
        let mut replay_queue: std::collections::VecDeque<HotkeyEvent> = Default::default();

        loop {
            self.apply_pending_language(state.is_idle());
            tokio::select! {
                Some(language) = async {
                    #[cfg(target_os = "linux")]
                    if let Some(tray) = &mut self.tray { return tray.next_language().await; }
                    std::future::pending::<Option<&'static str>>().await
                } => {
                    match self.queue_language(language) {
                        Ok(()) => {
                            #[cfg(target_os = "linux")]
                            if let (Some(tray), Some(next)) = (&self.tray, &self.pending_language) {
                                let mut menu = crate::language_selection::LanguageMenu::for_config(next);
                                menu.explanation = "Saved: applies to the next dictation, not the current utterance".into();
                                tray.update_language(menu);
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "Tray language selection failed");
                            #[cfg(target_os = "linux")]
                            if let Some(tray) = &self.tray {
                                let mut menu = crate::language_selection::LanguageMenu::for_config(
                                    self.pending_language.as_ref().unwrap_or(&self.config));
                                menu.explanation = format!("Language change failed: {error}");
                                tray.update_language(menu);
                            }
                            send_notification("Voxtype: Language change failed", &error,
                                self.config.output.notification.show_engine_icon, self.config.engine,
                                &self.config.output.notification.urgency).await;
                        }
                    }
                }
                // Handle hotkey events (only if hotkey listener is enabled)
                Some(hotkey_event) = async {
                    if let Some(event) = replay_queue.pop_front() {
                        return Some(event);
                    }
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
                            self.hotkey_down = true;
                            if matches!(state, State::Transcribing { .. }) && self.standby_capture.is_some() {
                                tracing::debug!("Busy transcribing; recording continues on the standby capture");
                                held_events.push(HotkeyEvent::Pressed { model_override, profile_override });
                                continue;
                            }
                            if state.is_idle() {
                                if !self.accept_model_override(model_override.as_deref()).await {
                                    continue;
                                }
                                // Write profile override file if a profile modifier was held
                                if let Some(ref profile_name) = profile_override {
                                    write_profile_override(profile_name);
                                }

                                tracing::info!("Recording started");

                                // Send notification if enabled
                                if self.config.output.notification.on_recording_start {
                                    send_notification_with_lifetime("Push to Talk Active", "Recording...", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency, Lifetime::UntilClosed).await;
                                }

                                // Prepare model for transcription
                                self.prepare_for_recording(model_override.as_deref());

                                // Pause or duck playback before either capture path opens
                                // the microphone.
                                self.suppress_recording_media().await;

                                // Try streaming first; fall through to batch if the engine
                                // doesn't support streaming or setup fails.
                                if self.try_start_streaming(
                                    &mut state,
                                    &mut audio_capture,
                                    &mut streaming_handle,
                                    &mut streaming_session,
                                    &mut streaming_chain,
                                    model_override.clone(),
                                    false,
                                ).await {
                                    tracing::info!("Streaming session started (push-to-talk)");
                                    self.stop_standby_capture().await;
                                } else {
                                    // Create and start audio capture
                                    tracing::debug!("Creating audio capture with device: {}", self.config.audio.device);
                                    let capture = match self.standby_capture.take() {
                                        Some(capture) => {
                                            tracing::debug!("Recording on the standby capture");
                                            Ok(capture)
                                        }
                                        None => self.start_recording_capture(false).await,
                                    };
                                    match capture {
                                        Ok(capture) => {
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
                                            self.begin_osd_session(&state);
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
                                            self.discard_recording_profile();
                                        }
                                    }
                                }
                            }
                        }

                        (HotkeyEvent::Released, ActivationMode::PushToTalk) => {
                            tracing::debug!("Received HotkeyEvent::Released (push-to-talk), state.is_recording() = {}", state.is_recording());
                            self.hotkey_down = false;
                            if matches!(state, State::Transcribing { .. })
                                && held_events.iter().any(|e| matches!(e, HotkeyEvent::Pressed { .. }))
                            {
                                held_events.push(HotkeyEvent::Released);
                                continue;
                            }
                            self.flush_deferred_output().await;
                            if state.is_streaming() {
                                self.request_streaming_stop(
                                    &mut audio_capture,
                                    &mut streaming_chain,
                                    true,
                                )
                                .await;
                            } else if let State::Recording { model_override, .. } = &state {
                                let model_override = model_override.clone();

                                self.start_transcription_task(
                                    &mut state,
                                    &mut audio_capture,
                                    model_override,
                                ).await;
                            } else if state.is_eager_recording() {
                                // Handle eager recording stop - extract model_override first
                                let model_override = match &state {
                                    State::EagerRecording { model_override, .. } => model_override.clone(),
                                    _ => None,
                                };

                                let duration = state.recording_duration().unwrap_or_default();
                                tracing::info!("Eager recording stopped ({:.1}s)", duration.as_secs_f32());

                                // Stop audio capture and get remaining samples
                                if let Some(mut capture) = audio_capture.take() {
                                    if let Ok(final_samples) = capture.stop().await {
                                        // Add final samples to accumulated audio
                                        if let State::EagerRecording { accumulated_audio, .. } = &mut state {
                                            accumulated_audio.extend(final_samples);
                                        }
                                    }
                                }
                                self.restore_recording_media();

                                self.play_feedback(SoundEvent::RecordingStop);

                                end_recording_notification("Recording Stopped", "Transcribing...", &self.config.output.notification, self.config.engine).await;

                                let transcriber = if let Some(selected) = eager_transcriber.as_ref() {
                                    selected.clone()
                                } else {
                                    match self.get_transcriber_for_recording(model_override.as_deref()).await {
                                        Ok(t) => t,
                                        Err(()) => {
                                            state = State::Idle;
                                            self.update_state("idle");
                                            continue;
                                        }
                                    }
                                };

                                self.update_state("transcribing");

                                if let Some(text) = self.finish_eager_recording(&mut state, transcriber).await {
                                    // Move to outputting state and handle via transcription result flow
                                    state = State::Transcribing { audio: Vec::new() };
                                    self.handle_transcription_result(&mut state, Ok(Ok(text))).await;
                                } else {
                                    tracing::debug!("Eager recording produced empty result");
                                    self.publish_osd_outcome(osd::session::OutcomeKind::Empty, 0);
                                    self.reset_to_idle(&mut state).await;
                                }
                                eager_transcriber = None;
                            }
                        }

                        // === TOGGLE MODE ===
                        (HotkeyEvent::Pressed { model_override, profile_override }, ActivationMode::Toggle) => {
                            tracing::debug!("Received HotkeyEvent::Pressed (toggle), state.is_idle() = {}, state.is_recording() = {}, model_override = {:?}, profile_override = {:?}",
                                state.is_idle(), state.is_recording(), model_override, profile_override);

                            if state.is_idle() {
                                if !self.accept_model_override(model_override.as_deref()).await {
                                    continue;
                                }
                                // Write profile override file if a profile modifier was held
                                if let Some(ref profile_name) = profile_override {
                                    write_profile_override(profile_name);
                                }

                                // Start recording
                                tracing::info!("Recording started (toggle mode)");

                                if self.config.output.notification.on_recording_start {
                                    send_notification_with_lifetime("Recording Started", "Press hotkey again to stop", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency, Lifetime::UntilClosed).await;
                                }

                                // Prepare model for transcription
                                self.prepare_for_recording(model_override.as_deref());

                                self.suppress_recording_media().await;

                                if self.try_start_streaming(
                                    &mut state,
                                    &mut audio_capture,
                                    &mut streaming_handle,
                                    &mut streaming_session,
                                    &mut streaming_chain,
                                    model_override.clone(),
                                    false,
                                ).await {
                                    tracing::info!("Streaming session started (toggle)");
                                } else {
                                    match self.start_recording_capture(false).await {
                                        Ok(capture) => {
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
                                            self.begin_osd_session(&state);
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
                                            self.discard_recording_profile();
                                        }
                                    }
                                }
                            } else if state.is_streaming() {
                                self.request_streaming_stop(
                                    &mut audio_capture,
                                    &mut streaming_chain,
                                    false,
                                )
                                .await;
                            } else if let State::Recording { model_override: current_model_override, .. } = &state {
                                let model_override = current_model_override.clone();

                                // Stop recording and start transcription
                                self.start_transcription_task(
                                    &mut state,
                                    &mut audio_capture,
                                    model_override,
                                ).await;
                            } else if state.is_eager_recording() {
                                // Handle eager recording stop in toggle mode - extract model_override first
                                let model_override = match &state {
                                    State::EagerRecording { model_override, .. } => model_override.clone(),
                                    _ => None,
                                };

                                let duration = state.recording_duration().unwrap_or_default();
                                tracing::info!("Eager recording stopped ({:.1}s)", duration.as_secs_f32());

                                // Stop audio capture and get remaining samples
                                if let Some(mut capture) = audio_capture.take() {
                                    if let Ok(final_samples) = capture.stop().await {
                                        if let State::EagerRecording { accumulated_audio, .. } = &mut state {
                                            accumulated_audio.extend(final_samples);
                                        }
                                    }
                                }
                                self.restore_recording_media();

                                self.play_feedback(SoundEvent::RecordingStop);

                                end_recording_notification("Recording Stopped", "Transcribing...", &self.config.output.notification, self.config.engine).await;

                                let transcriber = if let Some(selected) = eager_transcriber.as_ref() {
                                    selected.clone()
                                } else {
                                    match self.get_transcriber_for_recording(model_override.as_deref()).await {
                                        Ok(t) => t,
                                        Err(()) => {
                                            state = State::Idle;
                                            self.update_state("idle");
                                            continue;
                                        }
                                    }
                                };

                                self.update_state("transcribing");

                                if let Some(text) = self.finish_eager_recording(&mut state, transcriber).await {
                                    state = State::Transcribing { audio: Vec::new() };
                                    self.handle_transcription_result(&mut state, Ok(Ok(text))).await;
                                } else {
                                    tracing::debug!("Eager recording produced empty result");
                                    self.publish_osd_outcome(osd::session::OutcomeKind::Empty, 0);
                                    self.reset_to_idle(&mut state).await;
                                }
                                eager_transcriber = None;
                            }
                        }

                        (HotkeyEvent::Released, ActivationMode::Toggle) => {
                            // In toggle mode, we ignore key release events
                            tracing::trace!("Ignoring HotkeyEvent::Released in toggle mode");
                        }

                        // === PASTE-LAST KEY (works in both modes) ===
                        (HotkeyEvent::PasteLast, _) => {
                            if state.is_idle() {
                                self.paste_last_dictation();
                            } else {
                                tracing::debug!("Ignoring the paste-last key while a dictation is in progress");
                            }
                        }

                        // === CANCEL KEY (works in both modes) ===
                        (HotkeyEvent::Cancel, _) => {
                            tracing::debug!("Received HotkeyEvent::Cancel");
                            self.hotkey_down = false;
                            // Cancelling a recording queued behind a
                            // transcription drops that recording, not the
                            // dictation being transcribed.
                            if !held_events.is_empty() {
                                held_events.clear();
                                continue;
                            }
                            self.flush_deferred_output().await;
                            self.stop_standby_capture().await;
                            self.cancel_requested_session(
                                &mut state,
                                &mut audio_capture,
                                &mut streaming_handle,
                                &mut streaming_session,
                                &mut streaming_chain,
                                &mut eager_transcriber,
                            ).await;
                        }
                    }
                }

                // Check for recording timeout and cancel requests
                _ = recording_poll.tick(), if state.is_recording() => {
                    // Check for cancel request first
                    if check_cancel_requested() {
                        self.cancel_requested_session(
                            &mut state,
                            &mut audio_capture,
                            &mut streaming_handle,
                            &mut streaming_session,
                            &mut streaming_chain,
                            &mut eager_transcriber,
                        ).await;
                        continue;
                    }

                    // Populate eager transcriber cache on first poll
                    if eager_transcriber.is_none() && state.is_eager_recording() {
                        let model_override = match &state {
                            State::EagerRecording { model_override, .. } => model_override.clone(),
                            _ => None,
                        };
                        let separate_override = model_override.as_deref().is_some_and(|name| {
                            matches!(
                                model_catalog::override_route(&self.config, name),
                                Ok(model_catalog::OverrideRoute::Separate { .. })
                            )
                        });
                        let model_override = model_override.as_deref();
                        if separate_override {
                            // Chunks wait until the override model has loaded;
                            // whatever is left over is transcribed at stop.
                            if let Some(name) = model_override {
                                eager_transcriber = self.ready_override(name).await;
                            }
                        } else {
                            eager_transcriber = self.transcriber_preloaded.clone();
                        }
                        if !separate_override
                            && eager_transcriber.is_none()
                            && self.config.engine
                                == crate::config::TranscriptionEngine::Whisper
                        {
                            // Whisper engine: get from model manager
                            if let Some(ref mut mm) = self.model_manager {
                                match mm.get_prepared_transcriber(model_override) {
                                    Ok(t) => {
                                        tracing::debug!("Created eager transcriber for chunk dispatch");
                                        eager_transcriber = Some(t);
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
                        if let Some(ref mut capture) = audio_capture {
                            let new_samples = capture.get_samples().await;
                            if !new_samples.is_empty() {
                                accumulated_audio.extend(new_samples);
                            }
                        }

                        if let Some(ref transcriber) = eager_transcriber {
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
                                    self.history_note(|p| {
                                        p.entry.ended_by = crate::history::EndedBy::Silence
                                    });
                                    self.stop_active_recording(&mut state, &mut audio_capture, &mut streaming_chain, &mut eager_transcriber)
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
                    if let (true, Some(elapsed)) =
                        (audio_capture.is_some(), state.recording_duration())
                    {
                        match cap_warning_due(elapsed, max_duration) {
                            Some(left) if !cap_warned => {
                                cap_warned = true;
                                self.warn_recording_cap(left, max_duration).await;
                            }
                            Some(_) => {}
                            None => cap_warned = false,
                        }
                    }

                    let timeout_fired = audio_capture.is_some()
                        && state.recording_duration().is_some_and(|d| d > max_duration);
                    if timeout_fired {
                        self.osd_limit_reached = true;
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
                            self.request_streaming_stop(
                                &mut audio_capture,
                                &mut streaming_chain,
                                false,
                            )
                            .await;
                            self.notify_recording_capped(max_duration).await;
                            continue;
                        }

                        tracing::warn!(
                            "Recording timeout ({:.0}s limit), transcribing captured audio",
                            max_duration.as_secs_f32()
                        );


                        let model_override = match &state {
                            State::Recording { model_override, .. } => model_override.clone(),
                            State::EagerRecording { model_override, .. } => model_override.clone(),
                            _ => None,
                        };

                        if state.is_eager_recording() {
                            if let Some(mut capture) = audio_capture.take() {
                                if let Ok(final_samples) = capture.stop().await {
                                    if let State::EagerRecording { accumulated_audio, .. } = &mut state {
                                        accumulated_audio.extend(final_samples);
                                    }
                                }
                            }
                            self.restore_recording_media();

                            let transcriber = if let Some(selected) = eager_transcriber.as_ref() {
                                selected.clone()
                            } else {
                                match self.get_transcriber_for_recording(model_override.as_deref()).await {
                                    Ok(transcriber) => transcriber,
                                    Err(()) => {
                                        self.reset_to_idle(&mut state).await;
                                        continue;
                                    }
                                }
                            };

                            self.update_state("transcribing");
                            self.notify_recording_capped(max_duration).await;

                            if let Some(text) = self.finish_eager_recording(&mut state, transcriber).await {
                                state = State::Transcribing { audio: Vec::new() };
                                self.handle_transcription_result(&mut state, Ok(Ok(text))).await;
                            } else {
                                tracing::debug!("Eager recording timeout produced empty result");
                                self.publish_osd_outcome(osd::session::OutcomeKind::Empty, 0);
                                self.reset_to_idle(&mut state).await;
                            }
                            eager_transcriber = None;
                        } else {
                            for (_, task) in self.eager_chunk_tasks.drain(..) {
                                task.abort();
                            }

                            // After the "Recording Stopped" notification this
                            // posts, so it is not replaced by that one.
                            if self.start_transcription_task(
                                &mut state,
                                &mut audio_capture,
                                model_override,
                            ).await {
                                self.notify_recording_capped(max_duration).await;
                            }
                        }
                    }
                }

                // Handle SIGUSR1 - start recording (for compositor keybindings)
                _ = sigusr1.recv() => {
                    tracing::debug!("Received SIGUSR1 (start recording)");
                    let start_request = StartRequest::read();
                    // A `record cancel` sent just before this start is still
                    // waiting for its poll tick. Apply it first so commands
                    // take effect in the order they were issued; otherwise
                    // this start lands on the still-live session and is lost.
                    if cancel_applies_to(&state) && check_cancel_requested() {
                        self.cancel_requested_session(
                            &mut state,
                            &mut audio_capture,
                            &mut streaming_handle,
                            &mut streaming_session,
                            &mut streaming_chain,
                            &mut eager_transcriber,
                        ).await;
                    }
                    if !state.is_idle() {
                        tracing::info!(
                            "Ignoring start request: daemon is {} (stop or cancel it first)",
                            state
                        );
                    } else if self.start_request_predates_idle(start_request.issued_at) {
                        // Sent while the last dictation was still being
                        // typed or pasted; the event loop only got to it
                        // now. Treat it as the busy daemon would have.
                        tracing::info!(
                            "Ignoring start request sent while the previous dictation was still being delivered"
                        );
                    } else {
                        // Read model override from file (set by `voxtype record start --model X`)
                        let model_override = read_model_override();
                        if !self.accept_model_override(model_override.as_deref()).await {
                            continue;
                        }
                        tracing::info!("Recording started (external trigger), model_override = {:?}", model_override);

                        if self.config.output.notification.on_recording_start {
                            send_notification_with_lifetime("Recording Started", "External trigger", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency, Lifetime::UntilClosed).await;
                        }

                        // Prepare model for transcription
                        self.prepare_for_recording(model_override.as_deref());

                        self.suppress_recording_media().await;

                        // A start request scopes the stale-cancel check to
                        // this start; hotkey starts carry none.
                        self.start_request = start_request.issued_at;
                        if self.try_start_streaming(
                            &mut state,
                            &mut audio_capture,
                            &mut streaming_handle,
                            &mut streaming_session,
                            &mut streaming_chain,
                            model_override.clone(),
                            true,
                        ).await {
                            tracing::info!("Streaming session started (SIGUSR1)");
                        } else {
                            match self.start_recording_capture(true).await {
                                Ok(capture) => {
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
                                    self.begin_osd_session(&state);
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
                                }
                            }
                        }
                        self.start_request = None;
                    }
                }

                // Handle SIGUSR2 - stop recording (for compositor keybindings)
                _ = sigusr2.recv() => {
                    tracing::debug!("Received SIGUSR2 (stop recording)");
                    // Same ordering rule as SIGUSR1: a cancel issued before
                    // this stop discards the recording instead of the stop
                    // transcribing it.
                    if cancel_applies_to(&state) && check_cancel_requested() {
                        self.cancel_requested_session(
                            &mut state,
                            &mut audio_capture,
                            &mut streaming_handle,
                            &mut streaming_session,
                            &mut streaming_chain,
                            &mut eager_transcriber,
                        ).await;
                    }
                    self.stop_active_recording(&mut state, &mut audio_capture, &mut streaming_chain, &mut eager_transcriber).await;
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
                    if held_events.is_empty() {
                        self.stop_standby_capture().await;
                    }
                    replay_queue.extend(held_events.drain(..));
                }

                Some(report) = async {
                    match self.dead_input_rx.as_mut() {
                        Some(receiver) => receiver.recv().await,
                        None => std::future::pending().await,
                    }
                }, if self.dead_input_rx.is_some() && state.is_recording() => {
                    self.abort_recording_dead_input(
                        report,
                        &mut state,
                        &mut audio_capture,
                        &mut streaming_handle,
                        &mut streaming_session,
                        &mut streaming_chain,
                        &mut eager_transcriber,
                    )
                    .await;
                }

                // Streaming event pump (active only while State::Streaming).
                event = async {
                    match streaming_handle.as_mut() {
                        Some(h) => h.events.recv().await,
                        None => std::future::pending().await,
                    }
                }, if state.is_streaming() && streaming_handle.is_some() => {
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
                            if let Some(s) = streaming_session.as_mut() {
                                if file_output {
                                    s.observe_partial_delta(&text);
                                } else if let Some(chain) = streaming_chain.as_ref() {
                                    if let Err(e) = s.type_partial_delta(
                                        chain,
                                        text,
                                        self.config.output.pre_output_command.as_deref(),
                                        self.config.output.post_output_command.as_deref(),
                                    ).await {
                                        tracing::warn!("Streaming partial delta type failed: {}", e);
                                    }
                                } else {
                                    s.observe_partial_delta(&text);
                                }
                                if let State::Streaming { typed_chars, .. } = &mut state {
                                    *typed_chars = s.typed_chars();
                                }
                            }
                        }
                        Some(StreamingEvent::Final { text, .. }) => {
                            if let Some(s) = streaming_session.as_mut() {
                                if file_output {
                                    // Raw on purpose: file-mode text is
                                    // processed once, whole, at write time
                                    // in end_streaming — that also catches
                                    // matches spanning segment boundaries.
                                    s.commit_segment_silent(&text);
                                } else if let Some(chain) = streaming_chain.as_ref() {
                                    let profile = self.recording_profile();
                                    let vocabulary = profile
                                        .as_ref()
                                        .and_then(|name| self.profile_vocabularies.get(name));
                                    if let Err(e) = s.commit_segment(
                                        chain,
                                        &text,
                                        Some((&self.text_processor, vocabulary)),
                                        self.config.output.pre_output_command.as_deref(),
                                        self.config.output.post_output_command.as_deref(),
                                    ).await {
                                        tracing::error!("Streaming commit_segment failed: {}", e);
                                    }
                                } else {
                                    s.commit_segment_silent(&text);
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
                            if let Some(s) = streaming_session.as_mut() {
                                if file_output {
                                    // Raw on purpose — see the Final arm.
                                    s.replace_and_commit_silent(backspace, &text);
                                } else if let Some(chain) = streaming_chain.as_ref() {
                                    let profile = self.recording_profile();
                                    let vocabulary = profile
                                        .as_ref()
                                        .and_then(|name| self.profile_vocabularies.get(name));
                                    if let Err(e) = s.replace_and_commit(
                                        chain,
                                        backspace,
                                        &text,
                                        Some((&self.text_processor, vocabulary)),
                                        self.config.output.pre_output_command.as_deref(),
                                        self.config.output.post_output_command.as_deref(),
                                    ).await {
                                        tracing::error!("Streaming replace_and_commit failed: {}", e);
                                    }
                                } else {
                                    s.replace_and_commit_silent(backspace, &text);
                                }
                                if let State::Streaming { typed_chars, finalized_text, .. } = &mut state {
                                    *typed_chars = s.typed_chars();
                                    finalized_text.clear();
                                    finalized_text.push_str(s.finalized_text());
                                }
                            }
                        }
                        Some(StreamingEvent::Error(err)) => {
                            self.end_streaming_with_error(
                                err,
                                &mut state,
                                &mut audio_capture,
                                &mut streaming_handle,
                                &mut streaming_session,
                                &mut streaming_chain,
                            ).await;
                        }
                        Some(StreamingEvent::Ended) => {
                            self.end_streaming(&mut state, &mut audio_capture, &mut streaming_handle, &mut streaming_session, &mut streaming_chain, StreamingEndReason::Completed, ).await;
                        }
                        None => {
                            self.end_streaming(
                                &mut state,
                                &mut audio_capture,
                                &mut streaming_handle,
                                &mut streaming_session,
                                &mut streaming_chain,
                                StreamingEndReason::Failed(None),
                            ).await;
                        }
                    }
                }

                _ = async {
                    match self.streaming_stop_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                }, if state.is_streaming() && self.streaming_stop_deadline.is_some() => {
                    tracing::warn!("Streaming backend did not finish before the daemon stop deadline");
                    self.end_streaming(
                        &mut state,
                        &mut audio_capture,
                        &mut streaming_handle,
                        &mut streaming_session,
                        &mut streaming_chain,
                        StreamingEndReason::Failed(None),
                    ).await;
                }

                // Check for cancel during transcription
                _ = transcribing_poll.tick(), if matches!(state, State::Transcribing { .. }) => {
                    if check_cancel_requested() {
                        held_events.clear();
                        self.stop_standby_capture().await;
                        self.cancel_requested_session(
                            &mut state,
                            &mut audio_capture,
                            &mut streaming_handle,
                            &mut streaming_session,
                            &mut streaming_chain,
                            &mut eager_transcriber,
                        ).await;
                    }
                }

                // Clean up stale cancel file when idle and evict idle models
                _ = tokio::time::sleep(Duration::from_millis(500)), if matches!(state, State::Idle) => {
                    // Silently consume any stale cancel request, unless it
                    // may belong to a start signal that has not landed yet.
                    if !Config::runtime_dir().join(START_REQUEST_FILE).exists() {
                        let _ = check_cancel_requested();
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

                    self.handle_meeting_triggers().await;
                }

                // Process meeting audio chunks
                _ = tokio::time::sleep(Duration::from_millis(50)), if self.meeting_active() => {
                    // The 100ms polling branch is starved by this faster one
                    // while a meeting records, so every meeting trigger is
                    // handled here too. A start that went unread here used to
                    // sit on disk and begin a second meeting the moment this
                    // one stopped.
                    if self.handle_meeting_triggers().await {
                        continue;
                    }

                    // Get samples from dual audio capture
                    if let Some(ref mut capture) = self.meeting_audio_capture {
                        let dual_samples = capture.get_samples().await;
                        let liveness = self
                            .meeting_aec_liveness
                            .as_mut()
                            .map(|check| check.feed(&dual_samples.mic));
                        self.meeting_mic_buffer.extend(dual_samples.mic);
                        self.meeting_loopback_buffer.extend(dual_samples.loopback);
                        match liveness {
                            Some(audio::echo_cancel::Liveness::Alive) => {
                                self.meeting_aec_liveness = None;
                            }
                            Some(audio::echo_cancel::Liveness::Silent) => {
                                self.leave_silent_echo_canceller().await;
                            }
                            _ => {}
                        }

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
                        Some(MeetingEvent::Started { meeting_id, .. }) => {
                            // The HUD snapshot for this transition is already
                            // set synchronously in `start_meeting` (title,
                            // diarization info, started_at), ahead of this
                            // event reaching the select loop. Nothing to do.
                            tracing::info!("Meeting event: started {}", meeting_id);
                            #[cfg(unix)]
                            {
                                let view = self.config.meeting.view.clone();
                                let show = self.config.meeting.show_view_on_start;
                                tokio::task::spawn_blocking(move || {
                                    let what = crate::meeting::view::frontend::open_on_start(&view, show);
                                    tracing::info!("Meeting view on start: {}", what);
                                });
                            }
                        }
                        Some(MeetingEvent::ChunkProcessed { chunk_id, segments }) => {
                            tracing::debug!("Meeting event: chunk {} processed with {} segments",
                                chunk_id, segments.len());
                            self.publish_meeting_chunk(chunk_id, &segments);
                        }
                        Some(MeetingEvent::SpeakerRenamed { from, to }) => {
                            self.publish_meeting_rename(&from, &to);
                        }
                        Some(MeetingEvent::SegmentUpdated { segment }) => {
                            self.publish_segment_updated(&segment);
                        }
                        Some(MeetingEvent::SegmentDeleted { segment_id }) => {
                            self.publish_segment_deleted(segment_id);
                        }
                        Some(MeetingEvent::SummaryUpdated { meeting_id }) => {
                            if let Some(ref hub) = self.meeting_event_hub {
                                hub.publish_summary_updated(&meeting_id.to_string());
                            }
                        }
                        Some(MeetingEvent::Paused) => {
                            // `pause_meeting` already published this via
                            // `update_meeting_state("paused", ...)`.
                            tracing::info!("Meeting event: paused");
                        }
                        Some(MeetingEvent::Resumed) => {
                            // `resume_meeting` already published this via
                            // `update_meeting_state("recording", ...)`.
                            tracing::info!("Meeting event: resumed");
                        }
                        Some(MeetingEvent::Stopped { meeting_id }) => {
                            // `stop_meeting` already published the "idle"
                            // transition via `update_meeting_state`.
                            tracing::info!("Meeting event: stopped {}", meeting_id);
                        }
                        Some(MeetingEvent::Error(msg)) => {
                            tracing::error!("Meeting error: {}", msg);
                            if let Some(ref hub) = self.meeting_event_hub {
                                hub.publish_error(&msg);
                            }
                            self.notify_meeting_error(&msg).await;
                        }
                        None => {
                            // Channel closed
                            tracing::debug!("Meeting event channel closed");
                            self.meeting_event_rx = None;
                        }
                    }
                }

                // The recording announcement's progress
                update = async {
                    match self.meeting_announce_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                }, if self.meeting_announce_rx.is_some() => {
                    match update {
                        Some(update) => self.on_announcement_update(update),
                        None => self.meeting_announce_rx = None,
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

        // Stop any active dictation capture before shutting down and always
        // restore media that this daemon suppressed for the session.
        if let Some(handle) = streaming_handle.take() {
            // Abort rather than await: a backend stuck on a dead socket
            // would otherwise hold shutdown until the service manager kills it.
            let _ = handle.cancel.send(());
            handle.task.abort();
        }
        self.cut_streaming_audio();
        if let Some(mut capture) = audio_capture.take() {
            let _ = capture.stop().await;
        }
        self.restore_recording_media();
        notification::close_persistent().await;

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
        cleanup_profile_override();

        // Remove state file on shutdown
        if let Some(ref path) = self.state_file_path {
            cleanup_state_file(path);
        }

        // Remove meeting state file on shutdown
        if let Some(ref path) = self.meeting_state_file_path {
            cleanup_state_file(path);
        }

        crate::setup::accel::clear();

        // Remove PID file on shutdown
        if let Some(ref path) = self.pid_file_path {
            cleanup_pid_file(path);
        }

        // Remove the OSD audio level socket so a stale path doesn't
        // confuse the next daemon start.
        if let Some(ref hub) = self.level_hub {
            hub.cleanup();
        }

        // Same for the meeting event socket.
        if let Some(ref hub) = self.meeting_event_hub {
            hub.cleanup();
        }

        #[cfg(target_os = "linux")]
        if let Some(tray) = self.tray.take() {
            tray.shutdown().await;
        }

        tracing::info!("Daemon stopped");

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

/// Stops a background task when the daemon's run ends.
struct AbortOnDrop(Option<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    mod history_contracts {
        include!("daemon/history_contract_tests.rs");
    }

    #[cfg(all(target_os = "linux", feature = "cohere-gguf"))]
    #[tokio::test]
    async fn tray_language_next_backend_receives_hint_resident_and_on_demand() {
        if std::env::var_os("VOXTYPE_LANGUAGE_BACKEND_CHILD").is_none() {
            let fixture = tempfile::tempdir().unwrap();
            let worker = fixture.path().join("worker.py");
            let python = std::process::Command::new("python3")
                .args(["-c", "import sys; print(sys.executable)"])
                .output()
                .expect("python3 is required for the isolated protocol fixture");
            assert!(python.status.success());
            crate::test_support::write_script(
                &worker,
                &format!(
                    "#!{}\n{}",
                    String::from_utf8(python.stdout).unwrap().trim(),
                    r#"import sys,json,struct
config=json.loads(sys.argv[sys.argv.index('--config-json')+1])
print('READY',flush=True)
while True:
    count=sys.stdin.buffer.read(4)
    if not count: break
    size=struct.unpack('<I',count)[0]*4
    data=b''
    while len(data)<size: data+=sys.stdin.buffer.read(size-len(data))
    print(json.dumps({'ok':True,'text':'CONFIG_HINT:'+config['language']}),flush=True)
"#
                ),
            );

            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "daemon::tests::tray_language_next_backend_receives_hint_resident_and_on_demand", "--nocapture"])
                .env("VOXTYPE_LANGUAGE_BACKEND_CHILD", "1")
                .env("VOXTYPE_GGUF_WORKER", &worker).status().unwrap();
            assert!(status.success());
            return;
        }
        for on_demand in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let model = dir.path().join("cohere-transcribe-03-2026-Q4_K_M.gguf");
            std::fs::write(&model, b"protocol fixture, not a model").unwrap();
            let mut cfg = crate::config::Config::default();
            cfg.set_model_for(
                crate::config::TranscriptionEngine::Cohere,
                model.to_str().unwrap(),
            );
            cfg.cohere.as_mut().unwrap().on_demand_loading = on_demand;
            cfg.audio.feedback.enabled = false;
            let path = dir.path().join("config.toml");
            std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
            let mut daemon = super::Daemon::new(cfg, Some(path));
            daemon.prepare_for_recording(None);
            let old = daemon.get_transcriber_for_recording(None).await.unwrap();
            assert_eq!(old.transcribe(&[0.0; 16]).unwrap(), "CONFIG_HINT:en");
            daemon.queue_language("fr").unwrap();
            assert!(!daemon.apply_pending_language(false));
            assert_eq!(old.transcribe(&[0.0; 16]).unwrap(), "CONFIG_HINT:en");
            assert!(daemon.apply_pending_language(true));
            daemon.prepare_for_recording(None);
            let next = daemon.get_transcriber_for_recording(None).await.unwrap();
            assert_eq!(next.transcribe(&[0.0; 16]).unwrap(), "CONFIG_HINT:fr");
            assert_eq!(old.transcribe(&[0.0; 16]).unwrap(), "CONFIG_HINT:en");
        }
        println!("BACKEND_HINT_PASS: real daemon construction + GGUF client/protocol, resident/on-demand, in-flight immutable; fixture worker, not inference");
    }

    #[test]
    fn tray_language_is_deferred_and_rebuilds_cached_runtime_only_at_idle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[whisper]\nmodel = \"base\"\nlanguage = \"en\"\n").unwrap();
        let config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let mut daemon = super::Daemon::new(config, Some(path));
        daemon.queue_language("fr").unwrap();
        assert_eq!(daemon.config.active_language(), Some("en"));
        assert!(!daemon.apply_pending_language(false));
        assert_eq!(daemon.config.active_language(), Some("en"));
        assert!(daemon.apply_pending_language(true));
        assert_eq!(daemon.config.active_language(), Some("fr"));
        assert!(daemon.transcriber_preloaded.is_none());
        assert!(daemon.override_cache.is_none());
        assert!(daemon.model_manager.is_some());
        assert!(!daemon.apply_pending_language(true));
    }

    use super::*;
    use crate::config::{TranscriptionEngine, WhisperMode};
    use crate::transcribe::StreamingTranscriber;
    use std::fs;
    use tempfile::TempDir;

    /// Fallback output that records what it was handed.
    struct RecordingOutput {
        got: Arc<std::sync::Mutex<Option<String>>>,
        succeed: bool,
    }

    #[async_trait::async_trait]
    impl TextOutput for RecordingOutput {
        async fn output(&self, text: &str) -> std::result::Result<(), crate::error::OutputError> {
            *self.got.lock().unwrap() = Some(text.to_string());
            if self.succeed {
                Ok(())
            } else {
                Err(crate::error::OutputError::AllMethodsFailed)
            }
        }
        async fn is_available(&self) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "recording"
        }
    }

    /// The dictation path end to end (text cleanup, post-processing, file
    /// output), with what it logs captured as the daemon's formatter writes
    /// it. At info, the level the daemon and its systemd unit run at, the
    /// log says a dictation happened and how long it was, and carries none
    /// of its words at any stage. At debug the words are there, which is
    /// how a recognition problem gets reported.
    ///
    /// Runs in a process of its own so the daemon's runtime directory
    /// (state file, per-recording override files) is a scratch one, not
    /// the session's, and so the log capture is the only subscriber.
    #[tokio::test]
    async fn dictated_text_stays_out_of_the_log_at_the_default_level() {
        let scratch = TempDir::new().unwrap();
        if !crate::test_support::own_process(
            "daemon::tests::dictated_text_stays_out_of_the_log_at_the_default_level",
            |child| {
                child
                    .env("HOME", scratch.path())
                    .env("XDG_RUNTIME_DIR", scratch.path().join("runtime"))
                    .env("XDG_STATE_HOME", scratch.path().join("state"))
                    .env("XDG_CONFIG_HOME", scratch.path().join("config"))
                    .env_remove("DBUS_SESSION_BUS_ADDRESS")
                    .env_remove("WAYLAND_DISPLAY")
                    .env_remove("DISPLAY");
            },
        ) {
            return;
        }

        let spoken = "meet me at the quarry at nine";
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("dictation.txt");
        let mut config = Config::default();
        config.audio.feedback.enabled = false;
        config
            .text
            .replacements
            .insert("quarry".to_string(), "harbour".to_string());
        config.output.mode = OutputMode::File;
        config.output.file_path = Some(target.clone());
        config.output.fallback_to_clipboard = false;
        config.output.post_process = Some(crate::config::PostProcessConfig {
            command: "tr '[:lower:]' '[:upper:]'".to_string(),
            timeout_ms: 5000,
            trim: true,
            fallback_on_empty: true,
            edit_list: false,
            ..Default::default()
        });

        for level in [tracing::Level::INFO, tracing::Level::DEBUG] {
            let _ = fs::remove_file(&target);
            let capture = crate::test_support::LogCapture::at(level);
            let mut daemon = Daemon::new(config.clone(), None);
            let mut state = State::Transcribing { audio: Vec::new() };
            daemon
                .handle_transcription_result(&mut state, Ok(Ok(spoken.to_string())))
                .await;
            let logged = capture.text();
            drop(capture);

            assert!(state.is_idle());
            assert_eq!(
                fs::read_to_string(&target).unwrap(),
                "MEET ME AT THE HARBOUR AT NINE\n",
                "the dictation went through cleanup and post-processing"
            );
            assert!(
                logged.contains("Transcribed 29 chars (whisper engine)"),
                "{logged}"
            );
            assert!(logged.contains("Post-processed: changed: true"), "{logged}");

            // The words as recognized, after cleanup and after
            // post-processing.
            let stages = ["at the quarry", "at the harbour", "AT THE HARBOUR"];
            if level == tracing::Level::INFO {
                let lowered = logged.to_lowercase();
                for words in ["meet me", "quarry", "harbour", "at nine"] {
                    assert!(
                        !lowered.contains(words),
                        "dictated text in the log at info: {logged}"
                    );
                }
            } else {
                for words in stages {
                    assert!(logged.contains(words), "{words:?} not at debug: {logged}");
                }
            }
        }
    }

    /// A recording's `--profile` reaches every stage of the dictation, not
    /// only the first one that reads its override file (#1097). Each test
    /// runs in a process of its own, since the profile travels through the
    /// runtime directory, which must be a scratch one.
    mod recording_profile {
        use super::*;
        use crate::config::{PostProcessConfig, Profile, VocabularyConfig};

        fn in_scratch_runtime(test: &str) -> bool {
            let scratch = TempDir::new().unwrap();
            crate::test_support::own_process(
                &format!("daemon::tests::recording_profile::{test}"),
                |child| {
                    child
                        .env("HOME", scratch.path())
                        .env("XDG_RUNTIME_DIR", scratch.path().join("runtime"))
                        .env("XDG_STATE_HOME", scratch.path().join("state"))
                        .env("XDG_CONFIG_HOME", scratch.path().join("config"))
                        .env("XDG_DATA_HOME", scratch.path().join("data"))
                        .env_remove("DBUS_SESSION_BUS_ADDRESS")
                        .env_remove("WAYLAND_DISPLAY")
                        .env_remove("DISPLAY");
                },
            )
        }

        /// Default post-processing tags text DEFAULT:, the `notes` profile
        /// tags it NOTES:, and `calendar` adds a vocabulary to its command.
        fn config(target: &Path) -> Config {
            let mut config = Config::default();
            config.audio.feedback.enabled = false;
            config.whisper.on_demand_loading = false;
            config.output.mode = OutputMode::File;
            config.output.file_path = Some(target.to_path_buf());
            config.output.fallback_to_clipboard = false;
            config.output.post_process = Some(PostProcessConfig {
                command: "sed 's/^/DEFAULT:/'".to_string(),
                timeout_ms: 5000,
                trim: true,
                fallback_on_empty: true,
                edit_list: false,
                ..Default::default()
            });
            config.profiles.insert(
                "notes".to_string(),
                Profile {
                    post_process_command: Some("sed 's/^/NOTES:/'".to_string()),
                    post_process_timeout_ms: Some(5000),
                    ..Profile::default()
                },
            );
            config.profiles.insert(
                "calendar".to_string(),
                Profile {
                    vocabulary: VocabularyConfig {
                        terms: vec!["PipeWire".to_string()],
                        terms_file: None,
                    },
                    post_process_command: Some("sed 's/^/CALENDAR:/'".to_string()),
                    post_process_timeout_ms: Some(5000),
                    ..Profile::default()
                },
            );
            config
        }

        /// What `voxtype record start|stop --profile <name>` does before it
        /// signals the daemon.
        fn cli_names_profile(name: &str) {
            let dir = Config::runtime_dir();
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("profile_override"), name).unwrap();
        }

        /// Transcriber prep, which reads the profile's vocabulary again
        /// once the recording stops.
        fn prepare_transcriber(daemon: &mut Daemon) -> Vec<String> {
            daemon.active_profile_vocabulary_terms()
        }

        async fn deliver(daemon: &mut Daemon, spoken: &str) {
            let mut state = State::Transcribing { audio: Vec::new() };
            daemon
                .handle_transcription_result(&mut state, Ok(Ok(spoken.to_string())))
                .await;
            assert!(state.is_idle());
        }

        fn streaming_state(file_output_path: Option<PathBuf>) -> State {
            State::Streaming {
                started_at: std::time::Instant::now(),
                model_override: None,
                partial_buffer: String::new(),
                finalized_text: String::new(),
                typed_chars: 0,
                file_output_path,
            }
        }

        #[tokio::test]
        async fn batch_post_processing_runs_the_profile_command() {
            if !in_scratch_runtime("batch_post_processing_runs_the_profile_command") {
                return;
            }
            let dir = TempDir::new().unwrap();
            let target = dir.path().join("dictation.txt");
            let mut daemon = Daemon::new(config(&target), None);

            cli_names_profile("notes");
            daemon.prepare_for_recording(None);
            prepare_transcriber(&mut daemon);
            deliver(&mut daemon, "hello there").await;

            assert_eq!(fs::read_to_string(&target).unwrap(), "NOTES:hello there\n");
        }

        #[tokio::test]
        async fn profile_vocabulary_and_command_both_apply() {
            if !in_scratch_runtime("profile_vocabulary_and_command_both_apply") {
                return;
            }
            let dir = TempDir::new().unwrap();
            let target = dir.path().join("dictation.txt");
            let mut daemon = Daemon::new(config(&target), None);

            cli_names_profile("calendar");
            daemon.prepare_for_recording(None);
            assert_eq!(prepare_transcriber(&mut daemon), vec!["PipeWire"]);
            deliver(&mut daemon, "pipe wire works").await;

            assert_eq!(
                fs::read_to_string(&target).unwrap(),
                "CALENDAR:PipeWire works\n"
            );
        }

        #[tokio::test]
        async fn profile_output_mode_reaches_output() {
            if !in_scratch_runtime("profile_output_mode_reaches_output") {
                return;
            }
            let dir = TempDir::new().unwrap();
            let target = dir.path().join("dictation.txt");
            let mut config = config(&target);
            config.output.mode = OutputMode::Clipboard;
            config.profiles.insert(
                "to-file".to_string(),
                Profile {
                    output_mode: Some(OutputMode::File),
                    ..Profile::default()
                },
            );
            let mut daemon = Daemon::new(config, None);
            let typed = Arc::new(std::sync::Mutex::new(None));
            daemon.history_test_output_chain = Some(vec![Box::new(RecordingOutput {
                got: typed.clone(),
                succeed: true,
            })]);

            cli_names_profile("to-file");
            daemon.prepare_for_recording(None);
            prepare_transcriber(&mut daemon);
            deliver(&mut daemon, "hello there").await;

            assert_eq!(
                fs::read_to_string(&target).unwrap(),
                "DEFAULT:hello there\n"
            );
            assert_eq!(*typed.lock().unwrap(), None, "the default output ran");
        }

        #[tokio::test]
        async fn profile_named_only_at_stop_still_applies() {
            if !in_scratch_runtime("profile_named_only_at_stop_still_applies") {
                return;
            }
            let dir = TempDir::new().unwrap();
            let target = dir.path().join("dictation.txt");
            let mut daemon = Daemon::new(config(&target), None);

            daemon.prepare_for_recording(None);
            cli_names_profile("notes");
            prepare_transcriber(&mut daemon);
            deliver(&mut daemon, "hello there").await;

            assert_eq!(fs::read_to_string(&target).unwrap(), "NOTES:hello there\n");
        }

        #[tokio::test]
        async fn streamed_segments_keep_the_profile() {
            if !in_scratch_runtime("streamed_segments_keep_the_profile") {
                return;
            }
            let dir = TempDir::new().unwrap();
            let mut daemon = Daemon::new(config(&dir.path().join("unused.txt")), None);
            let typed = Arc::new(std::sync::Mutex::new(None));
            let chain: Vec<Box<dyn TextOutput>> = vec![Box::new(RecordingOutput {
                got: typed.clone(),
                succeed: true,
            })];

            cli_names_profile("calendar");
            daemon.prepare_for_recording(None);
            let mut state = streaming_state(None);
            let mut session = StreamingSession::new();
            // The event pump looks the vocabulary up for each final segment.
            for segment in ["pipe wire starts", "pipe wire stops"] {
                let profile = daemon.recording_profile();
                let vocabulary = profile
                    .as_ref()
                    .and_then(|name| daemon.profile_vocabularies.get(name));
                session
                    .commit_segment(
                        &chain,
                        segment,
                        Some((&daemon.text_processor, vocabulary)),
                        None,
                        None,
                    )
                    .await
                    .unwrap();
                let delivered = typed.lock().unwrap().clone().unwrap();
                assert!(
                    delivered.contains("PipeWire"),
                    "{segment:?} -> {delivered:?}"
                );
            }

            daemon
                .end_streaming(
                    &mut state,
                    &mut None,
                    &mut None,
                    &mut Some(session),
                    &mut Some(chain),
                    StreamingEndReason::Completed,
                )
                .await;
            assert!(state.is_idle());
            assert_eq!(daemon.recording_profile(), None, "outlived its dictation");
        }

        #[tokio::test]
        async fn streamed_file_output_keeps_the_profile_through_end_streaming() {
            if !in_scratch_runtime("streamed_file_output_keeps_the_profile_through_end_streaming") {
                return;
            }
            let dir = TempDir::new().unwrap();
            let target = dir.path().join("dictation.txt");
            let mut daemon = Daemon::new(config(&target), None);

            cli_names_profile("calendar");
            daemon.prepare_for_recording(None);
            let mut state = streaming_state(Some(target.clone()));
            let mut session = StreamingSession::new();
            session.commit_segment_silent("pipe wire works");
            daemon
                .end_streaming(
                    &mut state,
                    &mut None,
                    &mut None,
                    &mut Some(session),
                    &mut None,
                    StreamingEndReason::Completed,
                )
                .await;

            assert!(state.is_idle());
            assert_eq!(fs::read_to_string(&target).unwrap(), "PipeWire works\n");
            assert_eq!(daemon.recording_profile(), None, "outlived its dictation");
        }

        #[tokio::test]
        async fn a_failed_or_cancelled_dictation_leaves_no_profile_behind() {
            if !in_scratch_runtime("a_failed_or_cancelled_dictation_leaves_no_profile_behind") {
                return;
            }
            let dir = TempDir::new().unwrap();
            let target = dir.path().join("dictation.txt");
            let mut daemon = Daemon::new(config(&target), None);

            // Transcription fails.
            cli_names_profile("notes");
            daemon.prepare_for_recording(None);
            prepare_transcriber(&mut daemon);
            let mut state = State::Transcribing { audio: Vec::new() };
            daemon
                .handle_transcription_result(
                    &mut state,
                    Ok(Err(crate::error::TranscribeError::InferenceFailed(
                        "synthetic".to_string(),
                    ))),
                )
                .await;
            assert!(state.is_idle());
            assert_eq!(daemon.recording_profile, None, "survived a failure");

            // The recording is cancelled.
            cli_names_profile("notes");
            daemon.prepare_for_recording(None);
            let mut state = State::Recording {
                started_at: std::time::Instant::now(),
                model_override: None,
            };
            daemon
                .cancel_requested_session(
                    &mut state, &mut None, &mut None, &mut None, &mut None, &mut None,
                )
                .await;
            assert!(state.is_idle());
            assert_eq!(daemon.recording_profile, None, "survived a cancel");

            // The next dictation names no profile and gets the defaults.
            daemon.prepare_for_recording(None);
            prepare_transcriber(&mut daemon);
            deliver(&mut daemon, "hello there").await;
            assert_eq!(
                fs::read_to_string(&target).unwrap(),
                "DEFAULT:hello there\n"
            );
        }
    }

    /// Output config whose hooks log to `log`, noting whether `target`
    /// existed when each ran, so tests can see order relative to the write.
    fn hooked_output_config(log: &Path, target: &Path) -> crate::config::OutputConfig {
        let hook = |name: &str| {
            format!(
                "if [ -e '{t}' ]; then echo {name}:file >> '{l}'; else echo {name}:nofile >> '{l}'; fi",
                t = target.display(),
                l = log.display()
            )
        };
        crate::config::OutputConfig {
            pre_output_command: Some(hook("pre")),
            post_output_command: Some(hook("post")),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn file_output_runs_the_hooks_around_the_write() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("out.txt");
        let log = dir.path().join("hooks.log");
        let cfg = hooked_output_config(&log, &target);

        let delivery = deliver_to_file(&target, "hello", &cfg, &[]).await;

        assert_eq!(delivery, FileDelivery::Written);
        assert_eq!(fs::read_to_string(&target).unwrap(), "hello\n");
        // pre ran before the file existed, post after it was written.
        assert_eq!(fs::read_to_string(&log).unwrap(), "pre:nofile\npost:file\n");
    }

    #[tokio::test]
    async fn failed_file_write_falls_back_and_still_resets_the_submap() {
        let dir = TempDir::new().unwrap();
        // A parent that is a regular file fails even for root, unlike a
        // read-only directory.
        let blocker = dir.path().join("not-a-dir");
        fs::write(&blocker, "").unwrap();
        let target = blocker.join("out.txt");
        let log = dir.path().join("hooks.log");
        let cfg = hooked_output_config(&log, &target);
        let got = Arc::new(std::sync::Mutex::new(None));
        let fallback: Vec<Box<dyn TextOutput>> = vec![Box::new(RecordingOutput {
            got: got.clone(),
            succeed: true,
        })];

        let delivery = deliver_to_file(&target, "rescue me", &cfg, &fallback).await;

        assert!(
            matches!(delivery, FileDelivery::FellBack(_)),
            "{delivery:?}"
        );
        assert_eq!(got.lock().unwrap().as_deref(), Some("rescue me"));
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            "pre:nofile\npost:nofile\n"
        );
    }

    #[tokio::test]
    async fn failed_file_write_without_fallback_reports_the_loss() {
        let dir = TempDir::new().unwrap();
        let blocker = dir.path().join("not-a-dir");
        fs::write(&blocker, "").unwrap();
        let target = blocker.join("out.txt");
        let log = dir.path().join("hooks.log");
        let cfg = hooked_output_config(&log, &target);

        let delivery = deliver_to_file(&target, "gone", &cfg, &[]).await;

        assert!(matches!(delivery, FileDelivery::Lost(_)), "{delivery:?}");
        // The post hook runs even when nothing was delivered.
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            "pre:nofile\npost:nofile\n"
        );
    }

    #[tokio::test]
    async fn failed_file_write_with_failing_fallback_reports_the_loss() {
        let dir = TempDir::new().unwrap();
        let blocker = dir.path().join("not-a-dir");
        fs::write(&blocker, "").unwrap();
        let target = blocker.join("out.txt");
        let cfg = crate::config::OutputConfig::default();
        let got = Arc::new(std::sync::Mutex::new(None));
        let fallback: Vec<Box<dyn TextOutput>> = vec![Box::new(RecordingOutput {
            got: got.clone(),
            succeed: false,
        })];

        let delivery = deliver_to_file(&target, "gone", &cfg, &fallback).await;

        assert!(matches!(delivery, FileDelivery::Lost(_)), "{delivery:?}");
        assert_eq!(got.lock().unwrap().as_deref(), Some("gone"));
    }

    /// A cancel issued before a start/stop signal is applied first when the
    /// daemon has a live session, so `record cancel; record start` records
    /// again instead of the start landing on the session being cancelled.
    fn history_daemon(dir: &Path) -> Daemon {
        let mut config = Config::default();
        config.audio.feedback.enabled = false;
        config.state_file = None;
        config.history.enabled = true;
        let mut daemon = Daemon::new(config.clone(), None);
        daemon.history = Arc::new(std::sync::Mutex::new(
            crate::history::recorder::Recorder::with_paths(
                config.history.clone(),
                dir.join("history"),
                dir.join("meetings").join("index.db"),
            ),
        ));
        daemon
    }

    async fn wait_for_entries(dir: &Path, n: usize) -> Vec<crate::history::HistoryEntry> {
        let store = crate::history::HistoryStore::at(dir.join("history"));
        for _ in 0..200 {
            let all = store.all().unwrap();
            if all.len() >= n {
                return all;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("history entries never appeared");
    }

    #[tokio::test]
    async fn every_dictation_outcome_lands_in_history_once_idle() {
        use crate::history::{EndedBy, Outcome};
        let t = tempfile::tempdir().unwrap();
        let mut daemon = history_daemon(t.path());
        let recording = State::Recording {
            started_at: std::time::Instant::now(),
            model_override: None,
        };

        // Delivered.
        daemon.begin_osd_session(&recording);
        daemon.history_note(|p| p.texts("um hello there", "Hello there."));
        daemon.publish_osd_text_outcome("Hello there.");
        daemon.update_state("idle");
        // Cancelled.
        daemon.begin_osd_session(&recording);
        daemon.publish_osd_outcome(osd::session::OutcomeKind::Cancelled, 0);
        daemon.update_state("idle");
        // Cut off by max_duration_secs, then no speech.
        daemon.begin_osd_session(&recording);
        daemon.osd_limit_reached = true;
        daemon.publish_osd_outcome(osd::session::OutcomeKind::NoSpeech, 0);
        daemon.update_state("idle");

        let entries = wait_for_entries(t.path(), 3).await;
        let mut by_outcome: Vec<_> = entries.iter().map(|e| (e.outcome, e.ended_by)).collect();
        by_outcome.sort_by_key(|(o, _)| o.as_str());
        assert_eq!(
            by_outcome,
            vec![
                (Outcome::Cancelled, EndedBy::Cancelled),
                (Outcome::Delivered, EndedBy::Released),
                (Outcome::NoSpeech, EndedBy::Timeout),
            ]
        );
        let delivered = entries
            .iter()
            .find(|e| e.outcome == Outcome::Delivered)
            .unwrap();
        assert_eq!(delivered.raw_text, "um hello there");
        assert_eq!(delivered.text, "Hello there.");
        assert!(!delivered.edits.is_empty());
        assert_eq!(delivered.trigger, crate::history::Trigger::Hotkey);
        assert!(!delivered.engine.is_empty());

        // Idle again with nothing in flight writes nothing more.
        daemon.update_state("idle");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            crate::history::HistoryStore::at(t.path().join("history"))
                .all()
                .unwrap()
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn history_keeps_actual_transcriber_metadata_after_an_inference_error() {
        struct ReportedMetadata;
        impl Transcriber for ReportedMetadata {
            fn transcribe(
                &self,
                _: &[f32],
            ) -> std::result::Result<String, crate::error::TranscribeError> {
                panic!("The completion fixture must not perform inference")
            }
            fn runtime_metadata(&self) -> crate::transcribe::RuntimeMetadata {
                crate::transcribe::RuntimeMetadata {
                    model: Some("actual-fixture.gguf".into()),
                    backend: Some("fixture-cpu".into()),
                    model_load_ms: Some(17),
                }
            }
        }
        let fixture = tempfile::tempdir().unwrap();
        let mut daemon = history_daemon(fixture.path());
        daemon.begin_osd_session(&State::Recording {
            started_at: std::time::Instant::now(),
            model_override: None,
        });
        daemon.active_transcriber = Some(Arc::new(ReportedMetadata));
        let mut state = State::Transcribing { audio: Vec::new() };
        daemon
            .handle_transcription_result(
                &mut state,
                Ok(Err(crate::error::TranscribeError::InferenceFailed(
                    "synthetic inference failure".into(),
                ))),
            )
            .await;
        let entries = wait_for_entries(fixture.path(), 1).await;
        assert_eq!(entries[0].model, "actual-fixture.gguf");
        assert_eq!(entries[0].backend.as_deref(), Some("fixture-cpu"));
        assert_eq!(entries[0].model_load_ms, Some(17));
        assert_eq!(
            entries[0].outcome,
            crate::history::Outcome::TranscriptionFailed
        );
        assert!(!entries[0].delivered);
    }

    #[tokio::test]
    async fn history_cancelled_before_inference_does_not_invent_a_backend() {
        let fixture = tempfile::tempdir().unwrap();
        let mut daemon = history_daemon(fixture.path());
        daemon.begin_osd_session(&State::Recording {
            started_at: std::time::Instant::now(),
            model_override: None,
        });
        daemon.publish_osd_outcome(osd::session::OutcomeKind::Cancelled, 0);
        daemon.update_state("idle");
        let entries = wait_for_entries(fixture.path(), 1).await;
        assert!(
            entries[0].backend.is_none(),
            "A binary capability is not evidence that an inference backend ran"
        );
        assert!(entries[0].model_load_ms.is_none());
        assert_eq!(entries[0].outcome, crate::history::Outcome::Cancelled);
    }

    #[tokio::test]
    async fn history_keeps_configured_language_for_a_cancelled_dictation() {
        let t = tempfile::tempdir().unwrap();
        let mut daemon = history_daemon(t.path());
        daemon.config.engine = crate::config::TranscriptionEngine::Whisper;
        daemon.config.whisper.language = crate::config::LanguageConfig::Single("de".into());
        daemon.begin_osd_session(&State::Recording {
            started_at: std::time::Instant::now(),
            model_override: None,
        });
        daemon.publish_osd_outcome(osd::session::OutcomeKind::Cancelled, 0);
        daemon.update_state("idle");
        let entries = wait_for_entries(t.path(), 1).await;
        assert_eq!(entries[0].language.as_deref(), Some("de"));
    }

    #[tokio::test]
    async fn history_keeps_configured_language_after_file_delivery() {
        const PROBE: &str = "configured-language-file-delivery";
        if std::env::var("VOXTYPE_HISTORY_METADATA_PROBE").as_deref() != Ok(PROBE) {
            // Run the actual completion path with a private runtime and no
            // inherited desktop, config, signal sentinels or output helpers.
            let fixture = tempfile::tempdir().unwrap();
            let runtime = fixture.path().join("runtime");
            std::fs::create_dir(&runtime).unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "daemon::tests::history_keeps_configured_language_after_file_delivery",
                    "--nocapture",
                ])
                .env_clear()
                .env("HOME", fixture.path())
                .env("XDG_CONFIG_HOME", fixture.path().join("config"))
                .env("XDG_DATA_HOME", fixture.path().join("data"))
                .env("XDG_RUNTIME_DIR", runtime)
                .env("TMPDIR", fixture.path())
                .env("RUST_MIN_STACK", "16777216")
                .env("VOXTYPE_HISTORY_METADATA_PROBE", PROBE)
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let fixture = tempfile::tempdir().unwrap();
        let mut daemon = history_daemon(fixture.path());
        daemon.config.engine = crate::config::TranscriptionEngine::Whisper;
        daemon.config.whisper.language = crate::config::LanguageConfig::Single("de".into());
        daemon.config.output.mode = crate::config::OutputMode::File;
        daemon.config.output.file_path = Some(fixture.path().join("transcript.txt"));
        daemon.config.output.fallback_to_clipboard = false;
        daemon.config.output.notification.on_transcription = false;
        daemon.begin_osd_session(&State::Recording {
            started_at: std::time::Instant::now(),
            model_override: None,
        });
        let mut state = State::Transcribing { audio: Vec::new() };
        daemon
            .handle_transcription_result(&mut state, Ok(Ok("synthetic invoice".into())))
            .await;
        let entries = wait_for_entries(fixture.path(), 1).await;
        assert_eq!(entries[0].language.as_deref(), Some("de"));
        assert_eq!(entries[0].raw_text, "synthetic invoice");
        assert_eq!(entries[0].output.as_ref().unwrap().mode, "file");
        assert!(entries[0].delivered);
    }

    #[tokio::test]
    async fn history_records_the_text_as_it_was_delivered() {
        const PROBE: &str = "delivered-text";
        if std::env::var("VOXTYPE_HISTORY_METADATA_PROBE").as_deref() != Ok(PROBE) {
            // The real completion path, with a private runtime and no
            // inherited desktop, config or output helpers.
            let fixture = tempfile::tempdir().unwrap();
            let runtime = fixture.path().join("runtime");
            std::fs::create_dir(&runtime).unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "daemon::tests::history_records_the_text_as_it_was_delivered",
                    "--nocapture",
                ])
                .env_clear()
                .env("HOME", fixture.path())
                .env("XDG_CONFIG_HOME", fixture.path().join("config"))
                .env("XDG_DATA_HOME", fixture.path().join("data"))
                .env("XDG_RUNTIME_DIR", runtime)
                .env("TMPDIR", fixture.path())
                .env("RUST_MIN_STACK", "16777216")
                .env("VOXTYPE_HISTORY_METADATA_PROBE", PROBE)
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        // Curly quotes and an apostrophe, which the output chain straightens
        // for the keyboard tools, and a dash it leaves alone.
        const HEARD: &str = "He said \u{201C}hello\u{201D} \u{2014} it\u{2019}s fine.";
        const TYPED: &str = "He said \"hello\" \u{2014} it's fine.";

        let fixture = tempfile::tempdir().unwrap();
        let mut daemon = history_daemon(fixture.path());
        daemon.config.engine = crate::config::TranscriptionEngine::Whisper;
        daemon.config.output.wait_for_modifier_release = false;
        daemon.config.output.notification.on_transcription = false;
        daemon.config.text.spoken_punctuation = false;
        let got = Arc::new(std::sync::Mutex::new(None));
        daemon.history_test_output_chain = Some(vec![Box::new(RecordingOutput {
            got: got.clone(),
            succeed: true,
        })]);
        daemon.begin_osd_session(&State::Recording {
            started_at: std::time::Instant::now(),
            model_override: None,
        });
        let mut state = State::Transcribing { audio: Vec::new() };
        daemon
            .handle_transcription_result(&mut state, Ok(Ok(HEARD.into())))
            .await;

        let typed = got.lock().unwrap().clone().expect("nothing was delivered");
        assert_eq!(
            typed, TYPED,
            "the fixture's expectation of the output chain"
        );
        let entries = wait_for_entries(fixture.path(), 1).await;
        let entry = &entries[0];
        assert!(entry.delivered);
        assert_eq!(entry.text, typed, "history differs from what was typed");
        assert_eq!(entry.best_text(), typed, "Copy would not reproduce it");
        assert_eq!(entry.raw_text, HEARD, "the engine's wording must be kept");
        assert!(
            !entry.edits.is_empty(),
            "the change from heard to typed is an automatic edit"
        );
    }

    /// History is where the user chose to keep dictated words; the log is
    /// not. A dictation recorded to history is in the day file and, at info
    /// (the level the daemon and its systemd unit run at), nowhere in the
    /// log, and a history write that fails says so without quoting the
    /// dictation it lost. The rest of the history code is covered the same
    /// way by `tests/history_privacy.rs`.
    ///
    /// Runs in a process of its own so the capture is the process's
    /// subscriber: the recorder writes, and warns, on a blocking thread.
    #[tokio::test]
    async fn history_keeps_the_words_and_the_log_at_info_does_not() {
        let scratch = TempDir::new().unwrap();
        if !crate::test_support::own_process(
            "daemon::tests::history_keeps_the_words_and_the_log_at_info_does_not",
            |child| {
                child
                    .env("HOME", scratch.path())
                    .env("XDG_RUNTIME_DIR", scratch.path().join("runtime"))
                    .env("XDG_STATE_HOME", scratch.path().join("state"))
                    .env("XDG_CONFIG_HOME", scratch.path().join("config"))
                    .env("XDG_DATA_HOME", scratch.path().join("data"))
                    .env("RUST_MIN_STACK", "16777216")
                    .env_remove("DBUS_SESSION_BUS_ADDRESS")
                    .env_remove("WAYLAND_DISPLAY")
                    .env_remove("DISPLAY");
            },
        ) {
            return;
        }
        let capture = crate::test_support::LogCapture::for_process(tracing::Level::INFO);

        const KEPT: &str = "meet me at the quarry at nine";
        const LOST: &str = "the harbour master has the invoice";
        let fixture = tempfile::tempdir().unwrap();
        let history_dir = fixture.path().join("history");
        let mut daemon = history_daemon(fixture.path());
        daemon.config.engine = crate::config::TranscriptionEngine::Whisper;
        daemon.config.output.wait_for_modifier_release = false;
        daemon.config.output.notification.on_transcription = false;
        let (finished, receiver) = std::sync::mpsc::channel();
        daemon.history.lock().unwrap().test_finished = Some(finished);
        let receiver = Arc::new(std::sync::Mutex::new(receiver));

        for spoken in [KEPT, LOST] {
            if spoken == LOST {
                // The store refuses a symlinked directory, so this write
                // fails and the recorder has to say so.
                let moved = fixture.path().join("history-moved");
                std::fs::rename(&history_dir, &moved).unwrap();
                std::os::unix::fs::symlink(&moved, &history_dir).unwrap();
            }
            let got = Arc::new(std::sync::Mutex::new(None));
            daemon.history_test_output_chain = Some(vec![Box::new(RecordingOutput {
                got: got.clone(),
                succeed: true,
            })]);
            daemon.begin_osd_session(&State::Recording {
                started_at: std::time::Instant::now(),
                model_override: None,
            });
            let mut state = State::Transcribing { audio: Vec::new() };
            daemon
                .handle_transcription_result(&mut state, Ok(Ok(spoken.to_string())))
                .await;
            assert!(
                got.lock().unwrap().is_some(),
                "{spoken:?} was not delivered"
            );
            let receiver = receiver.clone();
            tokio::task::spawn_blocking(move || {
                receiver
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .expect("the history write never finished")
            })
            .await
            .unwrap();
        }

        let kept = crate::history::HistoryStore::at(fixture.path().join("history-moved"))
            .all()
            .unwrap();
        assert_eq!(kept.len(), 1, "one dictation recorded, one write refused");
        assert_eq!(kept[0].raw_text, KEPT, "history keeps the words");

        let logged = capture.text();
        for line in [
            "Transcribed 29 chars (whisper engine)",
            "Transcribed 34 chars (whisper engine)",
            "Could not record dictation history",
        ] {
            assert!(logged.contains(line), "{line:?} not logged:\n{logged}");
        }
        let lowered = logged.to_lowercase();
        for words in ["meet me", "quarry", "at nine", "harbour", "invoice"] {
            assert!(
                !lowered.contains(words),
                "dictated text ({words:?}) in the log at info:\n{logged}"
            );
        }
    }

    #[tokio::test]
    async fn history_delivered_text_is_untouched_when_delivery_changes_nothing() {
        let mut pending = crate::history::recorder::Recorder::with_paths(
            crate::config::HistoryConfig {
                enabled: true,
                ..Default::default()
            },
            std::env::temp_dir().join("voxtype-unused-history"),
            std::env::temp_dir().join("voxtype-unused-index.db"),
        )
        .begin()
        .expect("history is on");
        pending.texts("um plain text", "Plain text.");
        let edits = pending.entry.edits.clone();
        pending.delivered_text("Plain text.");
        assert_eq!(pending.entry.text, "Plain text.");
        assert_eq!(pending.entry.edits, edits);
        pending.delivered_text("Plain 'text'.");
        assert_eq!(pending.entry.text, "Plain 'text'.");
        assert_eq!(pending.entry.raw_text, "um plain text");
    }

    #[tokio::test]
    async fn history_off_records_nothing() {
        let t = tempfile::tempdir().unwrap();
        let mut daemon = history_daemon(t.path());
        daemon.history = Arc::new(std::sync::Mutex::new(
            crate::history::recorder::Recorder::with_paths(
                crate::config::HistoryConfig::default(),
                t.path().join("history"),
                t.path().join("index.db"),
            ),
        ));
        daemon.begin_osd_session(&State::Recording {
            started_at: std::time::Instant::now(),
            model_override: None,
        });
        daemon.publish_osd_text_outcome("secret");
        daemon.update_state("idle");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!t.path().join("history").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn history_excludes_delivery_destination_before_post_hook_restores_focus() {
        const NAME: &str =
            "daemon::tests::history_excludes_delivery_destination_before_post_hook_restores_focus";
        if std::env::var_os("VOXTYPE_HISTORY_FOCUS_CHILD").is_none() {
            for mode in ["batch", "streaming", "file-fallback", "allowed", "unknown"] {
                let root = tempfile::tempdir().unwrap();
                let bin = root.path().join("bin");
                let runtime = root.path().join("runtime");
                std::fs::create_dir(&bin).unwrap();
                std::fs::create_dir(&runtime).unwrap();
                for name in ["sh", "cat", "which"] {
                    std::os::unix::fs::symlink(which::which(name).unwrap(), bin.join(name))
                        .unwrap();
                }
                let shell = which::which("sh").unwrap();
                // The clipboard tool: wl-copy on Linux, pbcopy on macOS.
                for name in ["wl-copy", "pbcopy"] {
                    crate::test_support::write_script(&bin.join(name), &format!(
                    "#!{}\ncat \"$HOME/focus\" > \"$HOME/delivered-app\"\ncat > \"$HOME/captured-text\"\n", shell.display()));
                }
                let result = tokio::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", NAME, "--nocapture"])
                    .env_clear()
                    .env("HOME", root.path())
                    .env("TMPDIR", root.path())
                    .env("PATH", &bin)
                    .env("XDG_RUNTIME_DIR", &runtime)
                    .env("XDG_CONFIG_HOME", root.path().join("config"))
                    .env("XDG_DATA_HOME", root.path().join("data"))
                    .env("NIRI_SOCKET", runtime.join("synthetic-focus.sock"))
                    .env("WAYLAND_DISPLAY", "synthetic-no-display-socket")
                    .env("RUST_MIN_STACK", "16777216")
                    .env("VOXTYPE_HISTORY_FOCUS_CHILD", "1")
                    .env("VOXTYPE_HISTORY_FOCUS_MODE", mode)
                    .kill_on_drop(true)
                    .output()
                    .await
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{mode}: {}{}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
            }
            return;
        }
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let root = PathBuf::from(std::env::var_os("HOME").unwrap());
        let focus = root.join("focus");
        std::fs::write(&focus, "synthetic-editor").unwrap();
        let listener =
            tokio::net::UnixListener::bind(std::env::var_os("NIRI_SOCKET").unwrap()).unwrap();
        let start_observed = Arc::new(tokio::sync::Notify::new());
        let signal = start_observed.clone();
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let mut stream = tokio::io::BufReader::new(stream);
                let mut request = String::new();
                stream.read_line(&mut request).await.unwrap();
                assert_eq!(request, "\"FocusedWindow\"\n");
                let app = std::fs::read_to_string(&focus).unwrap();
                let reply = serde_json::json!({"Ok":{"FocusedWindow":{"app_id":app,"title":"Synthetic fixture"}}});
                stream
                    .get_mut()
                    .write_all(format!("{reply}\n").as_bytes())
                    .await
                    .unwrap();
                signal.notify_one();
            }
        });
        let mut daemon = history_daemon(&root);
        daemon.config.output.mode = crate::config::OutputMode::Clipboard;
        daemon.config.output.append_text = None;
        daemon.config.output.wait_for_modifier_release = false;
        daemon.config.output.notification.on_transcription = false;
        let mode = std::env::var("VOXTYPE_HISTORY_FOCUS_MODE").unwrap();
        let destination = match mode.as_str() {
            "allowed" => "synthetic-destination",
            "unknown" => "",
            _ => "Bitwarden",
        };
        daemon.config.output.pre_output_command =
            Some(format!("printf '%s' '{destination}' > \"$HOME/focus\""));
        daemon.config.output.post_output_command =
            Some("printf synthetic-editor > \"$HOME/focus\"".into());
        if mode == "file-fallback" {
            std::fs::write(root.join("blocked-parent"), "synthetic non-directory").unwrap();
            daemon.config.output.mode = crate::config::OutputMode::File;
            daemon.config.output.file_path = Some(root.join("blocked-parent/transcript.txt"));
            daemon.config.output.fallback_to_clipboard = true;
        }
        let (finished, receiver) = std::sync::mpsc::channel();
        daemon.history.lock().unwrap().test_finished = Some(finished);
        daemon.begin_osd_session(&State::Recording {
            started_at: std::time::Instant::now(),
            model_override: None,
        });
        tokio::time::timeout(Duration::from_secs(5), start_observed.notified())
            .await
            .unwrap();
        let mut state = State::Transcribing { audio: Vec::new() };
        if mode == "streaming" {
            let chain = output::create_output_chain(&daemon.config.output);
            let mut session =
                StreamingSession::new().with_history_context(daemon.history_context());
            session
                .commit_segment(
                    &chain,
                    "synthetic sensitive words",
                    None,
                    daemon.config.output.pre_output_command.as_deref(),
                    daemon.config.output.post_output_command.as_deref(),
                )
                .await
                .unwrap();
            daemon.history_note(|pending| {
                pending.texts("synthetic sensitive words", session.finalized_text())
            });
            daemon.publish_osd_text_outcome(session.finalized_text());
            daemon.update_state("idle");
        } else {
            daemon
                .handle_transcription_result(&mut state, Ok(Ok("synthetic sensitive words".into())))
                .await;
        }
        tokio::task::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)).unwrap())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("delivered-app")).unwrap(),
            destination
        );
        assert_eq!(
            std::fs::read_to_string(root.join("focus")).unwrap(),
            "synthetic-editor"
        );
        assert!(!std::fs::read_to_string(root.join("captured-text"))
            .unwrap()
            .is_empty());
        server.abort();
        let entries = crate::history::HistoryStore::at(root.join("history"))
            .all()
            .unwrap();
        if mode == "allowed" || mode == "unknown" {
            assert_eq!(entries.len(), 1);
            assert_eq!(
                entries[0].app.as_deref(),
                if mode == "allowed" {
                    Some(destination)
                } else {
                    None
                }
            );
            assert!(entries[0].window_title.is_none());
        } else {
            assert!(
                entries.is_empty(),
                "excluded delivery was retained after the output hook restored an allowed focus"
            );
            assert!(!root.join("meetings/index.db").exists());
        }
    }

    #[test]
    fn pending_cancel_applies_to_live_sessions_only() {
        let now = std::time::Instant::now();
        let live = [
            State::Recording {
                started_at: now,
                model_override: None,
            },
            State::EagerRecording {
                started_at: now,
                model_override: None,
                accumulated_audio: Vec::new(),
                chunks_sent: 0,
                chunk_results: Vec::new(),
                tasks_in_flight: 0,
            },
            State::Streaming {
                started_at: now,
                model_override: None,
                partial_buffer: String::new(),
                finalized_text: String::new(),
                typed_chars: 0,
                file_output_path: None,
            },
            State::Transcribing { audio: Vec::new() },
        ];
        for state in &live {
            assert!(cancel_applies_to(state), "{state}");
        }

        // Idle triggers are consumed at the next capture start (#606);
        // output is already on its way and has no cancel path.
        assert!(!cancel_applies_to(&State::Idle));
        assert!(!cancel_applies_to(&State::Outputting {
            text: String::new()
        }));
    }

    struct FixedTranscriber(&'static str);

    impl Transcriber for FixedTranscriber {
        fn transcribe(&self, _samples: &[f32]) -> TranscriptionResult {
            Ok(self.0.to_string())
        }
    }

    /// The event loop is the only reader of a meeting's events, and it runs
    /// the meeting's work inline, so nothing the meeting publishes may wait
    /// for a reader. Naming a speaker who had 52 lines published 52 events
    /// into a channel that held 32: the loop waited on itself, and the
    /// daemon stopped transcribing, dictating and answering `meeting stop`
    /// until it was restarted. Here the daemon applies that edit with its
    /// receiver unread, as inside the loop's trigger handler, and then
    /// takes the next command.
    #[tokio::test]
    async fn an_edit_of_many_lines_leaves_the_daemon_running() {
        const LINES: u32 = 52;
        let dir = TempDir::new().unwrap();
        let mut config = Config::default();
        config.audio.feedback.enabled = false;
        // Nothing written to the runtime directory, nothing shown.
        config.state_file = None;
        config.meeting.status_notifications = "never".to_string();
        let mut daemon = Daemon::new(config, None);

        let meeting_config = meeting::MeetingConfig {
            enabled: true,
            storage: StorageConfig {
                storage_path: dir.path().to_path_buf(),
                retain_audio: false,
                max_meetings: 0,
                my_name: None,
            },
            ..Default::default()
        };
        // The channel `start_meeting` gives a meeting.
        let (tx, mut rx) = meeting::event_channel();
        let mut running =
            MeetingDaemon::for_test(meeting_config, Arc::new(FixedTranscriber("")), None, tx);
        let id = running.start(Some("Call".to_string()), None).await.unwrap();
        assert!(matches!(rx.try_recv(), Ok(MeetingEvent::Started { .. })));
        let transcript = &mut running.current_meeting_mut().unwrap().transcript;
        for i in 0..LINES {
            let start = u64::from(i) * 5_000;
            let mut line =
                meeting::TranscriptSegment::new(i, start, start + 4_000, format!("line {i}"), i);
            line.source = meeting::AudioSource::Loopback;
            line.speaker_id = Some("Remote 2".to_string());
            transcript.add_segment(line);
        }
        // Saved as after every chunk, then what `voxtype meeting label`
        // stores before it drops its trigger.
        let saved = transcript.clone();
        running.storage().save_transcript(&id, &saved).unwrap();
        running
            .storage()
            .set_speaker_label(&id, "Remote 2", "Chris")
            .unwrap();
        daemon.meeting_daemon = Some(running);
        daemon.meeting_event_rx = Some(rx);

        let wait = Duration::from_secs(5);
        tokio::time::timeout(wait, daemon.apply_meeting_edits())
            .await
            .expect("the daemon waited on its own meeting events: it would hang here");
        tokio::time::timeout(wait, daemon.pause_meeting())
            .await
            .expect("the daemon did not get to the next command")
            .unwrap();
        assert!(daemon.meeting_daemon.as_ref().unwrap().state().is_paused());

        // Back in the loop, every changed line is there to publish.
        let rx = daemon.meeting_event_rx.as_mut().unwrap();
        let events: Vec<MeetingEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let named = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    MeetingEvent::SegmentUpdated { segment }
                        if segment.speaker_label.as_deref() == Some("Chris")
                )
            })
            .count();
        assert_eq!(named, LINES as usize);
        assert!(matches!(events.last(), Some(MeetingEvent::Paused)));
    }

    fn parakeet_daemon(on_demand: bool) -> Daemon {
        let mut config = Config::default();
        config.audio.feedback.enabled = false;
        config.set_model_for(TranscriptionEngine::Parakeet, "parakeet-tdt-0.6b-v2");
        config.parakeet.as_mut().unwrap().on_demand_loading = on_demand;
        Daemon::new(config, None)
    }

    #[test]
    fn override_config_swaps_the_engine_and_turns_off_streaming() {
        let mut base = Config::default();
        base.set_model_for(TranscriptionEngine::Parakeet, "parakeet-tdt-0.6b-v2");
        base.parakeet.as_mut().unwrap().streaming = true;
        base.whisper.streaming = true;

        let whisper = override_config(&base, TranscriptionEngine::Whisper, "base.en");
        assert_eq!(whisper.engine, TranscriptionEngine::Whisper);
        assert_eq!(whisper.whisper.model, "base.en");
        assert!(!whisper.whisper.streaming);
        assert_eq!(whisper.whisper.remote_model, None);

        let parakeet =
            override_config(&base, TranscriptionEngine::Parakeet, "parakeet-tdt-0.6b-v3");
        let section = parakeet.parakeet.unwrap();
        assert_eq!(section.model, "parakeet-tdt-0.6b-v3");
        assert!(!section.streaming);

        // The daemon's own config is untouched.
        assert_eq!(base.engine, TranscriptionEngine::Parakeet);
        assert!(base.whisper.streaming);
    }

    #[test]
    fn remote_whisper_override_names_the_remote_model() {
        let mut base = Config::default();
        base.set_model_for(TranscriptionEngine::Parakeet, "parakeet-tdt-0.6b-v2");
        base.whisper.mode = Some(WhisperMode::Remote);
        let config = override_config(&base, TranscriptionEngine::Whisper, "large-v3");
        assert_eq!(config.whisper.remote_model.as_deref(), Some("large-v3"));
    }

    #[tokio::test]
    async fn override_transcriber_reuses_the_cached_instance() {
        let mut daemon = parakeet_daemon(false);
        let cached: Arc<dyn Transcriber> = Arc::new(FixedTranscriber("cached"));
        daemon.override_cache = Some(("base.en".to_string(), cached.clone()));

        let got = daemon
            .override_transcriber("base.en", TranscriptionEngine::Whisper, "base.en")
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&got, &cached));
        assert!(
            daemon.override_cache.is_some(),
            "kept loaded for the next override"
        );

        // Starting another recording with the same override doesn't reload.
        daemon.start_override_load("base.en", TranscriptionEngine::Whisper, "base.en", vec![]);
        assert!(daemon.override_load_task.is_none());
    }

    #[tokio::test]
    async fn on_demand_override_is_released_after_use() {
        let mut daemon = parakeet_daemon(true);
        let cached: Arc<dyn Transcriber> = Arc::new(FixedTranscriber("cached"));
        daemon.override_cache = Some(("base.en".to_string(), cached.clone()));

        let got = daemon
            .override_transcriber("base.en", TranscriptionEngine::Whisper, "base.en")
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&got, &cached));
        assert!(daemon.override_cache.is_none());
    }

    #[tokio::test]
    async fn eager_chunks_pick_up_a_finished_override_load() {
        let mut daemon = parakeet_daemon(false);
        let task: TranscriberLoadTask = tokio::task::spawn_blocking(|| {
            Ok(Arc::new(FixedTranscriber("loaded")) as Arc<dyn Transcriber>)
        });
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
        daemon.override_load_task = Some(("base.en".to_string(), task));

        // A different override's load isn't touched.
        assert!(daemon.ready_override("small.en").await.is_none());
        assert!(daemon.override_load_task.is_some());

        let ready = daemon.ready_override("base.en").await.unwrap();
        assert_eq!(ready.transcribe(&[]).unwrap(), "loaded");
        assert!(daemon.override_load_task.is_none());
        assert!(matches!(&daemon.override_cache, Some((name, _)) if name == "base.en"));
    }

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
    fn discarding_a_recording_clears_every_override_it_can_carry() {
        // The cancel paths used to clear four of these and leave
        // auto_submit/shift_enter behind for the next recording.
        let dir = TempDir::new().unwrap();
        for name in RECORDING_OVERRIDE_FILES {
            fs::write(dir.path().join(name), "x").unwrap();
        }
        fs::write(dir.path().join("unrelated"), "keep").unwrap();
        discard_recording_overrides_in(dir.path());
        for name in RECORDING_OVERRIDE_FILES {
            assert!(!dir.path().join(name).exists(), "{name} survived");
        }
        assert!(dir.path().join("unrelated").exists());
    }

    #[test]
    fn override_file_list_matches_the_readers() {
        // Every per-recording override the daemon reads must be discarded.
        for name in [
            "output_mode",
            "model",
            "profile",
            "auto_submit",
            "shift_enter",
            "smart_auto_submit",
        ] {
            assert!(
                RECORDING_OVERRIDE_FILES.contains(&format!("{name}_override").as_str()),
                "{name}"
            );
        }
    }

    #[test]
    fn an_empty_transcript_leaves_no_newline_behind() {
        let dir = TempDir::new().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();

        // Overwrite still replaces the previous transcript, with nothing.
        let overwrite = dir.path().join("overwrite.txt");
        fs::write(&overwrite, "previous transcript\n").unwrap();
        runtime
            .block_on(write_transcription_to_file(
                &overwrite,
                "",
                &FileMode::Overwrite,
            ))
            .unwrap();
        assert_eq!(fs::read_to_string(&overwrite).unwrap(), "");

        // Append adds nothing and doesn't create a missing file.
        let append = dir.path().join("append.txt");
        fs::write(&append, "first\n").unwrap();
        runtime
            .block_on(write_transcription_to_file(&append, "", &FileMode::Append))
            .unwrap();
        assert_eq!(fs::read_to_string(&append).unwrap(), "first\n");
        let missing = dir.path().join("missing.txt");
        runtime
            .block_on(write_transcription_to_file(&missing, "", &FileMode::Append))
            .unwrap();
        assert!(!missing.exists());

        // Non-empty text keeps its trailing newline.
        runtime
            .block_on(write_transcription_to_file(
                &append,
                "second",
                &FileMode::Append,
            ))
            .unwrap();
        assert_eq!(fs::read_to_string(&append).unwrap(), "first\nsecond\n");
    }

    #[test]
    fn overwrite_is_atomic_from_a_readers_point_of_view() {
        // A reader that polls for a non-empty transcript must never observe a
        // partial one, so the staged file must not be the target path and the
        // target must appear complete in one step.
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("dictation.txt");
        let staged = temp_sibling(&target);
        assert_ne!(staged, target);
        assert_eq!(staged.parent(), target.parent());

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(write_transcription_to_file(
                &target,
                "the quick brown fox",
                &FileMode::Overwrite,
            ))
            .unwrap();

        assert_eq!(
            fs::read_to_string(&target).unwrap().trim_end(),
            "the quick brown fox"
        );
        assert!(!staged.exists(), "staging file must not survive the write");
    }

    #[test]
    fn overwrite_replaces_previous_transcript() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("dictation.txt");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for text in ["first pass", "second"] {
            runtime
                .block_on(write_transcription_to_file(
                    &target,
                    text,
                    &FileMode::Overwrite,
                ))
                .unwrap();
        }
        assert_eq!(fs::read_to_string(&target).unwrap().trim_end(), "second");
    }

    #[test]
    fn sidecar_sits_beside_the_transcript() {
        assert_eq!(
            result_sidecar_path(std::path::Path::new("/run/user/1000/x/dictation.txt")),
            std::path::PathBuf::from("/run/user/1000/x/dictation.txt.done")
        );
    }

    #[test]
    fn sidecar_reports_a_terminal_outcome() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("dictation.txt");

        write_result_sidecar(&target, &TranscriptOutcome::ok(19));
        let body = fs::read_to_string(result_sidecar_path(&target)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["chars"], 19);
        assert!(parsed.get("message").is_none(), "ok carries no message");

        write_result_sidecar(&target, &TranscriptOutcome::empty());
        let body = fs::read_to_string(result_sidecar_path(&target)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(parsed["status"], "empty");
        assert_eq!(parsed["chars"], 0);

        write_result_sidecar(&target, &TranscriptOutcome::error("disk went away"));
        let body = fs::read_to_string(result_sidecar_path(&target)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(parsed["status"], "error");
        assert_eq!(parsed["message"], "disk went away");
    }

    #[test]
    fn test_validate_diarization_override_accepts_allowlist() {
        assert_eq!(
            validate_diarization_override("simple".to_string()),
            Some("simple".to_string())
        );
        assert_eq!(
            validate_diarization_override("ml".to_string()),
            Some("ml".to_string())
        );
    }

    #[test]
    fn test_validate_diarization_override_rejects_unknown() {
        // Random unknown value the daemon should never propagate.
        assert_eq!(validate_diarization_override("bogus".to_string()), None);

        // Path-traversal flavor — `ALLOWED_DIARIZATION_OVERRIDES` is an exact
        // string match so traversal can't sneak through, but the test pins
        // the contract.
        assert_eq!(
            validate_diarization_override("../../etc/passwd".to_string()),
            None
        );

        // Empty string (already filtered by `read_trimmed_nonempty` before
        // this function is reached, but defense-in-depth).
        assert_eq!(validate_diarization_override(String::new()), None);

        // Common case variations that look like the right thing but aren't:
        // case-sensitive match prevents these.
        assert_eq!(validate_diarization_override("ML".to_string()), None);
        assert_eq!(validate_diarization_override("Simple".to_string()), None);

        // Whitespace-padded values shouldn't slip through if the trim step
        // upstream somehow didn't fire.
        assert_eq!(validate_diarization_override(" ml".to_string()), None);
        assert_eq!(validate_diarization_override("ml ".to_string()), None);
    }

    #[test]
    fn test_validate_diarization_override_const_in_sync() {
        // Pin the allowlist contents so a future expansion of the CLI's
        // `value_parser` requires touching this test, keeping the daemon
        // and CLI surfaces in sync.
        assert_eq!(ALLOWED_DIARIZATION_OVERRIDES, &["simple", "ml", "hybrid"]);
    }

    /// #636: the OSD suppression marker is created and removed, never
    /// rewritten, because both OSD frontends treat "file exists" as the
    /// signal. Absent is the overwhelmingly common case and must be cheap.
    #[test]
    fn test_osd_suppression_marker_lifecycle() {
        with_test_runtime_dir(|dir| {
            let marker = dir.join("osd_suppressed");
            assert!(!marker.exists(), "marker must start absent");

            set_osd_suppressed_at(&marker, true);
            assert!(marker.exists(), "marker not written");

            // Idempotent: setting it twice is not an error and leaves one file.
            set_osd_suppressed_at(&marker, true);
            assert!(marker.exists());

            set_osd_suppressed_at(&marker, false);
            assert!(!marker.exists(), "marker not cleared");

            // Clearing an already-absent marker must not panic or error.
            set_osd_suppressed_at(&marker, false);
            assert!(!marker.exists());
        });
    }

    #[test]
    fn cancel_issued_after_a_start_request_targets_it() {
        // `record start; record cancel` back to back.
        assert!(cancel_targets_start(Some(100), Some(150)));
        assert!(cancel_targets_start(Some(100), Some(100)));
    }

    #[test]
    fn leftover_cancels_never_target_a_start() {
        // An idle `record cancel`, then a start: #606's stale trigger.
        assert!(!cancel_targets_start(Some(200), Some(150)));
        // Hotkey starts carry no request.
        assert!(!cancel_targets_start(None, Some(150)));
        // Cancels from an older CLI carry no stamp.
        assert!(!cancel_targets_start(Some(100), None));
        assert!(!cancel_targets_start(None, None));
    }

    #[test]
    fn trigger_stamps_round_trip_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel");
        let first = trigger_stamp();
        std::fs::write(&path, &first).unwrap();
        assert_eq!(read_trigger_stamp(&path), first.parse().ok());
        assert!(trigger_stamp().parse::<u128>().unwrap() >= first.parse::<u128>().unwrap());

        std::fs::write(&path, "cancel").unwrap();
        assert_eq!(read_trigger_stamp(&path), None);
        assert_eq!(read_trigger_stamp(&dir.path().join("missing")), None);
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
    fn file_override_with_a_path_sets_file_mode_and_path() {
        let resolved = apply_output_override(
            &crate::config::OutputConfig::default(),
            Some(OutputOverride::FileWithPath(PathBuf::from("/tmp/out.txt"))),
        );

        assert_eq!(resolved.mode, OutputMode::File);
        assert_eq!(resolved.file_path, Some(PathBuf::from("/tmp/out.txt")));
    }

    #[test]
    fn bare_file_override_keeps_the_configured_path() {
        let base = crate::config::OutputConfig {
            file_path: Some(PathBuf::from("/tmp/configured.txt")),
            ..crate::config::OutputConfig::default()
        };

        let resolved = apply_output_override(&base, Some(OutputOverride::Mode(OutputMode::File)));

        assert_eq!(resolved.mode, OutputMode::File);
        assert_eq!(
            resolved.file_path,
            Some(PathBuf::from("/tmp/configured.txt"))
        );
    }

    #[test]
    fn mode_overrides_replace_only_the_mode() {
        let base = crate::config::OutputConfig {
            mode: OutputMode::File,
            append_text: Some("!".to_string()),
            ..crate::config::OutputConfig::default()
        };

        for mode in [OutputMode::Type, OutputMode::Clipboard, OutputMode::Paste] {
            let resolved = apply_output_override(&base, Some(OutputOverride::Mode(mode.clone())));

            assert_eq!(resolved.mode, mode);
            assert_eq!(resolved.append_text, Some("!".to_string()));
        }
    }

    #[test]
    fn absent_override_preserves_the_mode() {
        let base = crate::config::OutputConfig {
            mode: OutputMode::Paste,
            ..crate::config::OutputConfig::default()
        };

        let resolved = apply_output_override(&base, None);

        assert_eq!(resolved.mode, OutputMode::Paste);
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

    #[test]
    fn cap_warning_comes_once_the_lead_is_reached() {
        let secs = Duration::from_secs;
        // 600s cap: warn from 30s left.
        assert_eq!(cap_warning_due(secs(500), secs(600)), None);
        assert_eq!(cap_warning_due(secs(570), secs(600)), Some(secs(30)));
        assert_eq!(cap_warning_due(secs(590), secs(600)), Some(secs(10)));
        assert_eq!(cap_warning_due(secs(700), secs(600)), Some(secs(0)));
        // Default 60s cap: warn from 10s left.
        assert_eq!(cap_warning_due(secs(45), secs(60)), None);
        assert_eq!(cap_warning_due(secs(50), secs(60)), Some(secs(10)));
        // Very short caps never warn.
        assert_eq!(cap_warning_due(secs(15), secs(20)), None);
    }

    #[test]
    fn instance_lock_creates_a_missing_runtime_dir() {
        with_test_runtime_dir(|dir| {
            let runtime = dir.join("voxtype");
            assert!(acquire_instance_lock(&runtime).is_ok());
            assert!(runtime.join("voxtype.lock").exists());
        });
    }

    #[test]
    fn instance_lock_refuses_a_live_holder_and_leaves_its_lockfile() {
        with_test_runtime_dir(|dir| {
            let lock_path = dir.join("voxtype.lock");
            let live = std::process::id().to_string();
            std::fs::write(&lock_path, &live).unwrap();

            assert!(acquire_instance_lock(dir).is_err());
            assert_eq!(std::fs::read_to_string(&lock_path).unwrap(), live);
        });
    }

    #[test]
    fn instance_lock_recovers_a_stale_lockfile() {
        with_test_runtime_dir(|dir| {
            std::fs::write(dir.join("voxtype.lock"), "99999999").unwrap();
            assert!(acquire_instance_lock(dir).is_ok());
            let holder = std::fs::read_to_string(dir.join("voxtype.lock")).unwrap();
            assert_eq!(holder.trim(), std::process::id().to_string());
        });
    }
    #[derive(Clone, Copy)]
    enum FakeStreamingMode {
        Error,
        NeverEndsAfterStop,
    }

    struct FakeStreamingTranscriber {
        mode: FakeStreamingMode,
        recovered_text: &'static str,
        batch_calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Transcriber for FakeStreamingTranscriber {
        fn transcribe(&self, samples: &[f32]) -> TranscriptionResult {
            assert_eq!(samples.len(), 4_800);
            assert!(samples.iter().all(|sample| *sample == 0.25));
            self.batch_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.recovered_text.to_string())
        }

        fn as_streaming(&self) -> Option<&dyn crate::transcribe::StreamingTranscriber> {
            Some(self)
        }
    }

    impl crate::transcribe::StreamingTranscriber for FakeStreamingTranscriber {
        fn start_stream(
            &self,
            mut samples_rx: tokio::sync::mpsc::Receiver<Vec<f32>>,
        ) -> std::result::Result<StreamHandle, crate::error::TranscribeError> {
            let (events_tx, events_rx) = tokio::sync::mpsc::channel(1);
            let (cancel, cancel_rx) = tokio::sync::oneshot::channel();
            let task = match self.mode {
                FakeStreamingMode::Error => tokio::spawn(async move {
                    let _ = events_tx
                        .send(StreamingEvent::Error(
                            crate::error::TranscribeError::InferenceFailed(
                                "planned failure".into(),
                            ),
                        ))
                        .await;
                    Ok(())
                }),
                FakeStreamingMode::NeverEndsAfterStop => tokio::spawn(async move {
                    while samples_rx.recv().await.is_some() {}
                    let _ = cancel_rx.await;
                    std::future::pending::<std::result::Result<(), crate::error::TranscribeError>>()
                        .await
                }),
            };
            Ok(StreamHandle {
                events: events_rx,
                cancel,
                task,
            })
        }
    }

    struct FakeCapture(Option<Vec<f32>>);

    #[async_trait::async_trait]
    impl AudioCapture for FakeCapture {
        async fn start(
            &mut self,
        ) -> std::result::Result<tokio::sync::mpsc::Receiver<Vec<f32>>, crate::error::AudioError>
        {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(rx)
        }

        async fn stop(&mut self) -> std::result::Result<Vec<f32>, crate::error::AudioError> {
            Ok(self.0.take().unwrap_or_default())
        }

        async fn get_samples(&mut self) -> Vec<f32> {
            Vec::new()
        }
    }

    fn recovery_samples() -> Vec<f32> {
        vec![0.25; 4_800]
    }

    fn recovery_test_daemon(transcriber: Arc<dyn Transcriber>, output_path: &Path) -> Daemon {
        let config = Config {
            engine: TranscriptionEngine::Soniox,
            // Recovery prefers the configured [whisper] backend over the
            // streaming engine that failed. Point it at a model that does
            // not exist, so building it fails on every machine and the test
            // exercises the documented last resort: the running engine's
            // own batch path, which is the injected fake here.
            whisper: crate::config::WhisperConfig {
                model: "/nonexistent/recovery-model.bin".to_string(),
                ..Default::default()
            },
            audio: crate::config::AudioConfig {
                feedback: crate::config::AudioFeedbackConfig {
                    enabled: false,
                    ..Default::default()
                },
                ..Default::default()
            },
            vad: crate::config::VadConfig {
                enabled: false,
                ..Default::default()
            },
            state_file: None,
            output: crate::config::OutputConfig {
                mode: OutputMode::File,
                file_path: Some(output_path.with_file_name("configured-output.txt")),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut daemon = Daemon::new(config, None);
        daemon.transcriber_preloaded = Some(transcriber);
        daemon
    }

    async fn assert_streaming_recovery_output(
        daemon: &mut Daemon,
        state: &mut State,
        output_path: &Path,
    ) {
        let task = daemon
            .transcription_task
            .take()
            .expect("streaming recovery must start normal batch transcription");
        daemon.handle_transcription_result(state, task.await).await;
        assert!(state.is_idle());
        assert_eq!(
            fs::read_to_string(output_path).unwrap(),
            "recovered dictation\n"
        );
        assert!(!output_path.with_file_name("configured-output.txt").exists());
    }

    #[tokio::test]
    async fn streaming_backend_error_recovers_capture_with_batch_transcription() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("recovered.txt");
        let batch_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fake = Arc::new(FakeStreamingTranscriber {
            mode: FakeStreamingMode::Error,
            recovered_text: "recovered dictation",
            batch_calls: batch_calls.clone(),
        });
        let mut daemon = recovery_test_daemon(fake.clone(), &output);
        let (_samples_tx, samples_rx) = tokio::sync::mpsc::channel(1);
        let mut streaming_handle = Some(fake.start_stream(samples_rx).unwrap());
        assert!(matches!(
            streaming_handle.as_mut().unwrap().events.recv().await,
            Some(StreamingEvent::Error(_))
        ));
        let mut state = State::Streaming {
            started_at: Instant::now(),
            model_override: None,
            partial_buffer: String::new(),
            finalized_text: String::new(),
            typed_chars: 0,
            file_output_path: Some(output.clone()),
        };
        let mut capture: Option<Box<dyn AudioCapture>> =
            Some(Box::new(FakeCapture(Some(recovery_samples()))));
        let mut streaming_session = Some(StreamingSession::new());
        let mut streaming_chain = Some(Vec::new());

        daemon
            .end_streaming(
                &mut state,
                &mut capture,
                &mut streaming_handle,
                &mut streaming_session,
                &mut streaming_chain,
                StreamingEndReason::Failed(Some("planned failure".into())),
            )
            .await;

        assert_streaming_recovery_output(&mut daemon, &mut state, &output).await;
        assert_eq!(batch_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn streaming_stop_watchdog_recovers_when_backend_never_ends() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("recovered.txt");
        let batch_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fake = Arc::new(FakeStreamingTranscriber {
            mode: FakeStreamingMode::NeverEndsAfterStop,
            recovered_text: "recovered dictation",
            batch_calls: batch_calls.clone(),
        });
        let mut daemon = recovery_test_daemon(fake.clone(), &output);
        let (samples_tx, samples_rx) = tokio::sync::mpsc::channel(1);
        let mut streaming_handle = Some(fake.start_stream(samples_rx).unwrap());
        drop(samples_tx);
        let mut state = State::Streaming {
            started_at: Instant::now(),
            model_override: None,
            partial_buffer: String::new(),
            finalized_text: String::new(),
            typed_chars: 0,
            file_output_path: Some(output.clone()),
        };
        let mut capture: Option<Box<dyn AudioCapture>> =
            Some(Box::new(FakeCapture(Some(recovery_samples()))));
        let mut streaming_session = Some(StreamingSession::new());
        let mut streaming_chain = Some(Vec::new());

        daemon
            .request_streaming_stop(&mut capture, &mut streaming_chain, true)
            .await;
        assert!(capture.is_none());
        assert!(streaming_chain.is_none());
        assert!(daemon.streaming_stop_deadline.is_some());

        let completed = tokio::time::timeout(
            Duration::from_millis(100),
            daemon.end_streaming(
                &mut state,
                &mut capture,
                &mut streaming_handle,
                &mut streaming_session,
                &mut streaming_chain,
                StreamingEndReason::Failed(None),
            ),
        )
        .await;

        assert!(
            completed.is_ok(),
            "watchdog teardown must not await a backend that accepted stop without Ended"
        );
        assert_streaming_recovery_output(&mut daemon, &mut state, &output).await;
        assert_eq!(batch_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn buffered_streaming_text_uses_the_file_target_selected_at_start() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("streaming-output.txt");
        let batch_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fake = Arc::new(FakeStreamingTranscriber {
            mode: FakeStreamingMode::Error,
            recovered_text: "unused",
            batch_calls: batch_calls.clone(),
        });
        let mut daemon = recovery_test_daemon(fake.clone(), &output);
        let (_samples_tx, samples_rx) = tokio::sync::mpsc::channel(1);
        let mut streaming_handle = Some(fake.start_stream(samples_rx).unwrap());
        let mut state = State::Streaming {
            started_at: Instant::now(),
            model_override: None,
            partial_buffer: String::new(),
            finalized_text: String::new(),
            typed_chars: 0,
            file_output_path: Some(output.clone()),
        };
        let mut capture = None;
        let mut streaming_session = Some(StreamingSession::new());
        streaming_session
            .as_mut()
            .unwrap()
            .commit_segment_silent("buffered dictation");
        let mut streaming_chain = None;

        daemon
            .end_streaming(
                &mut state,
                &mut capture,
                &mut streaming_handle,
                &mut streaming_session,
                &mut streaming_chain,
                StreamingEndReason::Completed,
            )
            .await;
        // A completed file-output session writes its own text, through the
        // same processing and file delivery as before recovery existed.
        assert!(daemon.transcription_task.is_none());
        assert!(state.is_idle());
        assert_eq!(fs::read_to_string(&output).unwrap(), "buffered dictation\n");
        assert!(!output.with_file_name("configured-output.txt").exists());
        assert_eq!(batch_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    fn non_file_streaming_state() -> State {
        State::Streaming {
            started_at: Instant::now(),
            model_override: None,
            partial_buffer: String::new(),
            finalized_text: String::new(),
            typed_chars: 0,
            file_output_path: None,
        }
    }

    /// Finals that arrive after a stop disowned the typing surface are held
    /// silently. When nothing was typed, they are the whole dictation and
    /// go through the batch result path instead of being dropped.
    #[tokio::test]
    async fn disowned_late_text_is_delivered_through_the_batch_path() {
        let dir = TempDir::new().unwrap();
        let fake = Arc::new(FakeStreamingTranscriber {
            mode: FakeStreamingMode::Error,
            recovered_text: "unused",
            batch_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        });
        let mut daemon = recovery_test_daemon(fake, &dir.path().join("out.txt"));
        let mut state = non_file_streaming_state();
        let mut session = Some(StreamingSession::new());
        session
            .as_mut()
            .unwrap()
            .commit_segment_silent("late words");
        let mut chain: Option<Vec<Box<dyn TextOutput>>> = None;

        daemon
            .end_streaming(
                &mut state,
                &mut None,
                &mut None,
                &mut session,
                &mut chain,
                StreamingEndReason::Completed,
            )
            .await;

        assert!(matches!(state, State::Transcribing { .. }));
        let task = daemon
            .transcription_task
            .take()
            .expect("late text must reach the batch result path");
        assert_eq!(task.await.unwrap().unwrap(), "late words");
    }

    /// With the typing surface still attached, the session's text already
    /// went through the output chain, so a failed delivery leaves
    /// typed_chars at zero. That text must not be sent a second time.
    #[tokio::test]
    async fn text_that_went_through_the_chain_is_not_redelivered() {
        let dir = TempDir::new().unwrap();
        let fake = Arc::new(FakeStreamingTranscriber {
            mode: FakeStreamingMode::Error,
            recovered_text: "unused",
            batch_calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        });
        let mut daemon = recovery_test_daemon(fake, &dir.path().join("out.txt"));
        let mut state = non_file_streaming_state();
        let mut session = Some(StreamingSession::new());
        session
            .as_mut()
            .unwrap()
            .commit_segment_silent("already attempted");
        let mut chain: Option<Vec<Box<dyn TextOutput>>> = Some(Vec::new());

        daemon
            .end_streaming(
                &mut state,
                &mut None,
                &mut None,
                &mut session,
                &mut chain,
                StreamingEndReason::Completed,
            )
            .await;

        assert!(daemon.transcription_task.is_none());
        assert!(state.is_idle());
    }

    /// #1004: a start sent while the last dictation was being delivered is
    /// only handled once the event loop is free again, after the daemon has
    /// gone idle. It must be ignored like a start handled while busy, not
    /// start a recording late.
    #[test]
    fn a_start_issued_while_busy_is_ignored_once_idle() {
        let config = Config {
            state_file: None,
            ..Default::default()
        };
        let daemon = Daemon::new(config, None);
        let stamp = || trigger_stamp().parse::<u128>().unwrap();

        // Idle since startup: every stamped start is current.
        assert!(!daemon.start_request_predates_idle(Some(stamp())));

        daemon.update_state("recording");
        daemon.update_state("transcribing");
        let sent_during_output = stamp();
        std::thread::sleep(Duration::from_millis(2));
        daemon.update_state("idle");
        assert!(daemon.start_request_predates_idle(Some(sent_during_output)));

        // A repeated idle write must not move the boundary past a start
        // that was sent while idle.
        std::thread::sleep(Duration::from_millis(2));
        let sent_while_idle = stamp();
        std::thread::sleep(Duration::from_millis(2));
        daemon.update_state("idle");
        assert!(!daemon.start_request_predates_idle(Some(sent_while_idle)));

        // A bare SIGUSR1 carries no stamp and keeps working.
        assert!(!daemon.start_request_predates_idle(None));
    }
}
