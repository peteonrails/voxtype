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
# replacements = { "vox type" = "voxtype" }

# [status]
# Status display icons for Waybar/tray integrations
#
# Icon theme (or path to custom theme file):
#   Font-based (require specific fonts):
#     - "emoji"     - Default emoji icons (🎙️ 🎤 ⏳)
#     - "nerd-font" - Nerd Font icons (requires Nerd Font)
#     - "material"  - Material Design Icons (requires MDI font)
#     - "phosphor"  - Phosphor Icons (requires Phosphor font)
#     - "codicons"  - VS Code icons (requires Codicons font)
#     - "omarchy"   - Omarchy distro icons
#   Universal (no special fonts needed):
#     - "minimal"   - Simple Unicode (○ ● ◐ ×)
#     - "dots"      - Geometric shapes (◯ ⬤ ◔ ◌)
#     - "arrows"    - Media player style (▶ ● ↻ ■)
#     - "text"      - Plain text ([MIC] [REC] [...] [OFF])
# icon_theme = "emoji"
#
# Per-state icon overrides (optional, takes precedence over theme)
# [status.icons]
# idle = "🎙️"
# recording = "🎤"
# transcribing = "⏳"
# stopped = ""
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

    /// Status display configuration (icons for Waybar/tray integrations)
    #[serde(default)]
    pub status: StatusConfig,

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

/// Status display configuration for Waybar/tray integrations
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusConfig {
    /// Icon theme: "emoji", "nerd-font", "omarchy", "minimal", or path to custom theme
    #[serde(default = "default_icon_theme")]
    pub icon_theme: String,

    /// Per-state icon overrides (optional, takes precedence over theme)
    #[serde(default)]
    pub icons: StatusIconOverrides,
}

fn default_icon_theme() -> String {
    "emoji".to_string()
}

impl Default for StatusConfig {
    fn default() -> Self {
        Self {
            icon_theme: default_icon_theme(),
            icons: StatusIconOverrides::default(),
        }
    }
}

/// Per-state icon overrides for status display
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct StatusIconOverrides {
    pub idle: Option<String>,
    pub recording: Option<String>,
    pub transcribing: Option<String>,
    pub stopped: Option<String>,
}

/// Resolved icons for each state (after applying theme + overrides)
#[derive(Debug, Clone)]
pub struct ResolvedIcons {
    pub idle: String,
    pub recording: String,
    pub transcribing: String,
    pub stopped: String,
}

impl StatusConfig {
    /// Resolve icons by loading theme and applying any overrides
    pub fn resolve_icons(&self) -> ResolvedIcons {
        // Start with theme defaults
        let mut icons = load_icon_theme(&self.icon_theme);

        // Apply per-state overrides
        if let Some(ref icon) = self.icons.idle {
            icons.idle = icon.clone();
        }
        if let Some(ref icon) = self.icons.recording {
            icons.recording = icon.clone();
        }
        if let Some(ref icon) = self.icons.transcribing {
            icons.transcribing = icon.clone();
        }
        if let Some(ref icon) = self.icons.stopped {
            icons.stopped = icon.clone();
        }

        icons
    }
}

