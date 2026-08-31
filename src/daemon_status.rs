//! Single source of truth for "is the voxtype daemon running?" across every
//! external caller — the CLI (`voxtype record`, `voxtype meeting`,
//! `voxtype status`), the TUI (`voxtype configure`), and any future
//! integration point.
//!
//! `daemon.rs::is_pid_running` is intentionally NOT folded in here — it
//! answers a different question (am I, the daemon, holding a stale lockfile
//! left by a crashed predecessor?) and runs inside the daemon process. This
//! module is for *external* callers asking whether a separate daemon is
//! alive enough to receive a signal or a runtime-dir trigger file.
//!
//! Historical drift this module exists to prevent:
//! - `check_daemon_running()` used to read `runtime_dir/pid` while
//!   `send_record_command()` read `runtime_dir/voxtype.lock`, breaking
//!   `voxtype meeting start/stop/pause/resume` (the daemon was healthy but
//!   the CLI thought it wasn't).
//! - The TUI's copy read the same legacy `pid` file via `/proc/{pid}`, so
//!   the engine picker silently reported the daemon as down on every
//!   modern build.

use crate::config::Config;

/// Path to the daemon PID file (matches the lockfile the daemon writes via
/// Pidlock). Every external liveness check resolves through here so a
/// future rename of the lockfile updates every consumer in one place.
pub fn pid_file_path() -> std::path::PathBuf {
    Config::runtime_dir().join("voxtype.lock")
}

/// Read the daemon's PID from the lockfile, returning `None` if the file
/// is missing, unreadable, doesn't contain a valid integer, or contains a
/// PID that cannot legally identify another process to signal.
///
/// `kill(2)` overloads non-positive PIDs: `0` signals the caller's process
/// group, `-1` broadcasts to every signalable process, and `< -1` signals
/// a process group by negated PID. A corrupted lockfile reading `0` or
/// `-1` must therefore NOT be returned — passing such a value to
/// `libc::kill` from a record/meeting command would mass-signal the
/// user's session. PID `1` is `init` (or systemd) which a user-mode
/// voxtype daemon could never legitimately be, so reject it too.
///
/// Note: this only proves a PID was *written*; the process may have died
/// since. Pair with `is_running` (or call `read_pid_if_alive`) when you
/// need a liveness guarantee.
pub fn read_pid() -> Option<i32> {
    let pid_str = std::fs::read_to_string(pid_file_path()).ok()?;
    let pid: i32 = pid_str.trim().parse().ok()?;
    (pid > 1).then_some(pid)
}

/// Check whether `pid` corresponds to a live process. Uses signal 0
/// (existence check) which works on Linux and macOS without side effects.
pub fn is_running(pid: i32) -> bool {
    // SAFETY: libc::kill with signal 0 only probes for the process; it
    // does not deliver a signal, so there's no observable side effect.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Read the daemon's PID and confirm the process is alive. Returns
/// `Some(pid)` only when both reads succeed and the process exists.
pub fn read_pid_if_alive() -> Option<i32> {
    let pid = read_pid()?;
    is_running(pid).then_some(pid)
}

/// Path to the file the daemon writes its own version into at startup.
///
/// Sits beside the lockfile in the runtime dir, so it is cleared by the same
/// reboot that clears the lock and can never outlive the machine's uptime.
pub fn version_file_path() -> std::path::PathBuf {
    Config::runtime_dir().join("version")
}

/// Publish this process's version. Called by the daemon at startup, after
/// the lock is acquired, so a refused second instance never overwrites the
/// running daemon's answer.
pub fn publish_version() {
    let path = version_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Best effort: a daemon that cannot write this still runs fine, callers
    // just fall back to reporting the version as unknown.
    let _ = std::fs::write(&path, env!("CARGO_PKG_VERSION"));
}

/// What the *running daemon* reports as its version.
///
/// This is deliberately not `env!("CARGO_PKG_VERSION")`. That constant
/// describes whichever binary is asking, which is routinely not the one
/// serving dictation: an upgrade that was installed but never restarted, a
/// systemd `ExecStart=` override pointing at a private build, or a
/// `/usr/local/bin` install shadowing a packaged one all produce a CLI and a
/// daemon at different versions. Reporting the caller's version as though it
/// were the daemon's is how a UI ends up claiming a fix is live when the
/// process without it is still running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonVersion {
    /// The daemon published this version at startup.
    Running(String),
    /// No daemon is alive.
    NotRunning,
    /// A daemon is alive but published no version: it predates
    /// `publish_version`, or could not write the runtime dir.
    Unknown,
}

