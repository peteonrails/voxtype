//! Audio capture and feedback configuration.

use serde::{Deserialize, Serialize};

/// Audio capture configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioConfig {
    /// PipeWire/PulseAudio device name, or "default"
    #[serde(default = "default_audio_device")]
    pub device: String,

    /// Sample rate in Hz (whisper expects 16000)
    #[serde(default = "default_audio_sample_rate")]
    pub sample_rate: u32,

    /// Maximum recording duration in seconds (safety limit)
    #[serde(default = "default_audio_max_duration_secs")]
    pub max_duration_secs: u32,

    /// Pause MPRIS media players during recording and resume on stop
    #[serde(default)]
    pub pause_media: bool,

    /// MPRIS player bus-name suffixes to skip when pausing. Matched against
    /// the part after `org.mpris.MediaPlayer2.` either exactly or as a
    /// `<entry>.<instance>` prefix (e.g. `"chromium"` matches
    /// `chromium.instance123`). Useful for ignoring browsers whose MPRIS
    /// status is unreliable, or background players you never want paused.
    #[serde(default)]
    pub pause_media_ignored_players: Vec<String>,

    /// Lower active media stream volume during recording and restore on stop
    #[serde(default)]
    pub duck_media: bool,

    /// Fraction of its current amplitude a ducked stream keeps, in percent
    /// (50 = half the amplitude, -6 dB). Converted internally to PulseAudio's
    /// cubic percentage scale.
    #[serde(default = "default_duck_media_volume_percent")]
    pub duck_media_volume_percent: u8,

    /// Fade duration in milliseconds for the ducking ramp (0 = instant)
    #[serde(default = "default_duck_media_fade_ms")]
    pub duck_media_fade_ms: u32,

    /// Wait for the input device to deliver real audio before playing the
    /// recording-start cue and showing the OSD. Devices resuming from idle
    /// suspend produce ~0.5s of digital silence; without this gate, users
    /// who speak as soon as they see/hear the cue lose their first word.
    #[serde(default = "default_audio_wait_for_device")]
    pub wait_for_device: bool,

    /// Audio feedback settings
    #[serde(default)]
    pub feedback: AudioFeedbackConfig,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            device: default_audio_device(),
            sample_rate: default_audio_sample_rate(),
            max_duration_secs: default_audio_max_duration_secs(),
            pause_media: false,
            pause_media_ignored_players: Vec::new(),
            duck_media: false,
            duck_media_volume_percent: default_duck_media_volume_percent(),
            duck_media_fade_ms: default_duck_media_fade_ms(),
            wait_for_device: default_audio_wait_for_device(),
            feedback: AudioFeedbackConfig::default(),
        }
    }
}

fn default_audio_device() -> String {
    "default".to_string()
}

fn default_audio_sample_rate() -> u32 {
    16000
}

fn default_audio_max_duration_secs() -> u32 {
    60
}

/// 34 rather than 70 because the value now means amplitude directly. Before
/// the cube-root correction the configured percentage was applied to
/// PulseAudio's own cubic scale, so the shipped default of 70 actually left
/// 0.70^3 = 0.343 of the amplitude. 34 reproduces that same audible depth
/// under the corrected meaning; keeping 70 would have quietly turned a -9.3 dB
/// duck into a -3.1 dB one for everyone on defaults.
fn default_duck_media_volume_percent() -> u8 {
    34
}

fn default_duck_media_fade_ms() -> u32 {
    150
}

fn default_audio_wait_for_device() -> bool {
    true
}

/// Audio feedback configuration for sound cues
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioFeedbackConfig {
    /// Enable audio feedback sounds
    #[serde(default)]
    pub enabled: bool,

    /// Sound theme: "default", "subtle", "mechanical", or path to custom theme directory
    #[serde(default = "default_sound_theme")]
    pub theme: String,

    /// Volume level (0.0 to 1.0)
    #[serde(default = "default_volume")]
    pub volume: f32,
}

fn default_sound_theme() -> String {
    "default".to_string()
}

fn default_volume() -> f32 {
    0.7
}

impl Default for AudioFeedbackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            theme: default_sound_theme(),
            volume: default_volume(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_for_device_defaults_to_true() {
        assert!(AudioConfig::default().wait_for_device);
        // Old configs without the field must keep the default
        let cfg: AudioConfig = toml::from_str("").unwrap();
        assert!(cfg.wait_for_device);
    }

    #[test]
    fn wait_for_device_can_be_disabled() {
        let cfg: AudioConfig = toml::from_str("wait_for_device = false").unwrap();
        assert!(!cfg.wait_for_device);
    }
}
