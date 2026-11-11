//! `[meeting] post_meeting_command`: a shell command run once a meeting has
//! stopped and its folder is final.
//!
//! The command runs through `sh -c` in the meeting folder, with
//! `VOXTYPE_MEETING_ID`, `VOXTYPE_MEETING_DIR` and `VOXTYPE_MEETING_MANIFEST`
//! set. It runs in its own process group, off the daemon's event loop, and
//! is killed (with anything it started) when it runs past its timeout. Its
//! output and exit status go to the daemon log; nothing it does can stop or
//! hold up the daemon.

use crate::meeting::{MeetingId, MeetingStorage, StorageConfig};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// Bytes of the command's output kept for the log.
const LOG_OUTPUT_LIMIT: usize = 2000;

/// How a post-meeting command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookOutcome {
    /// Exited with status 0
    Succeeded,
    /// Exited with this status (`None` when killed by a signal)
    Failed(Option<i32>),
    /// Ran past its timeout and was killed
    TimedOut,
    /// Could not be started
    NotStarted(String),
}

/// `[meeting] post_meeting_command` and its timeout.
#[derive(Debug, Clone, Default)]
pub struct PostMeetingSettings {
    pub command: Option<String>,
    pub timeout: Duration,
}

impl PostMeetingSettings {
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            command: config
                .meeting
                .post_meeting_command
                .clone()
                .filter(|c| !c.trim().is_empty()),
            timeout: Duration::from_secs(config.meeting.post_meeting_timeout_secs),
        }
    }
}

/// What runs after a meeting stops: rewrite its manifest, then run the
/// post-meeting command if one is configured. Built by
/// [`MeetingDaemon::post_meeting`](crate::meeting::MeetingDaemon::post_meeting)
/// and spawned by the daemon, after the auto-summary when there is one, so
/// the command sees the finished folder.
pub struct PostMeeting {
    pub storage: StorageConfig,
    pub meeting_id: MeetingId,
    pub settings: PostMeetingSettings,
}

impl PostMeeting {
    /// Rewrite the manifest and run the command. Returns how the command
    /// ended, or `None` when there was none to run.
    pub async fn run(self) -> Option<HookOutcome> {
        let written = MeetingStorage::open(self.storage).and_then(|storage| {
            let manifest = storage.write_manifest(&self.meeting_id)?;
            let dir = storage.get_meeting_path(&self.meeting_id)?;
            Ok((dir, manifest))
        });
        let command = self.settings.command?;
        let (dir, manifest) = match written {
            Ok(paths) => paths,
            Err(e) => {
                tracing::warn!(
                    "post_meeting_command not run for {}: {}",
                    self.meeting_id,
                    e
                );
                return Some(HookOutcome::NotStarted(e.to_string()));
            }
        };
        Some(
            run_command(
                &command,
                &self.meeting_id,
                &dir,
                &manifest,
                self.settings.timeout,
            )
            .await,
        )
    }
}

/// Run `command` for a finished meeting, logging how it went.
pub async fn run_command(
    command: &str,
    meeting_id: &MeetingId,
    dir: &Path,
    manifest: &PathBuf,
    timeout: Duration,
) -> HookOutcome {
    tracing::info!(
        "Running post_meeting_command for {}: {}",
        meeting_id,
        command
    );
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(dir)
        .env("VOXTYPE_MEETING_ID", meeting_id.to_string())
        .env("VOXTYPE_MEETING_DIR", dir)
        .env("VOXTYPE_MEETING_MANIFEST", manifest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Its own group, so a timeout kills whatever it started too, and
        // a signal meant for the daemon's terminal does not reach it.
        .process_group(0);
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::warn!("post_meeting_command could not start: {}", e);
            return HookOutcome::NotStarted(e.to_string());
        }
    };
    let pid = child.id();

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let stdout = log_text(&output.stdout);
            let stderr = log_text(&output.stderr);
            if output.status.success() {
                tracing::info!(
                    "post_meeting_command finished for {}{}",
                    meeting_id,
                    if stdout.is_empty() {
                        String::new()
                    } else {
                        format!(": {stdout}")
                    }
                );
                HookOutcome::Succeeded
            } else {
                tracing::warn!(
                    "post_meeting_command failed for {} ({}): {}",
                    meeting_id,
                    output.status,
                    if stderr.is_empty() { &stdout } else { &stderr }
                );
                HookOutcome::Failed(output.status.code())
            }
        }
        Ok(Err(e)) => {
            tracing::warn!("post_meeting_command for {}: {}", meeting_id, e);
            HookOutcome::Failed(None)
        }
        Err(_) => {
            if let Some(pid) = pid {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            tracing::warn!(
                "post_meeting_command for {} ran past {}s and was killed",
                meeting_id,
                timeout.as_secs()
            );
            HookOutcome::TimedOut
        }
    }
}

/// The command's output, trimmed and cut to a loggable length.
fn log_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    match text.char_indices().nth(LOG_OUTPUT_LIMIT) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text.to_string(),
    }
}