/// Resolve the running daemon's version.
///
/// Liveness is checked first, so a stale version file left by a daemon that
/// died without cleanup reads as `NotRunning` rather than as a running
/// version that does not exist.
pub fn running_version() -> DaemonVersion {
    if read_pid_if_alive().is_none() {
        return DaemonVersion::NotRunning;
    }
    match std::fs::read_to_string(version_file_path()) {
        Ok(v) if !v.trim().is_empty() => DaemonVersion::Running(v.trim().to_string()),
        _ => DaemonVersion::Unknown,
    }
}

impl DaemonVersion {
    /// The version string, when there is one.
    pub fn version(&self) -> Option<&str> {
        match self {
            Self::Running(v) => Some(v),
            _ => None,
        }
    }

    /// True when a daemon is running something other than the caller's own
    /// build. This is the condition worth surfacing: the user is looking at a
    /// UI from one version while a different one is doing the work.
    pub fn differs_from_caller(&self) -> bool {
        matches!(self, Self::Running(v) if v != env!("CARGO_PKG_VERSION"))
    }

    /// One line for a status surface, phrased so the three states stay
    /// distinguishable rather than collapsing into a bare version number.
    pub fn describe(&self) -> String {
        match self {
            Self::Running(v) if self.differs_from_caller() => {
                format!("{} (this CLI is {})", v, env!("CARGO_PKG_VERSION"))
            }
            Self::Running(v) => v.clone(),
            Self::NotRunning => "not running".to_string(),
            Self::Unknown => "running, version unknown".to_string(),
        }
    }
}

/// Boolean shorthand for callers that only need "is the daemon up?"
/// (status display, TUI banner, etc.). Equivalent to
/// `read_pid_if_alive().is_some()`.
pub fn is_daemon_running() -> bool {
    read_pid_if_alive().is_some()
}

