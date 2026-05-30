//! Notification configuration.

use serde::{Deserialize, Serialize};

use super::default_true;

/// Notification configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NotificationConfig {
    /// Notify when recording starts (hotkey pressed)
    #[serde(default)]
    pub on_recording_start: bool,

    /// Notify when recording stops (hotkey released, transcription starting)
    #[serde(default)]
    pub on_recording_stop: bool,

    /// Notify with transcribed text after transcription completes
    #[serde(default = "default_true")]
    pub on_transcription: bool,

    /// Show engine icon in notification title (🦜 for Parakeet, 🗣️ for Whisper)
    #[serde(default)]
    pub show_engine_icon: bool,

    /// Notification urgency level: "low", "normal", or "critical".
    /// On GNOME, "low" notifications go straight to the drawer without a popup banner.
    #[serde(default = "default_notification_urgency")]
    pub urgency: String,
}

fn default_notification_urgency() -> String {
    "normal".to_string()
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            on_recording_start: false,
            on_recording_stop: false,
            on_transcription: true,
            show_engine_icon: false,
            urgency: default_notification_urgency(),
        }
    }
}
