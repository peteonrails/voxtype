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

mod audio;
mod default_config;
pub mod engines;
mod hotkey;
mod language;
mod meeting;
mod notification;
mod output;
mod profile;
mod status;
mod text;
mod vad;
mod whisper;

pub use audio::{AudioConfig, AudioFeedbackConfig};
pub use default_config::{default_config_content, DEFAULT_CONFIG};
pub use engines::{
    CohereConfig, DolphinConfig, MoonshineConfig, OmnilingualConfig, ParaformerConfig,
    ParakeetConfig, ParakeetModelType, SenseVoiceConfig, SonioxConfig, TranscriptionEngine,
};
pub use hotkey::{ActivationMode, HotkeyConfig};
pub use language::LanguageConfig;
pub use meeting::{
    MeetingAudioConfig, MeetingConfig, MeetingDiarizationConfig, MeetingSummaryConfig,
};
pub use notification::NotificationConfig;
pub use output::{
    default_language_to_layout, AppliedLanguageXkbHint, FileMode, OutputConfig, OutputDriver,
    OutputMode,
};
pub use profile::{PostProcessConfig, Profile};
pub use status::{ResolvedIcons, StatusConfig, StatusIconOverrides};
pub use text::TextConfig;
pub use vad::{VadBackend, VadConfig};
pub use whisper::{WhisperConfig, WhisperMode};

#[cfg(test)]
use hotkey::default_hotkey_key;
#[cfg(test)]
use status::load_icon_theme;

pub(super) fn default_true() -> bool {
    true
}

fn default_state_file() -> Option<String> {
    Some("auto".to_string())
}

/// Root configuration structure
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub hotkey: HotkeyConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub whisper: WhisperConfig,
    #[serde(default)]
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

    /// Cohere Transcribe configuration (optional, only used when engine = "cohere")
    #[serde(default)]
    pub cohere: Option<CohereConfig>,

    /// Soniox cloud streaming WebSocket STT configuration
    /// (optional, only used when engine = "soniox")
    #[serde(default)]
    pub soniox: Option<SonioxConfig>,

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

    /// On-screen display visualizer configuration. Controls whether the
    /// daemon spawns the `voxtype-osd` child and how it renders.
    #[serde(default)]
    pub osd: crate::osd::config::OsdConfig,

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

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig::default(),
            audio: AudioConfig::default(),
            whisper: WhisperConfig::default(),
            output: OutputConfig::default(),
            engine: TranscriptionEngine::default(),
            parakeet: None,
            moonshine: None,
            sensevoice: None,
            paraformer: None,
            dolphin: None,
            omnilingual: None,
            cohere: None,
            soniox: None,
            text: TextConfig::default(),
            vad: VadConfig::default(),
            status: StatusConfig::default(),
            osd: crate::osd::config::OsdConfig::default(),
            meeting: MeetingConfig::default(),
            state_file: default_state_file(),
            profiles: HashMap::new(),
        }
    }
}

impl Config {
    /// Returns true if the active engine is configured for streaming output.
    ///
    /// Used to decide whether to auto-promote push-to-talk to toggle activation:
    /// streaming output types characters at the cursor while the user is still
    /// holding the hotkey, which clobbers libinput's held-key state tracker on
    /// Hyprland/Sway/River. New streaming backends plug into this gate without
    /// editing the daemon.
    pub fn streaming_active(&self) -> bool {
        match self.engine {
            TranscriptionEngine::Parakeet => {
                self.parakeet.as_ref().map(|p| p.streaming).unwrap_or(false)
            }
            // Missing [soniox] section → don't auto-promote PTT. The
            // transcriber will fail to initialize anyway (no api_key); we
            // shouldn't change hotkey behaviour for a config that can't
            // run. Same shape as the Parakeet arm: explicit opt-in only.
            TranscriptionEngine::Soniox => self
                .soniox
                .as_ref()
                .map(|s| s.streaming && !s.async_api)
                .unwrap_or(false),
            _ => false,
        }
    }

