//! The daemon's runtime files, and the one place their names are written down.
//!
//! Two processes exchange these files by name: `voxtype record start` and
//! friends write a sentinel, the daemon consumes it. The meeting triggers work
//! the same way in the other direction. When the name is written in two
//! modules, the two can disagree and nothing fails to compile, which is exactly
//! what happened to the lockfile (see `daemon_status`).
//!
//! The directory is a field rather than an environment read. Production calls
//! [`RuntimePaths::from_env`]; a test supplies a temporary directory. That is
//! what lets a test drive a whole daemon cycle without touching the running
//! daemon's lock, its sentinels, or its state file.

use crate::config::{Config, OutputMode};
use std::path::{Path, PathBuf};

/// A resolved runtime directory, with the name of every file kept in it.
///
/// The names live here; the directory comes from the caller. Files owned by
/// another module (the lockfile, the published version, the OSD level socket)
/// delegate their name to that module rather than repeating it.
#[derive(Debug, Clone)]
pub struct RuntimePaths {
    dir: PathBuf,
}

impl RuntimePaths {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The process's own runtime directory: `$XDG_RUNTIME_DIR/voxtype`.
    pub fn from_env() -> Self {
        Self::new(Config::runtime_dir())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// The lockfile marking the running daemon, named by `daemon_status`.
    pub fn lock(&self) -> PathBuf {
        crate::daemon_status::pid_file_path_in(self.dir())
    }

    /// The version file the running daemon publishes, named by `daemon_status`.
    pub fn version(&self) -> PathBuf {
        crate::daemon_status::version_file_path_in(self.dir())
    }

    /// The Unix socket OSD frontends subscribe to, named by `audio::levels`.
    pub fn level_socket(&self) -> PathBuf {
        crate::audio::levels::default_socket_path_in(self.dir())
    }

    /// Marker telling OSD frontends to stay hidden for the recording in flight.
    pub fn osd_suppressed(&self) -> PathBuf {
        self.file("osd_suppressed")
    }

    /// One-shot cancel request, consumed by the daemon's polling arm.
    pub fn cancel(&self) -> PathBuf {
        self.file("cancel")
    }

    /// One-shot output mode override (`voxtype record start --type` and kin).
    pub fn output_mode_override(&self) -> PathBuf {
        self.file("output_mode_override")
    }

    /// One-shot profile override, from the profile modifier key or `--profile`.
    pub fn profile_override(&self) -> PathBuf {
        self.file("profile_override")
    }

    /// One-shot boolean override; the daemon reads these by name.
    pub fn bool_override(&self, name: &str) -> PathBuf {
        self.file(&format!("{}_override", name))
    }

    /// One-shot model override.
    pub fn model_override(&self) -> PathBuf {
        self.file("model_override")
    }

    pub fn meeting_start(&self) -> PathBuf {
        self.file("meeting_start")
    }

    pub fn meeting_start_diarization(&self) -> PathBuf {
        self.file("meeting_start_diarization")
    }

    pub fn meeting_stop(&self) -> PathBuf {
        self.file("meeting_stop")
    }

    pub fn meeting_pause(&self) -> PathBuf {
        self.file("meeting_pause")
    }

    pub fn meeting_resume(&self) -> PathBuf {
        self.file("meeting_resume")
    }

    /// Current meeting state, written by the daemon for external readers.
    pub fn meeting_state(&self) -> PathBuf {
        self.file("meeting_state")
    }

    // === OSD suppression marker ===

    pub fn set_osd_suppressed(&self, suppressed: bool) {
        set_osd_suppressed_at(&self.osd_suppressed(), suppressed)
    }

    // === Cancel ===

    /// Check if cancel has been requested (via file trigger)
    pub fn check_cancel_requested(&self) -> bool {
        let cancel_file = self.cancel();
        if cancel_file.exists() {
            // Remove the file to acknowledge the cancel
            let _ = std::fs::remove_file(&cancel_file);
            true
        } else {
            false
        }
    }

    /// Clean up any stale cancel file on startup
    pub fn cleanup_cancel_file(&self) {
        let cancel_file = self.cancel();
        if cancel_file.exists() {
            let _ = std::fs::remove_file(&cancel_file);
        }
    }

    // === Output mode override ===

    /// Read and consume the output mode override file
    /// Format: "type", "clipboard", "paste", "file", or "file:/path/to/file.txt"
    pub fn read_output_mode_override(&self) -> Option<OutputOverride> {
        let override_file = self.output_mode_override();
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
    pub fn cleanup_output_mode_override(&self) {
        let override_file = self.output_mode_override();
        let _ = std::fs::remove_file(&override_file);
    }

    /// Where this recording's transcript would land, without consuming anything.
    ///
    /// The output override is only read once transcription succeeds, so the paths
    /// that bail out earlier (too short, no speech) do not know the transcript
    /// path and cannot report an outcome. This peeks the pending override so those
    /// paths can still publish a completion sidecar, leaving the sentinel for the
    /// normal consuming read.
    pub fn peek_file_output_path(&self, config: &Config) -> Option<PathBuf> {
        let override_file = self.output_mode_override();
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

    // === Profile override ===

    /// Read and consume the profile override file
    /// Returns the profile name if the file exists and is valid, None otherwise
    pub fn read_profile_override(&self) -> Option<String> {
        let profile_file = self.profile_override();
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
    pub fn cleanup_profile_override(&self) {
        let profile_file = self.profile_override();
        let _ = std::fs::remove_file(&profile_file);
    }

    /// Write a profile override file so the daemon uses the named profile for
    /// post-processing. Same mechanism as `voxtype record start --profile <name>`.
    pub fn write_profile_override(&self, profile_name: &str) {
        let profile_file = self.profile_override();
        if let Err(e) = std::fs::write(&profile_file, profile_name) {
            tracing::warn!("Failed to write profile override: {}", e);
        } else {
            tracing::info!("Profile modifier activated: {}", profile_name);
        }
    }

    // === Boolean overrides ===

    /// Read and consume a boolean override file from the runtime directory.
    /// Returns Some(true) or Some(false) if the file exists and is valid, None otherwise.
    pub fn read_bool_override(&self, name: &str) -> Option<bool> {
        let override_file = self.bool_override(name);
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
    pub fn cleanup_bool_override(&self, name: &str) {
        let override_file = self.bool_override(name);
        let _ = std::fs::remove_file(&override_file);
    }

    // === Model override ===

    /// Read and consume the model override file
    /// Returns the model name if the file exists, None otherwise
    pub fn read_model_override(&self) -> Option<String> {
        let override_file = self.model_override();
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
    pub fn cleanup_model_override(&self) {
        let override_file = self.model_override();
        let _ = std::fs::remove_file(&override_file);
    }

    // === Meeting triggers ===

    /// Check for meeting start command (via file trigger)
    pub fn check_meeting_start(&self) -> Option<MeetingStartTrigger> {
        let start_file = self.meeting_start();
        if !start_file.exists() {
            return None;
        }

        let title = read_trimmed_nonempty(&start_file);

        // Diarization override is written by the CLI handler before the start
        // trigger. Re-validate against the allowlist (see
        // `validate_diarization_override` for the rationale).
        let diarization_file = self.meeting_start_diarization();
        let diarization =
            read_trimmed_nonempty(&diarization_file).and_then(validate_diarization_override);
        let _ = std::fs::remove_file(&diarization_file);

        // Remove the start trigger last to acknowledge the command.
        let _ = std::fs::remove_file(&start_file);

        Some(MeetingStartTrigger { title, diarization })
    }

    /// Check for meeting stop command (via file trigger)
    pub fn check_meeting_stop(&self) -> bool {
        let stop_file = self.meeting_stop();
        if stop_file.exists() {
            let _ = std::fs::remove_file(&stop_file);
            true
        } else {
            false
        }
    }

    /// Check for meeting pause command (via file trigger)
    pub fn check_meeting_pause(&self) -> bool {
        let pause_file = self.meeting_pause();
        if pause_file.exists() {
            let _ = std::fs::remove_file(&pause_file);
            true
        } else {
            false
        }
    }

    /// Check for meeting resume command (via file trigger)
    pub fn check_meeting_resume(&self) -> bool {
        let resume_file = self.meeting_resume();
        if resume_file.exists() {
            let _ = std::fs::remove_file(&resume_file);
            true
        } else {
            false
        }
    }

    /// Clean up any stale meeting command files on startup
    pub fn cleanup_meeting_files(&self) {
        for file in [
            self.meeting_start(),
            self.meeting_start_diarization(),
            self.meeting_stop(),
            self.meeting_pause(),
            self.meeting_resume(),
        ] {
            if file.exists() {
                let _ = std::fs::remove_file(&file);
            }
        }
    }
}

/// Path-taking half of the OSD suppression marker, so the marker lifecycle is
/// testable without a `RuntimePaths`.
pub fn set_osd_suppressed_at(path: &Path, suppressed: bool) {
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

/// Output mode override result, which may include a file path for file mode
#[derive(Debug, PartialEq)]
pub enum OutputOverride {
    Mode(OutputMode),
    FileWithPath(PathBuf),
}

/// A pending meeting-start trigger with optional title and diarization override.
pub struct MeetingStartTrigger {
    pub title: Option<String>,
    pub diarization: Option<String>,
}

/// Read a file and return its trimmed contents, or None if missing or empty.
///
/// Logs read failures so transient FS / permission errors aren't silent: a
/// trigger file existing but being unreadable previously looked identical to
/// "no file" and would then be consumed by the caller's remove_file.
fn read_trimmed_nonempty(path: &Path) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, RuntimePaths) {
        let dir = tempfile::TempDir::new().unwrap();
        let paths = RuntimePaths::new(dir.path());
        (dir, paths)
    }

    #[test]
    fn every_name_sits_in_the_runtime_directory() {
        let (dir, paths) = paths();
        for path in [
            paths.osd_suppressed(),
            paths.cancel(),
            paths.output_mode_override(),
            paths.profile_override(),
            paths.bool_override("auto_submit"),
            paths.model_override(),
            paths.meeting_start(),
            paths.meeting_start_diarization(),
            paths.meeting_stop(),
            paths.meeting_pause(),
            paths.meeting_resume(),
            paths.meeting_state(),
            paths.lock(),
            paths.version(),
            paths.level_socket(),
        ] {
            assert_eq!(path.parent(), Some(dir.path()), "{}", path.display());
        }
    }

    #[test]
    fn names_match_the_files_the_cli_writes() {
        // These strings are the contract with `voxtype record` / `voxtype
        // meeting`: a rename here without a rename there silently stops the
        // daemon from seeing the command.
        let (_dir, paths) = paths();
        assert!(paths.cancel().ends_with("cancel"));
        assert!(paths
            .bool_override("auto_submit")
            .ends_with("auto_submit_override"));
        assert!(paths
            .bool_override("shift_enter")
            .ends_with("shift_enter_override"));
        assert!(paths.bool_override("no_osd").ends_with("no_osd_override"));
        assert!(paths.lock().ends_with("voxtype.lock"));
        assert!(paths.level_socket().ends_with("audio.sock"));
    }

    #[test]
    fn cancel_is_consumed_once() {
        let (_dir, paths) = paths();
        assert!(!paths.check_cancel_requested());
        std::fs::write(paths.cancel(), "").unwrap();
        assert!(paths.check_cancel_requested());
        assert!(!paths.check_cancel_requested());
        assert!(!paths.cancel().exists());
    }

    #[test]
    fn bool_override_is_consumed_once_and_rejects_junk() {
        let (_dir, paths) = paths();
        assert_eq!(paths.read_bool_override("auto_submit"), None);

        std::fs::write(paths.bool_override("auto_submit"), "true").unwrap();
        assert_eq!(paths.read_bool_override("auto_submit"), Some(true));
        assert_eq!(paths.read_bool_override("auto_submit"), None);

        std::fs::write(paths.bool_override("shift_enter"), "maybe").unwrap();
        assert_eq!(paths.read_bool_override("shift_enter"), None);
        assert!(!paths.bool_override("shift_enter").exists());
    }

    #[test]
    fn output_mode_override_carries_a_file_path() {
        let (_dir, paths) = paths();
        std::fs::write(paths.output_mode_override(), "file:/tmp/x.txt").unwrap();
        assert_eq!(
            paths.read_output_mode_override(),
            Some(OutputOverride::FileWithPath(PathBuf::from("/tmp/x.txt")))
        );
    }

    #[test]
    fn meeting_start_reads_title_and_validates_diarization() {
        let (_dir, paths) = paths();
        std::fs::write(paths.meeting_start(), "Standup\n").unwrap();
        std::fs::write(paths.meeting_start_diarization(), "ml").unwrap();

        let trigger = paths.check_meeting_start().expect("trigger");
        assert_eq!(trigger.title.as_deref(), Some("Standup"));
        assert_eq!(trigger.diarization.as_deref(), Some("ml"));
        assert!(!paths.meeting_start().exists());
        assert!(!paths.meeting_start_diarization().exists());

        std::fs::write(paths.meeting_start(), "Standup\n").unwrap();
        std::fs::write(paths.meeting_start_diarization(), "nonsense").unwrap();
        let trigger = paths.check_meeting_start().expect("trigger");
        assert_eq!(trigger.diarization, None);
    }

    /// #636: the OSD suppression marker is created and removed, never
    /// rewritten, because both OSD frontends treat "file exists" as the
    /// signal. Absent is the overwhelmingly common case and must be cheap.
    #[test]
    fn osd_suppression_marker_lifecycle() {
        let (dir, paths) = paths();
        let marker = dir.path().join("osd_suppressed");
        assert!(!marker.exists(), "marker must start absent");

        paths.set_osd_suppressed(true);
        assert!(marker.exists(), "marker not written");

        // Idempotent: setting it twice is not an error and leaves one file.
        paths.set_osd_suppressed(true);
        assert!(marker.exists());

        paths.set_osd_suppressed(false);
        assert!(!marker.exists(), "marker not cleared");

        // Clearing an already-absent marker must not panic or error.
        paths.set_osd_suppressed(false);
        assert!(!marker.exists());
    }

    #[test]
    fn meeting_command_files_are_cleaned_up() {
        let (_dir, paths) = paths();
        std::fs::write(paths.meeting_stop(), "").unwrap();
        std::fs::write(paths.meeting_pause(), "").unwrap();
        std::fs::write(paths.meeting_resume(), "").unwrap();
        paths.cleanup_meeting_files();
        assert!(!paths.meeting_stop().exists());
        assert!(!paths.meeting_pause().exists());
        assert!(!paths.meeting_resume().exists());
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
}
