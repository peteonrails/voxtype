//! Whether a meeting view is on screen, from the presence file its host
//! writes.
//!
//! Contract: `$XDG_RUNTIME_DIR/voxtype/meeting-view.state` holds
//! `{"visible": bool, "minimized": bool, "host": "omarchy" | "standalone" |
//! "gtk4" | "terminal", "pid": <host pid>, "updated_at": <unix seconds>}`.
//! Writers replace it atomically on every show, hide or minimize change and
//! every 10 s while visible. A reader counts the view as present only when
//! it is visible (a minimized pill counts), the file is under 30 s old, and
//! the host process is still alive, so a crashed or killed host can never
//! silence notifications for long.
//!
//! The terminal `voxtype meeting watch` can write the same file (host
//! "terminal"); that writer lands with it on the 1.3.1 branch.

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const STATE_FILE: &str = "meeting-view.state";

/// A file older than this is a host that stopped refreshing it.
pub const STALE_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ViewState {
    pub visible: bool,
    #[serde(default)]
    pub minimized: bool,
    #[serde(default)]
    pub host: String,
    pub pid: u32,
    pub updated_at: u64,
}

pub fn state_path() -> PathBuf {
    crate::config::Config::runtime_dir().join(STATE_FILE)
}

/// Is a meeting view on screen right now?
pub fn view_visible() -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    view_visible_at(&state_path(), now, |pid| {
        Path::new(&format!("/proc/{pid}")).exists()
    })
}

/// [`view_visible`] with the clock and the liveness check passed in.
pub fn view_visible_at(path: &Path, now: u64, alive: impl Fn(u32) -> bool) -> bool {
    let Some(state) = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<ViewState>(&s).ok())
    else {
        return false;
    };
    (state.visible || state.minimized)
        && now.saturating_sub(state.updated_at) < STALE_SECS
        && state.pid > 0
        && alive(state.pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn write(dir: &Path, json: &str) -> PathBuf {
        let path = dir.join(STATE_FILE);
        std::fs::write(&path, json).unwrap();
        path
    }

    fn state(visible: bool, minimized: bool, pid: u32, updated_at: u64) -> String {
        format!(
            r#"{{"visible": {visible}, "minimized": {minimized}, "host": "omarchy", "pid": {pid}, "updated_at": {updated_at}}}"#
        )
    }

    #[test]
    fn a_fresh_visible_view_with_a_live_host_counts() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), &state(true, false, 42, NOW - 5));
        assert!(view_visible_at(&path, NOW, |p| p == 42));
    }

    #[test]
    fn a_minimized_pill_counts() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), &state(true, true, 42, NOW));
        assert!(view_visible_at(&path, NOW, |_| true));
        let path = write(dir.path(), &state(false, true, 42, NOW));
        assert!(view_visible_at(&path, NOW, |_| true));
    }

    #[test]
    fn hidden_stale_dead_absent_or_garbage_do_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let hidden = write(dir.path(), &state(false, false, 42, NOW));
        assert!(!view_visible_at(&hidden, NOW, |_| true));
        let stale = write(dir.path(), &state(true, false, 42, NOW - STALE_SECS));
        assert!(!view_visible_at(&stale, NOW, |_| true));
        let dead = write(dir.path(), &state(true, false, 42, NOW));
        assert!(!view_visible_at(&dead, NOW, |_| false));
        let no_pid = write(dir.path(), &state(true, false, 0, NOW));
        assert!(!view_visible_at(&no_pid, NOW, |_| true));
        assert!(!view_visible_at(&dir.path().join("missing"), NOW, |_| true));
        let garbage = write(dir.path(), "{\"visible\": tru");
        assert!(!view_visible_at(&garbage, NOW, |_| true));
    }

    #[test]
    fn the_real_reader_checks_proc_for_the_pid() {
        // This process is alive; pid 0 never is a host.
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), &state(true, false, std::process::id(), NOW));
        assert!(view_visible_at(&path, NOW, |pid| {
            Path::new(&format!("/proc/{pid}")).exists()
        }));
    }
}