/// Load an icon theme by name or from a custom file path
fn load_icon_theme(theme: &str) -> ResolvedIcons {
    match theme {
        "emoji" => ResolvedIcons {
            idle: "🎙️".to_string(),
            recording: "🎤".to_string(),
            transcribing: "⏳".to_string(),
            stopped: "".to_string(),
        },
        "nerd-font" => ResolvedIcons {
            // Nerd Font icons: microphone, circle, spinner, microphone-slash
            idle: "\u{f130}".to_string(),      // nf-fa-microphone
            recording: "\u{f111}".to_string(), // nf-fa-circle (filled)
            transcribing: "\u{f110}".to_string(), // nf-fa-spinner
            stopped: "\u{f131}".to_string(),   // nf-fa-microphone_slash
        },
        "omarchy" => ResolvedIcons {
            // Icons from hyprwhspr for Omarchy compatibility
            idle: "\u{ec12}".to_string(),
            recording: "\u{ec1c}".to_string(),
            transcribing: "\u{ec1c}".to_string(),
            stopped: "\u{ec12}".to_string(),
        },
        "minimal" => ResolvedIcons {
            idle: "○".to_string(),
            recording: "●".to_string(),
            transcribing: "◐".to_string(),
            stopped: "×".to_string(),
        },
        "material" => ResolvedIcons {
            // Material Design Icons (requires MDI font)
            idle: "\u{f036c}".to_string(),      // mdi-microphone
            recording: "\u{f040a}".to_string(), // mdi-record
            transcribing: "\u{f04ce}".to_string(), // mdi-sync
            stopped: "\u{f036d}".to_string(),   // mdi-microphone-off
        },
        "phosphor" => ResolvedIcons {
            // Phosphor Icons (requires Phosphor font)
            idle: "\u{e43a}".to_string(),      // ph-microphone
            recording: "\u{e438}".to_string(), // ph-record
            transcribing: "\u{e225}".to_string(), // ph-circle-notch (spinner)
            stopped: "\u{e43b}".to_string(),   // ph-microphone-slash
        },
        "codicons" => ResolvedIcons {
            // VS Code Codicons (requires Codicons font)
            idle: "\u{eb51}".to_string(),      // codicon-mic
            recording: "\u{ebfc}".to_string(), // codicon-record
            transcribing: "\u{eb4c}".to_string(), // codicon-sync
            stopped: "\u{eb52}".to_string(),   // codicon-mute
        },
        "text" => ResolvedIcons {
            // Plain text labels (no special fonts required)
            idle: "[MIC]".to_string(),
            recording: "[REC]".to_string(),
            transcribing: "[...]".to_string(),
            stopped: "[OFF]".to_string(),
        },
        "dots" => ResolvedIcons {
            // Unicode geometric shapes (no special fonts required)
            idle: "◯".to_string(),        // U+25EF white circle
            recording: "⬤".to_string(),   // U+2B24 black large circle
            transcribing: "◔".to_string(), // U+25D4 circle with upper right quadrant black
            stopped: "◌".to_string(),      // U+25CC dotted circle
        },
        "arrows" => ResolvedIcons {
            // Media player style (no special fonts required)
            idle: "▶".to_string(),        // U+25B6 play
            recording: "●".to_string(),   // U+25CF black circle
            transcribing: "↻".to_string(), // U+21BB clockwise arrow
            stopped: "■".to_string(),      // U+25A0 black square
        },
        path => load_custom_icon_theme(path).unwrap_or_else(|e| {
            tracing::warn!("Failed to load custom icon theme '{}': {}, using emoji", path, e);
            load_icon_theme("emoji")
        }),
    }
}

