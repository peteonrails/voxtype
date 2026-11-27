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
//! Devices stay open between output calls: closing an evdev node waits for an
//! RCU grace period in the kernel (10-35 ms each), which made every call cost
//! a few hundred milliseconds. Each call still lists `/dev/input`, which is
//! cheap, and opens only nodes that are new or were re-created, so hotplug
//! keeps working.

use evdev::{AttributeSet, Device, KeyCode as Key};
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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

/// An event node, told apart from a later node at the same path by its inode.
type Node = (PathBuf, u64);

/// Devices kept open across guards. `others` remembers which nodes are not
/// keyboards so they are not reopened. Generic over the device so the
/// bookkeeping can be tested without `/dev/input`.
struct Cache<D> {
    keyboards: HashMap<PathBuf, (u64, D)>,
    others: HashMap<PathBuf, u64>,
}

impl<D> Cache<D> {
    fn new() -> Self {
        Self {
            keyboards: HashMap::new(),
            others: HashMap::new(),
        }
    }

    /// Forget nodes that were removed or re-created since the last call.
    /// Returns their devices, to be closed, and the listed nodes to open.
    fn reconcile<'a>(&mut self, listed: &'a [Node]) -> (Vec<D>, Vec<&'a Node>) {
        let current: HashMap<&PathBuf, u64> = listed.iter().map(|(p, i)| (p, *i)).collect();
        let gone: Vec<PathBuf> = self
            .keyboards
            .iter()
            .filter(|(p, (ino, _))| current.get(p) != Some(ino))
            .map(|(p, _)| p.clone())
            .collect();
        let stale = gone
            .into_iter()
            .filter_map(|p| self.keyboards.remove(&p).map(|(_, d)| d))
            .collect();
        self.others.retain(|p, ino| current.get(p) == Some(ino));

        let new = listed
            .iter()
            .filter(|(path, _)| {
                !self.keyboards.contains_key(path) && !self.others.contains_key(path)
            })
            .collect();
        (stale, new)
    }

    /// Store what opening new nodes found. Returns the devices that aren't
    /// keyboards, to be closed.
    fn record(&mut self, opened: Vec<(&Node, D, bool)>) -> Vec<D> {
        let mut stale = Vec::new();
        for ((path, ino), device, is_keyboard) in opened {
            if is_keyboard {
                self.keyboards.insert(path.clone(), (*ino, device));
            } else {
                self.others.insert(path.clone(), *ino);
                stale.push(device);
            }
        }
        stale
    }

    /// Lend every cached keyboard to a guard.
    fn take_keyboards(&mut self) -> Vec<(Node, D)> {
        self.keyboards
            .drain()
            .map(|(path, (ino, device))| ((path, ino), device))
            .collect()
    }

    /// Take back a guard's keyboards. Where another guard reopened the node
    /// meanwhile, the cache keeps that device and the returned one comes back
    /// to be closed.
    fn give_back(&mut self, devices: impl IntoIterator<Item = (Node, D)>) -> Vec<D> {
        let mut stale = Vec::new();
        for ((path, ino), device) in devices {
            match self.keyboards.entry(path) {
                std::collections::hash_map::Entry::Occupied(_) => stale.push(device),
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert((ino, device));
                }
            }
        }
        stale
    }
}

static CACHE: Mutex<Option<Cache<Device>>> = Mutex::new(None);

/// Close off the output path: a close blocks for an RCU grace period.
fn close_later(devices: Vec<Device>) {
    if !devices.is_empty() {
        std::thread::spawn(move || drop(devices));
    }
}

