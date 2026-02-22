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

# Modifier key to select secondary model (evdev input mode only)
# When held while pressing the hotkey, uses whisper.secondary_model instead
# Example: model_modifier = "LEFTSHIFT"  # Shift+hotkey uses secondary model
# model_modifier = "LEFTSHIFT"

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
# Transcription backend: "local" or "remote"
# - local: Use whisper.cpp locally (default)
# - remote: Send audio to a remote whisper.cpp server or OpenAI-compatible API
# backend = "local"

# Model to use for transcription (local backend)
# Options: tiny, tiny.en, base, base.en, small, small.en, medium, medium.en, large-v3, large-v3-turbo
# .en models are English-only but faster and more accurate for English
# large-v3-turbo is faster than large-v3 with minimal accuracy loss (recommended for GPU)
# Or provide absolute path to a custom .bin model file
model = "base.en"

# Language for transcription
# Options:
#   - Single language: "en", "fr", "de", etc.
#   - Auto-detect all: "auto"
#   - Constrained auto-detect: ["en", "fr"] (detects from allowed set only)
# The array form helps with multilingual users where Whisper might misdetect
# the language, especially for short sentences.
# See: https://github.com/openai/whisper#available-models-and-languages
language = "en"

# Translate non-English speech to English
translate = false

# Number of CPU threads for inference (omit for auto-detection)
# threads = 4

# Initial prompt to provide context for transcription
# Use this to hint at terminology, proper nouns, or formatting conventions.
# Example: "Technical discussion about Rust, TypeScript, and Kubernetes."
# initial_prompt = ""

# --- Multi-model settings ---
#
# Secondary model for difficult audio (used with hotkey.model_modifier or CLI --model)
# secondary_model = "large-v3-turbo"
#
# List of available models that can be requested via CLI --model flag
# available_models = ["large-v3-turbo", "medium.en"]
#
# Maximum models to keep loaded in memory (LRU eviction when exceeded)
# Default: 2 (primary + one secondary). Only applies when gpu_isolation = false.
# max_loaded_models = 2
#
# Seconds before unloading idle secondary models (0 = never auto-unload)
# Default: 300 (5 minutes). Only applies when gpu_isolation = false.
# cold_model_timeout_secs = 300

# --- Eager processing settings ---
#
# Enable eager input processing (transcribe chunks while recording continues)
# Reduces perceived latency on slower machines by processing audio in parallel.
# eager_processing = false
#
# Duration of each audio chunk in seconds (default: 5.0)
# eager_chunk_secs = 5.0
#
# Overlap between chunks in seconds (helps catch words at boundaries, default: 0.5)
# eager_overlap_secs = 0.5

# --- Remote backend settings (used when backend = "remote") ---
#
# Remote server endpoint URL (required for remote backend)
# Examples:
#   - whisper.cpp server: "http://192.168.1.100:8080"
#   - OpenAI API: "https://api.openai.com"
# remote_endpoint = "http://192.168.1.100:8080"
#
# Model name to send to remote server (default: "whisper-1")
# remote_model = "whisper-1"
#
# API key for remote server (optional, or use VOXTYPE_WHISPER_API_KEY env var)
# remote_api_key = ""
#
# Timeout for remote requests in seconds (default: 30)
# remote_timeout_secs = 30

[output]
# Primary output mode: "type" or "clipboard"
# - type: Simulates keyboard input at cursor position (requires ydotool)
# - clipboard: Copies text to clipboard (requires wl-copy)
mode = "type"

# Fall back to clipboard if typing fails
fallback_to_clipboard = true

# Custom driver order for type mode (optional)
# Default order: wtype -> dotool -> ydotool -> clipboard
# Customize to prefer a specific driver or change the fallback order.
# Available drivers: wtype, dotool, ydotool, clipboard
# Example: prefer ydotool over dotool:
#   driver_order = ["wtype", "ydotool", "dotool", "clipboard"]
# Example: use only ydotool, no fallback:
#   driver_order = ["ydotool"]
# driver_order = ["wtype", "dotool", "ydotool", "clipboard"]

# Delay between typed characters in milliseconds
# 0 = fastest possible, increase if characters are dropped
type_delay_ms = 0

# Automatically submit (send Enter key) after outputting transcribed text
# Useful for chat applications, command lines, or forms where you want
# to auto-submit after dictation
# auto_submit = true

# Convert newlines to Shift+Enter instead of regular Enter
# Useful for applications where Enter submits (e.g., Cursor IDE, Slack, Discord)
# shift_enter_newlines = false

# Restore clipboard content after paste mode (default: false)
# Saves clipboard before transcription, restores it after paste keystroke
# Only applies to mode = "paste". Useful when you want to preserve your
# existing clipboard content across dictation operations.
# restore_clipboard = false

# Delay after paste before restoring clipboard (milliseconds)
# Allows time for the paste operation to complete (default: 200)
# restore_clipboard_delay_ms = 200

# Pre/post output hooks (optional)
# Commands to run before and after typing output. Useful for compositor integration.
# Example: Block modifier keys during typing with Hyprland submap:
#   pre_output_command = "hyprctl dispatch submap voxtype_suppress"
#   post_output_command = "hyprctl dispatch submap reset"
# See troubleshooting docs for the required Hyprland submap configuration.

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

# [vad]
# Voice Activity Detection - filters silence-only recordings
# Prevents Whisper hallucinations on silent audio
#
# enabled = false      # Enable VAD (off by default)
# threshold = 0.5      # 0.0 = sensitive, 1.0 = aggressive
# min_speech_duration_ms = 100  # Minimum speech required

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

# [profiles]
# Named profiles for context-specific post-processing
# Use with: voxtype record start --profile slack
#
# [profiles.slack]
# post_process_command = "ollama run llama3.2:1b 'Format for Slack...'"
#
# [profiles.code]
# post_process_command = "ollama run llama3.2:1b 'Format as code comment...'"
# output_mode = "clipboard"
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
    #[serde(default)]
    pub whisper: WhisperConfig,
    pub output: OutputConfig,

    /// Transcription engine: "whisper" (default) or "parakeet"
    /// Parakeet requires: cargo build --features parakeet
    #[serde(default)]
    pub engine: TranscriptionEngine,

    /// Parakeet configuration (optional, only used when engine = "parakeet")
    #[serde(default)]
    pub parakeet: Option<ParakeetConfig>,

    /// Moonshine configuration (optional, only used when engine = "moonshine")
    #[serde(default)]
    pub moonshine: Option<MoonshineConfig>,

    /// SenseVoice configuration (optional, only used when engine = "sensevoice")
    #[serde(default)]
    pub sensevoice: Option<SenseVoiceConfig>,

    /// Paraformer configuration (optional, only used when engine = "paraformer")
    #[serde(default)]
    pub paraformer: Option<ParaformerConfig>,

    /// Dolphin configuration (optional, only used when engine = "dolphin")
    #[serde(default)]
    pub dolphin: Option<DolphinConfig>,

    /// Omnilingual configuration (optional, only used when engine = "omnilingual")
    #[serde(default)]
    pub omnilingual: Option<OmnilingualConfig>,

    /// Text processing configuration (replacements, spoken punctuation)
    #[serde(default)]
    pub text: TextConfig,

    /// Voice Activity Detection configuration
    /// When enabled, filters silence-only recordings before transcription
    #[serde(default)]
    pub vad: VadConfig,

    /// Status display configuration (icons for Waybar/tray integrations)
    #[serde(default)]
    pub status: StatusConfig,

    /// Meeting transcription configuration
    #[serde(default)]
    pub meeting: MeetingConfig,

    /// Optional path to state file for external integrations (e.g., Waybar)
    /// When set, the daemon writes current state ("idle", "recording", "transcribing")
    /// to this file whenever state changes.
    /// Example: "/run/user/1000/voxtype/state" or use "auto" for default location
    #[serde(default = "default_state_file")]
    pub state_file: Option<String>,

    /// Named profiles for context-specific settings
    /// Example: [profiles.slack], [profiles.code]
    /// Use with: `voxtype record start --profile slack`
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,
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

fn default_context_window_optimization() -> bool {
    false
}

fn default_max_loaded_models() -> usize {
    2 // Primary model + one secondary
}

fn default_cold_model_timeout() -> u64 {
    300 // 5 minutes
}

fn default_eager_chunk_secs() -> f32 {
    5.0
}

fn default_eager_overlap_secs() -> f32 {
    0.5
}

fn default_whisper_model() -> String {
    "base.en".to_string()
}

