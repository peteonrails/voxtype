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
    fn test_system_path_constant() {
        assert_eq!(
            Config::system_path(),
            PathBuf::from("/etc/voxtype/config.toml")
        );
        assert_eq!(Config::SYSTEM_PATH, "/etc/voxtype/config.toml");
    }

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
