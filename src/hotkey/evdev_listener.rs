//! evdev-based hotkey listener with device hotplug support
//!
//! Uses the Linux evdev interface to detect key presses at the kernel level.
//! This works on all Wayland compositors because it bypasses the display server.
//!
//! Uses inotify to detect device changes (hotplug, screenlock, suspend/resume)
//! and automatically re-enumerates devices when needed.
//!
//! The user must be in the 'input' group to access /dev/input/* devices.

use super::{HotkeyEvent, HotkeyListener};
use crate::config::HotkeyConfig;
use crate::error::HotkeyError;
use evdev::{Device, InputEventKind, Key};
use inotify::{Inotify, WatchMask};
use std::collections::{HashMap, HashSet};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

/// evdev-based hotkey listener
pub struct EvdevListener {
    /// The key to listen for
    target_key: Key,
    /// Modifier keys that must be held
    modifier_keys: HashSet<Key>,
    /// Optional cancel key
    cancel_key: Option<Key>,
    /// Optional model modifier key (when held, use secondary model)
    model_modifier: Option<Key>,
    /// Secondary model to use when model_modifier is held
    secondary_model: Option<String>,
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

        // Parse optional cancel key
        let cancel_key = config
            .cancel_key
            .as_ref()
            .map(|k| parse_key_name(k))
            .transpose()?;

        // Parse optional model modifier key
        let model_modifier = config
            .model_modifier
            .as_ref()
            .map(|k| parse_key_name(k))
            .transpose()?;

        // Verify we can access /dev/input (permission check)
        std::fs::read_dir("/dev/input")
            .map_err(|e| HotkeyError::DeviceAccess(format!("/dev/input: {}", e)))?;

