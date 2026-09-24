//! Global hotkey detection for Linux.
//!
//! Supports kernel input events through evdev and desktop-managed shortcuts
//! through the XDG GlobalShortcuts portal.

pub(crate) mod auto_listener;
pub mod evdev_listener;
pub(crate) mod portal_listener;

use crate::config::{HotkeyBackend, HotkeyConfig};
use crate::error::HotkeyError;
use async_trait::async_trait;
use std::collections::HashSet;
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
}

/// Trait for hotkey detection implementations
#[async_trait]
pub trait HotkeyListener: Send {
    /// Start listening for hotkey events
    /// Returns a channel receiver for events
    async fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>, HotkeyError>;

    /// Stop listening and clean up
    async fn stop(&mut self) -> Result<(), HotkeyError>;
}

/// Factory function to create the appropriate hotkey listener
pub fn create_listener(
    config: &HotkeyConfig,
    secondary_model: Option<String>,
    profiles: &HashSet<String>,
) -> Result<Box<dyn HotkeyListener>, HotkeyError> {
    match config.backend {
        HotkeyBackend::Evdev => {
            let mut listener = evdev_listener::EvdevListener::new(config)?;
            listener.set_secondary_model(secondary_model);
            Ok(Box::new(listener))
        }
        HotkeyBackend::Portal => Ok(Box::new(portal_listener::PortalListener::new(
            config,
            secondary_model,
            profiles,
            portal_listener::OnPermanentFailure::NotifyUser,
        ))),
        HotkeyBackend::Auto => Ok(Box::new(auto_listener::AutoListener::new(
            config,
            secondary_model,
            profiles,
        ))),
    }
}