    /// Clone this config with engine-specific overrides for meeting (long-form)
    /// transcription. Currently:
    ///
    /// - **Soniox:** forces `async_api = true`. Meetings feed fixed-size audio
    ///   chunks (30s default) to `Transcriber::transcribe()` — the realtime WS
    ///   would open a fresh socket per chunk, pay connect latency, and bill by
    ///   WS-duration. The async REST path (`stt-async-v4`) is purpose-built
    ///   for this: bills audio-seconds, gives higher accuracy, integrates with
    ///   speaker diarization, and survives network hiccups.
    ///
    /// The dictation path still reads the raw config, so a user who set
    /// `async_api = false` (the default) keeps live-partial WS dictation while
    /// meetings transparently use the async API.
    pub fn with_meeting_mode_overrides(&self) -> Self {
        let mut cfg = self.clone();
        if matches!(cfg.engine, TranscriptionEngine::Soniox) {
            if let Some(ref mut sx) = cfg.soniox {
                if !sx.async_api {
                    tracing::info!(
                        "Soniox meeting mode: routing to async API (stt-async-v4); dictation path unchanged"
                    );
                    sx.async_api = true;
                }
            }
        }
        cfg
    }

    /// System-wide config path used as a fallback when no user config exists.
    pub const SYSTEM_PATH: &'static str = "/etc/voxtype/config.toml";

    /// Get the default user config file path (XDG)
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "voxtype")
            .map(|dirs| dirs.config_dir().join("config.toml"))
    }

    /// Get the system-wide config file path.
    pub fn system_path() -> PathBuf {
        PathBuf::from(Self::SYSTEM_PATH)
    }

    /// Resolve which config file should actually be loaded, in priority order:
    /// 1. User config (`~/.config/voxtype/config.toml`)
    /// 2. System-wide config (`/etc/voxtype/config.toml`)
    ///
    /// Returns `None` if neither exists, in which case the caller should fall
    /// back to built-in defaults. This does not consider the `--config` CLI
    /// flag; callers handle that explicitly.
    pub fn resolve_existing_path() -> Option<PathBuf> {
        if let Some(user) = Self::default_path() {
            if user.exists() {
                return Some(user);
            }
        }
        let system = Self::system_path();
        if system.exists() {
            return Some(system);
        }
        None
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
        cachedir::ensure_tag(&models_dir)
            .unwrap_or_else(|e| tracing::warn!("could not tag models dir: {e}"));

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
            TranscriptionEngine::Cohere => self
                .cohere
                .as_ref()
                .map(|c| c.on_demand_loading)
                .unwrap_or(false),
            // Soniox is a cloud backend; nothing to load on demand.
            TranscriptionEngine::Soniox => false,
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
            TranscriptionEngine::Cohere => self
                .cohere
                .as_ref()
                .map(|c| c.model.as_str())
                .unwrap_or("cohere (not configured)"),
            TranscriptionEngine::Soniox => self
                .soniox
                .as_ref()
                .map(|s| s.model.as_str())
                .unwrap_or("soniox (not configured)"),
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
/// Parse a TOML config string by layering it over default values.
///
/// Renders `Config::default()` to a TOML `Value`, deep-merges the user's TOML
/// on top, then deserializes back into `Config`. Tables merge recursively
/// (user keys win when both sides have them); other values (scalars, arrays)
/// take user-supplied values verbatim. The result: any user TOML that's a
/// subset of the full config, down to a single `[audio.feedback] enabled = true`
/// produces a valid Config with defaults filled in for everything else.
pub fn parse_config_with_defaults(contents: &str) -> Result<Config, toml::de::Error> {
    let defaults = toml::Value::try_from(Config::default())
        .expect("Config::default() must be serializable to TOML");
    let user: toml::Value = toml::from_str(contents)?;
    let mut merged = defaults;
    merge_toml_values(&mut merged, user);
    merged.try_into()
}

/// Deep-merge `overlay` onto `base`. Tables merge recursively; for any other
/// value type (or when the two sides have mismatched types), `overlay` wins.
/// Arrays are replaced wholesale rather than concatenated. Extending a
/// defaulted list (e.g. `language_to_layout`) requires the user to spell out
/// the full replacement value.
fn merge_toml_values(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                if let Some(existing) = b.get_mut(&k) {
                    merge_toml_values(existing, v);
                } else {
                    b.insert(k, v);
                }
            }
        }
        (slot, other) => *slot = other,
    }
}