fn default_state_file() -> Option<String> {
    Some("auto".to_string())
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
            idle: "\u{f130}".to_string(),         // nf-fa-microphone
            recording: "\u{f111}".to_string(),    // nf-fa-circle (filled)
            transcribing: "\u{f110}".to_string(), // nf-fa-spinner
            stopped: "\u{f131}".to_string(),      // nf-fa-microphone_slash
        },
        "omarchy" => ResolvedIcons {
            // Material Design icons matching Omarchy waybar config
            idle: "\u{ec12}".to_string(), // nf-md-microphone_outline
            recording: "\u{f036c}".to_string(), // nf-md-microphone
            transcribing: "\u{f051f}".to_string(), // nf-md-timer_sand
            stopped: "\u{ec12}".to_string(), // nf-md-microphone_outline
        },
        "minimal" => ResolvedIcons {
            idle: "○".to_string(),
            recording: "●".to_string(),
            transcribing: "◐".to_string(),
            stopped: "×".to_string(),
        },
        "material" => ResolvedIcons {
            // Material Design Icons (requires MDI font)
            idle: "\u{f036c}".to_string(),         // mdi-microphone
            recording: "\u{f040a}".to_string(),    // mdi-record
            transcribing: "\u{f04ce}".to_string(), // mdi-sync
            stopped: "\u{f036d}".to_string(),      // mdi-microphone-off
        },
        "phosphor" => ResolvedIcons {
            // Phosphor Icons (requires Phosphor font)
            idle: "\u{e43a}".to_string(),         // ph-microphone
            recording: "\u{e438}".to_string(),    // ph-record
            transcribing: "\u{e225}".to_string(), // ph-circle-notch (spinner)
            stopped: "\u{e43b}".to_string(),      // ph-microphone-slash
        },
        "codicons" => ResolvedIcons {
            // VS Code Codicons (requires Codicons font)
            idle: "\u{eb51}".to_string(),         // codicon-mic
            recording: "\u{ebfc}".to_string(),    // codicon-record
            transcribing: "\u{eb4c}".to_string(), // codicon-sync
            stopped: "\u{eb52}".to_string(),      // codicon-mute
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
            idle: "◯".to_string(),         // U+25EF white circle
            recording: "⬤".to_string(),    // U+2B24 black large circle
            transcribing: "◔".to_string(), // U+25D4 circle with upper right quadrant black
            stopped: "◌".to_string(),      // U+25CC dotted circle
        },
        "arrows" => ResolvedIcons {
            // Media player style (no special fonts required)
            idle: "▶".to_string(),         // U+25B6 play
            recording: "●".to_string(),    // U+25CF black circle
            transcribing: "↻".to_string(), // U+21BB clockwise arrow
            stopped: "■".to_string(),      // U+25A0 black square
        },
        path => load_custom_icon_theme(path).unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to load custom icon theme '{}': {}, using emoji",
                path,
                e
            );
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

    let contents =
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read theme file: {}", e))?;

    #[derive(Deserialize)]
    struct ThemeFile {
        idle: Option<String>,
        recording: Option<String>,
        transcribing: Option<String>,
        stopped: Option<String>,
    }

    let theme: ThemeFile =
        toml::from_str(&contents).map_err(|e| format!("Invalid theme file: {}", e))?;

    // Start with emoji defaults, override with file values
    let base = load_icon_theme("emoji");
    Ok(ResolvedIcons {
        idle: theme.idle.unwrap_or(base.idle),
        recording: theme.recording.unwrap_or(base.recording),
        transcribing: theme.transcribing.unwrap_or(base.transcribing),
        stopped: theme.stopped.unwrap_or(base.stopped),
    })
}

/// Whisper execution mode (how whisper runs)
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum WhisperMode {
    /// Local transcription using whisper.cpp FFI
    #[default]
    Local,
    /// Remote transcription via OpenAI-compatible API
    Remote,
    /// CLI transcription using whisper-cli subprocess
    /// Fallback for systems where whisper-rs FFI doesn't work (e.g., glibc 2.42+)
    Cli,
}

/// Language configuration supporting single language or array of allowed languages
///
/// Supports three modes:
/// - Single language: `language = "en"` - use this specific language
/// - Auto-detect: `language = "auto"` - let Whisper detect from all languages
/// - Constrained auto-detect: `language = ["en", "fr"]` - detect from allowed set
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum LanguageConfig {
    /// Single language code (e.g., "en", "fr", "auto")
    Single(String),
    /// Array of allowed language codes for constrained auto-detection
    Multiple(Vec<String>),
}

impl Default for LanguageConfig {
    fn default() -> Self {
        LanguageConfig::Single("en".to_string())
    }
}

impl LanguageConfig {
    /// Convert to a vector of language codes
    pub fn as_vec(&self) -> Vec<String> {
        match self {
            LanguageConfig::Single(s) => vec![s.clone()],
            LanguageConfig::Multiple(v) => v.clone(),
        }
    }

    /// Check if this is the "auto" setting (unconstrained auto-detection)
    pub fn is_auto(&self) -> bool {
        matches!(self, LanguageConfig::Single(s) if s == "auto")
    }

    /// Check if multiple languages are configured (constrained auto-detection)
    pub fn is_multiple(&self) -> bool {
        matches!(self, LanguageConfig::Multiple(v) if v.len() > 1)
    }

    /// Get the first/primary language (used for fallback or single-language mode)
    pub fn primary(&self) -> &str {
        match self {
            LanguageConfig::Single(s) => s,
            LanguageConfig::Multiple(v) => v.first().map(|s| s.as_str()).unwrap_or("en"),
        }
    }

    /// Parse from a comma-separated string (used for CLI argument passing)
    ///
    /// Examples:
    /// - "en" -> Single("en")
    /// - "auto" -> Single("auto")
    /// - "en,fr,de" -> Multiple(["en", "fr", "de"])
    pub fn from_comma_separated(s: &str) -> Self {
        let parts: Vec<String> = s.split(',').map(|p| p.trim().to_string()).collect();
        if parts.len() == 1 {
            LanguageConfig::Single(parts.into_iter().next().unwrap())
        } else {
            LanguageConfig::Multiple(parts)
        }
    }
}

/// Whisper speech-to-text configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhisperConfig {
    /// Execution mode: "local" or "remote" (preferred field name)
    #[serde(default)]
    pub mode: Option<WhisperMode>,

    /// DEPRECATED: Use `mode` instead. Kept for backwards compatibility.
    #[serde(default)]
    pub backend: Option<WhisperMode>,

    /// Model name: tiny, base, small, medium, large-v3, large-v3-turbo
    /// Can also be an absolute path to a .bin file
    #[serde(default = "default_whisper_model")]
    pub model: String,

    /// Language configuration: single code, "auto", or array of allowed languages
    /// Examples: "en", "auto", ["en", "fr"]
    #[serde(default)]
    pub language: LanguageConfig,

    /// Translate to English if source language is not English
    #[serde(default)]
    pub translate: bool,

    /// Number of threads for inference (None = auto-detect)
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,

    /// Enable GPU memory isolation mode (default: false)
    /// When true, transcription runs in a subprocess that exits after each
    /// transcription, ensuring GPU memory is fully released between recordings.
    /// This is especially useful on laptops with hybrid graphics to prevent
    /// the GPU from staying active when not in use.
    /// Note: This option only applies when mode = "local".
    #[serde(default)]
    pub gpu_isolation: bool,

    /// Optimize context window for short recordings (default: true)
    /// When enabled, uses a smaller context window proportional to audio length
    /// for clips under 22.5 seconds. This significantly speeds up transcription
    /// on both CPU and GPU. If transcription seems unstable, set this to false.
    #[serde(default = "default_context_window_optimization")]
    pub context_window_optimization: bool,

    // --- Eager processing settings ---
    /// Enable eager input processing (transcribe chunks while recording continues)
    /// When enabled, audio is split into chunks and transcribed in parallel with
    /// continued recording. This reduces perceived latency on slower machines.
    #[serde(default)]
    pub eager_processing: bool,

    /// Duration of each audio chunk in seconds for eager processing
    #[serde(default = "default_eager_chunk_secs")]
    pub eager_chunk_secs: f32,

    /// Overlap between adjacent chunks in seconds for eager processing
    /// Overlap helps catch words at chunk boundaries
    #[serde(default = "default_eager_overlap_secs")]
    pub eager_overlap_secs: f32,

    /// Initial prompt to provide context for transcription
    /// Use this to hint at terminology, proper nouns, or formatting conventions.
    /// Example: "Technical discussion about Rust, TypeScript, and Kubernetes."
    #[serde(default)]
    pub initial_prompt: Option<String>,

    // --- Multi-model settings ---
    /// Secondary model to use when hotkey.model_modifier is held
    /// Example: "large-v3-turbo" for difficult audio
    #[serde(default)]
    pub secondary_model: Option<String>,

    /// List of available models that can be selected via CLI --model flag
    /// These models can be loaded on-demand when requested
    #[serde(default)]
    pub available_models: Vec<String>,

    /// Maximum number of models to keep loaded in memory (LRU eviction)
    /// Default: 2 (primary model + one secondary)
    /// Only applies when gpu_isolation = false
    #[serde(default = "default_max_loaded_models")]
    pub max_loaded_models: usize,

    /// Seconds before unloading idle secondary models from memory
    /// Default: 300 (5 minutes). Set to 0 to never auto-unload.
    /// Only applies when gpu_isolation = false
    #[serde(default = "default_cold_model_timeout")]
    pub cold_model_timeout_secs: u64,

    // --- Remote backend settings ---
    /// Remote server endpoint URL (e.g., "http://192.168.1.100:8080")
    /// Required when mode = "remote"
    #[serde(default)]
    pub remote_endpoint: Option<String>,

    /// Model name to send to remote server (default: "whisper-1")
    #[serde(default)]
    pub remote_model: Option<String>,

    /// API key for remote server (optional, can also use VOXTYPE_WHISPER_API_KEY env var)
    #[serde(default)]
    pub remote_api_key: Option<String>,

    /// Timeout for remote requests in seconds (default: 30)
    #[serde(default)]
    pub remote_timeout_secs: Option<u64>,

    // --- CLI backend settings ---
    /// Path to whisper-cli binary (optional, searches PATH if not set)
    /// Used when mode = "cli"
    #[serde(default)]
    pub whisper_cli_path: Option<String>,
}

