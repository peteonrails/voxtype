//! Configuration loading and types for voxtype
//!
//! Configuration is loaded in layers:
//! 1. Built-in defaults
//! 2. Config file (~/.config/voxtype/config.toml)
//! 3. Environment variables (VOXTYPE_*)
//! 4. CLI arguments (highest priority)

use crate::error::VoxtypeError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Default configuration file content
pub const DEFAULT_CONFIG: &str = r#"# Voxtype Configuration
#
# Location: ~/.config/voxtype/config.toml
# All settings can be overridden via CLI flags

# State file for external integrations (Waybar, polybar, etc.)
# Use "auto" for default location ($XDG_RUNTIME_DIR/voxtype/state),
# a custom path, or "disabled" to turn off. The daemon writes state
# ("idle", "recording", "transcribing") to this file whenever it changes.
# Required for `voxtype record toggle` and `voxtype status` commands.
state_file = "auto"

[hotkey]
# Key to hold for push-to-talk
# Common choices: SCROLLLOCK, PAUSE, RIGHTALT, F13-F24
# Use `evtest` to find key names for your keyboard
key = "SCROLLLOCK"

# Optional modifier keys that must also be held
# Example: modifiers = ["LEFTCTRL", "LEFTALT"]
modifiers = []

# Activation mode: "push_to_talk" or "toggle"
# - push_to_talk: Hold hotkey to record, release to transcribe (default)
# - toggle: Press hotkey once to start recording, press again to stop
# mode = "push_to_talk"

# Enable built-in hotkey detection (default: true)
# Set to false when using compositor keybindings (Hyprland, Sway) instead
# When disabled, use `voxtype record start/stop/toggle` to control recording
# enabled = true

[audio]
# Audio input device ("default" uses system default)
# List devices with: pactl list sources short
device = "default"

# Sample rate in Hz (whisper expects 16000)
sample_rate = 16000

# Maximum recording duration in seconds (safety limit)
max_duration_secs = 60

# [audio.feedback]
# Enable audio feedback sounds (beeps when recording starts/stops)
# enabled = true
#
# Sound theme: "default", "subtle", "mechanical", or path to custom theme directory
# theme = "default"
#
# Volume level (0.0 to 1.0)
# volume = 0.7

[whisper]
# Model to use for transcription
# Options: tiny, tiny.en, base, base.en, small, small.en, medium, medium.en, large-v3, large-v3-turbo
# .en models are English-only but faster and more accurate for English
# large-v3-turbo is faster than large-v3 with minimal accuracy loss (recommended for GPU)
# Or provide absolute path to a custom .bin model file
model = "base.en"

# Language for transcription
# Use "en" for English, "auto" for auto-detection
# See: https://github.com/openai/whisper#available-models-and-languages
language = "en"

# Translate non-English speech to English
translate = false

# Number of CPU threads for inference (omit for auto-detection)
# threads = 4

[output]
# Primary output mode: "type" or "clipboard"
# - type: Simulates keyboard input at cursor position (requires ydotool)
# - clipboard: Copies text to clipboard (requires wl-copy)
mode = "type"

# Fall back to clipboard if typing fails
fallback_to_clipboard = true

# Delay between typed characters in milliseconds
# 0 = fastest possible, increase if characters are dropped
type_delay_ms = 0

# Post-processing command (optional)
# Pipe transcribed text through an external command for cleanup before output.
# The command receives text on stdin and outputs processed text on stdout.
# Useful for LLM-based text cleanup, grammar correction, filler word removal.
# On any failure (timeout, error), falls back to original transcription.
#
# [output.post_process]
# command = "ollama run llama3.2:1b 'Clean up this dictation. Fix grammar, remove filler words. Output only the cleaned text:'"
# timeout_ms = 30000  # 30 second timeout (generous for LLM)

[output.notification]
# Show notification when recording starts (hotkey pressed)
on_recording_start = false

# Show notification when recording stops (transcription beginning)
on_recording_stop = false

# Show notification with transcribed text after transcription completes
on_transcription = true

# [text]
# Text processing options (word replacements, spoken punctuation)
#
# Enable spoken punctuation conversion (e.g., say "period" to get ".")
# spoken_punctuation = false
#
# Custom word replacements (case-insensitive)
# replacements = { "hyperwhisper" = "hyprwhspr" }
"#;

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

/// Root configuration structure
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub audio: AudioConfig,
    pub whisper: WhisperConfig,
    pub output: OutputConfig,

    /// Text processing configuration (replacements, spoken punctuation)
    #[serde(default)]
    pub text: TextConfig,

    /// Optional path to state file for external integrations (e.g., Waybar)
    /// When set, the daemon writes current state ("idle", "recording", "transcribing")
    /// to this file whenever state changes.
    /// Example: "/run/user/1000/voxtype/state" or use "auto" for default location
    #[serde(default)]
    pub state_file: Option<String>,
}

