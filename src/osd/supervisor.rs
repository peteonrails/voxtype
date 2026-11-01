//! Daemon-side supervisor for the OSD frontend.
//!
//! When `[osd] enabled = true`, the daemon spawns `voxtype-osd` as a child
//! process and keeps it running for the lifetime of the daemon. The OSD
//! frontend connects to the level socket the daemon owns, so spawning the
//! child after the socket is bound avoids a startup race where the
//! frontend hits ENOENT and waits its full reconnect interval.
//!
//! Restart policy: respawn with exponential backoff (1s → 30s cap), reset
//! to 1s after any run that lasted >60s. Three rapid back-to-back failures
//! suggest the binary is broken or missing — log a clear error and stop
//! retrying so the daemon doesn't busy-loop.
//!
//! On shutdown, drop the supervisor handle. `tokio::process::Command` with
//! `kill_on_drop(true)` sends SIGKILL to the child when the parent task is
//! aborted, which is fine for the OSD: it owns no daemon-shared state and
//! the frontend's own `--reconnect-secs` loop handles a clean re-attach
//! after `systemctl --user restart voxtype`.
//!
//! `kill_on_drop` only runs when the supervisor task is dropped, which a
//! daemon that is killed or crashes never gets to do, and every such exit
//! used to leave a `qs` orphan drawing its own pill over the next daemon's.
//! On Linux the child also asks the kernel for SIGTERM when the daemon
//! dies (`PR_SET_PDEATHSIG`); that survives the dispatcher's `exec` into
//! the frontend, so it covers the process actually drawing the OSD.
//!
//! ### `VOXTYPE_OSD_SUPERVISED`
//!
//! We set `VOXTYPE_OSD_SUPERVISED=1` on the child's environment so the
//! dispatcher (`voxtype-osd`) knows it was spawned by the daemon rather
//! than invoked interactively. When dispatching to the Quickshell
//! launcher the dispatcher then appends `--no-daemonize`, which tells
//! the launcher to skip `qs -d`. Without this, qs would fork off into
//! its own session, the supervisor's child slot would exit immediately,
//! `kill_on_drop` would have nothing to kill, and the supervisor would
//! respawn in a loop thinking the child died (see issue #395).

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::task::JoinHandle;

const OSD_BINARY: &str = "voxtype-osd";
const RESTART_MIN: Duration = Duration::from_secs(1);
const RESTART_MAX: Duration = Duration::from_secs(30);
const HEALTHY_RUN: Duration = Duration::from_secs(60);
const RAPID_FAIL_THRESHOLD: u32 = 3;
const RAPID_FAIL_WINDOW: Duration = Duration::from_secs(5);

fn resolve_osd_binary() -> PathBuf {
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            let sibling = dir.join(OSD_BINARY);
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    PathBuf::from(OSD_BINARY)
}

/// Have the kernel send the OSD child SIGTERM when the daemon exits, however
/// it exits.
#[cfg(target_os = "linux")]
fn die_with_daemon(cmd: &mut Command) {
    let daemon_pid = std::process::id() as libc::pid_t;
    // SAFETY: the closure runs in the forked child before exec and only
    // makes async-signal-safe calls (prctl, getppid, raise).
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The daemon may have died between fork and prctl; the death
            // signal would then never come.
            if libc::getppid() != daemon_pid {
                libc::raise(libc::SIGTERM);
            }
            Ok(())
        });
    }
}

/// Spawn a tokio task that supervises `voxtype-osd`. The returned handle's
/// drop kills the child via `kill_on_drop`. Holding the handle keeps the
/// supervisor alive for the daemon's lifetime.
pub fn spawn() -> JoinHandle<()> {
    tokio::spawn(supervise())
}

async fn supervise() {
    let mut backoff = RESTART_MIN;
    let mut rapid_fails: u32 = 0;
    let mut rapid_window_start = Instant::now();

    loop {
        let started = Instant::now();
        let osd_binary = resolve_osd_binary();
        let mut cmd = Command::new(&osd_binary);
        cmd.kill_on_drop(true);
        // Tell the dispatcher this child is supervised. The dispatcher
        // uses this to suppress qs's daemonize fork when handing off to
        // the Quickshell launcher (otherwise kill_on_drop has nothing
        // to kill — see #395).
        cmd.env("VOXTYPE_OSD_SUPERVISED", "1");
        #[cfg(target_os = "linux")]
        die_with_daemon(&mut cmd);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Failed to spawn `{}`: {}. OSD will not be displayed. \
                     Install the OSD frontend or set `[osd] enabled = false`.",
                    osd_binary.display(),
                    e
                );
                return;
            }
        };

        tracing::info!("OSD child started (pid {:?})", child.id());

        let exit = match child.wait().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("OSD child wait error: {}", e);
                return;
            }
        };

        let ran_for = started.elapsed();
        tracing::info!("OSD child exited: status={} ran_for={:?}", exit, ran_for);

        if ran_for >= HEALTHY_RUN {
            backoff = RESTART_MIN;
            rapid_fails = 0;
        } else {
            if rapid_window_start.elapsed() > RAPID_FAIL_WINDOW {
                rapid_fails = 0;
                rapid_window_start = Instant::now();
            }
            rapid_fails += 1;
            if rapid_fails >= RAPID_FAIL_THRESHOLD {
                tracing::error!(
                    "OSD child exited {} times within {:?}. Giving up — check that \
                     voxtype-osd-gtk4 (or voxtype-osd-native) is installed and that \
                     the [osd] config is valid. The daemon will keep running.",
                    rapid_fails,
                    RAPID_FAIL_WINDOW
                );
                return;
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RESTART_MAX);
    }
}