impl WhisperConfig {
    /// Get the effective execution mode, preferring `mode` over deprecated `backend`
    pub fn effective_mode(&self) -> WhisperMode {
        // Prefer `mode` if set
        if let Some(mode) = self.mode {
            return mode;
        }
        // Fall back to deprecated `backend` with warning
        if let Some(backend) = self.backend {
            tracing::warn!("DEPRECATED: [whisper] backend is deprecated, use 'mode' instead");
            tracing::warn!(
                "  Change 'backend = \"{}\"' to 'mode = \"{}\"' in config.toml",
                match backend {
                    WhisperMode::Local => "local",
                    WhisperMode::Remote => "remote",
                    WhisperMode::Cli => "cli",
                },
                match backend {
                    WhisperMode::Local => "local",
                    WhisperMode::Remote => "remote",
                    WhisperMode::Cli => "cli",
                }
            );
            return backend;
        }
        WhisperMode::default()
    }
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            mode: None,    // Defaults to Local via effective_mode()
            backend: None, // Deprecated alias
            model: "base.en".to_string(),
            language: LanguageConfig::default(),
            translate: false,
            threads: None,
            on_demand_loading: default_on_demand_loading(),
            gpu_isolation: false,
            context_window_optimization: default_context_window_optimization(),
            eager_processing: false,
            eager_chunk_secs: default_eager_chunk_secs(),
            eager_overlap_secs: default_eager_overlap_secs(),
            initial_prompt: None,
            secondary_model: None,
            available_models: vec![],
            max_loaded_models: default_max_loaded_models(),
            cold_model_timeout_secs: default_cold_model_timeout(),
            remote_endpoint: None,
            remote_model: None,
            remote_api_key: None,
            remote_timeout_secs: None,
            whisper_cli_path: None,
        }
    }
}

/// Parakeet model architecture type
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ParakeetModelType {
    /// CTC (Connectionist Temporal Classification) - faster, character-level output
    Ctc,
    /// TDT (Token-Duration-Transducer) - recommended, proper punctuation and word boundaries
    #[default]
    Tdt,
}

/// Parakeet speech-to-text configuration (ONNX-based, alternative to Whisper)
/// Requires: cargo build --features parakeet
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParakeetConfig {
    /// Path to model directory containing ONNX model files
    /// For TDT: encoder-model.onnx, decoder_joint-model.onnx, vocab.txt
    /// For CTC: model.onnx, tokenizer.json
    pub model: String,

    /// Model architecture type: "tdt" (default, recommended) or "ctc"
    /// Auto-detected from model directory structure if not specified
    #[serde(default)]
    pub model_type: Option<ParakeetModelType>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for ParakeetConfig {
    fn default() -> Self {
        Self {
            model: "parakeet-tdt-0.6b-v3".to_string(),
            model_type: None, // Auto-detect
            on_demand_loading: false,
        }
    }
}

/// Moonshine speech-to-text configuration (ONNX-based, encoder-decoder ASR)
/// Requires: cargo build --features moonshine
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MoonshineConfig {
    /// Model name or path to directory containing ONNX model files
    /// Expects: encoder_model.onnx, decoder_model_merged.onnx, tokenizer.json
    /// Short names: "tiny" (27M params), "base" (61M params)
    pub model: String,

    /// Use quantized model variants for faster CPU inference (default: true)
    /// Falls back to full precision if quantized files are not found
    #[serde(default = "default_true")]
    pub quantized: bool,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for MoonshineConfig {
    fn default() -> Self {
        Self {
            model: "base".to_string(),
            quantized: true,
            threads: None,
            on_demand_loading: false,
        }
    }
}

/// SenseVoice speech-to-text configuration (ONNX-based, CTC encoder-only ASR)
/// Requires: cargo build --features sensevoice
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SenseVoiceConfig {
    /// Model name or path to directory containing ONNX model files
    /// Expects: model.int8.onnx (or model.onnx), tokens.txt
    /// Short name: "sensevoice-small" (default)
    pub model: String,

    /// Language for transcription: "auto", "zh", "en", "ja", "ko", "yue" (default: "auto")
    #[serde(default = "default_sensevoice_language")]
    pub language: String,

    /// Enable inverse text normalization (adds punctuation) (default: true)
    #[serde(default = "default_true")]
    pub use_itn: bool,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

fn default_sensevoice_language() -> String {
    "auto".to_string()
}

impl Default for SenseVoiceConfig {
    fn default() -> Self {
        Self {
            model: "sensevoice-small".to_string(),
            language: "auto".to_string(),
            use_itn: true,
            threads: None,
            on_demand_loading: false,
        }
    }
}

/// Paraformer speech-to-text configuration (FunASR ONNX-based CTC encoder)
/// Requires: cargo build --features paraformer
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParaformerConfig {
    /// Model name or path to ONNX model directory
    /// Expects: model.onnx (or model.int8.onnx), tokens.txt
    pub model: String,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for ParaformerConfig {
    fn default() -> Self {
        Self {
            model: "paraformer-zh".to_string(),
            threads: None,
            on_demand_loading: false,
        }
    }
}

/// Dolphin speech-to-text configuration (ONNX-based CTC encoder, dictation-optimized)
/// Requires: cargo build --features dolphin
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DolphinConfig {
    /// Model name or path to ONNX model directory
    pub model: String,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for DolphinConfig {
    fn default() -> Self {
        Self {
            model: "dolphin-base".to_string(),
            threads: None,
            on_demand_loading: false,
        }
    }
}

/// Omnilingual speech-to-text configuration (FunASR ONNX-based, 50+ languages)
/// Requires: cargo build --features omnilingual
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OmnilingualConfig {
    /// Model name or path to ONNX model directory
    pub model: String,

    /// Number of CPU threads for ONNX Runtime inference
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

impl Default for OmnilingualConfig {
    fn default() -> Self {
        Self {
            model: "omnilingual-large".to_string(),
            threads: None,
            on_demand_loading: false,
        }
    }
}

/// Transcription engine selection (which ASR technology to use)
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptionEngine {
    /// Use Whisper (whisper.cpp via whisper-rs) - default
    #[default]
    Whisper,
    /// Use Parakeet (NVIDIA's FastConformer via ONNX Runtime)
    /// Requires: cargo build --features parakeet
    Parakeet,
    /// Use Moonshine (encoder-decoder ASR via ONNX Runtime)
    /// Requires: cargo build --features moonshine
    Moonshine,
    /// Use SenseVoice (Alibaba FunAudioLLM CTC model via ONNX Runtime)
    /// Requires: cargo build --features sensevoice
    SenseVoice,
    /// Use Paraformer (FunASR CTC encoder via ONNX Runtime)
    /// Requires: cargo build --features paraformer
    Paraformer,
    /// Use Dolphin (dictation-optimized CTC encoder via ONNX Runtime)
    /// Requires: cargo build --features dolphin
    Dolphin,
    /// Use Omnilingual (FunASR 50+ language CTC encoder via ONNX Runtime)
    /// Requires: cargo build --features omnilingual
    Omnilingual,
}

/// VAD backend selection
///
/// Determines which voice activity detection algorithm to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VadBackend {
    /// Auto-select based on transcription engine (default)
    /// - Whisper engine: uses Whisper VAD (requires model download)
    /// - Parakeet engine: uses Energy VAD (no model needed)
    #[default]
    Auto,
    /// Energy-based VAD using RMS amplitude analysis
    /// Fast, no model download required, works with any engine
    Energy,
    /// Whisper VAD using whisper-rs built-in Silero model (GGML format)
    /// More accurate but requires downloading ggml-silero-vad.bin
    Whisper,
}

/// Voice Activity Detection configuration
///
/// VAD filters silence-only recordings before transcription to prevent
/// Whisper hallucinations when processing silence.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VadConfig {
    /// Enable Voice Activity Detection (default: false)
    /// When enabled, recordings with no detected speech are rejected before transcription
    #[serde(default)]
    pub enabled: bool,

    /// VAD backend to use (default: auto)
    /// - auto: Whisper VAD for Whisper engine, Energy VAD for Parakeet
    /// - energy: Simple RMS-based detection, no model needed
    /// - whisper: Silero VAD via whisper-rs, requires model download
    #[serde(default)]
    pub backend: VadBackend,

    /// Speech detection threshold (0.0-1.0, default: 0.5)
    /// Higher values require more confident speech detection
    #[serde(default = "default_vad_threshold")]
    pub threshold: f32,

    /// Minimum speech duration in milliseconds (default: 100)
    /// Recordings with less speech than this are rejected
    #[serde(default = "default_min_speech_duration_ms")]
    pub min_speech_duration_ms: u32,

    /// Path to VAD model file (optional, for Whisper VAD backend)
    /// If not set, uses the default model location (~/.local/share/voxtype/models/)
    #[serde(default)]
    pub model: Option<String>,
}

fn default_vad_threshold() -> f32 {
    0.5
}

fn default_min_speech_duration_ms() -> u32 {
    100
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: VadBackend::default(),
            threshold: default_vad_threshold(),
            min_speech_duration_ms: default_min_speech_duration_ms(),
            model: None,
        }
    }
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

/// Meeting transcription configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingConfig {
    /// Enable meeting mode
    #[serde(default)]
    pub enabled: bool,

    /// Duration of each audio chunk in seconds
    #[serde(default = "default_chunk_duration")]
    pub chunk_duration_secs: u32,

    /// Storage path for meetings ("auto" for default location)
    /// Default: ~/.local/share/voxtype/meetings/
    #[serde(default = "default_storage_path")]
    pub storage_path: String,

    /// Retain raw audio files after transcription
    #[serde(default)]
    pub retain_audio: bool,

    /// Maximum meeting duration in minutes (0 = unlimited)
    #[serde(default = "default_max_duration")]
    pub max_duration_mins: u32,

    /// Meeting audio configuration
    #[serde(default)]
    pub audio: MeetingAudioConfig,

    /// Diarization configuration
    #[serde(default)]
    pub diarization: MeetingDiarizationConfig,

    /// Summarization configuration
    #[serde(default)]
    pub summary: MeetingSummaryConfig,
}

/// Meeting audio configuration for dual capture
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingAudioConfig {
    /// Microphone device (uses main audio.device if not specified)
    #[serde(default = "default_mic_device")]
    pub mic_device: String,

    /// Loopback device for capturing remote participants
    /// Options: "auto" (detect), "disabled", or specific device name
    #[serde(default = "default_loopback")]
    pub loopback_device: String,

    /// Echo cancellation mode for removing speaker bleed-through from mic
    /// Options: "auto" (GTCRN neural enhancement + transcript dedup), "disabled"
    /// The GTCRN model (~523KB) is auto-downloaded on first meeting start.
    /// For system-level echo cancellation, configure PipeWire's echo-cancel module
    /// and set this to "disabled".
    #[serde(default = "default_echo_cancel")]
    pub echo_cancel: String,
}

fn default_mic_device() -> String {
    "default".to_string()
}

fn default_loopback() -> String {
    "auto".to_string()
}

fn default_echo_cancel() -> String {
    "auto".to_string()
}

impl Default for MeetingAudioConfig {
    fn default() -> Self {
        Self {
            mic_device: default_mic_device(),
            loopback_device: default_loopback(),
            echo_cancel: default_echo_cancel(),
        }
    }
}

/// Meeting diarization configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingDiarizationConfig {
    /// Enable speaker diarization
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Diarization backend: "simple", "ml", or "remote"
    #[serde(default = "default_diarization_backend")]
    pub backend: String,

    /// Maximum number of speakers to detect
    #[serde(default = "default_max_speakers")]
    pub max_speakers: u32,
}

fn default_diarization_backend() -> String {
    "simple".to_string()
}

fn default_max_speakers() -> u32 {
    10
}

fn default_chunk_duration() -> u32 {
    30
}

fn default_storage_path() -> String {
    "auto".to_string()
}

fn default_max_duration() -> u32 {
    180
}

impl Default for MeetingDiarizationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: default_diarization_backend(),
            max_speakers: default_max_speakers(),
        }
    }
}

