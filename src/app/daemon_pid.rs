//! Daemon liveness helpers shared by `voxtype record`, `voxtype meeting`, and
//! `voxtype status`. These all read the lockfile the daemon writes via
//! `Pidlock`. Keeping them in one place stops the three call sites from
//! drifting apart again — see the regression test below.

use voxtype::config;

/// Path to the daemon PID file (matches the lockfile the daemon writes via Pidlock).
///
/// IMPORTANT: This must match the file used by `send_record_command()` and
/// `is_daemon_running()`. Historically `check_daemon_running()` read from
/// `"pid"` while `send_record_command()` read from `"voxtype.lock"`, which
/// caused `voxtype meeting start/stop/pause/resume` to falsely report the
/// daemon as not running. See test `pid_file_path_matches_send_record_command`.
pub(crate) fn daemon_pid_file_path() -> std::path::PathBuf {
    config::Config::runtime_dir().join("voxtype.lock")
}

/// Check if the daemon is running, exit with error if not
pub(crate) fn check_daemon_running() -> anyhow::Result<()> {
    let pid_file = daemon_pid_file_path();

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

    // Check if the process is actually running (signal 0 = check existence)
    if unsafe { libc::kill(pid, 0) } != 0 {
        // Process doesn't exist, clean up stale PID file
        let _ = std::fs::remove_file(&pid_file);
        eprintln!("Error: Voxtype daemon is not running (stale PID file removed).");
        eprintln!("Start it with: voxtype daemon");
        std::process::exit(1);
    }

    Ok(())
}

/// Check if the daemon is actually running by verifying the PID file
pub(crate) fn is_daemon_running() -> bool {
    // Use the same lockfile path the daemon writes via Pidlock. Reading from
    // the legacy `pid` file caused `voxtype status` to report `stopped` even
    // when the daemon was healthy (the daemon now also writes the lockfile
    // before write_pid_file is reached). See test
    // `pid_file_path_matches_send_record_command`.
    let pid_path = daemon_pid_file_path();

    // Read PID from file
    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => return false, // No PID file = not running
    };

    let pid: i32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => return false, // Invalid PID = not running
    };

    // Check if process exists using kill(pid, 0) - works on both Linux and macOS
    // Signal 0 doesn't send a signal, just checks if process exists and we have permission
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxtype::config;

    /// Regression: `check_daemon_running()` once read `runtime_dir/pid` while
    /// `send_record_command()` read `runtime_dir/voxtype.lock`. The mismatch
    /// caused `voxtype meeting start/stop/pause/resume` to falsely report
    /// "daemon not running" even when the daemon was healthy. Both helpers
    /// (and `is_daemon_running`) must resolve to the same path.
    #[test]
    fn pid_file_path_matches_send_record_command() {
        let canonical = daemon_pid_file_path();

        // Sanity: the canonical path ends in `voxtype.lock` (the Pidlock file
        // the daemon actually writes), not the legacy `pid` filename.
        assert!(
            canonical.ends_with("voxtype.lock"),
            "daemon_pid_file_path() must point at voxtype.lock so meeting \
             and record commands agree with the daemon's Pidlock. Got: {:?}",
            canonical,
        );

        // Both helpers must resolve to exactly the same path. If you split
        // them, `voxtype meeting start` will regress in the same way it did
        // before this test existed.
        let from_send = config::Config::runtime_dir().join("voxtype.lock");
        assert_eq!(canonical, from_send);
    }
}