/// Load a custom icon theme from a TOML file
fn load_custom_icon_theme(path: &str) -> Result<ResolvedIcons, String> {
    let path = PathBuf::from(path);
    if !path.exists() {
        return Err(format!("Theme file not found: {}", path.display()));
    }

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read theme file: {}", e))?;

    #[derive(Deserialize)]
    struct ThemeFile {
        idle: Option<String>,
        recording: Option<String>,
        transcribing: Option<String>,
        stopped: Option<String>,
    }

    let theme: ThemeFile = toml::from_str(&contents)
        .map_err(|e| format!("Invalid theme file: {}", e))?;

    // Start with emoji defaults, override with file values
    let base = load_icon_theme("emoji");
    Ok(ResolvedIcons {
        idle: theme.idle.unwrap_or(base.idle),
        recording: theme.recording.unwrap_or(base.recording),
        transcribing: theme.transcribing.unwrap_or(base.transcribing),
        stopped: theme.stopped.unwrap_or(base.stopped),
    })
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
    /// Example: { "vox type" = "voxtype" }
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
            status: StatusConfig::default(),
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

    #[test]
    fn test_builtin_icon_themes() {
        // Test all built-in themes load correctly
        let themes = ["emoji", "nerd-font", "material", "phosphor", "codicons",
                      "omarchy", "minimal", "dots", "arrows", "text"];

        for theme in themes {
            let icons = load_icon_theme(theme);
            assert!(!icons.idle.is_empty() || theme == "emoji", "Theme {} should have idle icon", theme);
            assert!(!icons.recording.is_empty(), "Theme {} should have recording icon", theme);
            assert!(!icons.transcribing.is_empty(), "Theme {} should have transcribing icon", theme);
            // stopped can be empty for some themes
        }
    }

    #[test]
    fn test_emoji_theme_icons() {
        let icons = load_icon_theme("emoji");
        assert!(icons.idle.contains("🎙"));
        assert!(icons.recording.contains("🎤"));
        assert!(icons.transcribing.contains("⏳"));
        assert!(icons.stopped.is_empty());
    }

    #[test]
    fn test_text_theme_icons() {
        let icons = load_icon_theme("text");
        assert_eq!(icons.idle, "[MIC]");
        assert_eq!(icons.recording, "[REC]");
        assert_eq!(icons.transcribing, "[...]");
        assert_eq!(icons.stopped, "[OFF]");
    }

    #[test]
    fn test_minimal_theme_icons() {
        let icons = load_icon_theme("minimal");
        assert_eq!(icons.idle, "○");
        assert_eq!(icons.recording, "●");
        assert_eq!(icons.transcribing, "◐");
        assert_eq!(icons.stopped, "×");
    }

    #[test]
    fn test_status_config_default() {
        let status = StatusConfig::default();
        assert_eq!(status.icon_theme, "emoji");
        assert!(status.icons.idle.is_none());
        assert!(status.icons.recording.is_none());
    }

    #[test]
    fn test_status_config_resolve_icons() {
        let status = StatusConfig {
            icon_theme: "text".to_string(),
            icons: StatusIconOverrides::default(),
        };
        let icons = status.resolve_icons();
        assert_eq!(icons.idle, "[MIC]");
        assert_eq!(icons.recording, "[REC]");
    }

    #[test]
    fn test_status_config_icon_overrides() {
        let status = StatusConfig {
            icon_theme: "emoji".to_string(),
            icons: StatusIconOverrides {
                idle: None,
                recording: Some("🔴".to_string()),
                transcribing: None,
                stopped: Some("⚫".to_string()),
            },
        };
        let icons = status.resolve_icons();
        // idle should be from emoji theme
        assert!(icons.idle.contains("🎙"));
        // recording should be overridden
        assert_eq!(icons.recording, "🔴");
        // transcribing should be from emoji theme
        assert!(icons.transcribing.contains("⏳"));
        // stopped should be overridden
        assert_eq!(icons.stopped, "⚫");
    }

    #[test]
    fn test_parse_status_config_from_toml() {
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

            [status]
            icon_theme = "nerd-font"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.status.icon_theme, "nerd-font");
    }

    #[test]
    fn test_parse_status_icon_overrides_from_toml() {
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

            [status]
            icon_theme = "emoji"

            [status.icons]
            recording = "🔴"
            stopped = "⚫"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.status.icon_theme, "emoji");
        assert!(config.status.icons.idle.is_none());
        assert_eq!(config.status.icons.recording, Some("🔴".to_string()));
        assert!(config.status.icons.transcribing.is_none());
        assert_eq!(config.status.icons.stopped, Some("⚫".to_string()));

        let icons = config.status.resolve_icons();
        assert_eq!(icons.recording, "🔴");
        assert_eq!(icons.stopped, "⚫");
    }

    #[test]
    fn test_invalid_theme_falls_back_to_emoji() {
        // Non-existent file path should fall back to emoji
        let icons = load_icon_theme("/nonexistent/path/theme.toml");
        assert!(icons.idle.contains("🎙"));
    }

    #[test]
    fn test_custom_theme_file() {
        use std::io::Write;

        // Create a temporary theme file
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        writeln!(temp_file, r#"
            idle = "IDLE"
            recording = "REC"
            transcribing = "BUSY"
            stopped = "OFF"
        "#).unwrap();

        let icons = load_icon_theme(temp_file.path().to_str().unwrap());
        assert_eq!(icons.idle, "IDLE");
        assert_eq!(icons.recording, "REC");
        assert_eq!(icons.transcribing, "BUSY");
        assert_eq!(icons.stopped, "OFF");
    }

    #[test]
    fn test_custom_theme_file_partial() {
        use std::io::Write;

        // Create a theme file with only some icons (others should default to emoji)
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        writeln!(temp_file, r#"
            recording = "🔴"
        "#).unwrap();

        let icons = load_icon_theme(temp_file.path().to_str().unwrap());
        // Only recording is overridden, others fall back to emoji
        assert!(icons.idle.contains("🎙"));
        assert_eq!(icons.recording, "🔴");
        assert!(icons.transcribing.contains("⏳"));
    }
}