pub fn load_config(path: Option<&Path>) -> Result<Config, VoxtypeError> {
    // Start with defaults
    let mut config = Config::default();

    // Determine config file path. If --config wasn't passed, walk the
    // documented lookup chain: user config -> /etc/voxtype/config.toml.
    let config_path = match path {
        Some(p) => Some(PathBuf::from(p)),
        None => Config::resolve_existing_path(),
    };

    // Load from file if it exists. Layer the user's TOML over a default-rendered
    // Config so partial sections inherit defaults for fields the user didn't set.
    // Without this, a user config containing only `[hotkey]` or `[audio.feedback]`
    // would fail with "missing field 'audio'" / "missing field 'device'", because
    // serde's per-field defaults only fill in what the user omitted at the *field*
    // level, not what they omitted at a parent-section level (#421).
    if let Some(ref path) = config_path {
        if path.exists() {
            tracing::debug!("Loading config from {:?}", path);
            let contents = std::fs::read_to_string(path)
                .map_err(|e| VoxtypeError::Config(format!("Failed to read config: {}", e)))?;

            config = parse_config_with_defaults(&contents)
                .map_err(|e| VoxtypeError::Config(format!("Invalid config: {}", e)))?;
        } else {
            tracing::debug!("Config file not found at {:?}, using defaults", path);
        }
    } else {
        tracing::debug!("No config file found at user or system path, using built-in defaults");
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
            "cohere" => config.engine = TranscriptionEngine::Cohere,
            "soniox" => config.engine = TranscriptionEngine::Soniox,
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
    if let Ok(val) = std::env::var("VOXTYPE_GPU_DEVICE") {
        if let Ok(n) = val.parse::<i32>() {
            config.whisper.gpu_device = Some(n);
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_FLASH_ATTENTION") {
        config.whisper.flash_attention = parse_bool_env(&val);
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
    if let Ok(val) = std::env::var("VOXTYPE_PAUSE_MEDIA") {
        config.audio.pause_media = parse_bool_env(&val);
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
    if std::env::var("VOXTYPE_WTYPE_SHIFT_PREFIX")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        config.output.wtype_shift_prefix = true;
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
    if let Ok(variant) = std::env::var("VOXTYPE_DOTOOL_XKB_VARIANT") {
        config.output.dotool_xkb_variant = Some(variant);
    }
    if let Ok(layout) = std::env::var("VOXTYPE_EITYPE_XKB_LAYOUT") {
        config.output.eitype_xkb_layout = Some(layout);
    }
    if let Ok(variant) = std::env::var("VOXTYPE_EITYPE_XKB_VARIANT") {
        config.output.eitype_xkb_variant = Some(variant);
    }

    // Remote whisper
    if let Ok(endpoint) = std::env::var("VOXTYPE_REMOTE_ENDPOINT") {
        config.whisper.remote_endpoint = Some(endpoint);
    }
    if let Ok(key) = std::env::var("VOXTYPE_WHISPER_API_KEY") {
        config.whisper.remote_api_key = Some(key);
    }

    // Soniox
    if let Ok(key) = std::env::var("SONIOX_API_KEY") {
        config
            .soniox
            .get_or_insert_with(SonioxConfig::default)
            .api_key = Some(key);
    }
    if let Ok(val) = std::env::var("VOXTYPE_RESTORE_CLIPBOARD") {
        config.output.restore_clipboard = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_RESTORE_CLIPBOARD_DELAY_MS") {
        if let Ok(ms) = val.parse::<u32>() {
            config.output.restore_clipboard_delay_ms = ms;
        }
    }
    if let Ok(val) = std::env::var("VOXTYPE_SMART_AUTO_SUBMIT") {
        config.text.smart_auto_submit = parse_bool_env(&val);
    }
    if let Ok(val) = std::env::var("VOXTYPE_FILTER_FILLERS") {
        config.text.filter_filler_words = parse_bool_env(&val);
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
    fn meeting_mode_forces_soniox_async_when_user_had_realtime() {
        let cfg = Config {
            engine: TranscriptionEngine::Soniox,
            soniox: Some(SonioxConfig {
                api_key: Some("k".into()),
                async_api: false,
                ..SonioxConfig::default()
            }),
            ..Config::default()
        };
        let meeting_cfg = cfg.with_meeting_mode_overrides();
        assert!(meeting_cfg.soniox.as_ref().unwrap().async_api);
        // Original config untouched — dictation path keeps realtime.
        assert!(!cfg.soniox.as_ref().unwrap().async_api);
    }

    #[test]
    fn meeting_mode_preserves_explicit_soniox_async() {
        let cfg = Config {
            engine: TranscriptionEngine::Soniox,
            soniox: Some(SonioxConfig {
                api_key: Some("k".into()),
                async_api: true,
                ..SonioxConfig::default()
            }),
            ..Config::default()
        };
        let meeting_cfg = cfg.with_meeting_mode_overrides();
        assert!(meeting_cfg.soniox.as_ref().unwrap().async_api);
    }

    #[test]
    fn meeting_mode_is_noop_for_non_soniox_engines() {
        let cfg = Config::default(); // engine = Whisper
        let meeting_cfg = cfg.with_meeting_mode_overrides();
        assert_eq!(meeting_cfg.engine, cfg.engine);
        assert_eq!(meeting_cfg.whisper.model, cfg.whisper.model);
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.hotkey.key, default_hotkey_key());
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
                streaming: None,
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
        assert_eq!(config.vad_threshold, 0.01);
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
            vad_threshold = 0.001

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
        assert_eq!(config.meeting.audio.vad_threshold, 0.001);
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

    #[test]
    fn test_system_path_constant() {
        assert_eq!(
            Config::system_path(),
            PathBuf::from("/etc/voxtype/config.toml")
        );
        assert_eq!(Config::SYSTEM_PATH, "/etc/voxtype/config.toml");
    }

    #[test]
    fn test_load_config_explicit_path() {
        // Explicit --config should always be used regardless of fallback.
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
                [hotkey]
                key = "F12"

                [audio]
                device = "default"
                sample_rate = 16000
                max_duration_secs = 30

                [whisper]
                model = "tiny.en"
                language = "en"

                [output]
                mode = "clipboard"
            "#,
        )
        .unwrap();

        let config = load_config(Some(&config_path)).unwrap();
        assert_eq!(config.hotkey.key, "F12");
        assert_eq!(config.whisper.model, "tiny.en");
        assert_eq!(config.output.mode, OutputMode::Clipboard);
    }

    #[test]
    fn test_default_language_to_layout_common_cases() {
        let map = default_language_to_layout();
        // English maps to "us", the XKB convention.
        assert_eq!(map.get("en"), Some(&"us".to_string()));
        // Russian, German, French, Spanish are direct passthroughs.
        assert_eq!(map.get("ru"), Some(&"ru".to_string()));
        assert_eq!(map.get("de"), Some(&"de".to_string()));
        assert_eq!(map.get("fr"), Some(&"fr".to_string()));
        assert_eq!(map.get("es"), Some(&"es".to_string()));
        // Greek uses "gr", not "el".
        assert_eq!(map.get("el"), Some(&"gr".to_string()));
        // Japanese / Korean map to common XKB names.
        assert_eq!(map.get("ja"), Some(&"jp".to_string()));
        assert_eq!(map.get("ko"), Some(&"kr".to_string()));
    }

    #[test]
    fn test_output_config_default_includes_language_layout_map() {
        let cfg = Config::default();
        assert!(!cfg.output.language_to_layout.is_empty());
        assert_eq!(
            cfg.output.language_to_layout.get("en"),
            Some(&"us".to_string())
        );
        assert!(cfg.output.language_to_variant.is_empty());
        // New eitype layout fields are unset by default; the layout is
        // inferred from the detected language only when both fields are
        // empty (see daemon::handle_transcription_result).
        assert!(cfg.output.eitype_xkb_layout.is_none());
        assert!(cfg.output.eitype_xkb_variant.is_none());
    }

    #[test]
    fn test_parse_eitype_layout_from_toml() {
        let toml_str = r#"
            [hotkey]
            key = "PAUSE"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
            eitype_xkb_layout = "ru"
            eitype_xkb_variant = "phonetic"
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.output.eitype_xkb_layout, Some("ru".to_string()));
        assert_eq!(
            config.output.eitype_xkb_variant,
            Some("phonetic".to_string())
        );
    }

    #[test]
    fn test_parse_language_to_layout_override() {
        // User can override individual mappings (e.g. Brazilian Portuguese
        // typically needs the `br` layout, not `pt`). Providing the field
        // replaces the built-in defaults; users are expected to copy
        // entries they want to keep (documented in CONFIGURATION.md).
        let toml_str = r#"
            [hotkey]
            key = "PAUSE"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [output.language_to_layout]
            pt = "br"
            en = "dvorak"
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.output.language_to_layout.get("pt"),
            Some(&"br".to_string())
        );
        assert_eq!(
            config.output.language_to_layout.get("en"),
            Some(&"dvorak".to_string())
        );
    }

    #[test]
    fn test_parse_language_to_variant() {
        let toml_str = r#"
            [hotkey]
            key = "PAUSE"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = ["en", "ru"]

            [output]
            mode = "type"

            [output.language_to_layout]
            en = "us"
            ru = "ru"

            [output.language_to_variant]
            ru = "phonetic"
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.output.language_to_variant.get("ru"),
            Some(&"phonetic".to_string())
        );
        assert!(!config.output.language_to_variant.contains_key("en"));
    }

    #[test]
    fn test_apply_language_xkb_hint_applies_layout_and_variant() {
        let mut output = Config::default().output;
        output
            .language_to_variant
            .insert("ru".to_string(), "phonetic".to_string());

        let applied = output.apply_language_xkb_hint("ru");

        assert_eq!(applied.layout, Some("ru".to_string()));
        assert_eq!(applied.variant, Some("phonetic".to_string()));
        assert!(applied.eitype_layout_applied);
        assert!(applied.dotool_layout_applied);
        assert!(applied.eitype_variant_applied);
        assert!(applied.dotool_variant_applied);
        assert_eq!(output.eitype_xkb_layout, Some("ru".to_string()));
        assert_eq!(output.dotool_xkb_layout, Some("ru".to_string()));
        assert_eq!(output.eitype_xkb_variant, Some("phonetic".to_string()));
        assert_eq!(output.dotool_xkb_variant, Some("phonetic".to_string()));
    }

    #[test]
    fn test_apply_language_xkb_hint_does_not_leak_variant_between_languages() {
        let mut output = Config::default().output;
        output
            .language_to_variant
            .insert("ru".to_string(), "phonetic".to_string());

        let applied = output.apply_language_xkb_hint("en");

        assert_eq!(applied.layout, Some("us".to_string()));
        assert_eq!(applied.variant, None);
        assert_eq!(output.eitype_xkb_layout, Some("us".to_string()));
        assert_eq!(output.dotool_xkb_layout, Some("us".to_string()));
        assert_eq!(output.eitype_xkb_variant, None);
        assert_eq!(output.dotool_xkb_variant, None);
    }

    #[test]
    fn test_apply_language_xkb_hint_preserves_explicit_variant() {
        let mut output = Config::default().output;
        output.eitype_xkb_variant = Some("explicit-eitype".to_string());
        output.dotool_xkb_variant = Some("explicit-dotool".to_string());
        output
            .language_to_variant
            .insert("ru".to_string(), "phonetic".to_string());

        let applied = output.apply_language_xkb_hint("ru");

        assert_eq!(applied.variant, Some("phonetic".to_string()));
        assert!(!applied.eitype_variant_applied);
        assert!(!applied.dotool_variant_applied);
        assert_eq!(
            output.eitype_xkb_variant,
            Some("explicit-eitype".to_string())
        );
        assert_eq!(
            output.dotool_xkb_variant,
            Some("explicit-dotool".to_string())
        );
    }

    // ----- Partial-config layering (regression tests for #421) -----

    #[test]
    fn empty_toml_yields_default_config() {
        // Invariant: parse_config_with_defaults("") must produce exactly
        // Config::default(). If this ever fails, defaults have drifted
        // between the hand-rolled Config::default() and the serde-default
        // path. They are supposed to be the same fact in one place.
        let parsed = parse_config_with_defaults("").expect("empty TOML must succeed");
        let defaults = Config::default();
        // Compare via TOML round-trip because Config doesn't impl PartialEq.
        let parsed_toml = toml::Value::try_from(&parsed).expect("Config must serialize to TOML");
        let default_toml = toml::Value::try_from(&defaults).expect("Config must serialize to TOML");
        assert_eq!(parsed_toml, default_toml);
    }

    #[test]
    fn partial_config_with_only_hotkey_section() {
        // AlexCzar's TUI symptom (#421): a config with only [hotkey] used to
        // be rejected with "missing field 'audio'" because Config required
        // top-level audio/output sections.
        let toml = r#"
            [hotkey]
            key = "RIGHTALT"
        "#;
        let cfg =
            parse_config_with_defaults(toml).expect("partial config must layer over defaults");
        assert_eq!(cfg.hotkey.key, "RIGHTALT");
        // Defaults must fill in for everything else.
        assert_eq!(cfg.audio.device, "default");
        assert_eq!(cfg.audio.sample_rate, 16000);
        assert_eq!(cfg.output.mode, OutputMode::Type);
    }

    #[test]
    fn partial_audio_feedback_does_not_require_device() {
        // AlexCzar's daemon symptom (#421): writing [audio.feedback] used to
        // fail with "missing field 'device'" because AudioConfig::device had
        // no serde default. Now it falls back to default_audio_device().
        let toml = r#"
            engine = "whisper"

            [hotkey]
            key = "RIGHTALT"

            [audio.feedback]
            enabled = true
            theme = "subtle"

            [whisper]
            backend = "local"
            model = "large-v3-turbo"
            language = ["en", "ru"]
        "#;
        let cfg = parse_config_with_defaults(toml).expect("AlexCzar's config must parse");
        assert!(cfg.audio.feedback.enabled);
        assert_eq!(cfg.audio.feedback.theme, "subtle");
        // Audio fields the user didn't set come from defaults.
        assert_eq!(cfg.audio.device, "default");
        assert_eq!(cfg.audio.sample_rate, 16000);
        // Whisper sub-fields the user didn't set come from defaults too.
        assert_eq!(cfg.whisper.model, "large-v3-turbo");
        assert!(!cfg.whisper.translate);
    }

    #[test]
    fn partial_section_does_not_clobber_sibling_defaults() {
        // User specifies one audio.feedback field; volume should keep its
        // default rather than getting reset by the partial sub-table.
        let toml = r#"
            [audio.feedback]
            enabled = true
        "#;
        let cfg = parse_config_with_defaults(toml).expect("partial sub-table must layer");
        assert!(cfg.audio.feedback.enabled);
        assert_eq!(cfg.audio.feedback.volume, 0.7);
        assert_eq!(cfg.audio.feedback.theme, "default");
    }

    #[test]
    fn user_arrays_replace_default_arrays() {
        // Arrays are replaced wholesale, not concatenated. A user-specified
        // language list overrides the default rather than appending to it.
        let toml = r#"
            [whisper]
            language = ["ja", "ko"]
        "#;
        let cfg = parse_config_with_defaults(toml).expect("array replacement must work");
        // Just confirm the user value wins; the precise default shape isn't
        // load-bearing here, but their values should not appear in the result.
        let langs_toml =
            toml::Value::try_from(&cfg.whisper.language).expect("LanguageConfig must serialize");
        let langs_str = langs_toml.to_string();
        assert!(langs_str.contains("ja"));
        assert!(langs_str.contains("ko"));
    }

    #[test]
    fn merge_toml_values_recurses_into_tables() {
        // Unit test for the merge primitive itself.
        let mut base: toml::Value = toml::from_str(
            r#"
            [audio]
            device = "default"
            sample_rate = 16000

            [audio.feedback]
            enabled = false
            volume = 0.7
            "#,
        )
        .unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
            [audio.feedback]
            enabled = true
            "#,
        )
        .unwrap();

        merge_toml_values(&mut base, overlay);

        // audio.device preserved
        assert_eq!(
            base.get("audio").and_then(|a| a.get("device")),
            Some(&toml::Value::String("default".to_string()))
        );
        // audio.feedback.enabled overridden
        assert_eq!(
            base.get("audio")
                .and_then(|a| a.get("feedback"))
                .and_then(|f| f.get("enabled")),
            Some(&toml::Value::Boolean(true))
        );
        // audio.feedback.volume preserved
        assert_eq!(
            base.get("audio")
                .and_then(|a| a.get("feedback"))
                .and_then(|f| f.get("volume")),
            Some(&toml::Value::Float(0.7))
        );
    }

    #[test]
    fn parse_rejects_unknown_field_types() {
        // A type mismatch (string where a number is expected) must still
        // surface as an error after merging defaults.
        let toml = r#"
            [audio]
            sample_rate = "not a number"
        "#;
        let result = parse_config_with_defaults(toml);
        assert!(result.is_err(), "type mismatch must still error");
    }
}
