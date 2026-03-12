//! macOS global hotkey support using rdev
//!
//! Provides global keyboard event capture on macOS using the rdev crate.
//! Requires Accessibility permission to be granted to the terminal/app.
//!
//! Fallback: If rdev doesn't work (permissions not granted), users can use
//! Hammerspoon or Karabiner-Elements to trigger `voxtype record toggle`.

use crate::config::HotkeyConfig;
use crate::error::{HotkeyError, Result};
use rdev::{listen, Event, EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Hotkey events that can be sent from the listener
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The hotkey was pressed (model_override not supported on macOS, always None)
    Pressed {
        model_override: Option<String>,
    },
    Released,
    Cancel,
}

/// Hotkey listener trait for macOS
pub trait HotkeyListener: Send {
    /// Start listening for hotkey events
    fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>>;

    /// Stop listening
    fn stop(&mut self) -> Result<()>;
}

/// rdev-based hotkey listener for macOS
pub struct RdevHotkeyListener {
    target_key: Key,
    cancel_key: Option<Key>,
    running: Arc<AtomicBool>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl RdevHotkeyListener {
    /// Create a new rdev hotkey listener
    pub fn new(config: &HotkeyConfig) -> Result<Self> {
        let target_key = parse_key_name(&config.key)
            .ok_or_else(|| HotkeyError::UnknownKey(config.key.clone()))?;

        let cancel_key = config.cancel_key.as_ref().and_then(|k| parse_key_name(k));

        Ok(Self {
            target_key,
            cancel_key,
            running: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
        })
    }
}

impl HotkeyListener for RdevHotkeyListener {
    fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>> {
        // Check/request Accessibility permission before starting the listener.
        // This triggers the macOS system dialog if permission hasn't been granted.
        if !check_accessibility_permission() {
            tracing::warn!(
                "Accessibility permission not granted. \
                 macOS should have shown a permission dialog. \
                 Grant access in: System Settings > Privacy & Security > Accessibility"
            );
        }

        let (tx, rx) = mpsc::channel(32);
        let target_key = self.target_key;
        let cancel_key = self.cancel_key;
        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);

        // If Accessibility permission isn't granted, rdev::listen() creates a dead
        // event tap that never fires. The only fix is to restart the process after
        // permission is granted. Spawn a watcher that re-execs when permission appears.
        if !is_accessibility_granted() {
            let running_watcher = running.clone();
            std::thread::spawn(move || {
                loop {
                    if !running_watcher.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                    if is_accessibility_granted() {
                        tracing::info!(
                            "Accessibility permission granted, restarting daemon to activate hotkey..."
                        );
                        // Remove lock file so the new process can acquire it
                        let lock_path = crate::config::Config::runtime_dir().join("voxtype.lock");
                        let _ = std::fs::remove_file(&lock_path);
                        // Spawn a new daemon and exit. The dead CGEvent tap in this
                        // process can't be revived; a fresh process is needed.
                        let exe = std::env::current_exe().expect("current_exe");
                        let args: Vec<String> = std::env::args().skip(1).collect();
                        match std::process::Command::new(&exe).args(&args).spawn() {
                            Ok(_) => std::process::exit(0),
                            Err(e) => {
                                tracing::error!("Failed to restart: {}", e);
                                return;
                            }
                        }
                    }
                }
            });
        }

        let thread_handle = std::thread::spawn(move || {
            let tx_clone = tx.clone();
            let running_clone = running.clone();

            // Debounce: track last event time to prevent duplicate events
            let last_press = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(10)));
            let last_release = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(10)));
            let debounce_ms = 100; // Minimum ms between same event type

            let last_press_clone = last_press.clone();
            let last_release_clone = last_release.clone();

            let callback = move |event: Event| {
                if !running_clone.load(Ordering::SeqCst) {
                    return;
                }

                match event.event_type {
                    EventType::KeyPress(key) => {
                        if key == target_key {
                            let mut last = last_press_clone.lock().unwrap();
                            if last.elapsed() > Duration::from_millis(debounce_ms) {
                                *last = Instant::now();
                                let _ = tx_clone.blocking_send(HotkeyEvent::Pressed {
                                    model_override: None,
                                });
                            }
                        } else if Some(key) == cancel_key {
                            let _ = tx_clone.blocking_send(HotkeyEvent::Cancel);
                        }
                    }
                    EventType::KeyRelease(key) => {
                        if key == target_key {
                            let mut last = last_release_clone.lock().unwrap();
                            if last.elapsed() > Duration::from_millis(debounce_ms) {
                                *last = Instant::now();
                                let _ = tx_clone.blocking_send(HotkeyEvent::Released);
                            }
                        }
                    }
                    _ => {}
                }
            };

            // This blocks until an error occurs or the process is terminated
            if let Err(e) = listen(callback) {
                tracing::error!("rdev listen error: {:?}", e);
                tracing::warn!(
                    "Global hotkey capture failed. Grant Accessibility permission in \
                     System Settings > Privacy & Security > Accessibility, \
                     or use Hammerspoon for hotkey support."
                );
            }
        });

        self.thread_handle = Some(thread_handle);
        Ok(rx)
    }

    fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        // Note: rdev's listen() doesn't have a clean way to stop from another thread
        // The thread will stop when the process exits or on the next event
        Ok(())
    }
}