/// Hotkey detection configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HotkeyConfig {
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
}

/// Audio capture configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioConfig {
    /// PipeWire/PulseAudio device name, or "default"
    pub device: String,

    /// Sample rate in Hz (whisper expects 16000)
    pub sample_rate: u32,

    /// Maximum recording duration in seconds (safety limit)
    pub max_duration_secs: u32,

    /// Audio feedback settings
    #[serde(default)]
    pub feedback: AudioFeedbackConfig,
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

fn default_hotkey_key() -> String {
    "SCROLLLOCK".to_string()
}

fn default_sound_theme() -> String {
    "default".to_string()
}

fn default_volume() -> f32 {
    0.7
}

fn default_on_demand_loading() -> bool {
    false
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

/// Whisper speech-to-text configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhisperConfig {
    /// Model name: tiny, base, small, medium, large-v3, large-v3-turbo
    /// Can also be an absolute path to a .bin file
    pub model: String,

    /// Language code (en, es, fr, auto, etc.)
    pub language: String,

    /// Translate to English if source language is not English
    #[serde(default)]
    pub translate: bool,

    /// Number of threads for inference (None = auto-detect)
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

/// Text processing configuration
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TextConfig {
    /// Enable spoken punctuation conversion (e.g., "period" → ".")
    #[serde(default)]
    pub spoken_punctuation: bool,

    /// Custom word replacements (case-insensitive)
    /// Example: { "hyperwhisper" = "hyprwhspr" }
    #[serde(default)]
    pub replacements: HashMap<String, String>,
}

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
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            on_recording_start: false,
            on_recording_stop: false,
            on_transcription: true,
        }
    }
}

/// Post-processing command configuration
///
/// Pipes transcribed text through an external command for cleanup/formatting.
/// Commonly used with local LLMs (Ollama, llama.cpp) or text processing tools.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostProcessConfig {
    /// Shell command to execute
    /// Receives transcribed text on stdin, outputs processed text on stdout
    pub command: String,

    /// Timeout in milliseconds (default: 30000 = 30 seconds)
    #[serde(default = "default_post_process_timeout")]
    pub timeout_ms: u64,
}

fn default_post_process_timeout() -> u64 {
    30000 // 30 seconds - generous for LLM processing
}

/// Text output configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutputConfig {
    /// Primary output mode
    pub mode: OutputMode,

    /// Fall back to clipboard if typing fails
    #[serde(default = "default_true")]
    pub fallback_to_clipboard: bool,

    /// Notification settings
    #[serde(default)]
    pub notification: NotificationConfig,

    /// Delay between typed characters (ms), 0 for fastest
    #[serde(default)]
    pub type_delay_ms: u32,

    /// Optional post-processing command configuration
    /// Pipes transcribed text through an external command before output
    #[serde(default)]
    pub post_process: Option<PostProcessConfig>,
}

/// Output mode selection
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    /// Simulate keyboard input (requires ydotool)
    Type,
    /// Copy to clipboard (requires wl-copy)
    Clipboard,
    /// Copy to clipboard then paste with Ctrl+V (requires wl-copy and ydotool)
    Paste,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig {
                key: "SCROLLLOCK".to_string(),
                modifiers: vec![],
                mode: ActivationMode::default(),
                enabled: true,
            },
            audio: AudioConfig {
                device: "default".to_string(),
                sample_rate: 16000,
                max_duration_secs: 60,
                feedback: AudioFeedbackConfig::default(),
            },
            whisper: WhisperConfig {
                model: "base.en".to_string(),
                language: "en".to_string(),
                translate: false,
                threads: None,
                on_demand_loading: default_on_demand_loading(),
            },
            output: OutputConfig {
                mode: OutputMode::Type,
                fallback_to_clipboard: true,
                notification: NotificationConfig::default(),
                type_delay_ms: 0,
                post_process: None,
            },
            text: TextConfig::default(),
            state_file: Some("auto".to_string()),
        }
    }
}

