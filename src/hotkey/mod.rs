//! Hotkey detection module
//!
//! Provides cross-platform hotkey detection:
//! - Linux: Uses kernel-level evdev interface for key event detection
//! - macOS: Uses CGEventTap for global key event capture (requires Accessibility permissions)
//!
//! On Linux, the user must be in the 'input' group.
//! On macOS, Accessibility permissions must be granted in System Settings.

#[cfg(target_os = "linux")]
pub mod evdev_listener;

#[cfg(target_os = "macos")]
pub mod macos;

use crate::config::HotkeyConfig;
use crate::error::HotkeyError;
use tokio::sync::mpsc;

/// Events emitted by the hotkey listener
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The hotkey was pressed
    Pressed,
    /// The hotkey was released
    Released,
    /// The cancel key was pressed (abort recording/transcription)
    Cancel,
}

/// Trait for hotkey detection implementations
#[async_trait::async_trait]
pub trait HotkeyListener: Send + Sync {
    /// Start listening for hotkey events
    /// Returns a channel receiver for events
    async fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>, HotkeyError>;

    /// Stop listening and clean up
    async fn stop(&mut self) -> Result<(), HotkeyError>;
}

/// Factory function to create the appropriate hotkey listener for the current platform
#[cfg(target_os = "linux")]
pub fn create_listener(config: &HotkeyConfig) -> Result<Box<dyn HotkeyListener>, HotkeyError> {
    Ok(Box::new(evdev_listener::EvdevListener::new(config)?))
}

/// Factory function to create the appropriate hotkey listener for the current platform
#[cfg(target_os = "macos")]
pub fn create_listener(config: &HotkeyConfig) -> Result<Box<dyn HotkeyListener>, HotkeyError> {
    Ok(Box::new(macos::MacOSListener::new(config)?))
}

/// Factory function for unsupported platforms
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn create_listener(_config: &HotkeyConfig) -> Result<Box<dyn HotkeyListener>, HotkeyError> {
    Err(HotkeyError::DeviceAccess(
        "Hotkey capture is only supported on Linux and macOS".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hotkey_event_equality() {
        assert_eq!(HotkeyEvent::Pressed, HotkeyEvent::Pressed);
        assert_eq!(HotkeyEvent::Released, HotkeyEvent::Released);
        assert_eq!(HotkeyEvent::Cancel, HotkeyEvent::Cancel);
        assert_ne!(HotkeyEvent::Pressed, HotkeyEvent::Released);
        assert_ne!(HotkeyEvent::Pressed, HotkeyEvent::Cancel);
    }

    #[test]
    fn test_hotkey_event_debug() {
        let pressed = HotkeyEvent::Pressed;
        let debug_str = format!("{:?}", pressed);
        assert!(debug_str.contains("Pressed"));
    }

    #[test]
    fn test_hotkey_event_clone() {
        let event = HotkeyEvent::Pressed;
        let cloned = event;
        assert_eq!(event, cloned);
    }
}
