//! Hotkey detection module
//!
//! Provides kernel-level key event detection using evdev.
//! This approach works on all Wayland compositors because it
//! operates at the Linux input subsystem level.
//!
//! Requires read access to /dev/input, which in practice means the `input`
//! group, and that lets every program the user runs read every keystroke.
//! Compositor keybindings calling `voxtype record` are the recommended path;
//! this listener is the fallback (docs/USER_MANUAL.md, "The input group
//! trade-off").

pub mod access;
pub mod evdev_listener;

use crate::config::HotkeyConfig;
use crate::error::HotkeyError;
use tokio::sync::mpsc;

/// Events emitted by the hotkey listener
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The hotkey was pressed, optionally with a model override and/or profile override
    Pressed {
        /// Model to use for this transcription (None = use default)
        model_override: Option<String>,
        /// Profile to activate for post-processing (None = use default)
        profile_override: Option<String>,
    },
    /// The hotkey was released
    Released,
    /// The cancel key was pressed (abort recording/transcription)
    Cancel,
    /// The paste-last key was pressed (send the last dictation again)
    PasteLast,
}

/// Trait for hotkey detection implementations
pub trait HotkeyListener: Send {
    /// Start listening for hotkey events
    /// Returns a channel receiver for events
    fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>, HotkeyError>;

    /// Stop listening and clean up
    fn stop(&mut self) -> Result<(), HotkeyError>;
}

/// Factory function to create the appropriate hotkey listener
pub fn create_listener(
    config: &HotkeyConfig,
    secondary_model: Option<String>,
) -> Result<Box<dyn HotkeyListener>, HotkeyError> {
    let mut listener = evdev_listener::EvdevListener::new(config)?;
    listener.set_secondary_model(secondary_model);
    Ok(Box::new(listener))
}
