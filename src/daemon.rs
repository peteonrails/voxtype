//! Daemon module - main event loop orchestration
//!
//! Coordinates the hotkey listener, audio capture, transcription,
//! and text output components.

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
#[cfg(target_os = "macos")]
use crate::notification;
use crate::output;
use crate::output::post_process::PostProcessor;
use crate::output::streaming::StreamingSession;
use crate::output::TextOutput;
use crate::state::{ChunkResult, State};
use crate::text::TextProcessor;
use crate::transcribe::{StreamHandle, StreamingEvent, Transcriber};
use pidlock::Pidlock;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::signal::unix::{signal, SignalKind};

/// Send a desktop notification with optional engine icon
async fn send_notification(
    title: &str,
    body: &str,
    show_engine_icon: bool,
    engine: crate::config::TranscriptionEngine,
    urgency: &str,
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
        let urgency_arg = format!("--urgency={}", crate::output::sanitize_urgency(urgency));
        // Synchronous + transient hints ([#345]): force a single Voxtype
        // notification slot the compositor overwrites in place, and prevent
        // status updates from accumulating in the notification history.
        let _ = Command::new("notify-send")
            .args([
                "--app-name=Voxtype",
                &urgency_arg,
                "--expire-time=2000",
                "-h",
                "string:x-canonical-private-synchronous:voxtype",
                "-h",
                "int:transient:1",
                &title,
                body,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }

    #[cfg(target_os = "macos")]
    {
        // terminal-notifier has no urgency concept; ignore the arg on macOS.
        let _ = urgency;
        let engine_for_icon = if show_engine_icon { Some(engine) } else { None };
        notification::send_with_engine(&title, body, engine_for_icon).await;
    }
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

/// Remove a boolean override file if it exists (for cleanup on cancel/error)
fn cleanup_bool_override(name: &str) {
    let override_file = Config::runtime_dir().join(format!("{}_override", name));
    let _ = std::fs::remove_file(&override_file);
}

// === Meeting Mode IPC ===

/// A pending meeting-start trigger with optional title and diarization override.
struct MeetingStartTrigger {
    title: Option<String>,
    diarization: Option<String>,
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
const ALLOWED_DIARIZATION_OVERRIDES: &[&str] = &["simple", "ml"];

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

    // Remove the start trigger last to acknowledge the command.
    let _ = std::fs::remove_file(&start_file);

    Some(MeetingStartTrigger { title, diarization })
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

/// Clean up any stale meeting command files on startup
fn cleanup_meeting_files() {
    let runtime_dir = Config::runtime_dir();
    for name in &[
        "meeting_start",
        "meeting_start_diarization",
        "meeting_stop",
        "meeting_pause",
        "meeting_resume",
    ] {
        let file = runtime_dir.join(name);
        if file.exists() {
            let _ = std::fs::remove_file(&file);
        }
    }
}

/// Mark any active/paused meetings as completed on daemon startup.
/// This handles meetings orphaned by a crash or daemon restart.
fn cleanup_stale_meetings(config: &Config) {
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

    // Ensure text ends with newline
    let output_text = if text.ends_with('\n') {
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
    /// Last post-processed text and when it was produced, for context in subsequent dictations
    last_dictation: Option<(String, Instant)>,
    /// Audio level broadcaster for the OSD (None when disabled or bind failed)
    level_hub: Option<audio::levels::LevelHub>,
    /// Active per-recording level emitter task; aborted when recording stops
    level_emitter_task: Option<tokio::task::JoinHandle<()>>,
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
    paused_media_players: Vec<String>,
    // Audio streams that were ducked when recording started (for restore on recording stop)
    ducked_media_streams: Vec<audio::media::DuckedMediaStream>,
    // In-flight media volume fade, down at recording start or up at stop. Held
    // so the next duck/restore can serialize against it; see
    // `duck_media_streams` for why capturing originals mid-fade is unsafe.
    media_fade_task: Option<tokio::task::JoinHandle<()>>,
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
            Some(Config::runtime_dir().join("meeting_state"))
        } else {
            None
        };

        Self {
            config,
            config_path,
            state_file_path,
            pid_file_path: None,
            audio_feedback,
            text_processor,
            post_processor,
            last_dictation: None,
            level_hub: None,
            level_emitter_task: None,
            streaming_drain_pump: None,
            osd_supervisor_task: None,
            model_manager: None,
            model_load_task: None,
            whisper_prepare_task: None,
            transcription_task: None,
            active_transcriber: None,
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
            paused_media_players: Vec::new(),
            ducked_media_streams: Vec::new(),
            media_fade_task: None,
        }
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
                if read_bool_override("no_osd").unwrap_or(false) {
                    set_osd_suppressed(true);
                }
            }
            "idle" | "stopped" => set_osd_suppressed(false),
            _ => {}
        }
    }

    /// Start a push-to-talk audio capture and (if enabled) a level emitter.
    ///
    /// Returns the capture handle on success. The chunk receiver from the
    /// capture is plumbed into the level hub so the OSD sees audio frames
    /// at 100 Hz during recording. The emitter task is tracked so it can
    /// be cleanly aborted when recording stops.
    async fn start_recording_capture(&mut self) -> std::result::Result<Box<dyn AudioCapture>, ()> {
        // A `record cancel` issued while idle leaves its trigger file behind,
        // and the idle-time sweep that was meant to consume it never runs:
        // its 500ms timer sits in a select! loop whose unconditional 100ms
        // poll arm recreates every timer each iteration, so the 500ms sleep
        // restarts forever. A stale trigger then kills this recording (and
        // each one after it) ~100-400ms in. Consume it here, at the single
        // point every recording path passes through, so a cancel can only
        // ever apply to a recording that was live when it was issued (#606).
        cleanup_cancel_file();
        match audio::create_capture(&self.config.audio) {
            Ok(mut capture) => match capture.start().await {
                Ok(chunk_rx) => {
                    if let Some(hub) = &self.level_hub {
                        // Cancel any prior emitter (defensive; should be idle).
                        if let Some(handle) = self.level_emitter_task.take() {
                            handle.abort();
                        }
                        let handle = audio::levels::spawn_emitter(chunk_rx, hub.frame_sink());
                        self.level_emitter_task = Some(handle);
                    }
                    // If level_hub is None we still return Ok; the chunk_rx
                    // is dropped here, matching previous behaviour.
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
        transcriber_preloaded: &Option<Arc<dyn Transcriber>>,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_handle: &mut Option<StreamHandle>,
        streaming_session: &mut Option<StreamingSession>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
        model_override: Option<String>,
    ) -> bool {
        // Same stale-trigger hazard as start_recording_capture: Streaming is
        // an is_recording() state, so a leftover cancel file would kill the
        // session moments after it starts. See #606.
        cleanup_cancel_file();
        let Some(transcriber) = transcriber_preloaded.as_ref() else {
            return false;
        };
        if transcriber.as_streaming().is_none() {
            return false;
        }

        let (capture, samples_rx) = match self.start_streaming_capture().await {
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

        *audio_capture = Some(capture);
        *streaming_handle = Some(handle);
        *streaming_session = Some(StreamingSession::new());
        *streaming_chain = Some(output::create_output_chain(&self.config.output));
        *state = State::Streaming {
            started_at: std::time::Instant::now(),
            model_override,
            partial_buffer: String::new(),
            finalized_text: String::new(),
            typed_chars: 0,
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
    async fn stop_streaming_capture(&mut self, audio_capture: &mut Option<Box<dyn AudioCapture>>) {
        self.cut_streaming_audio();
        self.start_streaming_drain_pump();
        if let Some(mut c) = audio_capture.take() {
            let _ = c.stop().await;
        }
        self.restore_recording_media();
    }

    async fn end_streaming(
        &mut self,
        state: &mut State,
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_handle: &mut Option<StreamHandle>,
        streaming_session: &mut Option<StreamingSession>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
    ) {
        if let Some(mut c) = audio_capture.take() {
            let _ = c.stop().await;
        }
        self.restore_recording_media();
        if let Some(h) = streaming_handle.take() {
            // Don't error on join failure; the task may have already
            // completed. We drop events implicitly here.
            let _ = h.task.await;
        }
        self.stop_streaming_drain_pump();
        *streaming_session = None;
        *streaming_chain = None;

        self.play_feedback(SoundEvent::TranscriptionComplete);

        if let Some(cmd) = &self.config.output.post_output_command {
            if let Err(e) = output::run_hook(cmd, "post_output").await {
                tracing::warn!("{}", e);
            }
        }

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
        audio_capture: &mut Option<Box<dyn AudioCapture>>,
        streaming_handle: &mut Option<StreamHandle>,
        streaming_session: &mut Option<StreamingSession>,
        streaming_chain: &mut Option<Vec<Box<dyn TextOutput>>>,
        notification_body: &str,
    ) {
        let backend_task = streaming_handle.take().map(|h| {
            let _ = h.cancel.send(());
            h.task
        });
        self.cut_streaming_audio();
        if let Some(mut c) = audio_capture.take() {
            let _ = c.stop().await;
        }
        self.restore_recording_media();
        if let Some(task) = backend_task {
            let _ = task.await;
        }
        if let Some(s) = streaming_session.as_mut() {
            if let Err(e) = s.rewind().await {
                tracing::warn!("Streaming rewind failed: {}", e);
            }
        }
        self.stop_streaming_drain_pump();
        *streaming_session = None;
        *streaming_chain = None;

        cleanup_output_mode_override();
        cleanup_model_override();
        cleanup_profile_override();
        cleanup_bool_override("auto_submit");
        cleanup_bool_override("shift_enter");
        cleanup_bool_override("smart_auto_submit");
        *state = State::Idle;
        self.update_state("idle");
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
    ) -> std::result::Result<(Box<dyn AudioCapture>, tokio::sync::mpsc::Receiver<Vec<f32>>), ()>
    {
        match audio::create_capture(&self.config.audio) {
            Ok(mut capture) => match capture.start().await {
                Ok(chunk_rx) => {
                    // Bounded; backed-up streaming backend drops chunks
                    // rather than back-pressuring the capture.
                    let (streaming_tx, streaming_rx) = tokio::sync::mpsc::channel::<Vec<f32>>(64);

                    if let Some(handle) = self.level_emitter_task.take() {
                        handle.abort();
                    }
                    let handle = if let Some(hub) = &self.level_hub {
                        audio::levels::spawn_emitter_with_streaming_tap(
                            chunk_rx,
                            hub.frame_sink(),
                            Some(streaming_tx),
                        )
                    } else {
                        // No OSD: still need to drive chunk_rx → streaming_tx.
                        tokio::spawn(async move {
                            let mut rx = chunk_rx;
                            while let Some(chunk) = rx.recv().await {
                                if streaming_tx.try_send(chunk).is_err() {
                                    // Backend slow or gone; drop and keep going.
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
                crate::config::TranscriptionEngine::Parakeet
                | crate::config::TranscriptionEngine::Moonshine
                | crate::config::TranscriptionEngine::SenseVoice
                | crate::config::TranscriptionEngine::Paraformer
                | crate::config::TranscriptionEngine::Dolphin
                | crate::config::TranscriptionEngine::Omnilingual
                | crate::config::TranscriptionEngine::Cohere
                | crate::config::TranscriptionEngine::Soniox => {
                    if let Some(ref t) = transcriber_preloaded {
                        Ok(t.clone())
                    } else {
                        tracing::error!("Parakeet transcriber not preloaded");
                        self.play_feedback(SoundEvent::Error);
                        Err(())
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
        if let Some(path) = peek_file_output_path(&self.config) {
            write_result_sidecar(&path, &TranscriptOutcome::empty());
        }
    }

    /// Reset state to idle and run post_output_command to reset compositor submap
    /// Call this when exiting from recording/transcribing without normal output flow
    async fn reset_to_idle(&mut self, state: &mut State) {
        cleanup_output_mode_override();
        cleanup_model_override();
        cleanup_profile_override();
        cleanup_bool_override("auto_submit");
        cleanup_bool_override("shift_enter");
        cleanup_bool_override("smart_auto_submit");
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
        model_override: Option<String>,
        transcriber_preloaded: &Option<Arc<dyn Transcriber>>,
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

            if self.config.output.notification.on_recording_stop {
                send_notification(
                    "Recording Stopped",
                    "Transcribing...",
                    self.config.output.notification.show_engine_icon,
                    self.config.engine,
                    &self.config.output.notification.urgency,
                )
                .await;
            }

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
                    *state = State::Transcribing {
                        audio: samples.clone(),
                    };
                    self.update_state("transcribing");

                    let transcriber = match self
                        .get_transcriber_for_recording(
                            model_override.as_deref(),
                            transcriber_preloaded,
                        )
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

                    // Consume the per-recording boolean overrides before any
                    // early return below. File output returns without building
                    // an output chain, and these sentinels used to survive it:
                    // the next recording (typically the user's own hotkey, in
                    // type mode) then picked them up. Any client that passes
                    // --no-auto-submit with --file hit this on every dictation.
                    let auto_submit_override = read_bool_override("auto_submit");
                    let shift_enter_override = read_bool_override("shift_enter");

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

                    let output_chain = output::create_output_chain(&output_config);

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
                }
                self.reset_to_idle(state).await;
            }
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
        cleanup_cancel_file();
        cleanup_profile_override();

        // Clean up any stale meeting command files
        cleanup_meeting_files();

        // Mark any orphaned active meetings as completed
        cleanup_stale_meetings(&self.config);

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
        if self.config.osd.enabled && self.level_hub.is_some() {
            self.osd_supervisor_task = Some(crate::osd::supervisor::spawn());
        }

        // Check if another instance is already running (single-instance safeguard)
        let lock_path = Config::runtime_dir().join("voxtype.lock");
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
                crate::config::TranscriptionEngine::Parakeet
                | crate::config::TranscriptionEngine::Moonshine
                | crate::config::TranscriptionEngine::SenseVoice
                | crate::config::TranscriptionEngine::Paraformer
                | crate::config::TranscriptionEngine::Dolphin
                | crate::config::TranscriptionEngine::Omnilingual
                | crate::config::TranscriptionEngine::Cohere
                | crate::config::TranscriptionEngine::Soniox => {
                    // Non-Whisper engines do their own setup; Soniox just validates
                    // API key + endpoint at construction (no model to download).
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
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let mut hotkey_rx = if let Some(ref mut listener) = hotkey_listener {
            match listener.start() {
                Ok(rx) => Some(rx),
                Err(e) => {
                    tracing::warn!("Failed to start hotkey listener: {}. Use 'voxtype record' commands instead.", e);
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
        // Cached transcriber for eager chunk processing during recording
        let mut eager_transcriber: Option<Arc<dyn Transcriber>> = None;

        // Streaming session locals (Some only while State::Streaming).
        let mut streaming_handle: Option<StreamHandle> = None;
        let mut streaming_session: Option<StreamingSession> = None;
        let mut streaming_chain: Option<Vec<Box<dyn TextOutput>>> = None;

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
                                // Write profile override file if a profile modifier was held
                                if let Some(ref profile_name) = profile_override {
                                    write_profile_override(profile_name);
                                }

                                tracing::info!("Recording started");

                                // Send notification if enabled
                                if self.config.output.notification.on_recording_start {
                                    send_notification("Push to Talk Active", "Recording...", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency).await;
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
                                        crate::config::TranscriptionEngine::Parakeet
                                        | crate::config::TranscriptionEngine::Moonshine
                                        | crate::config::TranscriptionEngine::SenseVoice
                | crate::config::TranscriptionEngine::Paraformer
                | crate::config::TranscriptionEngine::Dolphin
                | crate::config::TranscriptionEngine::Omnilingual
                | crate::config::TranscriptionEngine::Cohere
                | crate::config::TranscriptionEngine::Soniox => {
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
                                                match mm.prepare_model(model_override.as_deref()) {
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
                | crate::config::TranscriptionEngine::Soniox => {
                                            if let Some(ref t) = transcriber_preloaded {
                                                let transcriber = t.clone();
                                                tokio::task::spawn_blocking(move || {
                                                    transcriber.prepare();
                                                });
                                            }
                                        }
                                    }
                                }

                                // Pause or duck playback before either capture path opens
                                // the microphone.
                                self.suppress_recording_media().await;

                                // Try streaming first; fall through to batch if the engine
                                // doesn't support streaming or setup fails.
                                if self.try_start_streaming(
                                    &transcriber_preloaded,
                                    &mut state,
                                    &mut audio_capture,
                                    &mut streaming_handle,
                                    &mut streaming_session,
                                    &mut streaming_chain,
                                    model_override.clone(),
                                ).await {
                                    tracing::info!("Streaming session started (push-to-talk)");
                                } else {
                                    // Create and start audio capture
                                    tracing::debug!("Creating audio capture with device: {}", self.config.audio.device);
                                    match self.start_recording_capture().await {
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
                                            cleanup_profile_override();
                                        }
                                    }
                                }
                            }
                        }

                        (HotkeyEvent::Released, ActivationMode::PushToTalk) => {
                            tracing::debug!("Received HotkeyEvent::Released (push-to-talk), state.is_recording() = {}", state.is_recording());
                            if state.is_streaming() {
                                tracing::debug!("Streaming push-to-talk released; closing audio capture and disowning session");
                                self.stop_streaming_capture(&mut audio_capture).await;
                                // Drop session/chain so the backend's
                                // post-stop flush emission is dropped at
                                // the event pump instead of typed.
                                // Matches the SIGUSR2 stop path.
                                streaming_session = None;
                                streaming_chain = None;
                            } else if let State::Recording { model_override, .. } = &state {
                                let model_override = model_override.clone();

                                self.start_transcription_task(
                                    &mut state,
                                    &mut audio_capture,
                                    model_override,
                                    &transcriber_preloaded,
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

                                if self.config.output.notification.on_recording_stop {
                                    send_notification("Recording Stopped", "Transcribing...", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency).await;
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
                                eager_transcriber = None;
                            }
                        }

                        // === TOGGLE MODE ===
                        (HotkeyEvent::Pressed { model_override, profile_override }, ActivationMode::Toggle) => {
                            tracing::debug!("Received HotkeyEvent::Pressed (toggle), state.is_idle() = {}, state.is_recording() = {}, model_override = {:?}, profile_override = {:?}",
                                state.is_idle(), state.is_recording(), model_override, profile_override);

                            if state.is_idle() {
                                // Write profile override file if a profile modifier was held
                                if let Some(ref profile_name) = profile_override {
                                    write_profile_override(profile_name);
                                }

                                // Start recording
                                tracing::info!("Recording started (toggle mode)");

                                if self.config.output.notification.on_recording_start {
                                    send_notification("Recording Started", "Press hotkey again to stop", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency).await;
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
                                        crate::config::TranscriptionEngine::Parakeet
                                        | crate::config::TranscriptionEngine::Moonshine
                                        | crate::config::TranscriptionEngine::SenseVoice
                | crate::config::TranscriptionEngine::Paraformer
                | crate::config::TranscriptionEngine::Dolphin
                | crate::config::TranscriptionEngine::Omnilingual
                | crate::config::TranscriptionEngine::Cohere
                | crate::config::TranscriptionEngine::Soniox => {
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
                                                match mm.prepare_model(model_override.as_deref()) {
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
                | crate::config::TranscriptionEngine::Soniox => {
                                            if let Some(ref t) = transcriber_preloaded {
                                                let transcriber = t.clone();
                                                tokio::task::spawn_blocking(move || {
                                                    transcriber.prepare();
                                                });
                                            }
                                        }
                                    }
                                }

                                self.suppress_recording_media().await;

                                if self.try_start_streaming(
                                    &transcriber_preloaded,
                                    &mut state,
                                    &mut audio_capture,
                                    &mut streaming_handle,
                                    &mut streaming_session,
                                    &mut streaming_chain,
                                    model_override.clone(),
                                ).await {
                                    tracing::info!("Streaming session started (toggle)");
                                } else {
                                    match self.start_recording_capture().await {
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
                                            cleanup_profile_override();
                                        }
                                    }
                                }
                            } else if state.is_streaming() {
                                tracing::info!("Toggle stop while streaming; closing capture");
                                self.stop_streaming_capture(&mut audio_capture).await;
                            } else if let State::Recording { model_override: current_model_override, .. } = &state {
                                let model_override = current_model_override.clone();

                                // Stop recording and start transcription
                                self.start_transcription_task(
                                    &mut state,
                                    &mut audio_capture,
                                    model_override,
                                    &transcriber_preloaded,
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

                                if self.config.output.notification.on_recording_stop {
                                    send_notification("Recording Stopped", "Transcribing...", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency).await;
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
                                eager_transcriber = None;
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
                                    &mut audio_capture,
                                    &mut streaming_handle,
                                    &mut streaming_session,
                                    &mut streaming_chain,
                                    "Recording discarded",
                                ).await;
                            } else if state.is_recording() {
                                tracing::info!("Recording cancelled via hotkey");

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

                                cleanup_output_mode_override();
                                cleanup_model_override();
                                cleanup_profile_override();
                                cleanup_bool_override("smart_auto_submit");
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
                                    send_notification("Cancelled", "Recording discarded", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency).await;
                                }
                            } else if matches!(state, State::Transcribing { .. }) {
                                tracing::info!("Transcription cancelled via hotkey");

                                // Abort the transcription task
                                if let Some(task) = self.transcription_task.take() {
                                    task.abort();
                                }
                                // Drop the cloned transcriber Arc so it isn't
                                // held until the next transcription.
                                self.active_transcriber = None;

                                cleanup_output_mode_override();
                                cleanup_model_override();
                                cleanup_profile_override();
                                cleanup_bool_override("smart_auto_submit");
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
                                    send_notification("Cancelled", "Transcription aborted", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency).await;
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

                        cleanup_output_mode_override();
                        cleanup_model_override();
                        cleanup_profile_override();
                        cleanup_bool_override("smart_auto_submit");
                        state = State::Idle;
                        eager_transcriber = None;
                        self.update_state("idle");
                        self.play_feedback(SoundEvent::Cancelled);

                        // Run post_output_command to reset compositor submap
                        if let Some(cmd) = &self.config.output.post_output_command {
                            if let Err(e) = output::run_hook(cmd, "post_output").await {
                                tracing::warn!("{}", e);
                            }
                        }

                        if self.config.output.notification.on_recording_stop {
                            send_notification("Cancelled", "Recording discarded", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency).await;
                        }

                        continue;
                    }

                    // Populate eager transcriber cache on first poll
                    if eager_transcriber.is_none() && state.is_eager_recording() {
                        let model_override = match &state {
                            State::EagerRecording { model_override, .. } => model_override.as_deref(),
                            _ => None,
                        };
                        eager_transcriber = transcriber_preloaded.clone();
                        if eager_transcriber.is_none()
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

                    // Check for recording timeout. Skip when audio_capture is
                    // already gone so we don't re-fire cleanup on every 100ms
                    // tick while the streaming session drains server-side
                    // (state stays Streaming until Ended arrives).
                    let timeout_fired = audio_capture.is_some()
                        && state.recording_duration().is_some_and(|d| d > max_duration);
                    if timeout_fired {
                        // Streaming has its own clean stop path: skip the
                        // batch_transcribe branch below to avoid opening a
                        // second WS session for audio already being processed
                        // by the active streaming one.
                        if state.is_streaming() {
                            tracing::warn!(
                                "Recording timeout ({:.0}s limit) while streaming; closing capture",
                                max_duration.as_secs_f32()
                            );
                            self.stop_streaming_capture(&mut audio_capture).await;
                            continue;
                        }

                        tracing::warn!(
                            "Recording timeout ({:.0}s limit), transcribing captured audio",
                            max_duration.as_secs_f32()
                        );

                        cleanup_output_mode_override();
                        cleanup_model_override();
                        cleanup_profile_override();
                        cleanup_bool_override("smart_auto_submit");

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

                            let transcriber = match self.get_transcriber_for_recording(
                                model_override.as_deref(),
                                &transcriber_preloaded,
                            ).await {
                                Ok(transcriber) => transcriber,
                                Err(()) => {
                                    self.reset_to_idle(&mut state).await;
                                    continue;
                                }
                            };

                            self.update_state("transcribing");

                            if let Some(text) = self.finish_eager_recording(&mut state, transcriber).await {
                                state = State::Transcribing { audio: Vec::new() };
                                self.handle_transcription_result(&mut state, Ok(Ok(text))).await;
                            } else {
                                tracing::debug!("Eager recording timeout produced empty result");
                                self.reset_to_idle(&mut state).await;
                            }
                            eager_transcriber = None;
                        } else {
                            for (_, task) in self.eager_chunk_tasks.drain(..) {
                                task.abort();
                            }

                            self.start_transcription_task(
                                &mut state,
                                &mut audio_capture,
                                model_override,
                                &transcriber_preloaded,
                            ).await;
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
                            send_notification("Recording Started", "External trigger", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency).await;
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
                                crate::config::TranscriptionEngine::Parakeet
                                | crate::config::TranscriptionEngine::Moonshine
                                | crate::config::TranscriptionEngine::SenseVoice
                | crate::config::TranscriptionEngine::Paraformer
                | crate::config::TranscriptionEngine::Dolphin
                | crate::config::TranscriptionEngine::Omnilingual
                | crate::config::TranscriptionEngine::Cohere
                | crate::config::TranscriptionEngine::Soniox => {
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
                                        match mm.prepare_model(model_override.as_deref()) {
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
                | crate::config::TranscriptionEngine::Soniox => {
                                    if let Some(ref t) = transcriber_preloaded {
                                        let transcriber = t.clone();
                                        tokio::task::spawn_blocking(move || {
                                            transcriber.prepare();
                                        });
                                    }
                                }
                            }
                        }

                        self.suppress_recording_media().await;

                        if self.try_start_streaming(
                            &transcriber_preloaded,
                            &mut state,
                            &mut audio_capture,
                            &mut streaming_handle,
                            &mut streaming_session,
                            &mut streaming_chain,
                            model_override.clone(),
                        ).await {
                            tracing::info!("Streaming session started (SIGUSR1)");
                        } else {
                            match self.start_recording_capture().await {
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
                    }
                }

                // Handle SIGUSR2 - stop recording (for compositor keybindings)
                _ = sigusr2.recv() => {
                    tracing::debug!("Received SIGUSR2 (stop recording)");
                    if state.is_streaming() {
                        tracing::info!("SIGUSR2 stop while streaming; closing capture and disowning session");
                        self.stop_streaming_capture(&mut audio_capture).await;
                        // Drop the typing surface synchronously so any
                        // Final/Partial events the backend emits while
                        // draining its internal buffer reach the event-pump
                        // arm with `streaming_session = None` and get
                        // discarded instead of typed into whatever window
                        // has focus by then.
                        streaming_session = None;
                        streaming_chain = None;
                    } else if let State::Recording { model_override, .. } = &state {
                        let model_override = model_override.clone();

                        self.start_transcription_task(
                            &mut state,
                            &mut audio_capture,
                            model_override,
                            &transcriber_preloaded,
                        ).await;
                    } else if state.is_eager_recording() {
                        // Handle eager recording stop via external trigger - extract model_override first
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

                        if self.config.output.notification.on_recording_stop {
                            send_notification("Recording Stopped", "Transcribing...", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency).await;
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
                        eager_transcriber = None;
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

                // Streaming event pump (active only while State::Streaming).
                event = async {
                    match streaming_handle.as_mut() {
                        Some(h) => h.events.recv().await,
                        None => std::future::pending().await,
                    }
                }, if state.is_streaming() && streaming_handle.is_some() => {
                    match event {
                        Some(StreamingEvent::Partial { text, .. }) => {
                            if let (Some(s), Some(chain)) =
                                (streaming_session.as_mut(), streaming_chain.as_ref())
                            {
                                if let Err(e) = s.type_partial_delta(
                                    chain,
                                    text,
                                    self.config.output.pre_output_command.as_deref(),
                                    self.config.output.post_output_command.as_deref(),
                                ).await {
                                    tracing::warn!("Streaming partial delta type failed: {}", e);
                                }
                                if let State::Streaming { typed_chars, .. } = &mut state {
                                    *typed_chars = s.typed_chars();
                                }
                            }
                        }
                        Some(StreamingEvent::Final { text, .. }) => {
                            if let (Some(s), Some(chain)) =
                                (streaming_session.as_mut(), streaming_chain.as_ref())
                            {
                                let pp = self.post_processor.as_ref();
                                if let Err(e) = s.commit_segment(
                                    chain,
                                    &text,
                                    pp,
                                    self.config.output.pre_output_command.as_deref(),
                                    self.config.output.post_output_command.as_deref(),
                                ).await {
                                    tracing::error!("Streaming commit_segment failed: {}", e);
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
                            if let (Some(s), Some(chain)) =
                                (streaming_session.as_mut(), streaming_chain.as_ref())
                            {
                                if let Err(e) = s.replace_and_commit(
                                    chain,
                                    backspace,
                                    &text,
                                    self.config.output.pre_output_command.as_deref(),
                                    self.config.output.post_output_command.as_deref(),
                                ).await {
                                    tracing::error!("Streaming replace_and_commit failed: {}", e);
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
                            self.end_streaming(
                                &mut state,
                                &mut audio_capture,
                                &mut streaming_handle,
                                &mut streaming_session,
                                &mut streaming_chain,
                            ).await;
                        }
                        Some(StreamingEvent::Ended) | None => {
                            self.end_streaming(
                                &mut state,
                                &mut audio_capture,
                                &mut streaming_handle,
                                &mut streaming_session,
                                &mut streaming_chain,
                            ).await;
                        }
                    }
                }

                // Check for cancel during transcription
                _ = tokio::time::sleep(Duration::from_millis(100)), if matches!(state, State::Transcribing { .. }) => {
                    if check_cancel_requested() {
                        tracing::info!("Transcription cancelled");

                        // Abort the transcription task
                        if let Some(task) = self.transcription_task.take() {
                            task.abort();
                        }
                        // Drop the cloned transcriber Arc so it isn't held
                        // until the next transcription.
                        self.active_transcriber = None;

                        cleanup_output_mode_override();
                        cleanup_model_override();
                        cleanup_profile_override();
                        cleanup_bool_override("smart_auto_submit");
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
                            send_notification("Cancelled", "Transcription aborted", self.config.output.notification.show_engine_icon, self.config.engine, &self.config.output.notification.urgency).await;
                        }
                    }
                }

                // Clean up stale cancel file when idle and evict idle models
                _ = tokio::time::sleep(Duration::from_millis(500)), if matches!(state, State::Idle) => {
                    // Silently consume any stale cancel request
                    let _ = check_cancel_requested();

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
                    if let Some(trigger) = check_meeting_start() {
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
                    if check_meeting_stop()
                        && self.meeting_daemon.is_some() {
                            tracing::debug!("Meeting stop requested via file trigger");
                            if let Err(e) = self.stop_meeting().await {
                                tracing::error!("Failed to stop meeting: {}", e);
                            }
                        }

                    // Check for meeting pause command
                    if check_meeting_pause()
                        && self.meeting_active() {
                            tracing::debug!("Meeting pause requested via file trigger");
                            if let Err(e) = self.pause_meeting().await {
                                tracing::error!("Failed to pause meeting: {}", e);
                            }
                        }

                    // Check for meeting resume command
                    if check_meeting_resume()
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
                    if check_meeting_stop() && self.meeting_daemon.is_some() {
                        tracing::debug!("Meeting stop requested via file trigger");
                        if let Err(e) = self.stop_meeting().await {
                            tracing::error!("Failed to stop meeting: {}", e);
                        }
                        continue;
                    }
                    if check_meeting_pause() && self.meeting_active() {
                        tracing::debug!("Meeting pause requested via file trigger");
                        if let Err(e) = self.pause_meeting().await {
                            tracing::error!("Failed to pause meeting: {}", e);
                        }
                        continue;
                    }
                    if check_meeting_resume()
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
        let streaming_task = streaming_handle.take().map(|handle| {
            let _ = handle.cancel.send(());
            handle.task
        });
        self.cut_streaming_audio();
        if let Some(mut capture) = audio_capture.take() {
            let _ = capture.stop().await;
        }
        self.restore_recording_media();
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
        cleanup_profile_override();

        // Remove state file on shutdown
        if let Some(ref path) = self.state_file_path {
            cleanup_state_file(path);
        }

        // Remove meeting state file on shutdown
        if let Some(ref path) = self.meeting_state_file_path {
            cleanup_state_file(path);
        }

        // Remove PID file on shutdown
        if let Some(ref path) = self.pid_file_path {
            cleanup_pid_file(path);
        }

        // Remove the OSD audio level socket so a stale path doesn't
        // confuse the next daemon start.
        if let Some(ref hub) = self.level_hub {
            hub.cleanup();
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
        assert_eq!(ALLOWED_DIARIZATION_OVERRIDES, &["simple", "ml"]);
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
