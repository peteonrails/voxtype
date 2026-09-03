//! Hotkey detection configuration.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::default_true;

/// Hotkey activation mode
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    /// Hold key to record, release to stop (default)
    #[default]
    PushToTalk,
    /// Press once to start recording, press again to stop
    Toggle,
}

/// Backend used to receive global hotkey events on Linux.
///
/// `crate::cli::HOTKEY_BACKENDS` mirrors these variants for the CLI's
/// `value_parser` and is pinned to them by a test in this file.
#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    Default,
    strum::IntoStaticStr,
    strum::Display,
    strum::EnumIter,
    strum::EnumString,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum HotkeyBackend {
    /// Read kernel input events from `/dev/input`.
    #[default]
    Evdev,
    /// Ask XDG Desktop Portal to manage global shortcuts.
    Portal,
    /// Prefer XDG Desktop Portal and use evdev when the portal is unavailable.
    Auto,
}

impl HotkeyBackend {
    /// The name this backend is written as in the config file, on the command
    /// line and in `VOXTYPE_HOTKEY_BACKEND`. Backed by `strum::IntoStaticStr`,
    /// so a new variant picks up its lowercase name without a match arm.
    pub fn name(self) -> &'static str {
        self.into()
    }
}

/// Hotkey detection configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HotkeyConfig {
    /// Backend used for global hotkeys on Linux.
    #[serde(default)]
    pub backend: HotkeyBackend,

    /// Key name (evdev KEY_* constant name, without the KEY_ prefix)
    /// Examples: "SCROLLLOCK", "RIGHTALT", "PAUSE", "F24"
    #[serde(default = "default_hotkey_key")]
    pub key: String,

    /// Optional modifier keys that must also be held
    /// Examples: ["LEFTCTRL"], ["LEFTALT", "LEFTSHIFT"]
    #[serde(default)]
    pub modifiers: Vec<String>,

    /// Activation mode: push_to_talk (hold to record) or toggle (press to start/stop)
    #[serde(default)]
    pub mode: ActivationMode,

    /// Enable built-in hotkey detection (default: true)
    /// Set to false when using compositor keybindings (Hyprland, Sway) instead
    /// When disabled, use `voxtype record start/stop/toggle` to control recording
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Optional cancel key (evdev KEY_* constant name, without KEY_ prefix)
    /// When pressed, cancels the current recording or transcription
    /// Examples: "ESC", "BACKSPACE", "F12"
    #[serde(default)]
    pub cancel_key: Option<String>,

    /// Optional modifier key for secondary model selection (evdev KEY_* name, without KEY_ prefix)
    /// When held while pressing the hotkey, uses secondary_model instead of the default model
    /// Examples: "LEFTSHIFT", "RIGHTALT", "LEFTCTRL"
    #[serde(default)]
    pub model_modifier: Option<String>,

    /// Optional modifier keys that activate named profiles (evdev KEY_* names, without KEY_ prefix)
    /// When held while pressing the hotkey, activates the named profile for post-processing
    /// Example: { "LEFTSHIFT" = "translate" } activates [profiles.translate] when Shift is held
    #[serde(default)]
    pub profile_modifiers: HashMap<String, String>,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            backend: HotkeyBackend::default(),
            key: default_hotkey_key(),
            modifiers: Vec::new(),
            mode: ActivationMode::default(),
            enabled: true,
            cancel_key: None,
            model_modifier: None,
            profile_modifiers: HashMap::new(),
        }
    }
}

pub(super) fn default_hotkey_key() -> String {
    #[cfg(target_os = "macos")]
    {
        "FN".to_string()
    }
    #[cfg(not(target_os = "macos"))]
    {
        "SCROLLLOCK".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use strum::IntoEnumIterator;

    /// Pin `crate::cli::HOTKEY_BACKENDS` to the `HotkeyBackend` variants, so a
    /// new variant cannot be accepted by the config file while
    /// `--hotkey-backend` rejects it.
    #[test]
    fn cli_hotkey_backends_lists_every_variant() {
        let from_enum: Vec<&str> = HotkeyBackend::iter().map(HotkeyBackend::name).collect();

        assert_eq!(
            from_enum,
            crate::cli::HOTKEY_BACKENDS,
            "src/cli/mod.rs::HOTKEY_BACKENDS is out of sync with HotkeyBackend. \
             Update the constant to match every variant's name() in declaration order."
        );
    }

    #[test]
    fn every_backend_name_parses_back_to_its_variant() {
        for backend in HotkeyBackend::iter() {
            assert_eq!(backend.to_string().parse(), Ok(backend));
        }
    }

    #[test]
    fn parse_backends() {
        for (value, expected) in [
            ("evdev", HotkeyBackend::Evdev),
            ("portal", HotkeyBackend::Portal),
            ("auto", HotkeyBackend::Auto),
        ] {
            let config: Config = toml::from_str(&format!("[hotkey]\nbackend = \"{value}\"\n"))
                .expect("hotkey backend should parse");

            assert_eq!(config.hotkey.backend, expected);
        }
    }

    #[test]
    fn backend_defaults_to_evdev() {
        let config: Config = toml::from_str("").expect("default config should parse");

        assert_eq!(config.hotkey.backend, HotkeyBackend::Evdev);
    }

    #[test]
    fn test_parse_hotkey_disabled_without_key() {
        // Regression test for GitHub issue #17
        // When hotkey is disabled, the key field should not be required
        let toml_str = r#"
            [hotkey]
            enabled = false

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.hotkey.enabled);
        assert_eq!(config.hotkey.key, default_hotkey_key()); // platform default
    }

    #[test]
    fn test_parse_toggle_mode() {
        let toml_str = r#"
            [hotkey]
            key = "F13"
            mode = "toggle"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [audio.feedback]
            enabled = true
            theme = "subtle"
            volume = 0.5

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hotkey.key, "F13");
        assert_eq!(config.hotkey.mode, ActivationMode::Toggle);
        assert!(config.audio.feedback.enabled);
        assert_eq!(config.audio.feedback.theme, "subtle");
        assert_eq!(config.audio.feedback.volume, 0.5);
    }

    #[test]
    fn test_parse_profile_modifiers() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [hotkey.profile_modifiers]
            LEFTSHIFT = "translate"
            RIGHTALT = "formal"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [profiles.translate]
            post_process_command = "translate.sh"

            [profiles.formal]
            post_process_command = "formal.sh"
            post_process_timeout_ms = 15000
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hotkey.profile_modifiers.len(), 2);
        assert_eq!(
            config.hotkey.profile_modifiers.get("LEFTSHIFT").unwrap(),
            "translate"
        );
        assert_eq!(
            config.hotkey.profile_modifiers.get("RIGHTALT").unwrap(),
            "formal"
        );
        assert!(config.get_profile("translate").is_some());
        assert!(config.get_profile("formal").is_some());
        assert_eq!(
            config
                .get_profile("translate")
                .unwrap()
                .post_process_command
                .as_deref(),
            Some("translate.sh")
        );
    }

    #[test]
    fn test_profile_modifiers_default_empty() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.hotkey.profile_modifiers.is_empty());
    }
}
