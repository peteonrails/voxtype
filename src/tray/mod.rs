//! System tray integration via StatusNotifierItem (SNI) protocol
//!
//! Provides a tray icon that reflects daemon state and accepts user interaction.
//! Requires a StatusNotifierHost (KDE Plasma, GNOME with AppIndicator extension,
//! Waybar with tray module, etc.).

mod sni;

/// Tray icon state, mirroring daemon state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Recording,
    Transcribing,
}

/// Events sent from the tray to the daemon
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    ToggleRecording,
    CancelTranscription,
    Quit,
}

/// Spawn the system tray icon.
///
/// Returns channels for bidirectional communication, or `None` if the tray
/// could not be started (e.g., no DBus session bus available).
///
/// - `Receiver<TrayEvent>`: events from user interaction (click, menu)
/// - `watch::Sender<TrayState>`: send state updates to the tray icon
pub fn spawn_tray() -> Option<(
    tokio::sync::mpsc::Receiver<TrayEvent>,
    tokio::sync::watch::Sender<TrayState>,
)> {
    sni::spawn()
}