/// Meeting summary configuration (Phase 5)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeetingSummaryConfig {
    /// Summarization backend: "local", "remote", or "disabled"
    #[serde(default = "default_summary_backend")]
    pub backend: String,

    /// Ollama URL for local backend
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,

    /// Ollama model name
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,

    /// Remote API endpoint for remote backend
    #[serde(default)]
    pub remote_endpoint: Option<String>,

    /// Remote API key
    #[serde(default)]
    pub remote_api_key: Option<String>,

    /// Request timeout in seconds
    #[serde(default = "default_summary_timeout")]
    pub timeout_secs: u64,
}

fn default_summary_backend() -> String {
    "disabled".to_string()
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_ollama_model() -> String {
    "llama3.2".to_string()
}

fn default_summary_timeout() -> u64 {
    120
}

impl Default for MeetingSummaryConfig {
    fn default() -> Self {
        Self {
            backend: default_summary_backend(),
            ollama_url: default_ollama_url(),
            ollama_model: default_ollama_model(),
            remote_endpoint: None,
            remote_api_key: None,
            timeout_secs: default_summary_timeout(),
        }
    }
}

impl Default for MeetingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            chunk_duration_secs: default_chunk_duration(),
            storage_path: default_storage_path(),
            retain_audio: false,
            max_duration_mins: default_max_duration(),
            audio: MeetingAudioConfig::default(),
            diarization: MeetingDiarizationConfig::default(),
            summary: MeetingSummaryConfig::default(),
        }
    }
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

    /// Show engine icon in notification title (🦜 for Parakeet, 🗣️ for Whisper)
    #[serde(default)]
    pub show_engine_icon: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            on_recording_start: false,
            on_recording_stop: false,
            on_transcription: true,
            show_engine_icon: false,
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

/// Named profile for context-specific settings
///
/// Profiles allow different post-processing commands (and other settings)
/// for different contexts like Slack, code editors, email, etc.
///
/// # Example Configuration
///
/// ```toml
/// [profiles.slack]
/// post_process_command = "cleanup-for-slack.sh"
///
/// [profiles.code]
/// post_process_command = "cleanup-for-code.sh"
/// ```
///
/// Use with: `voxtype record start --profile slack`
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Profile {
    /// Post-processing command for this profile
    /// Overrides [output.post_process.command] when the profile is active
    #[serde(default)]
    pub post_process_command: Option<String>,

    /// Timeout for post-processing in milliseconds (default: 30000)
    #[serde(default)]
    pub post_process_timeout_ms: Option<u64>,

    /// Output mode override for this profile
    #[serde(default)]
    pub output_mode: Option<OutputMode>,
}

fn default_post_process_timeout() -> u64 {
    30000 // 30 seconds - generous for LLM processing
}

fn default_restore_clipboard_delay() -> u32 {
    200 // 200ms - delay for paste to complete before restoring clipboard
}

