//! Hotkey detection module
//!
//! Provides kernel-level key event detection using evdev.
//! This approach works on all Wayland compositors because it
//! operates at the Linux input subsystem level.
//!
//! Requires the user to be in the 'input' group.

pub mod evdev_listener;

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

/// Factory function to create the appropriate hotkey listener
pub fn create_listener(config: &HotkeyConfig) -> Result<Box<dyn HotkeyListener>, HotkeyError> {
    Ok(Box::new(evdev_listener::EvdevListener::new(config)?))
}