/// Parse a key name string to rdev Key
fn parse_key_name(name: &str) -> Option<Key> {
    match name.to_uppercase().as_str() {
        // Function keys
        "F1" => Some(Key::F1),
        "F2" => Some(Key::F2),
        "F3" => Some(Key::F3),
        "F4" => Some(Key::F4),
        "F5" => Some(Key::F5),
        "F6" => Some(Key::F6),
        "F7" => Some(Key::F7),
        "F8" => Some(Key::F8),
        "F9" => Some(Key::F9),
        "F10" => Some(Key::F10),
        "F11" => Some(Key::F11),
        "F12" => Some(Key::F12),

        // Modifier keys
        "LEFTALT" | "LEFTOPT" | "LEFTOPTION" | "ALT" | "OPTION" => Some(Key::Alt),
        "RIGHTALT" | "RIGHTOPT" | "RIGHTOPTION" => Some(Key::AltGr),
        "LEFTCTRL" | "LEFTCONTROL" | "CTRL" | "CONTROL" => Some(Key::ControlLeft),
        "RIGHTCTRL" | "RIGHTCONTROL" => Some(Key::ControlRight),
        "LEFTSHIFT" | "SHIFT" => Some(Key::ShiftLeft),
        "RIGHTSHIFT" => Some(Key::ShiftRight),
        "LEFTMETA" | "LEFTCMD" | "LEFTCOMMAND" | "CMD" | "COMMAND" | "META" => Some(Key::MetaLeft),
        "RIGHTMETA" | "RIGHTCMD" | "RIGHTCOMMAND" => Some(Key::MetaRight),

        // Special keys
        "ESCAPE" | "ESC" => Some(Key::Escape),
        "SPACE" => Some(Key::Space),
        "TAB" => Some(Key::Tab),
        "CAPSLOCK" => Some(Key::CapsLock),
        "BACKSPACE" => Some(Key::Backspace),
        "ENTER" | "RETURN" => Some(Key::Return),

        // Navigation
        "UP" | "UPARROW" => Some(Key::UpArrow),
        "DOWN" | "DOWNARROW" => Some(Key::DownArrow),
        "LEFT" | "LEFTARROW" => Some(Key::LeftArrow),
        "RIGHT" | "RIGHTARROW" => Some(Key::RightArrow),
        "HOME" => Some(Key::Home),
        "END" => Some(Key::End),
        "PAGEUP" => Some(Key::PageUp),
        "PAGEDOWN" => Some(Key::PageDown),

        // Other
        "DELETE" => Some(Key::Delete),
        "INSERT" => Some(Key::Insert),
        "PAUSE" => Some(Key::Pause),
        "SCROLLLOCK" => Some(Key::ScrollLock),
        "PRINTSCREEN" => Some(Key::PrintScreen),
        "FN" | "FUNCTION" | "GLOBE" => Some(Key::Function),

        // Letters (for completeness, though unusual for hotkeys)
        "A" => Some(Key::KeyA),
        "B" => Some(Key::KeyB),
        "C" => Some(Key::KeyC),
        "D" => Some(Key::KeyD),
        "E" => Some(Key::KeyE),
        "F" => Some(Key::KeyF),
        "G" => Some(Key::KeyG),
        "H" => Some(Key::KeyH),
        "I" => Some(Key::KeyI),
        "J" => Some(Key::KeyJ),
        "K" => Some(Key::KeyK),
        "L" => Some(Key::KeyL),
        "M" => Some(Key::KeyM),
        "N" => Some(Key::KeyN),
        "O" => Some(Key::KeyO),
        "P" => Some(Key::KeyP),
        "Q" => Some(Key::KeyQ),
        "R" => Some(Key::KeyR),
        "S" => Some(Key::KeyS),
        "T" => Some(Key::KeyT),
        "U" => Some(Key::KeyU),
        "V" => Some(Key::KeyV),
        "W" => Some(Key::KeyW),
        "X" => Some(Key::KeyX),
        "Y" => Some(Key::KeyY),
        "Z" => Some(Key::KeyZ),

        _ => None,
    }
}

/// Create a hotkey listener for macOS
pub fn create_listener(config: &HotkeyConfig) -> Result<Box<dyn HotkeyListener>> {
    Ok(Box::new(RdevHotkeyListener::new(config)?))
}

/// Check if Accessibility permission is granted by trying to create an event tap.
/// Unlike AXIsProcessTrusted(), this is not cached and reflects the current state.
fn is_accessibility_granted() -> bool {
    use core_graphics::event::{CGEventTap, CGEventTapLocation, CGEventTapPlacement, CGEventTapOptions, CGEventType};

    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
        vec![CGEventType::KeyDown],
        |_, _, _| None,
    );
    tap.is_ok()
}

/// Check if Accessibility permission is granted, prompting the user if not.
///
/// Calls AXIsProcessTrustedWithOptions with kAXTrustedCheckOptionPrompt=true,
/// which makes macOS show the "App wants to control this computer" dialog
/// if permission hasn't been granted yet.
pub fn check_accessibility_permission() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: core_foundation::base::CFTypeRef) -> bool;
    }

    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;

    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);

    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef() as _) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_name() {
        assert_eq!(parse_key_name("F1"), Some(Key::F1));
        assert_eq!(parse_key_name("f1"), Some(Key::F1));
        assert_eq!(parse_key_name("RIGHTALT"), Some(Key::AltGr));
        assert_eq!(parse_key_name("rightoption"), Some(Key::AltGr));
        assert_eq!(parse_key_name("CMD"), Some(Key::MetaLeft));
        assert_eq!(parse_key_name("SCROLLLOCK"), Some(Key::ScrollLock));
        assert_eq!(parse_key_name("UNKNOWN"), None);
    }
}