impl Config {
    /// Get the default config file path
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "voxtype")
            .map(|dirs| dirs.config_dir().join("config.toml"))
    }

    /// Get the runtime directory for ephemeral files (state, sockets)
    pub fn runtime_dir() -> PathBuf {
        // Use XDG_RUNTIME_DIR if available, otherwise fall back to /tmp
        std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
            .join("voxtype")
    }

    /// Resolve the state file path from config
    /// Returns None if state_file is not configured or explicitly disabled
    /// Returns the resolved path if set to "auto" or an explicit path
    pub fn resolve_state_file(&self) -> Option<PathBuf> {
        self.state_file.as_ref().and_then(|path| {
            match path.to_lowercase().as_str() {
                "disabled" | "none" | "off" | "false" => None,
                "auto" => Some(Self::runtime_dir().join("state")),
                _ => Some(PathBuf::from(path)),
            }
        })
    }

    /// Get the config directory path
    pub fn config_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "voxtype")
            .map(|dirs| dirs.config_dir().to_path_buf())
    }

    /// Get the data directory path (for models)
    pub fn data_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "voxtype")
            .map(|dirs| dirs.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Get the models directory path
    pub fn models_dir() -> PathBuf {
        Self::data_dir().join("models")
    }

    /// Ensure all required directories exist
    /// Creates: config dir, data dir, and models dir
    pub fn ensure_directories() -> std::io::Result<()> {
        // Create config directory
        if let Some(config_dir) = Self::config_dir() {
            std::fs::create_dir_all(&config_dir)?;
            tracing::debug!("Ensured config directory exists: {:?}", config_dir);
        }

        // Create models directory (includes data dir)
        let models_dir = Self::models_dir();
        std::fs::create_dir_all(&models_dir)?;
        tracing::debug!("Ensured models directory exists: {:?}", models_dir);

        Ok(())
    }
}

/// Load configuration from file, with defaults for missing values
pub fn load_config(path: Option<&Path>) -> Result<Config, VoxtypeError> {
    // Start with defaults
    let mut config = Config::default();

    // Determine config file path
    let config_path = path
        .map(PathBuf::from)
        .or_else(Config::default_path);

    // Load from file if it exists
    if let Some(ref path) = config_path {
        if path.exists() {
            tracing::debug!("Loading config from {:?}", path);
            let contents = std::fs::read_to_string(path)
                .map_err(|e| VoxtypeError::Config(format!("Failed to read config: {}", e)))?;

            config = toml::from_str(&contents)
                .map_err(|e| VoxtypeError::Config(format!("Invalid config: {}", e)))?;
        } else {
            tracing::debug!("Config file not found at {:?}, using defaults", path);
        }
    }

    // Override from environment variables
    if let Ok(key) = std::env::var("VOXTYPE_HOTKEY") {
        config.hotkey.key = key;
    }
    if let Ok(model) = std::env::var("VOXTYPE_MODEL") {
        config.whisper.model = model;
    }
    if let Ok(mode) = std::env::var("VOXTYPE_OUTPUT_MODE") {
        config.output.mode = match mode.to_lowercase().as_str() {
            "clipboard" => OutputMode::Clipboard,
            "paste" => OutputMode::Paste,
            _ => OutputMode::Type,
        };
    }

    Ok(config)
}

/// Save configuration to file
#[allow(dead_code)]
pub fn save_config(config: &Config, path: &Path) -> Result<(), VoxtypeError> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| VoxtypeError::Config(format!("Failed to create config dir: {}", e)))?;
    }

    let contents = toml::to_string_pretty(config)
        .map_err(|e| VoxtypeError::Config(format!("Failed to serialize config: {}", e)))?;

    std::fs::write(path, contents)
        .map_err(|e| VoxtypeError::Config(format!("Failed to write config: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.hotkey.key, "SCROLLLOCK");
        assert_eq!(config.hotkey.mode, ActivationMode::PushToTalk);
        assert_eq!(config.audio.sample_rate, 16000);
        assert!(!config.audio.feedback.enabled);
        assert_eq!(config.whisper.model, "base.en");
        assert_eq!(config.output.mode, OutputMode::Type);
    }

    #[test]
    fn test_parse_config_toml() {
        let toml_str = r#"
            [hotkey]
            key = "PAUSE"
            modifiers = ["LEFTCTRL"]

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 30

            [whisper]
            model = "small.en"
            language = "en"

            [output]
            mode = "clipboard"

            [output.notification]
            on_recording_start = true
            on_recording_stop = true
            on_transcription = false
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hotkey.key, "PAUSE");
        assert_eq!(config.hotkey.modifiers, vec!["LEFTCTRL"]);
        assert_eq!(config.hotkey.mode, ActivationMode::PushToTalk); // default
        assert_eq!(config.whisper.model, "small.en");
        assert_eq!(config.output.mode, OutputMode::Clipboard);
        assert!(config.output.notification.on_recording_start);
        assert!(config.output.notification.on_recording_stop);
        assert!(!config.output.notification.on_transcription);
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
        assert_eq!(config.hotkey.key, "SCROLLLOCK"); // defaults to SCROLLLOCK
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
}