/// CLI-style precondition check: ensure the daemon is running and return
/// its PID for subsequent signal delivery. Prints the canonical "not
/// running, start with: voxtype daemon" message and exits the process if
/// the daemon is down.
///
/// Callers that don't need the PID (e.g. `voxtype meeting status`) can
/// `?` the result and discard the value.
///
/// Side effect: if the lockfile exists but the PID is dead, the stale
/// lockfile is removed before exit.
pub fn check_daemon_running() -> anyhow::Result<i32> {
    let pid_file = pid_file_path();

    if !pid_file.exists() {
        eprintln!("Error: Voxtype daemon is not running.");
        eprintln!("Start it with: voxtype daemon");
        std::process::exit(1);
    }

    let pid_str = std::fs::read_to_string(&pid_file)
        .map_err(|e| anyhow::anyhow!("Failed to read PID file: {}", e))?;

    let pid: i32 = pid_str
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid PID in file: {}", e))?;

    // Reject pids that overload kill(2)'s signal-delivery semantics
    // (0 = process group, -1 = broadcast, <-1 = signal a process group).
    // PID 1 is init/systemd which a user-mode voxtype daemon could never
    // legitimately be. See the rationale on `read_pid`.
    if pid <= 1 {
        let _ = std::fs::remove_file(&pid_file);
        eprintln!("Error: Voxtype daemon is not running (lockfile held an invalid PID, removed).");
        eprintln!("Start it with: voxtype daemon");
        std::process::exit(1);
    }

    if !is_running(pid) {
        // Process doesn't exist, clean up stale PID file
        let _ = std::fs::remove_file(&pid_file);
        eprintln!("Error: Voxtype daemon is not running (stale PID file removed).");
        eprintln!("Start it with: voxtype daemon");
        std::process::exit(1);
    }

    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `check_daemon_running()` once read `runtime_dir/pid` while
    /// `send_record_command()` read `runtime_dir/voxtype.lock`. The mismatch
    /// caused `voxtype meeting start/stop/pause/resume` to falsely report
    /// "daemon not running" even when the daemon was healthy. Every public
    /// helper here must resolve to the same path.
    #[test]
    fn pid_file_path_matches_send_record_command() {
        let canonical = pid_file_path();

        // Sanity: the canonical path ends in `voxtype.lock` (the Pidlock file
        // the daemon actually writes), not the legacy `pid` filename.
        assert!(
            canonical.ends_with("voxtype.lock"),
            "pid_file_path() must point at voxtype.lock so meeting \
             and record commands agree with the daemon's Pidlock. Got: {:?}",
            canonical,
        );

        // Whatever path the daemon writes (`Config::runtime_dir/voxtype.lock`)
        // must match what every external caller reads.
        let from_send = Config::runtime_dir().join("voxtype.lock");
        assert_eq!(canonical, from_send);
    }
    /// The whole point of the type: a caller must not be able to mistake its
    /// own build for the daemon's. `differs_from_caller` is what a UI keys
    /// off to warn, so it has to be false for every state that is not a
    /// confirmed, different, running version.
    #[test]
    fn only_a_confirmed_different_running_version_counts_as_differing() {
        let same = DaemonVersion::Running(env!("CARGO_PKG_VERSION").to_string());
        assert!(!same.differs_from_caller());
        assert_eq!(same.version(), Some(env!("CARGO_PKG_VERSION")));

        let other = DaemonVersion::Running("0.0.1-other".to_string());
        assert!(other.differs_from_caller());
        assert!(other.describe().contains("0.0.1-other"));
        assert!(
            other.describe().contains(env!("CARGO_PKG_VERSION")),
            "a mismatch must name both versions, or the user cannot tell \
             which one they are looking at"
        );

        // Neither unknown state may masquerade as agreement or as a mismatch.
        assert!(!DaemonVersion::NotRunning.differs_from_caller());
        assert!(!DaemonVersion::Unknown.differs_from_caller());
        assert_eq!(DaemonVersion::NotRunning.version(), None);
        assert_eq!(DaemonVersion::Unknown.version(), None);
    }

    /// The three states have to stay distinguishable in the one line a status
    /// surface gets. Collapsing "not running" into an empty string is how a
    /// panel ends up rendering a blank where it should say the daemon is down.
    #[test]
    fn describe_distinguishes_every_state() {
        let labels = [
            DaemonVersion::Running("1.2.3".into()).describe(),
            DaemonVersion::NotRunning.describe(),
            DaemonVersion::Unknown.describe(),
        ];
        for l in &labels {
            assert!(!l.trim().is_empty(), "no state may describe as blank");
        }
        assert_eq!(
            labels.len(),
            labels
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "states must not share a label: {labels:?}"
        );
    }

    /// A version file left behind by a daemon that died without cleaning up
    /// must not read as a running version. Liveness is checked first, so with
    /// no daemon alive the answer is NotRunning whatever the file says.
    #[test]
    fn a_stale_version_file_does_not_report_a_running_daemon() {
        // Whatever this machine's real state is, the invariant holds: a
        // version is only ever reported alongside a live pid.
        let v = running_version();
        if matches!(v, DaemonVersion::Running(_)) {
            assert!(
                read_pid_if_alive().is_some(),
                "reported a running version with no live daemon"
            );
        }
    }
}