/// Text output configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutputConfig {
    /// Primary output mode
    pub mode: OutputMode,

    /// Fall back to clipboard if typing fails
    #[serde(default = "default_true")]
    pub fallback_to_clipboard: bool,

    /// Custom driver order for type mode (overrides default: wtype -> dotool -> ydotool -> clipboard)
    /// Specify which drivers to try and in what order.
    /// Example: ["ydotool", "wtype"] to prefer ydotool over wtype
    #[serde(default)]
    pub driver_order: Option<Vec<OutputDriver>>,

    /// Notification settings
    #[serde(default)]
    pub notification: NotificationConfig,

    /// Delay between typed characters (ms), 0 for fastest
    #[serde(default)]
    pub type_delay_ms: u32,

    /// Delay before typing starts (ms), allows virtual keyboard to initialize
    /// Helps prevent first character from being dropped on some compositors
    #[serde(default)]
    pub pre_type_delay_ms: u32,

    /// DEPRECATED: Use pre_type_delay_ms instead. Kept for backwards compatibility.
    #[serde(default)]
    pub wtype_delay_ms: u32,

    /// Automatically submit (send Enter key) after outputting transcribed text
    /// Useful for chat applications, command lines, or forms where you want
    /// to auto-submit after dictation
    #[serde(default)]
    pub auto_submit: bool,

    /// Text to append after each transcription (e.g., " " for a space)
    /// Appended after the transcription but before auto_submit
    /// Useful for separating sentences when dictating paragraphs incrementally
    #[serde(default)]
    pub append_text: Option<String>,

    /// Convert newlines to Shift+Enter instead of regular Enter
    /// Useful for applications where Enter submits (e.g., Cursor IDE, Slack, Discord)
    #[serde(default)]
    pub shift_enter_newlines: bool,

    /// Command to run when recording starts (e.g., switch to compositor submap)
    /// Useful for entering a mode where cancel keybindings are effective
    #[serde(default)]
    pub pre_recording_command: Option<String>,

    /// Command to run before typing output (e.g., compositor submap switch)
    /// Useful for blocking modifier keys at the compositor level
    #[serde(default)]
    pub pre_output_command: Option<String>,

    /// Command to run after typing output (e.g., reset compositor submap)
    /// Runs even if typing fails, to ensure cleanup
    #[serde(default)]
    pub post_output_command: Option<String>,

    /// Optional post-processing command configuration
    /// Pipes transcribed text through an external command before output
    #[serde(default)]
    pub post_process: Option<PostProcessConfig>,

    /// Keystroke to simulate for paste mode (e.g., "ctrl+v", "shift+insert", "ctrl+shift+v")
    /// Defaults to "ctrl+v" if not specified
    #[serde(default)]
    pub paste_keys: Option<String>,

    /// Keyboard layout for dotool (e.g., "de" for German, "fr" for French)
    /// Required for non-US keyboard layouts when using dotool
    #[serde(default)]
    pub dotool_xkb_layout: Option<String>,

    /// Keyboard layout variant for dotool (e.g., "nodeadkeys")
    #[serde(default)]
    pub dotool_xkb_variant: Option<String>,

    /// File path for file output mode (required when mode = "file")
    /// Also used as default path for --output-file CLI flag
    #[serde(default)]
    pub file_path: Option<PathBuf>,

    /// File write mode: "overwrite" (default) or "append"
    /// Applies to both config-based file output and --output-file CLI flag
    #[serde(default)]
    pub file_mode: FileMode,

    /// Restore original clipboard content after paste mode completes
    /// Saves clipboard before transcription, restores it after paste keystroke
    #[serde(default)]
    pub restore_clipboard: bool,

    /// Delay after paste before restoring clipboard content (milliseconds)
    /// Allows time for the paste operation to complete
    #[serde(default = "default_restore_clipboard_delay")]
    pub restore_clipboard_delay_ms: u32,
}

impl OutputConfig {
    /// Get the effective pre-type delay, handling deprecated wtype_delay_ms
    pub fn effective_pre_type_delay_ms(&self) -> u32 {
        if self.wtype_delay_ms > 0 {
            if self.pre_type_delay_ms > 0 {
                // Both set - prefer new option, warn about deprecated
                tracing::warn!(
                    "Both pre_type_delay_ms and wtype_delay_ms are set. \
                     Using pre_type_delay_ms={}. wtype_delay_ms is deprecated.",
                    self.pre_type_delay_ms
                );
                self.pre_type_delay_ms
            } else {
                // Only deprecated option set - use it with warning
                tracing::warn!("wtype_delay_ms is deprecated, use pre_type_delay_ms instead");
                self.wtype_delay_ms
            }
        } else {
            self.pre_type_delay_ms
        }
    }
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
    /// Write transcription to a file
    File,
}

/// Output driver for typing text
/// Used to specify preferred drivers in the fallback chain
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputDriver {
    /// wtype - Wayland-native via virtual-keyboard protocol, best Unicode/CJK support
    Wtype,
    /// eitype - Wayland via libei/EI protocol, works on GNOME/KDE
    Eitype,
    /// dotool - Works on X11/Wayland/TTY, supports keyboard layouts
    Dotool,
    /// ydotool - Works on X11/Wayland/TTY, requires daemon
    Ydotool,
    /// Clipboard via wl-copy (Wayland)
    Clipboard,
    /// Clipboard via xclip (X11)
    Xclip,
}

impl std::fmt::Display for OutputDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputDriver::Wtype => write!(f, "wtype"),
            OutputDriver::Eitype => write!(f, "eitype"),
            OutputDriver::Dotool => write!(f, "dotool"),
            OutputDriver::Ydotool => write!(f, "ydotool"),
            OutputDriver::Clipboard => write!(f, "clipboard"),
            OutputDriver::Xclip => write!(f, "xclip"),
        }
    }
}

impl std::str::FromStr for OutputDriver {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "wtype" => Ok(OutputDriver::Wtype),
            "eitype" => Ok(OutputDriver::Eitype),
            "dotool" => Ok(OutputDriver::Dotool),
            "ydotool" => Ok(OutputDriver::Ydotool),
            "clipboard" => Ok(OutputDriver::Clipboard),
            "xclip" => Ok(OutputDriver::Xclip),
            _ => Err(format!(
                "Unknown driver '{}'. Valid options: wtype, eitype, dotool, ydotool, clipboard, xclip",
                s
            )),
        }
    }
}

/// File write mode when using file output
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum FileMode {
    /// Overwrite the file on each transcription (default)
    #[default]
    Overwrite,
    /// Append to the file on each transcription
    Append,
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
                cancel_key: None,
                model_modifier: None,
            },
            audio: AudioConfig {
                device: "default".to_string(),
                sample_rate: 16000,
                max_duration_secs: 60,
                feedback: AudioFeedbackConfig::default(),
            },
            whisper: WhisperConfig {
                mode: None,    // Defaults to Local via effective_mode()
                backend: None, // Deprecated alias
                model: "base.en".to_string(),
                language: LanguageConfig::default(),
                translate: false,
                threads: None,
                on_demand_loading: default_on_demand_loading(),
                gpu_isolation: false,
                context_window_optimization: default_context_window_optimization(),
                eager_processing: false,
                eager_chunk_secs: default_eager_chunk_secs(),
                eager_overlap_secs: default_eager_overlap_secs(),
                initial_prompt: None,
                secondary_model: None,
                available_models: vec![],
                max_loaded_models: default_max_loaded_models(),
                cold_model_timeout_secs: default_cold_model_timeout(),
                remote_endpoint: None,
                remote_model: None,
                remote_api_key: None,
                remote_timeout_secs: None,
                whisper_cli_path: None,
            },
            output: OutputConfig {
                mode: OutputMode::Type,
                fallback_to_clipboard: true,
                driver_order: None,
                notification: NotificationConfig::default(),
                type_delay_ms: 0,
                pre_type_delay_ms: 0,
                wtype_delay_ms: 0,
                auto_submit: false,
                append_text: None,
                shift_enter_newlines: false,
                pre_recording_command: None,
                pre_output_command: None,
                post_output_command: None,
                post_process: None,
                paste_keys: None,
                dotool_xkb_layout: None,
                dotool_xkb_variant: None,
                file_path: None,
                file_mode: FileMode::default(),
                restore_clipboard: false,
                restore_clipboard_delay_ms: default_restore_clipboard_delay(),
            },
            engine: TranscriptionEngine::default(),
            parakeet: None,
            moonshine: None,
            sensevoice: None,
            paraformer: None,
            dolphin: None,
            omnilingual: None,
            text: TextConfig::default(),
            vad: VadConfig::default(),
            status: StatusConfig::default(),
            meeting: MeetingConfig::default(),
            state_file: Some("auto".to_string()),
            profiles: HashMap::new(),
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
        self.state_file
            .as_ref()
            .and_then(|path| match path.to_lowercase().as_str() {
                "disabled" | "none" | "off" | "false" => None,
                "auto" => Some(Self::runtime_dir().join("state")),
                _ => Some(PathBuf::from(path)),
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

    /// Check if on-demand model loading is enabled for the active engine
    pub fn on_demand_loading(&self) -> bool {
        match self.engine {
            TranscriptionEngine::Whisper => self.whisper.on_demand_loading,
            TranscriptionEngine::Parakeet => self
                .parakeet
                .as_ref()
                .map(|p| p.on_demand_loading)
                .unwrap_or(false),
            TranscriptionEngine::Moonshine => self
                .moonshine
                .as_ref()
                .map(|m| m.on_demand_loading)
                .unwrap_or(false),
            TranscriptionEngine::SenseVoice => self
                .sensevoice
                .as_ref()
                .map(|s| s.on_demand_loading)
                .unwrap_or(false),
            TranscriptionEngine::Paraformer => self
                .paraformer
                .as_ref()
                .map(|p| p.on_demand_loading)
                .unwrap_or(false),
            TranscriptionEngine::Dolphin => self
                .dolphin
                .as_ref()
                .map(|d| d.on_demand_loading)
                .unwrap_or(false),
            TranscriptionEngine::Omnilingual => self
                .omnilingual
                .as_ref()
                .map(|o| o.on_demand_loading)
                .unwrap_or(false),
        }
    }

    /// Get the model name/path for the active engine (for logging)
    pub fn model_name(&self) -> &str {
        match self.engine {
            TranscriptionEngine::Whisper => &self.whisper.model,
            TranscriptionEngine::Parakeet => self
                .parakeet
                .as_ref()
                .map(|p| p.model.as_str())
                .unwrap_or("parakeet (not configured)"),
            TranscriptionEngine::Moonshine => self
                .moonshine
                .as_ref()
                .map(|m| m.model.as_str())
                .unwrap_or("moonshine (not configured)"),
            TranscriptionEngine::SenseVoice => self
                .sensevoice
                .as_ref()
                .map(|s| s.model.as_str())
                .unwrap_or("sensevoice (not configured)"),
            TranscriptionEngine::Paraformer => self
                .paraformer
                .as_ref()
                .map(|p| p.model.as_str())
                .unwrap_or("paraformer (not configured)"),
            TranscriptionEngine::Dolphin => self
                .dolphin
                .as_ref()
                .map(|d| d.model.as_str())
                .unwrap_or("dolphin (not configured)"),
            TranscriptionEngine::Omnilingual => self
                .omnilingual
                .as_ref()
                .map(|o| o.model.as_str())
                .unwrap_or("omnilingual (not configured)"),
        }
    }

    /// Get a named profile by name
    /// Returns None if the profile doesn't exist
    pub fn get_profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }

    /// List all available profile names
    pub fn profile_names(&self) -> Vec<&String> {
        self.profiles.keys().collect()
    }
}

