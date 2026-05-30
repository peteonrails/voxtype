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
/// is missing, unreadable, or doesn't contain a valid integer.
///
/// Note: this only proves a PID was *written*; the process may have died
/// since. Pair with `is_running` (or call `read_pid_if_alive`) when you
/// need a liveness guarantee.
pub fn read_pid() -> Option<i32> {
    let pid_str = std::fs::read_to_string(pid_file_path()).ok()?;
    pid_str.trim().parse().ok()
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
}
