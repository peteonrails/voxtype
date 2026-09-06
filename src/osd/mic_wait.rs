//! Mic-readiness flag for the OSD waiting state.
//!
//! Bluetooth mics only exist in the HSP/HFP profile. When capture starts
//! while the card is still on A2DP, PipeWire takes ~600ms to flip profiles
//! plus settle time, during which captured frames are digital silence. A
//! flat waveform is indistinguishable from a live-but-quiet mic, so the
//! user dictates into the void without knowing it.
//!
//! Protocol: the daemon touches [`flag_path`] when a recording starts on a
//! Bluetooth source and removes it for wired sources, on streaming stop
//! (whose drain pump would otherwise keep a stale flag visible), and at
//! daemon startup. Batch ends need no removal: frames stop flowing, the
//! OSD hides on its idle timeout, and the next start rewrites the flag.
//! Frontends render a waiting state while the flag is fresh (see
//! [`WAIT_CAP_SECS`]) and no frame above [`SILENCE_DBFS`] has arrived
//! since. The age cap bounds the state when the room is genuinely silent
//! and when a stale flag survives a crash.

use std::path::PathBuf;
use std::time::SystemTime;

/// Maximum flag age for the waiting state to apply.
pub const WAIT_CAP_SECS: f32 = 2.5;

/// Frames at or below this peak count as digital silence.
pub const SILENCE_DBFS: f32 = -80.0;

/// Runtime flag path: `$XDG_RUNTIME_DIR/voxtype/mic_waiting`
/// (mirrors the `osd_suppressed` convention).
pub fn flag_path() -> PathBuf {
    PathBuf::from(runtime_base()).join("voxtype/mic_waiting")
}

/// Create (or refresh) the waiting flag. Best-effort: failures are silent
/// because the flag is purely advisory — without it the OSD behaves as
/// before.
pub fn mark_waiting() {
    write_flag_at(&flag_path());
}

/// Remove the waiting flag. A missing file is fine.
pub fn clear_waiting() {
    remove_flag_at(&flag_path());
}

/// Waiting-flag mtime, if present and readable. Frontends compare it
/// against [`WAIT_CAP_SECS`] and their frame stream.
pub fn flag_mtime() -> Option<SystemTime> {
    flag_mtime_at(&flag_path())
}

/// Processing flag path: `$XDG_RUNTIME_DIR/voxtype/mic_processing`.
/// Set around the end-of-turn polish subprocess so the OSD can show a
/// working state instead of going dark before the rewrite lands. Unlike
/// the waiting flag it needs no age cap: it only exists while frames are
/// flowing (the drain pump runs through polish), and the OSD hides on its
/// idle timeout the moment they stop.
pub fn processing_path() -> PathBuf {
    PathBuf::from(runtime_base()).join("voxtype/mic_processing")
}

/// Set the processing flag (see [`processing_path`]).
pub fn mark_processing() {
    write_flag_at(&processing_path());
}

/// Clear the processing flag. A missing file is fine.
pub fn clear_processing() {
    remove_flag_at(&processing_path());
}

/// Whether the processing flag is currently set.
pub fn processing_active() -> bool {
    processing_path().exists()
}

fn runtime_base() -> String {
    std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }))
}

/// Path-parameterized cores, used by the wrappers above and by tests.
pub fn write_flag_at(path: &std::path::Path) {
    let _ = std::fs::write(path, b"");
}

/// Path-parameterized cores, used by the wrappers above and by tests.
pub fn remove_flag_at(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

/// Path-parameterized cores, used by the wrappers above and by tests.
pub fn flag_mtime_at(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Bluetooth source names contain `bluez` (e.g. `bluez_input.80:AA:...`).
/// Case-insensitive; used to gate the flag so wired mics never flash a
/// waiting state they don't need.
pub fn is_bluetooth_source(name: &str) -> bool {
    name.to_lowercase().contains("bluez")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_source_matching() {
        assert!(is_bluetooth_source("bluez_input.80:AA:1C:20:0D:41"));
        assert!(is_bluetooth_source("BLUEZ_OUTPUT.X"));
        assert!(!is_bluetooth_source("alsa_input.usb-Generic"));
        assert!(!is_bluetooth_source(""));
    }

    #[test]
    fn flag_roundtrip_in_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mic_waiting");
        assert!(flag_mtime_at(&path).is_none());
        write_flag_at(&path);
        let mtime = flag_mtime_at(&path).expect("flag written");
        assert!(mtime.elapsed().unwrap().as_secs() < 60);
        remove_flag_at(&path);
        assert!(flag_mtime_at(&path).is_none());
        // Removing twice is fine.
        remove_flag_at(&path);
    }
}