/// Parse a boolean from an environment variable value.
/// Only "1" and "true" (case-insensitive) are truthy; everything else is falsy.
fn parse_bool_env(val: &str) -> bool {
    val == "1" || val.eq_ignore_ascii_case("true")
}

/// Load configuration from file, with defaults for missing values
pub fn load_config(path: Option<&Path>) -> Result<Config, VoxtypeError> {
    // Start with defaults
    let mut config = Config::default();

    // Determine config file path
    let config_path = path.map(PathBuf::from).or_else(Config::default_path);

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
    // Hotkey
    if let Ok(key) = std::env::var("VOXTYPE_HOTKEY") {
        config.hotkey.key = key;
    }
    if let Ok(val) = std::env::var("VOXTYPE_HOTKEY_ENABLED") {
        config.hotkey.enabled = parse_bool_env(&val);
    }
    if let Ok(key) = std::env::var("VOXTYPE_CANCEL_KEY") {
        config.hotkey.cancel_key = Some(key);
    }

    // Whisper / engine
    if let Ok(model) = std::env::var("VOXTYPE_MODEL") {
        config.whisper.model = model;
    }
    if let Ok(engine) = std::env::var("VOXTYPE_ENGINE") {
        match engine.to_lowercase().as_str() {
            "whisper" => config.engine = TranscriptionEngine::Whisper,
            "parakeet" => config.engine = TranscriptionEngine::Parakeet,
            "moonshine" => config.engine = TranscriptionEngine::Moonshine,
            "sensevoice" => config.engine = TranscriptionEngine::SenseVoice,
            "paraformer" => config.engine = TranscriptionEngine::Paraformer,
            "dolphin" => config.engine = TranscriptionEngine::Dolphin,
            "omnilingual" => config.engine = TranscriptionEngine::Omnilingual,
            _ => tracing::warn!("Unknown VOXTYPE_ENGINE value: {}", engine),
        }
    }
    if let Ok(lang) = std::env::var("VOXTYPE_LANGUAGE") {
        config.whisper.language = LanguageConfig::from_comma_separated(&lang);
    }
    if let Ok(val) = std::env::var("VOXTYPE_TRANSLATE") {
        config.whisper.translate = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_THREADS") {
        if let Ok(n) = val.parse::<usize>() {
            config.whisper.threads = Some(n);
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_GPU_ISOLATION") {
        config.whisper.gpu_isolation = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_ON_DEMAND_LOADING") {
        config.whisper.on_demand_loading = parse_bool_env(&val);
    }

    // Audio
    if let Ok(device) = std::env::var("VOXTYPE_AUDIO_DEVICE") {
        config.audio.device = device;
    }
    if let Ok(val) = std::env::var("VOXTYPE_MAX_DURATION_SECS") {
        if let Ok(n) = val.parse::<u32>() {
            config.audio.max_duration_secs = n;
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_AUDIO_FEEDBACK") {
        config.audio.feedback.enabled = parse_bool_env(&val);
    }

    // Output
    if let Ok(mode) = std::env::var("VOXTYPE_OUTPUT_MODE") {
        config.output.mode = match mode.to_lowercase().as_str() {
            "clipboard" => OutputMode::Clipboard,
            "paste" => OutputMode::Paste,
            "file" => OutputMode::File,
            _ => OutputMode::Type,
        };
    }
    if let Ok(append_text) = std::env::var("VOXTYPE_APPEND_TEXT") {
        config.output.append_text = Some(append_text);
    }
    if let Ok(val) = std::env::var("VOXTYPE_AUTO_SUBMIT") {
        config.output.auto_submit = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_SHIFT_ENTER_NEWLINES") {
        config.output.shift_enter_newlines = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_PRE_TYPE_DELAY") {
        if let Ok(n) = val.parse::<u32>() {
            config.output.pre_type_delay_ms = n;
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_TYPE_DELAY") {
        if let Ok(n) = val.parse::<u32>() {
            config.output.type_delay_ms = n;
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_FALLBACK_TO_CLIPBOARD") {
        config.output.fallback_to_clipboard = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_SPOKEN_PUNCTUATION") {
        config.text.spoken_punctuation = parse_bool_env(&val);
    }
    if let Ok(keys) = std::env::var("VOXTYPE_PASTE_KEYS") {
        config.output.paste_keys = Some(keys);
    }
    if let Ok(layout) = std::env::var("VOXTYPE_DOTOOL_XKB_LAYOUT") {
        config.output.dotool_xkb_layout = Some(layout);
    }

    // Remote whisper
    if let Ok(endpoint) = std::env::var("VOXTYPE_REMOTE_ENDPOINT") {
        config.whisper.remote_endpoint = Some(endpoint);
    }
    if let Ok(key) = std::env::var("VOXTYPE_WHISPER_API_KEY") {
        config.whisper.remote_api_key = Some(key);
    }
    if let Ok(val) = std::env::var("VOXTYPE_RESTORE_CLIPBOARD") {
        config.output.restore_clipboard = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_RESTORE_CLIPBOARD_DELAY_MS") {
        if let Ok(ms) = val.parse::<u32>() {
            config.output.restore_clipboard_delay_ms = ms;
        }
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
        assert!(!config.output.auto_submit);
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
    fn test_parse_auto_submit() {
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
            auto_submit = true
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.output.auto_submit);
    }

    #[test]
    fn test_parse_auto_submit_defaults_false() {
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
        assert!(!config.output.auto_submit);
    }

    #[test]
    fn test_builtin_icon_themes() {
        // Test all built-in themes load correctly
        let themes = [
            "emoji",
            "nerd-font",
            "material",
            "phosphor",
            "codicons",
            "omarchy",
            "minimal",
            "dots",
            "arrows",
            "text",
        ];

        for theme in themes {
            let icons = load_icon_theme(theme);
            assert!(
                !icons.idle.is_empty() || theme == "emoji",
                "Theme {} should have idle icon",
                theme
            );
            assert!(
                !icons.recording.is_empty(),
                "Theme {} should have recording icon",
                theme
            );
            assert!(
                !icons.transcribing.is_empty(),
                "Theme {} should have transcribing icon",
                theme
            );
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
        writeln!(
            temp_file,
            r#"
            idle = "IDLE"
            recording = "REC"
            transcribing = "BUSY"
            stopped = "OFF"
        "#
        )
        .unwrap();

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
        writeln!(
            temp_file,
            r#"
            recording = "🔴"
        "#
        )
        .unwrap();

        let icons = load_icon_theme(temp_file.path().to_str().unwrap());
        // Only recording is overridden, others fall back to emoji
        assert!(icons.idle.contains("🎙"));
        assert_eq!(icons.recording, "🔴");
        assert!(icons.transcribing.contains("⏳"));
    }

    #[test]
    fn test_context_window_optimization_default_false() {
        // Default config should have context_window_optimization disabled
        // (disabled by default due to repetition issues with some models)
        let config = Config::default();
        assert!(!config.whisper.context_window_optimization);
    }

    #[test]
    fn test_context_window_optimization_can_be_enabled() {
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
            context_window_optimization = true

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.whisper.context_window_optimization);
    }

    #[test]
    fn test_context_window_optimization_defaults_when_omitted() {
        // When not specified in config, should default to false
        // (disabled by default due to repetition issues with some models)
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
        assert!(!config.whisper.context_window_optimization);
    }

    #[test]
    fn test_language_config_single() {
        let toml_str = r#"
            [whisper]
            model = "base.en"
            language = "fr"
        "#;

        #[derive(Deserialize)]
        struct TestConfig {
            whisper: WhisperConfig,
        }

        let config: TestConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.whisper.language,
            LanguageConfig::Single("fr".to_string())
        );
        assert!(!config.whisper.language.is_auto());
        assert!(!config.whisper.language.is_multiple());
        assert_eq!(config.whisper.language.primary(), "fr");
        assert_eq!(config.whisper.language.as_vec(), vec!["fr"]);
    }

    #[test]
    fn test_language_config_auto() {
        let toml_str = r#"
            [whisper]
            model = "large-v3"
            language = "auto"
        "#;

        #[derive(Deserialize)]
        struct TestConfig {
            whisper: WhisperConfig,
        }

        let config: TestConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.whisper.language,
            LanguageConfig::Single("auto".to_string())
        );
        assert!(config.whisper.language.is_auto());
        assert!(!config.whisper.language.is_multiple());
        assert_eq!(config.whisper.language.primary(), "auto");
    }

    #[test]
    fn test_language_config_array() {
        let toml_str = r#"
            [whisper]
            model = "large-v3-turbo"
            language = ["en", "fr", "de"]
        "#;

        #[derive(Deserialize)]
        struct TestConfig {
            whisper: WhisperConfig,
        }

        let config: TestConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.whisper.language,
            LanguageConfig::Multiple(vec!["en".to_string(), "fr".to_string(), "de".to_string()])
        );
        assert!(!config.whisper.language.is_auto());
        assert!(config.whisper.language.is_multiple());
        assert_eq!(config.whisper.language.primary(), "en");
        assert_eq!(config.whisper.language.as_vec(), vec!["en", "fr", "de"]);
    }

    #[test]
    fn test_language_config_single_element_array() {
        // A single-element array should not be considered "multiple"
        let toml_str = r#"
            [whisper]
            model = "base.en"
            language = ["en"]
        "#;

        #[derive(Deserialize)]
        struct TestConfig {
            whisper: WhisperConfig,
        }

        let config: TestConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.whisper.language.is_multiple());
        assert_eq!(config.whisper.language.primary(), "en");
    }

    #[test]
    fn test_language_config_default() {
        // Default should be "en"
        let config = LanguageConfig::default();
        assert_eq!(config, LanguageConfig::Single("en".to_string()));
        assert!(!config.is_auto());
        assert!(!config.is_multiple());
        assert_eq!(config.primary(), "en");
    }

    // =========================================================================
    // Engine and Mode Tests (v5 config schema)
    // =========================================================================

    #[test]
    fn test_parse_engine_whisper() {
        let toml_str = r#"
            engine = "whisper"

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
        assert_eq!(config.engine, TranscriptionEngine::Whisper);
    }

    #[test]
    fn test_parse_engine_parakeet() {
        let toml_str = r#"
            engine = "parakeet"

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

            [parakeet]
            model = "parakeet-tdt-0.6b-v3"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.engine, TranscriptionEngine::Parakeet);
        assert!(config.parakeet.is_some());
        assert_eq!(
            config.parakeet.as_ref().unwrap().model,
            "parakeet-tdt-0.6b-v3"
        );
    }

    #[test]
    fn test_engine_defaults_to_whisper() {
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
        assert_eq!(config.engine, TranscriptionEngine::Whisper);
    }

    #[test]
    fn test_output_driver_from_str() {
        assert_eq!(
            "wtype".parse::<OutputDriver>().unwrap(),
            OutputDriver::Wtype
        );
        assert_eq!(
            "dotool".parse::<OutputDriver>().unwrap(),
            OutputDriver::Dotool
        );
        assert_eq!(
            "ydotool".parse::<OutputDriver>().unwrap(),
            OutputDriver::Ydotool
        );
        assert_eq!(
            "clipboard".parse::<OutputDriver>().unwrap(),
            OutputDriver::Clipboard
        );
        assert_eq!(
            "xclip".parse::<OutputDriver>().unwrap(),
            OutputDriver::Xclip
        );
        // Case insensitive
        assert_eq!(
            "WTYPE".parse::<OutputDriver>().unwrap(),
            OutputDriver::Wtype
        );
        assert_eq!(
            "Ydotool".parse::<OutputDriver>().unwrap(),
            OutputDriver::Ydotool
        );
        assert_eq!(
            "XCLIP".parse::<OutputDriver>().unwrap(),
            OutputDriver::Xclip
        );
        // Invalid
        assert!("invalid".parse::<OutputDriver>().is_err());
    }

    #[test]
    fn test_output_driver_display() {
        assert_eq!(OutputDriver::Wtype.to_string(), "wtype");
        assert_eq!(OutputDriver::Dotool.to_string(), "dotool");
        assert_eq!(OutputDriver::Ydotool.to_string(), "ydotool");
        assert_eq!(OutputDriver::Clipboard.to_string(), "clipboard");
        assert_eq!(OutputDriver::Xclip.to_string(), "xclip");
    }

    #[test]
    fn test_parse_driver_order_from_toml() {
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
            driver_order = ["ydotool", "wtype", "clipboard"]
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let driver_order = config.output.driver_order.unwrap();
        assert_eq!(driver_order.len(), 3);
        assert_eq!(driver_order[0], OutputDriver::Ydotool);
        assert_eq!(driver_order[1], OutputDriver::Wtype);
        assert_eq!(driver_order[2], OutputDriver::Clipboard);
    }

    #[test]
    fn test_parse_whisper_mode_local() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            mode = "local"
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.whisper.mode, Some(WhisperMode::Local));
        assert_eq!(config.whisper.effective_mode(), WhisperMode::Local);
    }

    #[test]
    fn test_parse_whisper_mode_remote() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            mode = "remote"
            model = "base.en"
            language = "en"
            remote_endpoint = "http://localhost:8080"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.whisper.mode, Some(WhisperMode::Remote));
        assert_eq!(config.whisper.effective_mode(), WhisperMode::Remote);
    }

    #[test]
    fn test_whisper_backend_alias_local() {
        // Test that deprecated 'backend' field still works
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            backend = "local"
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.whisper.backend, Some(WhisperMode::Local));
        assert!(config.whisper.mode.is_none());
        // effective_mode should return the backend value
        assert_eq!(config.whisper.effective_mode(), WhisperMode::Local);
    }

    #[test]
    fn test_whisper_backend_alias_remote() {
        // Test that deprecated 'backend' field still works for remote
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            backend = "remote"
            model = "base.en"
            language = "en"
            remote_endpoint = "http://localhost:8080"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.whisper.backend, Some(WhisperMode::Remote));
        assert!(config.whisper.mode.is_none());
        // effective_mode should return the backend value
        assert_eq!(config.whisper.effective_mode(), WhisperMode::Remote);
    }

    #[test]
    fn test_whisper_mode_takes_precedence_over_backend() {
        // When both mode and backend are set, mode should take precedence
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            mode = "local"
            backend = "remote"
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.whisper.mode, Some(WhisperMode::Local));
        assert_eq!(config.whisper.backend, Some(WhisperMode::Remote));
        // mode takes precedence
        assert_eq!(config.whisper.effective_mode(), WhisperMode::Local);
    }

    #[test]
    fn test_whisper_effective_mode_defaults_to_local() {
        // When neither mode nor backend is set, effective_mode defaults to Local
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
        assert!(config.whisper.mode.is_none());
        assert!(config.whisper.backend.is_none());
        assert_eq!(config.whisper.effective_mode(), WhisperMode::Local);
    }

    // =========================================================================
    // ParakeetConfig and ParakeetModelType Tests
    // =========================================================================

    #[test]
    fn test_parse_parakeet_model_type_tdt() {
        let toml_str = r#"
            engine = "parakeet"

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

            [parakeet]
            model = "parakeet-tdt-0.6b-v3"
            model_type = "tdt"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let parakeet = config.parakeet.unwrap();
        assert_eq!(parakeet.model, "parakeet-tdt-0.6b-v3");
        assert_eq!(parakeet.model_type, Some(ParakeetModelType::Tdt));
    }

    #[test]
    fn test_parse_parakeet_model_type_ctc() {
        let toml_str = r#"
            engine = "parakeet"

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

            [parakeet]
            model = "parakeet-ctc-0.6b"
            model_type = "ctc"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let parakeet = config.parakeet.unwrap();
        assert_eq!(parakeet.model, "parakeet-ctc-0.6b");
        assert_eq!(parakeet.model_type, Some(ParakeetModelType::Ctc));
    }

    #[test]
    fn test_parakeet_model_type_defaults_to_none_for_auto_detection() {
        let toml_str = r#"
            engine = "parakeet"

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

            [parakeet]
            model = "parakeet-tdt-0.6b-v3"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let parakeet = config.parakeet.unwrap();
        // model_type should be None (will be auto-detected at runtime)
        assert!(parakeet.model_type.is_none());
    }

    #[test]
    fn test_parakeet_config_default() {
        let config = ParakeetConfig::default();
        assert_eq!(config.model, "parakeet-tdt-0.6b-v3");
        assert!(config.model_type.is_none());
        assert!(!config.on_demand_loading);
    }

    #[test]
    fn test_parakeet_model_type_enum_default() {
        // ParakeetModelType defaults to Tdt
        assert_eq!(ParakeetModelType::default(), ParakeetModelType::Tdt);
    }

    #[test]
    fn test_whisper_section_is_optional() {
        // The [whisper] section should be optional for Parakeet users
        // See: https://github.com/peteonrails/voxtype/issues/137
        //
        // We test this by deserializing into a struct that mirrors Config
        // but only has the fields we want to test (avoiding all required fields)
        #[derive(Debug, Deserialize)]
        struct PartialConfig {
            engine: TranscriptionEngine,
            #[serde(default)]
            whisper: WhisperConfig,
        }

        let toml = r#"
            engine = "parakeet"
        "#;

        let config: PartialConfig =
            toml::from_str(toml).expect("whisper section should be optional");
        assert_eq!(config.engine, TranscriptionEngine::Parakeet);
        assert_eq!(config.whisper.model, "base.en"); // Default value
    }

    #[test]
    fn test_config_on_demand_loading_whisper() {
        let config = Config::default();
        assert_eq!(config.engine, TranscriptionEngine::Whisper);
        // on_demand_loading method should return whisper's value
        assert!(!config.on_demand_loading());
    }

    #[test]
    fn test_config_model_name_whisper() {
        let config = Config::default();
        assert_eq!(config.model_name(), "base.en");
    }

    // =========================================================================
    // Profile Tests
    // =========================================================================
    #[test]
    fn test_profiles_default_empty() {
        let config = Config::default();
        assert!(config.profiles.is_empty());
        assert!(config.profile_names().is_empty());
        assert!(config.get_profile("slack").is_none());
    }

    #[test]
    fn test_parse_profiles_from_toml() {
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

            [profiles.slack]
            post_process_command = "cleanup-for-slack.sh"

            [profiles.code]
            post_process_command = "cleanup-for-code.sh"
            output_mode = "clipboard"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.profiles.len(), 2);

        let slack = config.get_profile("slack").unwrap();
        assert_eq!(
            slack.post_process_command,
            Some("cleanup-for-slack.sh".to_string())
        );
        assert!(slack.output_mode.is_none());

        let code = config.get_profile("code").unwrap();
        assert_eq!(
            code.post_process_command,
            Some("cleanup-for-code.sh".to_string())
        );
        assert_eq!(code.output_mode, Some(OutputMode::Clipboard));
    }

    #[test]
    fn test_parse_profile_with_timeout() {
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

            [profiles.slow]
            post_process_command = "slow-llm-command"
            post_process_timeout_ms = 60000
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let slow = config.get_profile("slow").unwrap();
        assert_eq!(
            slow.post_process_command,
            Some("slow-llm-command".to_string())
        );
        assert_eq!(slow.post_process_timeout_ms, Some(60000));
    }

    #[test]
    fn test_profile_names() {
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

            [profiles.alpha]
            post_process_command = "alpha-cmd"

            [profiles.beta]
            post_process_command = "beta-cmd"

            [profiles.gamma]
            post_process_command = "gamma-cmd"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let names: Vec<&str> = config.profile_names().iter().map(|s| s.as_str()).collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
        assert!(names.contains(&"gamma"));
    }

    #[test]
    fn test_profile_without_post_process_command() {
        // A profile can have only output_mode override without post_process_command
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

            [profiles.clipboard_only]
            output_mode = "clipboard"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let profile = config.get_profile("clipboard_only").unwrap();
        assert!(profile.post_process_command.is_none());
        assert_eq!(profile.output_mode, Some(OutputMode::Clipboard));
    }

    #[test]
    fn test_config_without_profiles_section() {
        // Config without [profiles] section should work (backwards compatibility)
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
        assert!(config.profiles.is_empty());
    }

    #[test]
    fn test_parse_driver_order_from_config() {
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
            driver_order = ["ydotool", "wtype", "clipboard"]
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let driver_order = config.output.driver_order.unwrap();
        assert_eq!(driver_order.len(), 3);
        assert_eq!(driver_order[0], OutputDriver::Ydotool);
        assert_eq!(driver_order[1], OutputDriver::Wtype);
        assert_eq!(driver_order[2], OutputDriver::Clipboard);
    }

    #[test]
    fn test_driver_order_not_set_by_default() {
        let config = Config::default();
        assert!(config.output.driver_order.is_none());
    }

    #[test]
    fn test_parse_config_without_driver_order() {
        // Ensure backwards compatibility - config without driver_order should work
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
        assert!(config.output.driver_order.is_none());
    }

    #[test]
    fn test_parse_single_driver_order() {
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
            driver_order = ["ydotool"]
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let driver_order = config.output.driver_order.unwrap();
        assert_eq!(driver_order.len(), 1);
        assert_eq!(driver_order[0], OutputDriver::Ydotool);
    }

    // =========================================================================
    // Meeting Config Tests
    // =========================================================================

    #[test]
    fn test_meeting_config_default() {
        let config = MeetingConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.chunk_duration_secs, 30);
        assert_eq!(config.storage_path, "auto");
        assert!(!config.retain_audio);
        assert_eq!(config.max_duration_mins, 180);
    }

    #[test]
    fn test_meeting_audio_config_default() {
        let config = MeetingAudioConfig::default();
        assert_eq!(config.mic_device, "default");
        assert_eq!(config.loopback_device, "auto");
    }

    #[test]
    fn test_meeting_diarization_config_default() {
        let config = MeetingDiarizationConfig::default();
        assert!(config.enabled);
        assert_eq!(config.backend, "simple");
        assert_eq!(config.max_speakers, 10);
    }

    #[test]
    fn test_meeting_summary_config_default() {
        let config = MeetingSummaryConfig::default();
        assert_eq!(config.backend, "disabled");
        assert_eq!(config.ollama_url, "http://localhost:11434");
        assert_eq!(config.ollama_model, "llama3.2");
        assert!(config.remote_endpoint.is_none());
        assert!(config.remote_api_key.is_none());
        assert_eq!(config.timeout_secs, 120);
    }

    #[test]
    fn test_meeting_config_in_default_config() {
        let config = Config::default();
        assert!(!config.meeting.enabled);
        assert_eq!(config.meeting.chunk_duration_secs, 30);
        assert_eq!(config.meeting.max_duration_mins, 180);
    }

    #[test]
    fn test_parse_meeting_config_from_toml() {
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

            [meeting]
            enabled = true
            chunk_duration_secs = 45
            storage_path = "/tmp/meetings"
            retain_audio = true
            max_duration_mins = 60
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.meeting.enabled);
        assert_eq!(config.meeting.chunk_duration_secs, 45);
        assert_eq!(config.meeting.storage_path, "/tmp/meetings");
        assert!(config.meeting.retain_audio);
        assert_eq!(config.meeting.max_duration_mins, 60);
    }

    #[test]
    fn test_parse_meeting_config_with_nested_sections() {
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

            [meeting]
            enabled = true

            [meeting.audio]
            mic_device = "hw:1"
            loopback_device = "disabled"

            [meeting.diarization]
            enabled = false
            backend = "ml"
            max_speakers = 5

            [meeting.summary]
            backend = "local"
            ollama_model = "mistral"
            timeout_secs = 60
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.meeting.audio.mic_device, "hw:1");
        assert_eq!(config.meeting.audio.loopback_device, "disabled");
        assert!(!config.meeting.diarization.enabled);
        assert_eq!(config.meeting.diarization.backend, "ml");
        assert_eq!(config.meeting.diarization.max_speakers, 5);
        assert_eq!(config.meeting.summary.backend, "local");
        assert_eq!(config.meeting.summary.ollama_model, "mistral");
        assert_eq!(config.meeting.summary.timeout_secs, 60);
    }

    #[test]
    fn test_meeting_config_backward_compatible_omitted() {
        // Config without [meeting] section should parse fine with defaults
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
        assert!(!config.meeting.enabled);
        assert_eq!(config.meeting.chunk_duration_secs, 30);
        assert_eq!(config.meeting.storage_path, "auto");
        assert_eq!(config.meeting.diarization.backend, "simple");
        assert_eq!(config.meeting.summary.backend, "disabled");
    }

    // =========================================================================
    // Clipboard Restore Tests
    // =========================================================================

    #[test]
    fn test_restore_clipboard_defaults() {
        let config = Config::default();
        assert!(!config.output.restore_clipboard);
        assert_eq!(config.output.restore_clipboard_delay_ms, 200);
    }

    #[test]
    fn test_restore_clipboard_deserialization() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 30

            [whisper]
            model = "base.en"

            [output]
            mode = "paste"
            restore_clipboard = true
            restore_clipboard_delay_ms = 500
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.output.restore_clipboard);
        assert_eq!(config.output.restore_clipboard_delay_ms, 500);
    }

    #[test]
    fn test_restore_clipboard_missing_uses_defaults() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 30

            [whisper]
            model = "base.en"

            [output]
            mode = "paste"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.output.restore_clipboard);
        assert_eq!(config.output.restore_clipboard_delay_ms, 200);
    }
}
