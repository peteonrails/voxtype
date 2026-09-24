use super::{
    AudioConfig, CohereConfig, DolphinConfig, HotkeyConfig, MeetingConfig, MoonshineConfig,
    OmnilingualConfig, OpenVinoConfig, OutputConfig, ParaformerConfig, ParakeetConfig, Profile,
    SenseVoiceConfig, SonioxConfig, StatusConfig, StreamingConfig, TextConfig, TranscriptionEngine,
    VadConfig, WhisperConfig,
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

    /// OpenVINO Whisper configuration (optional, only used when engine = "openvino")
    #[serde(default)]
    pub openvino: Option<OpenVinoConfig>,

    /// Soniox cloud streaming WebSocket STT configuration
    /// (optional, only used when engine = "soniox")
    #[serde(default)]
    pub soniox: Option<SonioxConfig>,

    /// Shared sliding-window streaming engine tuning, used by every batch
    /// backend wrapped in `transcribe::sliding_window` (currently `whisper`
    /// and `openvino`). `None` when config.toml has no `[streaming]`
    /// section, in which case each engine falls back to its own deprecated
    /// `streaming_*` fields — see `StreamingConfig::resolve`.
    #[serde(default)]
    pub streaming: Option<StreamingConfig>,

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
            openvino: None,
            soniox: None,
            streaming: None,
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
            // Same sliding-window engine and the same libinput held-key
            // hazard as OpenVino below — this arm was missing until now,
            // which meant push-to-talk users with `[whisper] streaming =
            // true` never got auto-promoted to toggle mode and could hit
            // the exact stuck-recording bug this gate exists to prevent.
            TranscriptionEngine::Whisper => self.whisper.streaming,
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
            // Same reasoning as Parakeet/Soniox: an absent [openvino] section
            // means the transcriber can't initialize anyway, so don't
            // auto-promote push-to-talk to toggle for a config that can't
            // run. Missing this arm previously left recording permanently
            // stuck open on the first real NPU/GPU streaming session, since
            // typing at the cursor while a key is physically held clobbers
            // libinput's held-key tracking on Hyprland/Sway/River.
            TranscriptionEngine::OpenVino => {
                self.openvino.as_ref().map(|o| o.streaming).unwrap_or(false)
            }
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

    /// Default user config file path: `<config_dir>/config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("config.toml"))
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

    /// Voxtype's user config directory, honoring `$XDG_CONFIG_HOME` (default
    /// `~/.config`) on every platform including macOS, where the `directories`
    /// crate would use `~/Library/Application Support` and ignore XDG (#448).
    pub fn config_dir() -> Option<PathBuf> {
        Self::xdg_dir(
            "XDG_CONFIG_HOME",
            ".config",
            directories::ProjectDirs::from("", "", "voxtype").map(|d| d.config_dir().to_path_buf()),
        )
    }

    /// Voxtype's user data directory (parent of the models dir), honoring
    /// `$XDG_DATA_HOME` (default `~/.local/share`); same scheme as [`Config::config_dir`].
    pub fn data_dir() -> PathBuf {
        Self::xdg_dir(
            "XDG_DATA_HOME",
            ".local/share",
            directories::ProjectDirs::from("", "", "voxtype").map(|d| d.data_dir().to_path_buf()),
        )
        .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Resolve a `voxtype` user dir. An explicit absolute `$xdg_var` wins;
    /// otherwise `$HOME/<default_rel>/voxtype`. Falls back to an existing
    /// `legacy` platform-native dir so an upgrade never orphans a prior install.
    fn xdg_dir(xdg_var: &str, default_rel: &str, legacy: Option<PathBuf>) -> Option<PathBuf> {
        if let Some(base) = std::env::var_os(xdg_var)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            return Some(base.join("voxtype"));
        }
        let xdg = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(|h| PathBuf::from(h).join(default_rel).join("voxtype"));
        match (xdg, legacy) {
            (Some(x), Some(l)) if x != l && !x.exists() && l.exists() => Some(l),
            (Some(x), _) => Some(x),
            (None, l) => l,
        }
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
            TranscriptionEngine::OpenVino => self
                .openvino
                .as_ref()
                .map(|o| o.on_demand_loading)
                .unwrap_or(false),
            // Soniox is a cloud backend; nothing to load on demand.
            TranscriptionEngine::Soniox => false,
        }
    }

    /// The language code configured for the active engine, if it has one.
    ///
    /// Engines disagree about where language lives, and several do not take a
    /// language at all (they detect it, or are single-language). Callers that
    /// need to adapt behaviour to language — filler-word filtering is the
    /// first — should ask here rather than reaching into one engine's config
    /// and being wrong for the other eight.
    ///
    /// Returns `None` for automatic detection and for engines without the
    /// concept, so callers can distinguish "English" from "unknown".
    pub fn active_language(&self) -> Option<&str> {
        let code = match self.engine {
            TranscriptionEngine::Whisper => match &self.whisper.language {
                super::language::LanguageConfig::Single(code) => code.as_str(),
                // A constrained detection set is not one language.
                super::language::LanguageConfig::Multiple(_) => return None,
            },
            TranscriptionEngine::Cohere => self.cohere.as_ref().map(|c| c.language.as_str())?,
            TranscriptionEngine::SenseVoice => {
                self.sensevoice.as_ref().map(|s| s.language.as_str())?
            }
            // Parakeet, Moonshine, Paraformer, Dolphin, Omnilingual and Soniox
            // either detect the language or are fixed to one.
            _ => return None,
        };

        match code {
            "" | "auto" => None,
            other => Some(other),
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
            TranscriptionEngine::OpenVino => self
                .openvino
                .as_ref()
                .map(|o| o.model.as_str())
                .unwrap_or("openvino (not configured)"),
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

    #[test]
    fn xdg_dir_resolution() {
        // Mutates $HOME / $XDG_CONFIG_HOME, like the other env tests here.
        let prev_home = std::env::var_os("HOME");
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_CONFIG_HOME");
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let xdg = tmp.path().join(".config/voxtype");
        let legacy = tmp.path().join("Library/Application Support/voxtype");
        let resolve = || Config::xdg_dir("XDG_CONFIG_HOME", ".config", Some(legacy.clone()));

        // Fresh install: XDG path, even before it exists.
        assert_eq!(resolve(), Some(xdg.clone()));
        // Only the legacy dir exists (upgrade): keep it, do not orphan config.
        std::fs::create_dir_all(&legacy).unwrap();
        assert_eq!(resolve(), Some(legacy.clone()));
        // Once the XDG dir exists too, it wins.
        std::fs::create_dir_all(&xdg).unwrap();
        assert_eq!(resolve(), Some(xdg));
        // Explicit absolute XDG_CONFIG_HOME overrides everything.
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/voxtype-xdg-abs");
        assert_eq!(
            Config::config_dir(),
            Some(PathBuf::from("/tmp/voxtype-xdg-abs/voxtype"))
        );

        match prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