        Ok(Self {
            target_key,
            modifier_keys,
            cancel_key,
            model_modifier,
            secondary_model: None, // Set later via set_secondary_model
            stop_signal: None,
        })
    }

    /// Set the secondary model to use when model_modifier is held
    pub fn set_secondary_model(&mut self, model: Option<String>) {
        self.secondary_model = model;
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
        let cancel_key = self.cancel_key;
        let model_modifier = self.model_modifier;
        let secondary_model = self.secondary_model.clone();

        // Spawn the listener task
        tokio::task::spawn_blocking(move || {
            if let Err(e) = evdev_listener_loop(
                target_key,
                modifier_keys,
                cancel_key,
                model_modifier,
                secondary_model,
                tx,
                stop_rx,
            ) {
                tracing::error!("Hotkey listener error: {}", e);
            }
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

/// Manages input devices with hotplug detection via inotify
struct DeviceManager {
    /// Map of device path to opened device
    devices: HashMap<PathBuf, Device>,
    /// inotify instance watching /dev/input
    inotify: Inotify,
    /// Buffer for inotify events
    inotify_buffer: [u8; 1024],
    /// Last time we did a full validation
    last_validation: Instant,
}

impl DeviceManager {
    /// Create a new device manager with inotify watcher
    fn new() -> Result<Self, HotkeyError> {
        let inotify = Inotify::init().map_err(|e| {
            HotkeyError::DeviceAccess(format!("Failed to initialize inotify: {}", e))
        })?;

        // Watch /dev/input for device creation and deletion
        inotify
            .watches()
            .add("/dev/input", WatchMask::CREATE | WatchMask::DELETE)
            .map_err(|e| HotkeyError::DeviceAccess(format!("Failed to watch /dev/input: {}", e)))?;

        let mut manager = Self {
            devices: HashMap::new(),
            inotify,
            inotify_buffer: [0u8; 1024],
            last_validation: Instant::now(),
        };

        // Initial device enumeration
        manager.enumerate_devices()?;

        if manager.devices.is_empty() {
            return Err(HotkeyError::NoKeyboard);
        }

        Ok(manager)
    }

    /// Enumerate all keyboard devices and open them
    fn enumerate_devices(&mut self) -> Result<(), HotkeyError> {
        let input_dir = std::fs::read_dir("/dev/input")
            .map_err(|e| HotkeyError::DeviceAccess(format!("/dev/input: {}", e)))?;

        for entry in input_dir.flatten() {
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

            // Skip if already open
            if self.devices.contains_key(&path) {
                continue;
            }

            // Try to open and check if it's a keyboard
            self.try_open_device(&path);
        }

        Ok(())
    }

    /// Try to open a device and add it if it's a keyboard
    fn try_open_device(&mut self, path: &PathBuf) {
        match Device::open(path) {
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
                    // Set device to non-blocking mode
                    let fd = device.as_raw_fd();
                    unsafe {
                        let flags = libc::fcntl(fd, libc::F_GETFL);
                        if flags != -1 {
                            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                        }
                    }

                    tracing::info!(
                        "Opened keyboard: {:?} ({:?})",
                        path,
                        device.name().unwrap_or("unknown")
                    );
                    self.devices.insert(path.clone(), device);
                }
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::PermissionDenied {
                    tracing::trace!("Skipping {:?}: {}", path, e);
                }
            }
        }
    }

    /// Check inotify for device changes (non-blocking)
    /// Returns true if devices changed
    fn check_for_device_changes(&mut self) -> bool {
        // Set inotify to non-blocking for this check
        let fd = self.inotify.as_raw_fd();
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags != -1 {
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }

        let events = match self.inotify.read_events(&mut self.inotify_buffer) {
            Ok(events) => events,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                return false;
            }
            Err(e) => {
                tracing::warn!("inotify read error: {}", e);
                return false;
            }
        };

        let mut changed = false;
        for event in events {
            if let Some(name) = event.name {
                let name_str = name.to_string_lossy();
                if name_str.starts_with("event") {
                    let path = PathBuf::from("/dev/input").join(&*name_str);

                    if event.mask.contains(inotify::EventMask::CREATE) {
                        tracing::debug!("Device created: {:?}", path);
                        changed = true;
                    } else if event.mask.contains(inotify::EventMask::DELETE) {
                        tracing::debug!("Device removed: {:?}", path);
                        self.devices.remove(&path);
                        changed = true;
                    }
                }
            }
        }

        changed
    }

    /// Handle device changes - wait for settle and re-enumerate
    fn handle_device_changes(&mut self) {
        // Wait for devices to settle (USB enumeration can be slow)
        std::thread::sleep(Duration::from_millis(150));

        // Re-enumerate to pick up new devices
        if let Err(e) = self.enumerate_devices() {
            tracing::warn!("Device enumeration failed: {}", e);
        }

        tracing::info!("Devices updated: {} keyboard(s) active", self.devices.len());
    }

    /// Validate that all devices are still accessible
    /// Returns true if any device was removed
    fn validate_devices(&mut self) -> bool {
        let mut stale_paths = Vec::new();

        for (path, device) in &self.devices {
            let fd = device.as_raw_fd();
            let link_path = format!("/proc/self/fd/{}", fd);

            // Check if the symlink still points to a valid device
            let is_valid = std::fs::read_link(&link_path)
                .map(|target| target.exists())
                .unwrap_or(false);

            if !is_valid {
                tracing::debug!("Device no longer valid: {:?}", path);
                stale_paths.push(path.clone());
            }
        }

        for path in &stale_paths {
            self.devices.remove(path);
        }

        !stale_paths.is_empty()
    }

    /// Poll all devices for events, handling errors gracefully
    fn poll_events(&mut self) -> Vec<(Key, i32)> {
        let mut events = Vec::new();
        let mut error_paths = Vec::new();

        for (path, device) in &mut self.devices {
            match device.fetch_events() {
                Ok(device_events) => {
                    for event in device_events {
                        if let InputEventKind::Key(key) = event.kind() {
                            events.push((key, event.value()));
                        }
                    }
                }
                Err(ref e) if e.raw_os_error() == Some(libc::ENODEV) => {
                    tracing::debug!("Device gone (ENODEV): {:?}", path);
                    error_paths.push(path.clone());
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No events available, this is normal for non-blocking
                }
                Err(e) => {
                    tracing::trace!("Device read error on {:?}: {}", path, e);
                }
            }
        }

        // Remove devices that returned errors
        for path in error_paths {
            self.devices.remove(&path);
        }

        events
    }

    /// Check if we have any devices
    fn has_devices(&self) -> bool {
        !self.devices.is_empty()
    }
}

