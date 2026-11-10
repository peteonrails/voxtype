//! Detects whether any modifier key is currently held, to avoid wtype/dotool/
//! ydotool/eitype synthesizing keystrokes that the compositor interprets as
//! keybindings (e.g. user is still pressing Super+Ctrl when transcription
//! completes).
//!
//! Uses `EVIOCGKEY` (via `evdev::Device::get_key_state`) to take a passive
//! snapshot of currently-pressed keys without consuming events. Works on any
//! Wayland compositor or X11 because it bypasses the display server.
//!
//! Degrades gracefully when `/dev/input` is not readable (user not in the
//! `input` group, no keyboard devices present, or all devices return permission
//! errors): the guard becomes `Disabled` and `wait_for_release` returns
//! immediately so output proceeds as before.
//!
//! Re-enumerated on every output call so hotplug (Bluetooth keyboards plugged
//! in mid-session) is handled implicitly without needing inotify watchers in
//! the output path.

use evdev::{AttributeSet, Device, KeyCode as Key};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Modifier keys whose presence we treat as "do not type yet". All of these
/// are routinely combined with letter keys to form compositor or application
/// keybindings.
const MODIFIER_KEYS: &[Key] = &[
    Key::KEY_LEFTCTRL,
    Key::KEY_RIGHTCTRL,
    Key::KEY_LEFTALT,
    Key::KEY_RIGHTALT,
    Key::KEY_LEFTSHIFT,
    Key::KEY_RIGHTSHIFT,
    Key::KEY_LEFTMETA,
    Key::KEY_RIGHTMETA,
];

/// Snapshot-based modifier-state checker.
pub enum ModifierGuard {
    /// At least one keyboard event device is readable; modifier checks are live.
    Active { devices: Vec<Device> },
    /// No readable keyboard devices (typically: user not in the `input` group).
    /// All checks short-circuit to "no modifier held" so output proceeds.
    Disabled,
}

impl Default for ModifierGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ModifierGuard {
    /// Probe `/dev/input/event*` and open every device that looks like a
    /// keyboard. Permission errors are silently downgraded to `Disabled` so
    /// that users without `input` group membership get the same behavior they
    /// had before this feature existed.
    pub fn new() -> Self {
        let entries = match std::fs::read_dir("/dev/input") {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "modifier_guard: /dev/input not readable, disabling \
                     (likely missing 'input' group membership)"
                );
                return Self::Disabled;
            }
        };

        let paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("event"))
            })
            .collect();

        // Opening an event device costs 10-35 ms on some laptops (the evdev
        // crate queries name, capabilities and axes up front), and this runs
        // before every output. One thread per device makes the scan as slow
        // as the slowest device instead of the sum of all of them.
        let start = Instant::now();
        let devices: Vec<Device> = std::thread::scope(|scope| {
            let handles: Vec<_> = paths
                .iter()
                .map(|path| scope.spawn(move || open_keyboard(path)))
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| handle.join().ok().flatten())
                .collect()
        });
        tracing::debug!(
            elapsed_ms = start.elapsed().as_millis() as u64,
            scanned = paths.len(),
            "modifier_guard: scanned input devices"
        );

        if devices.is_empty() {
            tracing::debug!("modifier_guard: no readable keyboard devices, disabling");
            Self::Disabled
        } else {
            tracing::debug!(
                count = devices.len(),
                "modifier_guard: tracking keyboard device(s)"
            );
            Self::Active { devices }
        }
    }

    /// Returns `true` if any modifier key is currently held on any tracked
    /// keyboard. Always returns `false` when `Disabled`.
    pub fn any_modifier_held(&mut self) -> bool {
        let Self::Active { devices } = self else {
            return false;
        };
        for device in devices.iter_mut() {
            // EVIOCGKEY returns the current pressed-key bitmap; cheap and does
            // not consume queued events.
            let state: AttributeSet<Key> = match device.get_key_state() {
                Ok(s) => s,
                Err(_) => continue,
            };
            if MODIFIER_KEYS.iter().any(|k| state.contains(*k)) {
                return true;
            }
        }
        false
    }

    /// Block until no modifier is held, or until `timeout` elapses. Returns
    /// `Ok(())` on clear release (or immediately when `Disabled`), and
    /// `Err(Timeout)` if modifiers were still held when the timeout expired.
    pub async fn wait_for_release(&mut self, timeout: Duration) -> Result<(), Timeout> {
        if matches!(self, Self::Disabled) {
            return Ok(());
        }
        let deadline = Instant::now() + timeout;
        let poll = Duration::from_millis(15);
        while self.any_modifier_held() {
            if Instant::now() >= deadline {
                return Err(Timeout);
            }
            tokio::time::sleep(poll).await;
        }
        Ok(())
    }
}

/// Open `path` if it is a keyboard. Per-device permission errors are common
/// (some event nodes are owned by other groups), so they are skipped silently.
fn open_keyboard(path: &Path) -> Option<Device> {
    let device = Device::open(path).ok()?;
    let is_keyboard = device
        .supported_keys()
        .map(|keys| {
            keys.contains(Key::KEY_A) && keys.contains(Key::KEY_Z) && keys.contains(Key::KEY_ENTER)
        })
        .unwrap_or(false);
    is_keyboard.then_some(device)
}

/// Returned when `wait_for_release` gives up because modifiers were still held
/// past the configured timeout.
#[derive(Debug, Clone, Copy)]
pub struct Timeout;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disabled_guard_returns_immediately() {
        let mut guard = ModifierGuard::Disabled;
        let start = Instant::now();
        let res = guard.wait_for_release(Duration::from_secs(60)).await;
        assert!(res.is_ok());
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "Disabled guard should not actually wait"
        );
    }

    #[tokio::test]
    async fn disabled_guard_reports_no_modifier_held() {
        let mut guard = ModifierGuard::Disabled;
        assert!(!guard.any_modifier_held());
    }

    #[test]
    fn new_does_not_panic_without_permission() {
        // Whether or not /dev/input is readable in the test environment, this
        // must produce a usable guard rather than panicking. The test runner
        // is typically not in the 'input' group, so we expect Disabled.
        let _ = ModifierGuard::new();
    }
}