/// Snapshot-based modifier-state checker.
pub enum ModifierGuard {
    /// At least one keyboard event device is readable; modifier checks are live.
    /// The devices are borrowed from `CACHE` and go back on drop.
    Active { devices: Vec<(Node, Device)> },
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
                     (no 'input' group membership, normal with compositor keybindings)"
                );
                return Self::Disabled;
            }
        };

        let mut listed: Vec<Node> = Vec::new();
        for entry in entries.flatten() {
            let path: PathBuf = entry.path();
            let is_event_device = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"));
            if !is_event_device {
                continue;
            }
            if let Ok(meta) = std::fs::metadata(&path) {
                listed.push((path, meta.ino()));
            }
        }

        let mut lock = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let cache = lock.get_or_insert_with(Cache::new);
        let (mut stale, new) = cache.reconcile(&listed);

        // Opening an event device costs 10-35 ms on some laptops (the evdev
        // crate queries name, capabilities and axes up front). One thread per
        // device makes the first scan, and any hotplug, as slow as the slowest
        // device instead of the sum of all of them.
        let start = Instant::now();
        let opened: Vec<(&Node, Device, bool)> = std::thread::scope(|scope| {
            let handles: Vec<_> = new
                .iter()
                .map(|node| scope.spawn(move || open_node(&node.0).map(|(d, k)| (*node, d, k))))
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| handle.join().ok().flatten())
                .collect()
        });
        if !new.is_empty() {
            tracing::debug!(
                elapsed_ms = start.elapsed().as_millis() as u64,
                opened = new.len(),
                "modifier_guard: opened new input devices"
            );
        }

        stale.extend(cache.record(opened));
        let devices = cache.take_keyboards();
        drop(lock);
        close_later(stale);

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
        for (_, device) in devices.iter_mut() {
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

impl Drop for ModifierGuard {
    fn drop(&mut self) {
        let Self::Active { devices } = self else {
            return;
        };
        let mut lock = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(cache) = lock.as_mut() else {
            return;
        };
        let stale = cache.give_back(devices.drain(..));
        drop(lock);
        close_later(stale);
    }
}

/// Open `path` and say whether it is a keyboard. Per-device permission errors
/// are common (some event nodes are owned by other groups), so they are
/// skipped silently and retried on the next call.
fn open_node(path: &Path) -> Option<(Device, bool)> {
    let device = Device::open(path).ok()?;
    let is_keyboard = device
        .supported_keys()
        .map(|keys| {
            keys.contains(Key::KEY_A) && keys.contains(Key::KEY_Z) && keys.contains(Key::KEY_ENTER)
        })
        .unwrap_or(false);
    Some((device, is_keyboard))
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
        // Opens the real keyboards under /dev/input (read-only, no grab).
        if !crate::test_support::real_input_devices_allowed("new_does_not_panic_without_permission")
        {
            return;
        }
        // Whether or not /dev/input is readable in the test environment, this
        // must produce a usable guard rather than panicking. The test runner
        // is typically not in the 'input' group, so we expect Disabled.
        let _ = ModifierGuard::new();
    }

    fn node(n: u32, ino: u64) -> Node {
        (PathBuf::from(format!("/dev/input/event{n}")), ino)
    }

    /// Run a scan the way `ModifierGuard::new` does, with `is_keyboard`
    /// standing in for opening the node. A device is its node's inode.
    fn scan(
        cache: &mut Cache<u64>,
        listed: &[Node],
        is_keyboard: impl Fn(&Node) -> bool,
    ) -> (Vec<u64>, Vec<Node>) {
        let (mut stale, new) = cache.reconcile(listed);
        let opened_nodes: Vec<Node> = new.iter().map(|n| (*n).clone()).collect();
        let opened = new.into_iter().map(|n| (n, n.1, is_keyboard(n))).collect();
        stale.extend(cache.record(opened));
        stale.sort();
        (stale, opened_nodes)
    }

    fn sorted(mut v: Vec<(Node, u64)>) -> Vec<(Node, u64)> {
        v.sort();
        v
    }

    #[test]
    fn first_scan_opens_every_node_and_keeps_keyboards() {
        let mut cache = Cache::new();
        let listed = [node(0, 10), node(1, 11), node(2, 12)];
        let (stale, opened) = scan(&mut cache, &listed, |n| n.1 != 11);
        assert_eq!(opened, listed);
        assert_eq!(stale, vec![11]);
        assert_eq!(
            sorted(cache.take_keyboards()),
            vec![(node(0, 10), 10), (node(2, 12), 12)]
        );
    }

    #[test]
    fn unchanged_nodes_are_not_reopened() {
        let mut cache = Cache::new();
        let listed = [node(0, 10), node(1, 11)];
        scan(&mut cache, &listed, |n| n.1 == 10);
        let (stale, opened) = scan(&mut cache, &listed, |_| unreachable!());
        assert!(opened.is_empty());
        assert!(stale.is_empty());
    }

    #[test]
    fn removed_keyboard_is_closed() {
        let mut cache = Cache::new();
        scan(&mut cache, &[node(0, 10), node(1, 11)], |_| true);
        let (stale, opened) = scan(&mut cache, &[node(0, 10)], |_| true);
        assert!(opened.is_empty());
        assert_eq!(stale, vec![11]);
        assert_eq!(cache.take_keyboards(), vec![(node(0, 10), 10)]);
    }

    #[test]
    fn recreated_keyboard_is_reopened() {
        let mut cache = Cache::new();
        scan(&mut cache, &[node(0, 10)], |_| true);
        let (stale, opened) = scan(&mut cache, &[node(0, 20)], |_| true);
        assert_eq!(opened, vec![node(0, 20)]);
        assert_eq!(stale, vec![10]);
        assert_eq!(cache.take_keyboards(), vec![(node(0, 20), 20)]);
    }

    #[test]
    fn recreated_other_node_is_reopened() {
        let mut cache = Cache::new();
        scan(&mut cache, &[node(0, 10)], |_| false);
        let (_, opened) = scan(&mut cache, &[node(0, 20)], |_| true);
        assert_eq!(opened, vec![node(0, 20)]);
        assert_eq!(cache.take_keyboards(), vec![(node(0, 20), 20)]);
    }

    #[test]
    fn removed_other_node_is_forgotten() {
        let mut cache = Cache::new();
        scan(&mut cache, &[node(0, 10)], |_| false);
        scan(&mut cache, &[], |_| false);
        assert!(cache.others.is_empty());
    }

    #[test]
    fn keyboards_lent_to_a_guard_come_back() {
        let mut cache = Cache::new();
        scan(&mut cache, &[node(0, 10)], |_| true);
        let lent = cache.take_keyboards();
        assert!(cache.keyboards.is_empty());
        assert!(cache.give_back(lent).is_empty());
        let (stale, opened) = scan(&mut cache, &[node(0, 10)], |_| unreachable!());
        assert!(opened.is_empty() && stale.is_empty());
    }

    #[test]
    fn second_guard_keeps_its_device_when_both_return() {
        let mut cache = Cache::new();
        scan(&mut cache, &[node(0, 10)], |_| true);
        let first = cache.take_keyboards();

        // A second guard finds the cache empty and opens the node itself.
        // Tag its device 99 to tell it apart.
        let (_, opened) = scan(&mut cache, &[node(0, 10)], |_| true);
        assert_eq!(opened, vec![node(0, 10)]);
        let second: Vec<_> = cache
            .take_keyboards()
            .into_iter()
            .map(|(n, _)| (n, 99))
            .collect();

        assert!(cache.give_back(second).is_empty());
        assert_eq!(cache.give_back(first), vec![10]);
        assert_eq!(cache.take_keyboards(), vec![(node(0, 10), 99)]);
    }
}