/// Main listener loop running in a blocking task
fn evdev_listener_loop(
    target_key: Key,
    modifier_keys: HashSet<Key>,
    cancel_key: Option<Key>,
    model_modifier: Option<Key>,
    secondary_model: Option<String>,
    tx: mpsc::Sender<HotkeyEvent>,
    mut stop_rx: oneshot::Receiver<()>,
) -> Result<(), HotkeyError> {
    let mut manager = DeviceManager::new()?;

    // Track currently held modifier keys
    let mut active_modifiers: HashSet<Key> = HashSet::new();

    // Track if model modifier is currently held
    let mut model_modifier_held = false;

    // Track if we're currently "pressed" (to handle repeat events)
    let mut is_pressed = false;

    if let Some(cancel) = cancel_key {
        tracing::info!(
            "Listening for {:?} (with modifiers: {:?}) and cancel key {:?} on {} device(s)",
            target_key,
            modifier_keys,
            cancel,
            manager.devices.len()
        );
    } else {
        tracing::info!(
            "Listening for {:?} (with modifiers: {:?}) on {} device(s)",
            target_key,
            modifier_keys,
            manager.devices.len()
        );
    }

    if let Some(mm) = model_modifier {
        if let Some(ref model) = secondary_model {
            tracing::info!(
                "Model modifier {:?} configured for secondary model '{}'",
                mm,
                model
            );
        }
    }

    loop {
        // Check for stop signal (non-blocking)
        match stop_rx.try_recv() {
            Ok(_) | Err(oneshot::error::TryRecvError::Closed) => {
                tracing::debug!("Hotkey listener stopping");
                return Ok(());
            }
            Err(oneshot::error::TryRecvError::Empty) => {}
        }

        // Check inotify for device changes
        if manager.check_for_device_changes() {
            // Clear state when devices change
            active_modifiers.clear();
            model_modifier_held = false;
            is_pressed = false;
            manager.handle_device_changes();
        }

        // Periodic validation (every 30 seconds)
        if manager.last_validation.elapsed() > Duration::from_secs(30) {
            if manager.validate_devices() {
                // Devices were removed, clear state
                active_modifiers.clear();
                model_modifier_held = false;
                is_pressed = false;
                tracing::debug!("Stale devices removed during validation");
            }
            manager.last_validation = Instant::now();
        }

        // If no devices, try to find some
        if !manager.has_devices() {
            tracing::warn!("No keyboard devices available, waiting...");
            std::thread::sleep(Duration::from_secs(1));
            if let Err(e) = manager.enumerate_devices() {
                tracing::debug!("Enumeration failed: {}", e);
            }
            continue;
        }

        // Poll all devices for events
        for (key, value) in manager.poll_events() {
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

            // Track model modifier state
            if let Some(mm) = model_modifier {
                if key == mm {
                    match value {
                        1 => model_modifier_held = true,
                        0 => model_modifier_held = false,
                        _ => {}
                    }
                }
            }

            // Check cancel key first (if configured)
            if let Some(cancel) = cancel_key {
                if key == cancel && value == 1 {
                    // Cancel key pressed (ignore repeats and releases)
                    tracing::debug!("Cancel key pressed");
                    if tx.blocking_send(HotkeyEvent::Cancel).is_err() {
                        return Ok(()); // Channel closed
                    }
                    continue;
                }
            }

            // Check target key
            if key == target_key {
                let modifiers_satisfied =
                    modifier_keys.iter().all(|m| active_modifiers.contains(m));

                if modifiers_satisfied {
                    match value {
                        1 if !is_pressed => {
                            // Key press (not repeat)
                            is_pressed = true;

                            // Determine model override based on model_modifier state
                            let model_override = if model_modifier_held {
                                secondary_model.clone()
                            } else {
                                None
                            };

                            if model_override.is_some() {
                                tracing::debug!(
                                    "Hotkey pressed with model override: {:?}",
                                    model_override
                                );
                            } else {
                                tracing::debug!("Hotkey pressed");
                            }

                            if tx
                                .blocking_send(HotkeyEvent::Pressed { model_override })
                                .is_err()
                            {
                                return Ok(()); // Channel closed
                            }
                        }
                        0 if is_pressed => {
                            // Key release
                            is_pressed = false;
                            tracing::debug!("Hotkey released");
                            if tx.blocking_send(HotkeyEvent::Released).is_err() {
                                return Ok(()); // Channel closed
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

        // Small sleep to avoid busy-waiting
        std::thread::sleep(Duration::from_millis(5));
    }
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
