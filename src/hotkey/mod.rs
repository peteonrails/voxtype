//! Hotkey detection module
//!
//! On Linux, provides kernel-level key event detection using evdev.
//! On macOS, hotkey detection is not built-in; use `voxtype record toggle`
//! bound to a system keyboard shortcut instead.

#[cfg(target_os = "linux")]
pub mod evdev_listener;

use crate::config::HotkeyConfig;
use crate::error::HotkeyError;
use tokio::sync::mpsc;

/// Events emitted by the hotkey listener
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The hotkey was pressed, optionally with a model override
    Pressed {
        /// Model to use for this transcription (None = use default)
        model_override: Option<String>,
    },
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
#[cfg(target_os = "linux")]
pub fn create_listener(
    config: &HotkeyConfig,
    secondary_model: Option<String>,
) -> Result<Box<dyn HotkeyListener>, HotkeyError> {
    let mut listener = evdev_listener::EvdevListener::new(config)?;
    listener.set_secondary_model(secondary_model);
    Ok(Box::new(listener))
}

/// Factory function to create the appropriate hotkey listener
/// On macOS, built-in hotkey detection is not available.
#[cfg(not(target_os = "linux"))]
pub fn create_listener(
    _config: &HotkeyConfig,
    _secondary_model: Option<String>,
) -> Result<Box<dyn HotkeyListener>, HotkeyError> {
    Err(HotkeyError::Evdev(
        "Built-in hotkey detection is not available on macOS. \
         Set [hotkey] enabled = false in config.toml and use \
         'voxtype record toggle' bound to a system keyboard shortcut."
            .to_string(),
    ))
}
