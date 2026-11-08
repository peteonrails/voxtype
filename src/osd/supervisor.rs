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
//!
//! ### Frontend fallback notice
//!
//! Before the first spawn the supervisor runs the same frontend selection
//! as the dispatcher (`crate::osd::select`), probing next to the
//! dispatcher it is about to launch. When an explicitly configured
//! frontend isn't available and another one stands in, or when no
//! frontend is installed at all, it logs a warning and sends one desktop
//! notification per daemon start. Respawns don't repeat it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::osd::config::OsdFrontend;
use crate::osd::select::{self, Environment};

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

/// Spawn a tokio task that supervises `voxtype-osd`. The returned handle's
/// drop kills the child via `kill_on_drop`. Holding the handle keeps the
/// supervisor alive for the daemon's lifetime.
///
/// `configured` is the daemon's `[osd] frontend`; `VOXTYPE_OSD_FRONTEND`
/// in the daemon's environment overrides it, as it does for the
/// dispatcher, which inherits that environment.
pub fn spawn(configured: OsdFrontend) -> JoinHandle<()> {
    tokio::spawn(supervise(configured))
}

/// Run the dispatcher's frontend selection from the daemon's side and
/// report a substitution once. Probes the directory of the dispatcher
/// that will be launched, then PATH, mirroring the dispatcher's own
/// lookup (it resolves its symlinks before looking next to itself).
async fn notify_frontend_fallback(configured: OsdFrontend, osd_binary: &Path) {
    let requested = std::env::var("VOXTYPE_OSD_FRONTEND")
        .ok()
        .and_then(|s| OsdFrontend::parse_str(&s))
        .unwrap_or(configured);
    let dispatcher = if osd_binary.is_absolute() {
        Some(osd_binary.to_path_buf())
    } else {
        which::which(osd_binary).ok()
    };
    // No dispatcher at all is reported by the spawn failure below.
    let Some(dir) = dispatcher
        .and_then(|d| std::fs::canonicalize(d).ok())
        .and_then(|d| d.parent().map(Path::to_path_buf))
    else {
        return;
    };
    let find = |name: &str| {
        let candidate = dir.join(name);
        if candidate.is_file() {
            Some(candidate)
        } else {
            which::which(name).ok()
        }
    };
    let selection = select::select(requested, Environment::probe(), find);
    match (selection.notice(), selection.chosen_frontend()) {
        (Some(notice), _) => {
            tracing::warn!("{}: {}", notice.title, notice.body);
            // Same 5 s sync path as the startup variant-mismatch notice.
            let _ = tokio::task::spawn_blocking(move || {
                crate::notification::send_sync(&notice.title, &notice.body)
            })
            .await;
        }
        (None, Some(chosen)) => tracing::info!(
            "OSD frontend: {} (configured: {})",
            chosen.as_str(),
            requested.as_str()
        ),
        (None, None) => {}
    }
}

async fn supervise(configured: OsdFrontend) {
    notify_frontend_fallback(configured, &resolve_osd_binary()).await;

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
                     the configured OSD frontend is installed and that the [osd] \
                     config is valid. The daemon will keep running.",
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
