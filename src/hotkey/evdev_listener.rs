//! evdev-based hotkey listener
//!
//! Uses the Linux evdev interface to detect key presses at the kernel level.
//! This works on all Wayland compositors because it bypasses the display server.
//!
//! The user must be in the 'input' group to access /dev/input/* devices.

use super::{HotkeyEvent, HotkeyListener};
use crate::config::HotkeyConfig;
use crate::error::HotkeyError;
use evdev::{Device, InputEventKind, Key};
use std::collections::HashSet;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};

/// evdev-based hotkey listener
pub struct EvdevListener {
    /// The key to listen for
    target_key: Key,
    /// Modifier keys that must be held
    modifier_keys: HashSet<Key>,
    /// Paths to keyboard devices
    device_paths: Vec<PathBuf>,
    /// Signal to stop the listener task
    stop_signal: Option<oneshot::Sender<()>>,
}

impl EvdevListener {
    /// Create a new evdev listener for the configured hotkey
    pub fn new(config: &HotkeyConfig) -> Result<Self, HotkeyError> {
        let target_key = parse_key_name(&config.key)?;

        let modifier_keys = config
            .modifiers
            .iter()
            .map(|k| parse_key_name(k))
            .collect::<Result<HashSet<_>, _>>()?;

        let device_paths = find_keyboard_devices()?;

        if device_paths.is_empty() {
            return Err(HotkeyError::NoKeyboard);
        }

        tracing::debug!(
            "Found {} keyboard device(s): {:?}",
            device_paths.len(),
            device_paths
        );

        Ok(Self {
            target_key,
            modifier_keys,
            device_paths,
            stop_signal: None,
        })
    }
}

#[async_trait::async_trait]
impl HotkeyListener for EvdevListener {
    async fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>, HotkeyError> {
        let (tx, rx) = mpsc::channel(32);
        let (stop_tx, stop_rx) = oneshot::channel();
        self.stop_signal = Some(stop_tx);

        let target_key = self.target_key;
        let modifier_keys = self.modifier_keys.clone();
        let device_paths = self.device_paths.clone();

        // Spawn the listener task
        tokio::task::spawn_blocking(move || {
            evdev_listener_loop(device_paths, target_key, modifier_keys, tx, stop_rx);
        });

        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), HotkeyError> {
        if let Some(stop) = self.stop_signal.take() {
            let _ = stop.send(());
        }
        Ok(())
    }
}

