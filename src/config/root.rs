use super::{
    AudioConfig, CohereConfig, DolphinConfig, HotkeyConfig, MeetingConfig, MoonshineConfig,
    OmnilingualConfig, OutputConfig, ParaformerConfig, ParakeetConfig, Profile, SenseVoiceConfig,
    SonioxConfig, StatusConfig, TextConfig, TranscriptionEngine, VadConfig, WhisperConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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
    /// When set, the daemon writes current state ("idle", "recording",
    /// "transcribing", "outputting") to this file whenever state changes.
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

#[cfg(test)]
mod tests {
    use super::super::hotkey::default_hotkey_key;
    use super::super::{ActivationMode, OutputMode};
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
    fn test_system_path_constant() {
        assert_eq!(
            Config::system_path(),
            PathBuf::from("/etc/voxtype/config.toml")
        );
        assert_eq!(Config::SYSTEM_PATH, "/etc/voxtype/config.toml");
    }
}