/// Main listener loop running in a blocking task
fn evdev_listener_loop(
    device_paths: Vec<PathBuf>,
    target_key: Key,
    modifier_keys: HashSet<Key>,
    tx: mpsc::Sender<HotkeyEvent>,
    mut stop_rx: oneshot::Receiver<()>,
) {
    // Open all keyboard devices in non-blocking mode
    let mut devices: Vec<Device> = device_paths
        .iter()
        .filter_map(|path| match Device::open(path) {
            Ok(device) => {
                // Set device to non-blocking mode so fetch_events doesn't block
                let fd = device.as_raw_fd();
                unsafe {
                    let flags = libc::fcntl(fd, libc::F_GETFL);
                    if flags != -1 {
                        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                    }
                }
                tracing::debug!("Opened device (non-blocking): {:?}", path);
                Some(device)
            }
            Err(e) => {
                tracing::warn!("Failed to open {:?}: {}", path, e);
                None
            }
        })
        .collect();

    if devices.is_empty() {
        tracing::error!("No keyboard devices could be opened");
        return;
    }

    // Track currently held modifier keys
    let mut active_modifiers: HashSet<Key> = HashSet::new();
    
    // Track if we're currently "pressed" (to handle repeat events)
    let mut is_pressed = false;

    tracing::info!(
        "Listening for {:?} (with modifiers: {:?})",
        target_key,
        modifier_keys
    );

    loop {
        // Check for stop signal (non-blocking)
        match stop_rx.try_recv() {
            Ok(_) | Err(oneshot::error::TryRecvError::Closed) => {
                tracing::debug!("Hotkey listener stopping");
                return;
            }
            Err(oneshot::error::TryRecvError::Empty) => {}
        }

        // Poll each device (all set to non-blocking mode)
        for device in &mut devices {
            // fetch_events returns immediately if no events (non-blocking)
            if let Ok(events) = device.fetch_events() {
                for event in events {
                    if let InputEventKind::Key(key) = event.kind() {
                        let value = event.value();

                        // Track modifier state
                        if modifier_keys.contains(&key) {
                            match value {
                                1 => {
                                    active_modifiers.insert(key);
                                }
                                0 => {
                                    active_modifiers.remove(&key);
                                }
                                _ => {}
                            }
                        }

                        // Check target key
                        if key == target_key {
                            let modifiers_satisfied = modifier_keys
                                .iter()
                                .all(|m| active_modifiers.contains(m));

                            if modifiers_satisfied {
                                match value {
                                    1 if !is_pressed => {
                                        // Key press (not repeat)
                                        is_pressed = true;
                                        tracing::debug!("Hotkey pressed");
                                        if tx.blocking_send(HotkeyEvent::Pressed).is_err() {
                                            return; // Channel closed
                                        }
                                    }
                                    0 if is_pressed => {
                                        // Key release
                                        is_pressed = false;
                                        tracing::debug!("Hotkey released");
                                        if tx.blocking_send(HotkeyEvent::Released).is_err() {
                                            return; // Channel closed
                                        }
                                    }
                                    2 => {
                                        // Key repeat - ignore
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        // Small sleep to avoid busy-waiting
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Find all keyboard input devices
fn find_keyboard_devices() -> Result<Vec<PathBuf>, HotkeyError> {
    let mut keyboards = Vec::new();

    let input_dir = std::fs::read_dir("/dev/input").map_err(|e| {
        HotkeyError::DeviceAccess(format!("/dev/input: {}", e))
    })?;

    for entry in input_dir {
        let entry = entry.map_err(|e| HotkeyError::DeviceAccess(e.to_string()))?;
        let path = entry.path();

        // Only look at event* devices
        let is_event_device = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("event"))
            .unwrap_or(false);

        if !is_event_device {
            continue;
        }

        // Try to open and check if it's a keyboard
        match Device::open(&path) {
            Ok(device) => {
                // Check if device has keyboard capabilities
                let has_keys = device
                    .supported_keys()
                    .map(|keys| {
                        // A keyboard should have at least some letter keys
                        keys.contains(Key::KEY_A)
                            && keys.contains(Key::KEY_Z)
                            && keys.contains(Key::KEY_ENTER)
                    })
                    .unwrap_or(false);

                if has_keys {
                    tracing::debug!(
                        "Found keyboard: {:?} ({:?})",
                        path,
                        device.name().unwrap_or("unknown")
                    );
                    keyboards.push(path);
                }
            }
            Err(e) => {
                // Permission denied is common for non-input-group users
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    return Err(HotkeyError::DeviceAccess(path.display().to_string()));
                }
                // Other errors (device busy, etc.) - just skip
                tracing::trace!("Skipping {:?}: {}", path, e);
            }
        }
    }

    Ok(keyboards)
}

/// Parse a key name string to evdev Key
fn parse_key_name(name: &str) -> Result<Key, HotkeyError> {
    // Normalize: uppercase and replace - or space with _
    let normalized: String = name
        .chars()
        .map(|c| match c {
            '-' | ' ' => '_',
            c => c.to_ascii_uppercase(),
        })
        .collect();

    // Add KEY_ prefix if not present
    let key_name = if normalized.starts_with("KEY_") {
        normalized
    } else {
        format!("KEY_{}", normalized)
    };

    // Map common key names to evdev Key variants
    let key = match key_name.as_str() {
        // Lock keys (good hotkey candidates)
        "KEY_SCROLLLOCK" => Key::KEY_SCROLLLOCK,
        "KEY_PAUSE" => Key::KEY_PAUSE,
        "KEY_CAPSLOCK" => Key::KEY_CAPSLOCK,
        "KEY_NUMLOCK" => Key::KEY_NUMLOCK,
        "KEY_INSERT" => Key::KEY_INSERT,

        // Modifier keys
        "KEY_LEFTALT" | "KEY_LALT" => Key::KEY_LEFTALT,
        "KEY_RIGHTALT" | "KEY_RALT" => Key::KEY_RIGHTALT,
        "KEY_LEFTCTRL" | "KEY_LCTRL" => Key::KEY_LEFTCTRL,
        "KEY_RIGHTCTRL" | "KEY_RCTRL" => Key::KEY_RIGHTCTRL,
        "KEY_LEFTSHIFT" | "KEY_LSHIFT" => Key::KEY_LEFTSHIFT,
        "KEY_RIGHTSHIFT" | "KEY_RSHIFT" => Key::KEY_RIGHTSHIFT,
        "KEY_LEFTMETA" | "KEY_LMETA" | "KEY_SUPER" => Key::KEY_LEFTMETA,
        "KEY_RIGHTMETA" | "KEY_RMETA" => Key::KEY_RIGHTMETA,

        // Function keys (F13-F24 are often unused and make good hotkeys)
        "KEY_F1" => Key::KEY_F1,
        "KEY_F2" => Key::KEY_F2,
        "KEY_F3" => Key::KEY_F3,
        "KEY_F4" => Key::KEY_F4,
        "KEY_F5" => Key::KEY_F5,
        "KEY_F6" => Key::KEY_F6,
        "KEY_F7" => Key::KEY_F7,
        "KEY_F8" => Key::KEY_F8,
        "KEY_F9" => Key::KEY_F9,
        "KEY_F10" => Key::KEY_F10,
        "KEY_F11" => Key::KEY_F11,
        "KEY_F12" => Key::KEY_F12,
        "KEY_F13" => Key::KEY_F13,
        "KEY_F14" => Key::KEY_F14,
        "KEY_F15" => Key::KEY_F15,
        "KEY_F16" => Key::KEY_F16,
        "KEY_F17" => Key::KEY_F17,
        "KEY_F18" => Key::KEY_F18,
        "KEY_F19" => Key::KEY_F19,
        "KEY_F20" => Key::KEY_F20,
        "KEY_F21" => Key::KEY_F21,
        "KEY_F22" => Key::KEY_F22,
        "KEY_F23" => Key::KEY_F23,
        "KEY_F24" => Key::KEY_F24,

        // Navigation keys
        "KEY_HOME" => Key::KEY_HOME,
        "KEY_END" => Key::KEY_END,
        "KEY_PAGEUP" => Key::KEY_PAGEUP,
        "KEY_PAGEDOWN" => Key::KEY_PAGEDOWN,
        "KEY_DELETE" => Key::KEY_DELETE,

        // Common keys that might be used
        "KEY_SPACE" => Key::KEY_SPACE,
        "KEY_ENTER" => Key::KEY_ENTER,
        "KEY_TAB" => Key::KEY_TAB,
        "KEY_BACKSPACE" => Key::KEY_BACKSPACE,
        "KEY_ESC" | "KEY_ESCAPE" => Key::KEY_ESC,
        "KEY_GRAVE" | "KEY_BACKTICK" => Key::KEY_GRAVE,

        // Media keys
        "KEY_MUTE" => Key::KEY_MUTE,
        "KEY_VOLUMEDOWN" => Key::KEY_VOLUMEDOWN,
        "KEY_VOLUMEUP" => Key::KEY_VOLUMEUP,
        "KEY_PLAYPAUSE" => Key::KEY_PLAYPAUSE,
        "KEY_NEXTSONG" => Key::KEY_NEXTSONG,
        "KEY_PREVIOUSSONG" => Key::KEY_PREVIOUSSONG,

        // If not found, return error with suggestions
        _ => {
            return Err(HotkeyError::UnknownKey(format!(
                "{}. Try: SCROLLLOCK, PAUSE, F13-F24, or run 'evtest' to find key names",
                name
            )));
        }
    };

    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_name() {
        assert_eq!(parse_key_name("SCROLLLOCK").unwrap(), Key::KEY_SCROLLLOCK);
        assert_eq!(parse_key_name("ScrollLock").unwrap(), Key::KEY_SCROLLLOCK);
        assert_eq!(
            parse_key_name("KEY_SCROLLLOCK").unwrap(),
            Key::KEY_SCROLLLOCK
        );
        assert_eq!(parse_key_name("F13").unwrap(), Key::KEY_F13);
        assert_eq!(parse_key_name("LEFTALT").unwrap(), Key::KEY_LEFTALT);
        assert_eq!(parse_key_name("LALT").unwrap(), Key::KEY_LEFTALT);
    }

    #[test]
    fn test_parse_key_name_error() {
        assert!(parse_key_name("INVALID_KEY_NAME").is_err());
    }
}
