use super::{
    AudioConfig, CohereConfig, DolphinConfig, ElevenLabsConfig, GgufConfig, HotkeyConfig,
    MeetingConfig, MoonshineConfig, OmnilingualConfig, OpenVinoConfig, OutputConfig,
    ParaformerConfig, ParakeetConfig, ParakeetModelType, Profile, SenseVoiceConfig, SonioxConfig,
    StatusConfig, StreamingConfig, StreamingRecoveryConfig, TextConfig, TranscriptionEngine,
    VadConfig, WhisperConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// The `--config` file, when one was given. See [`Config::set_path_override`].
static CONFIG_PATH_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn default_state_file() -> Option<String> {
    Some("auto".to_string())
}

/// What an engine's own settings say about the language it writes.
pub(crate) enum EngineLanguage<'a> {
    /// One explicit language.
    Fixed(&'a str),
    /// Detection limited to a set: a Whisper language array, Soniox hints.
    OneOf(Vec<&'a str>),
    /// Automatic detection, or an engine with no language setting.
    Open,
}

/// Where [`Config::text_language`] got its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextLanguageSource {
    /// The active engine's own language setting.
    Engine,
    /// The selected model writes only this language.
    Model,
    /// The user's `text.language`.
    Declared,
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

    /// Shared native GGUF configuration (optional, only used for engine = "gguf").
    #[serde(default)]
    pub gguf: Option<GgufConfig>,

    /// OpenVINO Whisper configuration (optional, only used when engine = "openvino")
    #[serde(default)]
    pub openvino: Option<OpenVinoConfig>,

    /// Soniox cloud streaming WebSocket STT configuration
    /// (optional, only used when engine = "soniox")
    #[serde(default)]
    pub soniox: Option<SonioxConfig>,

    /// ElevenLabs Scribe cloud transcription configuration
    /// (optional, only used when engine = "elevenlabs")
    #[serde(default)]
    pub elevenlabs: Option<ElevenLabsConfig>,

    /// Shared sliding-window streaming engine tuning, used by every batch
    /// backend wrapped in `transcribe::sliding_window` (currently `whisper`
    /// and `openvino`). `None` when config.toml has no `[streaming]`
    /// section, in which case each engine falls back to its own deprecated
    /// `streaming_*` fields — see `StreamingConfig::resolve`.
    #[serde(default)]
    pub streaming: Option<StreamingConfig>,

    /// Recovery settings for every streaming backend. Separate from
    /// `[streaming]` so changing recovery cannot override legacy tuning.
    #[serde(default)]
    pub streaming_recovery: Option<StreamingRecoveryConfig>,

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

    /// Native Linux StatusNotifierItem tray, enabled unless explicitly disabled.
    #[serde(default)]
    pub tray: TrayConfig,

    /// Omarchy desktop integration (the packaged Settings panel).
    #[serde(default)]
    pub omarchy: OmarchyConfig,

    /// Meeting transcription configuration
    #[serde(default)]
    pub meeting: MeetingConfig,

    /// Dictation history. Off unless enabled; new configs enable it.
    #[serde(default)]
    pub history: super::HistoryConfig,

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

/// Tray preferences are retained on all platforms; runtime support is Linux-only.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrayConfig {
    #[serde(default = "default_tray_enabled")]
    pub enabled: bool,
}

fn default_tray_enabled() -> bool {
    true
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            enabled: default_tray_enabled(),
        }
    }
}

/// Omarchy integration preferences. Ignored on desktops without Omarchy.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OmarchyConfig {
    /// Let the daemon link the packaged Settings panel into
    /// `~/.config/omarchy/plugins` on its first start, and refresh it after a
    /// package upgrade. `voxtype setup omarchy` works either way.
    #[serde(default = "default_link_settings_plugin")]
    pub link_settings_plugin: bool,
}

fn default_link_settings_plugin() -> bool {
    true
}

impl Default for OmarchyConfig {
    fn default() -> Self {
        Self {
            link_settings_plugin: default_link_settings_plugin(),
        }
    }
}

/// What [`Config::active_gguf`] returns: the engine's config section (also
/// its `--engine` name for the worker), its model, and its `gguf_backend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveGguf<'a> {
    pub section: &'static str,
    pub model: &'a str,
    pub backend: &'a str,
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
            gguf: None,
            openvino: None,
            soniox: None,
            elevenlabs: None,
            streaming: None,
            streaming_recovery: None,
            text: TextConfig::default(),
            vad: VadConfig::default(),
            status: StatusConfig::default(),
            osd: crate::osd::config::OsdConfig::default(),
            tray: TrayConfig::default(),
            omarchy: OmarchyConfig::default(),
            meeting: MeetingConfig::default(),
            history: super::HistoryConfig::default(),
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

    /// Clone this config for meeting (long-form) transcription: the
    /// `meeting start` choices in `start`, then engine-specific adjustments.
    ///
    /// - **Engine, model, language** from `start`. A model alone selects its
    ///   own engine; an engine alone keeps that engine's configured model. A
    ///   language is refused for a checkpoint that takes none.
    /// - **Soniox:** forces `async_api = true`. Meetings feed fixed-size audio
    ///   chunks (30s default) to `Transcriber::transcribe()` — the realtime WS
    ///   would open a fresh socket per chunk, pay connect latency, and bill by
    ///   WS-duration. The async REST path (`stt-async-v6`) is purpose-built
    ///   for this: bills audio-seconds, gives higher accuracy, integrates with
    ///   speaker diarization, and survives network hiccups.
    ///
    /// The dictation path still reads the raw config, so a user who set
    /// `async_api = false` (the default) keeps live-partial WS dictation while
    /// meetings transparently use the async API. Whether the engine and model
    /// are compiled in and installed is checked by the caller.
    pub fn with_meeting_mode_overrides(
        &self,
        start: &super::MeetingStartOptions,
    ) -> Result<Self, String> {
        let mut cfg = self.clone();

        let engine = match start.engine.as_deref() {
            Some(name) => Some(crate::config_set::parse_engine(name).ok_or_else(|| {
                format!(
                    "Unknown engine '{}'. Engines: {}",
                    name,
                    crate::config_set::ENGINE_NAMES.join(", ")
                )
            })?),
            None => None,
        };
        match (start.model.as_deref(), engine) {
            (Some(model), engine) => {
                let resolved = crate::model_catalog::resolve_model(model).ok_or_else(|| {
                    format!(
                        "Unknown model '{}'. Run `voxtype info models` to list available models.",
                        model
                    )
                })?;
                let model_engine: TranscriptionEngine = resolved
                    .engine
                    .parse()
                    .map_err(|_| format!("Model '{}' belongs to an unknown engine", model))?;
                if let Some(engine) = engine.filter(|e| *e != model_engine) {
                    return Err(format!(
                        "Model '{}' is a {} model, not a {} model",
                        model,
                        model_engine.name(),
                        engine.name()
                    ));
                }
                cfg = crate::model_catalog::override_config(&cfg, model_engine, &resolved.model);
            }
            (None, Some(engine)) => cfg.engine = engine,
            (None, None) => {}
        }

        if let Some(language) = start.language.as_deref() {
            let menu = crate::language_selection::LanguageMenu::for_config(&cfg);
            let offered = menu.choices.iter().any(|(code, _)| *code == language);
            if !offered && menu.current != language {
                return Err(format!(
                    "Language '{}' is not available for {}: {}",
                    language, menu.context, menu.explanation
                ));
            }
            match cfg.engine {
                TranscriptionEngine::Whisper => {
                    cfg.whisper.language = super::LanguageConfig::Single(language.to_string())
                }
                TranscriptionEngine::Cohere => {
                    cfg.cohere.get_or_insert_with(Default::default).language = language.to_string()
                }
                _ => {}
            }
        }

        if matches!(cfg.engine, TranscriptionEngine::Soniox) {
            if let Some(ref mut sx) = cfg.soniox {
                if !sx.async_api {
                    tracing::info!(
                        "Soniox meeting mode: routing to async API (stt-async-v6); dictation path unchanged"
                    );
                    sx.async_api = true;
                }
            }
        }
        Ok(cfg)
    }

    /// System-wide config path used as a fallback when no user config exists.
    pub const SYSTEM_PATH: &'static str = "/etc/voxtype/config.toml";

    /// Default user config file path: `<config_dir>/config.toml`, or the file
    /// given with `--config` once [`Config::set_path_override`] has run.
    pub fn default_path() -> Option<PathBuf> {
        if let Some(path) = CONFIG_PATH_OVERRIDE.get() {
            return Some(path.clone());
        }
        Self::config_dir().map(|dir| dir.join("config.toml"))
    }

    /// Make `default_path()` return the `--config` file for the rest of the
    /// process. Setup commands write the config through `default_path()`, so
    /// without this `voxtype --config X setup model --set M` edited the
    /// user's real config instead of X.
    pub fn set_path_override(path: PathBuf) {
        let _ = CONFIG_PATH_OVERRIDE.set(path);
    }

    /// The `--config` file this process was started with, if any, for
    /// passing on to child `voxtype` processes.
    pub fn path_override() -> Option<PathBuf> {
        CONFIG_PATH_OVERRIDE.get().cloned()
    }

    /// Get the system-wide config file path.
    ///
    /// `VOXTYPE_TEST_SYSTEM_CONFIG` replaces it, so integration tests can
    /// put a system config in a temp dir instead of reading the real `/etc`.
    pub fn system_path() -> PathBuf {
        std::env::var_os("VOXTYPE_TEST_SYSTEM_CONFIG")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(Self::SYSTEM_PATH))
    }

    /// Resolve which config file should actually be loaded, in priority order:
    /// 1. User config (`~/.config/voxtype/config.toml`)
    /// 2. System-wide config (`/etc/voxtype/config.toml`)
    ///
    /// Returns `None` if neither exists, in which case the caller should fall
    /// back to built-in defaults. This does not consider the `--config` CLI
    /// flag; callers handle that explicitly.
    ///
    /// A user config that is a symlink counts as present even when the file
    /// it names is missing (a Stow tree that is not mounted). Loading it
    /// gives defaults with a warning, and saving creates the file it names.
    /// Falling back to the system config instead made `config set` and
    /// `history pause` write `/etc/voxtype/config.toml`.
    pub fn resolve_existing_path() -> Option<PathBuf> {
        Self::resolve_existing_in(Self::default_path(), Self::system_path())
    }

    fn resolve_existing_in(user: Option<PathBuf>, system: PathBuf) -> Option<PathBuf> {
        if let Some(user) = user {
            if std::fs::symlink_metadata(&user).is_ok() {
                return Some(user);
            }
        }
        system.exists().then_some(system)
    }

    /// Get the runtime directory for ephemeral files (state, sockets)
    pub fn runtime_dir() -> PathBuf {
        Self::runtime_dir_from(std::env::var("XDG_RUNTIME_DIR").ok())
    }

    /// Use XDG_RUNTIME_DIR if available, otherwise fall back to
    /// [`Self::runtime_dir_fallback`].
    fn runtime_dir_from(xdg_runtime_dir: Option<String>) -> PathBuf {
        xdg_runtime_dir
            .map(PathBuf::from)
            .unwrap_or_else(Self::runtime_dir_fallback)
            .join("voxtype")
    }

    /// macOS never sets XDG_RUNTIME_DIR, and a fixed /tmp/voxtype is shared
    /// by every account on the Mac: /tmp is sticky, so one user's lock files
    /// block the daemon and menubar of every other user. The per-user
    /// `$TMPDIR` (what `temp_dir()` returns there) is private to the user.
    #[cfg(target_os = "macos")]
    fn runtime_dir_fallback() -> PathBuf {
        std::env::temp_dir()
    }

    /// Linux keeps /tmp: the Quickshell OSD, the Omarchy panel and user
    /// scripts fall back to /tmp/voxtype on their own, so following
    /// `$TMPDIR` here would split the daemon from them whenever TMPDIR is
    /// set and XDG_RUNTIME_DIR is not.
    #[cfg(not(target_os = "macos"))]
    fn runtime_dir_fallback() -> PathBuf {
        PathBuf::from("/tmp")
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

    /// Voxtype's user state directory, honoring `$XDG_STATE_HOME` (default
    /// `~/.local/state`). Holds durable records voxtype writes about itself,
    /// such as declined model upgrade offers, which never belong in the
    /// user-edited config file.
    pub fn state_dir() -> PathBuf {
        Self::xdg_dir("XDG_STATE_HOME", ".local/state", None)
            .unwrap_or_else(|| Self::data_dir().join("state"))
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
            TranscriptionEngine::Gguf => self
                .gguf
                .as_ref()
                .map(|g| g.on_demand_loading)
                .unwrap_or(false),
            TranscriptionEngine::OpenVino => self
                .openvino
                .as_ref()
                .map(|o| o.on_demand_loading)
                .unwrap_or(false),
            // Soniox is a cloud backend; nothing to load on demand.
            TranscriptionEngine::Soniox | TranscriptionEngine::ElevenLabs => false,
        }
    }

    /// The one language the active engine was told to write, if its own
    /// settings name one.
    ///
    /// This is the engine's answer and nothing else: the tray's language
    /// selector and the history entry report it. Text cleanup asks
    /// [`Self::text_language`], which starts here and then considers the
    /// model and the user's declared dictation language.
    ///
    /// Returns `None` for automatic detection, for a constrained detection
    /// set, and for engines without a language setting.
    pub fn active_language(&self) -> Option<&str> {
        match self.engine_language() {
            EngineLanguage::Fixed(code) => Some(code),
            EngineLanguage::OneOf(_) | EngineLanguage::Open => None,
        }
    }

    /// What the active engine's own settings say about the language of the
    /// text it writes.
    ///
    /// Engines disagree about where language lives, and several do not take
    /// a language at all (they detect it, or their model is single-language).
    pub(crate) fn engine_language(&self) -> EngineLanguage<'_> {
        let code = match self.engine {
            TranscriptionEngine::Whisper => match &self.whisper.language {
                super::language::LanguageConfig::Single(code) => Some(code.as_str()),
                // A constrained detection set is not one language.
                super::language::LanguageConfig::Multiple(codes) => {
                    return EngineLanguage::OneOf(codes.iter().map(String::as_str).collect())
                }
            },
            TranscriptionEngine::Cohere => self.cohere.as_ref().map(|c| c.language.as_str()),
            TranscriptionEngine::Gguf => self.gguf.as_ref().map(|g| g.language.as_str()),
            TranscriptionEngine::SenseVoice => {
                self.sensevoice.as_ref().map(|s| s.language.as_str())
            }
            // The OpenVINO engine runs Whisper checkpoints and passes this
            // language to them exactly as `whisper.language` is passed.
            TranscriptionEngine::OpenVino => self.openvino.as_ref().map(|o| o.language.as_str()),
            TranscriptionEngine::ElevenLabs => self
                .elevenlabs
                .as_ref()
                .and_then(|e| e.language_code.as_deref()),
            // Hints narrow what Soniox detects; they never pin one language.
            TranscriptionEngine::Soniox => {
                let hints = self
                    .soniox
                    .as_ref()
                    .map(|s| s.language_hints.as_slice())
                    .unwrap_or_default();
                return if hints.is_empty() {
                    EngineLanguage::Open
                } else {
                    EngineLanguage::OneOf(hints.iter().map(String::as_str).collect())
                };
            }
            // Parakeet, Moonshine, Paraformer, Dolphin and Omnilingual have
            // no language setting: they detect it, or their model is fixed
            // to one. `model_language` knows which.
            TranscriptionEngine::Parakeet
            | TranscriptionEngine::Moonshine
            | TranscriptionEngine::Paraformer
            | TranscriptionEngine::Dolphin
            | TranscriptionEngine::Omnilingual => None,
        };

        match code {
            None | Some("" | "auto") => EngineLanguage::Open,
            Some(code) => EngineLanguage::Fixed(code),
        }
    }

    /// The config key [`Self::engine_language`] reads for the active engine,
    /// for messages that tell the user what to change. Engines without a
    /// language setting name `text.language`, the only one that applies.
    pub(crate) fn engine_language_key(&self) -> &'static str {
        match self.engine {
            TranscriptionEngine::Whisper => "whisper.language",
            TranscriptionEngine::Cohere => "cohere.language",
            TranscriptionEngine::Gguf => "gguf.language",
            TranscriptionEngine::SenseVoice => "sensevoice.language",
            TranscriptionEngine::OpenVino => "openvino.language",
            TranscriptionEngine::Soniox => "soniox.language_hints",
            TranscriptionEngine::ElevenLabs => "elevenlabs.language_code",
            TranscriptionEngine::Parakeet
            | TranscriptionEngine::Moonshine
            | TranscriptionEngine::Paraformer
            | TranscriptionEngine::Dolphin
            | TranscriptionEngine::Omnilingual => "text.language",
        }
    }

    /// The one language the selected model can write, when the model
    /// registry knows the model and says it has only one: an English-only
    /// checkpoint is `Some("en")`. `None` for a multilingual model, a custom
    /// model path, and a remote Whisper server, whose model is not ours to
    /// look up.
    pub fn model_language(&self) -> Option<&'static str> {
        if self.engine == TranscriptionEngine::Whisper
            && self.whisper.effective_mode() == super::WhisperMode::Remote
        {
            return None;
        }
        crate::setup::model::model_only_language(self.engine, self.model_name())
    }

    /// The language text cleanup should treat finished dictation as, and
    /// where that answer came from. `None` means unknown: language-specific
    /// stages keep their language-neutral behaviour and English-only ones
    /// (LARD, #696) do not run.
    ///
    /// In order:
    ///
    /// 1. A language the engine was told to write wins outright. Whatever
    ///    else is configured, cleanup for one language never runs on text
    ///    the engine was told to produce in another.
    /// 2. A model that can only write one language settles it next: an
    ///    English-only model counts as English with nothing set anywhere.
    /// 3. Otherwise the language the user declared in `text.language`.
    ///
    /// When the engine's detection is limited to a set (a Whisper language
    /// array, Soniox hints), 2 and 3 only count if the set includes them: a
    /// declared language the engine was told not to detect is a
    /// contradiction, and the answer is unknown rather than a guess.
    pub fn text_language_source(&self) -> Option<(&str, TextLanguageSource)> {
        let allowed = match self.engine_language() {
            EngineLanguage::Fixed(code) => return Some((code, TextLanguageSource::Engine)),
            EngineLanguage::OneOf(codes) => Some(codes),
            EngineLanguage::Open => None,
        };
        let (code, source) = match self.model_language() {
            Some(code) => (code, TextLanguageSource::Model),
            None => (self.text.declared_language()?, TextLanguageSource::Declared),
        };
        let base = |code: &str| code.split(['-', '_']).next().map(str::to_ascii_lowercase);
        match allowed {
            Some(codes) if !codes.iter().any(|allowed| base(allowed) == base(code)) => None,
            _ => Some((code, source)),
        }
    }

    /// Why a declared `text.language` is not what text cleanup uses, as one
    /// sentence, or `None` when nothing is declared or the declaration is in
    /// effect (or merely repeats what the engine or model already says).
    ///
    /// `voxtype config set` and the settings panel show this, so a
    /// declaration that the engine's language or the model overrides is
    /// never left looking as if it applied.
    pub fn text_language_conflict(&self) -> Option<String> {
        let declared = self.text.declared_language()?;
        let base = |code: &str| code.split(['-', '_']).next().map(str::to_ascii_lowercase);
        let key = self.engine_language_key();
        let (used, because) = match self.text_language_source() {
            Some((_, TextLanguageSource::Declared)) => return None,
            Some((code, _)) if base(code) == base(declared) => return None,
            Some((code, TextLanguageSource::Engine)) => (
                Some(code),
                format!("{key} is {code}, and the engine's language wins"),
            ),
            Some((code, _)) => (
                Some(code),
                format!("{} only writes {code}", self.model_name()),
            ),
            None => (
                None,
                format!("{key} limits detection to languages that do not include it"),
            ),
        };
        Some(match used {
            Some(code) => format!(
                "text.language is {declared}, but text cleanup treats dictation as {code}: {because}"
            ),
            None => format!(
                "text.language is {declared}, but text cleanup does not use it: {because}"
            ),
        })
    }

    /// [`Self::text_language_source`] without the source: what to hand the
    /// text processor.
    pub fn text_language(&self) -> Option<&str> {
        self.text_language_source().map(|(code, _)| code)
    }

    /// Whether the active backend configuration emits plain text that needs
    /// punctuation restoration and true-casing. This is an allow-list so a
    /// new engine keeps its native output until its behavior is tested.
    pub fn needs_punctuation_casing(&self) -> bool {
        match self.engine {
            TranscriptionEngine::Parakeet => self
                .parakeet
                .as_ref()
                .is_some_and(|config| config.model_type == Some(ParakeetModelType::Ctc)),
            // SenseVoice's built-in ITN supplies punctuation. Preserve it
            // when enabled instead of lowercasing and reclassifying it.
            TranscriptionEngine::SenseVoice => self
                .sensevoice
                .as_ref()
                .is_some_and(|config| !config.use_itn),
            TranscriptionEngine::Paraformer
            | TranscriptionEngine::Dolphin
            | TranscriptionEngine::Omnilingual => true,
            _ => false,
        }
    }

    /// Whether the active engine will run its streaming pipeline.
    ///
    /// Streaming types text at the cursor as it is decoded, which means the
    /// end-of-utterance hooks that reshape a finished transcript never get a
    /// finished transcript to work on. Callers use this to tell the user which
    /// of their settings will not apply (#581) rather than letting them
    /// silently do nothing.
    pub fn streaming_enabled(&self) -> bool {
        match self.engine {
            TranscriptionEngine::Parakeet => self.parakeet.as_ref().is_some_and(|p| p.streaming),
            TranscriptionEngine::Soniox => self
                .soniox
                .as_ref()
                .is_some_and(|s| s.streaming && !s.async_api),
            _ => false,
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
            TranscriptionEngine::Gguf => self
                .gguf
                .as_ref()
                .map(|g| g.model.as_str())
                .unwrap_or("gguf (not configured)"),
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
            TranscriptionEngine::ElevenLabs => self
                .elevenlabs
                .as_ref()
                .map_or(super::engines::DEFAULT_ELEVENLABS_MODEL, |e| {
                    e.model.as_str()
                }),
        }
    }

    /// The active engine's GGUF settings when it runs a `.gguf` model. voxtype
    /// hands those to `voxtype-gguf-worker` instead of loading them itself,
    /// so they run on `<section>.gguf_backend` whatever the main binary's
    /// variant is.
    pub fn active_gguf(&self) -> Option<ActiveGguf<'_>> {
        use crate::model_selection::is_gguf;
        let (section, model, backend) = match self.engine {
            TranscriptionEngine::Gguf => self
                .gguf
                .as_ref()
                .map(|g| ("gguf", g.model.as_str(), g.gguf_backend.as_str()))?,
            TranscriptionEngine::Parakeet => {
                self.parakeet
                    .as_ref()
                    .filter(|p| is_gguf(&p.model))
                    .map(|p| ("parakeet", p.model.as_str(), p.gguf_backend.as_str()))?
            }
            TranscriptionEngine::Cohere => self
                .cohere
                .as_ref()
                .filter(|c| is_gguf(&c.model))
                .map(|c| ("cohere", c.model.as_str(), c.gguf_backend.as_str()))?,
            _ => return None,
        };
        Some(ActiveGguf {
            section,
            model,
            backend,
        })
    }

    /// Make `engine` active with `model` as its model, materializing the
    /// engine's section from defaults when the config doesn't have one.
    pub fn set_model_for(&mut self, engine: TranscriptionEngine, model: &str) {
        let model = model.to_string();
        self.engine = engine;
        match engine {
            TranscriptionEngine::Whisper => self.whisper.model = model,
            TranscriptionEngine::Parakeet => {
                let parakeet = self.parakeet.get_or_insert_with(Default::default);
                if parakeet.model != model {
                    // A TDT/CTC hint belongs to the old model; let the new
                    // one be auto-detected.
                    parakeet.model_type = None;
                }
                parakeet.model = model;
            }
            TranscriptionEngine::Moonshine => {
                self.moonshine.get_or_insert_with(Default::default).model = model
            }
            TranscriptionEngine::SenseVoice => {
                self.sensevoice.get_or_insert_with(Default::default).model = model
            }
            TranscriptionEngine::Paraformer => {
                self.paraformer.get_or_insert_with(Default::default).model = model
            }
            TranscriptionEngine::Dolphin => {
                self.dolphin.get_or_insert_with(Default::default).model = model
            }
            TranscriptionEngine::Omnilingual => {
                self.omnilingual.get_or_insert_with(Default::default).model = model
            }
            TranscriptionEngine::Cohere => {
                self.cohere.get_or_insert_with(Default::default).model = model
            }
            TranscriptionEngine::Gguf => {
                let gguf = self.gguf.get_or_insert_with(Default::default);
                if gguf.model != model || gguf.architecture.is_empty() {
                    if let Some(entry) = crate::setup::model::find_pending_gguf_model(&model)
                        .filter(|entry| entry.runnable)
                    {
                        gguf.architecture = entry.arch.to_string();
                        gguf.language = entry.languages.first().copied().unwrap_or("auto").into();
                    }
                }
                gguf.model = model;
            }
            TranscriptionEngine::OpenVino => {
                self.openvino.get_or_insert_with(Default::default).model = model
            }
            TranscriptionEngine::Soniox => {
                self.soniox.get_or_insert_with(Default::default).model = model
            }
            TranscriptionEngine::ElevenLabs => {
                self.elevenlabs.get_or_insert_with(Default::default).model = model
            }
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
    fn omarchy_panel_linking_defaults_on_and_can_be_turned_off() {
        assert!(Config::default().omarchy.link_settings_plugin);
        let old: Config = toml::from_str("[tray]\nenabled = true\n").unwrap();
        assert!(
            old.omarchy.link_settings_plugin,
            "configs without [omarchy] keep it on"
        );
        let off: Config = toml::from_str("[omarchy]\nlink_settings_plugin = false\n").unwrap();
        assert!(!off.omarchy.link_settings_plugin);
    }

    #[test]
    fn punctuation_casing_tracks_exact_engine_output_capability() {
        let mut config = Config::default();
        assert!(!config.needs_punctuation_casing());

        config.engine = TranscriptionEngine::Paraformer;
        assert!(config.needs_punctuation_casing());
        config.engine = TranscriptionEngine::Dolphin;
        assert!(config.needs_punctuation_casing());
        config.engine = TranscriptionEngine::Omnilingual;
        assert!(config.needs_punctuation_casing());

        config.engine = TranscriptionEngine::SenseVoice;
        config.sensevoice = Some(SenseVoiceConfig::default());
        assert!(!config.needs_punctuation_casing());
        config.sensevoice.as_mut().unwrap().use_itn = false;
        assert!(config.needs_punctuation_casing());

        config.engine = TranscriptionEngine::Parakeet;
        config.parakeet = Some(ParakeetConfig::default());
        assert!(!config.needs_punctuation_casing());
        config.parakeet.as_mut().unwrap().model_type = Some(ParakeetModelType::Ctc);
        assert!(config.needs_punctuation_casing());

        for engine in [
            TranscriptionEngine::Moonshine,
            TranscriptionEngine::Cohere,
            TranscriptionEngine::OpenVino,
            TranscriptionEngine::Soniox,
        ] {
            config.engine = engine;
            assert!(!config.needs_punctuation_casing(), "{engine}");
        }
    }

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
        let meeting_cfg = cfg
            .with_meeting_mode_overrides(&super::super::MeetingStartOptions::default())
            .unwrap();
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
        let meeting_cfg = cfg
            .with_meeting_mode_overrides(&super::super::MeetingStartOptions::default())
            .unwrap();
        assert!(meeting_cfg.soniox.as_ref().unwrap().async_api);
    }

    #[test]
    fn meeting_mode_is_noop_for_non_soniox_engines() {
        let cfg = Config::default(); // engine = Whisper
        let meeting_cfg = cfg
            .with_meeting_mode_overrides(&super::super::MeetingStartOptions::default())
            .unwrap();
        assert_eq!(meeting_cfg.engine, cfg.engine);
        assert_eq!(meeting_cfg.whisper.model, cfg.whisper.model);
    }

    /// #581: the daemon has to know whether the streaming path is active so
    /// it can tell the user which of their `[text]` settings will be ignored.
    /// Getting this predicate wrong is worse than not warning at all: a false
    /// positive tells someone their working config is broken.
    #[test]
    fn streaming_enabled_tracks_the_engine_that_will_actually_run() {
        // Whisper has no streaming pipeline.
        assert!(!Config::default().streaming_enabled());

        // Soniox streams by default, but the async API is batch: it uploads a
        // finished recording and polls, so the completed-transcript hooks do
        // run and must not be reported as ignored.
        let soniox = |async_api: bool| Config {
            engine: TranscriptionEngine::Soniox,
            soniox: Some(SonioxConfig {
                api_key: Some("k".into()),
                streaming: true,
                async_api,
                ..SonioxConfig::default()
            }),
            ..Config::default()
        };
        assert!(soniox(false).streaming_enabled());
        assert!(!soniox(true).streaming_enabled());

        // An engine selected without its config section cannot stream.
        let orphaned = Config {
            engine: TranscriptionEngine::Soniox,
            soniox: None,
            ..Config::default()
        };
        assert!(!orphaned.streaming_enabled());
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
        assert!(config.elevenlabs.is_none());
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
    fn a_dangling_user_config_link_is_used_instead_of_the_system_config() {
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("etc/config.toml");
        std::fs::create_dir_all(system.parent().unwrap()).unwrap();
        std::fs::write(&system, "[hotkey]\nkey = \"F12\"\n").unwrap();
        let user = dir.path().join("config.toml");

        // No user config at all: the system config is the fallback.
        assert_eq!(
            Config::resolve_existing_in(Some(user.clone()), system.clone()),
            Some(system.clone())
        );

        // A link to a file that is missing is still the user's config.
        std::os::unix::fs::symlink(dir.path().join("unmounted/config.toml"), &user).unwrap();
        assert_eq!(
            Config::resolve_existing_in(Some(user.clone()), system.clone()),
            Some(user)
        );
    }

    #[test]
    fn runtime_dir_prefers_xdg_and_falls_back_to_per_user_temp_dir() {
        assert_eq!(
            Config::runtime_dir_from(Some("/run/user/1000".to_string())),
            PathBuf::from("/run/user/1000/voxtype")
        );
        // On macOS, not a hardcoded /tmp/voxtype: that is shared by every
        // account on the Mac.
        #[cfg(target_os = "macos")]
        assert_eq!(
            Config::runtime_dir_from(None),
            std::env::temp_dir().join("voxtype")
        );
        // On Linux, unchanged: the QML frontends fall back to /tmp too.
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            Config::runtime_dir_from(None),
            PathBuf::from("/tmp/voxtype")
        );
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

    #[test]
    fn set_model_for_switches_engine_and_fills_missing_sections() {
        let mut config = Config::default();
        assert!(config.moonshine.is_none());
        config.set_model_for(TranscriptionEngine::Moonshine, "tiny");
        assert_eq!(config.engine, TranscriptionEngine::Moonshine);
        assert_eq!(config.model_name(), "tiny");

        config.parakeet = Some(crate::config::ParakeetConfig {
            model: "parakeet-tdt-0.6b-v2".to_string(),
            model_type: Some(crate::config::ParakeetModelType::Tdt),
            ..Default::default()
        });
        config.set_model_for(TranscriptionEngine::Parakeet, "parakeet-tdt-0.6b-v2");
        assert!(config.parakeet.as_ref().unwrap().model_type.is_some());
        // A different model's architecture is auto-detected, not inherited.
        config.set_model_for(TranscriptionEngine::Parakeet, "parakeet-tdt-0.6b-v3");
        assert!(config.parakeet.as_ref().unwrap().model_type.is_none());
    }

    // --- The language of the text, for cleanup (#696) ----------------------

    /// What the English-only stages ask of a language tag.
    fn is_english(language: Option<&str>) -> bool {
        language.and_then(|l| l.split(['-', '_']).next()) == Some("en")
    }

    const ENGINES: [TranscriptionEngine; 11] = [
        TranscriptionEngine::Whisper,
        TranscriptionEngine::Parakeet,
        TranscriptionEngine::Moonshine,
        TranscriptionEngine::SenseVoice,
        TranscriptionEngine::Paraformer,
        TranscriptionEngine::Dolphin,
        TranscriptionEngine::Omnilingual,
        TranscriptionEngine::Cohere,
        TranscriptionEngine::Gguf,
        TranscriptionEngine::OpenVino,
        TranscriptionEngine::Soniox,
    ];

    /// `engine` active with `model`, every other setting at its default.
    fn config_for(engine: TranscriptionEngine, model: &str) -> Config {
        let mut config = Config::default();
        config.set_model_for(engine, model);
        config
    }

    /// `config_for` with the engine's own language key set, for the engines
    /// that have one.
    fn with_engine_language(mut config: Config, language: &str) -> Config {
        match config.engine {
            TranscriptionEngine::Whisper => {
                config.whisper.language = crate::config::LanguageConfig::Single(language.into())
            }
            TranscriptionEngine::Cohere => {
                config.cohere.as_mut().unwrap().language = language.into()
            }
            TranscriptionEngine::Gguf => config.gguf.as_mut().unwrap().language = language.into(),
            TranscriptionEngine::SenseVoice => {
                config.sensevoice.as_mut().unwrap().language = language.into()
            }
            TranscriptionEngine::OpenVino => {
                config.openvino.as_mut().unwrap().language = language.into()
            }
            other => panic!("{} has no language key", other.name()),
        }
        config
    }

    fn declaring(mut config: Config, language: &str) -> Config {
        config.text.language = language.into();
        config
    }

    /// Every model the registry lists for `engine`, plus what it cannot
    /// know: a path and a name nobody registered.
    fn models_of(engine: TranscriptionEngine) -> Vec<String> {
        let mut models: Vec<String> = crate::model_catalog::model_catalog(engine.name())
            .into_iter()
            .map(str::to_string)
            .collect();
        models.push("/srv/models/custom".to_string());
        models.push("somebody-elses-model".to_string());
        if engine == TranscriptionEngine::Soniox {
            models.push("stt-rt-v4".to_string());
        }
        models
    }

    /// `active_language` as it was before text cleanup had its own
    /// resolution: the one answer every caller used, kept here so the tests
    /// below can show exactly which configurations now answer differently.
    fn language_before(config: &Config) -> Option<&str> {
        let code = match config.engine {
            TranscriptionEngine::Whisper => match &config.whisper.language {
                crate::config::LanguageConfig::Single(code) => code.as_str(),
                crate::config::LanguageConfig::Multiple(_) => return None,
            },
            TranscriptionEngine::Cohere => config.cohere.as_ref().map(|c| c.language.as_str())?,
            TranscriptionEngine::Gguf => config.gguf.as_ref().map(|g| g.language.as_str())?,
            TranscriptionEngine::SenseVoice => {
                config.sensevoice.as_ref().map(|s| s.language.as_str())?
            }
            _ => return None,
        };
        match code {
            "" | "auto" => None,
            other => Some(other),
        }
    }

    /// Engine language settings worth trying for `engine`: its default, and
    /// the ones that mean something different.
    fn engine_settings(engine: TranscriptionEngine, model: &str) -> Vec<Config> {
        let base = config_for(engine, model);
        let mut configs = vec![base.clone()];
        let codes: &[&str] = match engine {
            TranscriptionEngine::Whisper => &["auto", "", "en", "en-US", "fr", "pt", "de"],
            TranscriptionEngine::Cohere => &["en", "fr", "de"],
            TranscriptionEngine::Gguf => &["auto", "en", "en-US", "ja", "ru"],
            TranscriptionEngine::SenseVoice => &["auto", "en", "zh", "yue"],
            TranscriptionEngine::OpenVino => &["auto", "en", "fr", "pt"],
            _ => &[],
        };
        configs.extend(
            codes
                .iter()
                .map(|code| with_engine_language(base.clone(), code)),
        );
        if engine == TranscriptionEngine::Whisper {
            for set in [vec!["en", "fr"], vec!["fr", "de"], vec!["en"]] {
                let mut config = base.clone();
                config.whisper.language = crate::config::LanguageConfig::Multiple(
                    set.into_iter().map(str::to_string).collect(),
                );
                configs.push(config.clone());
                config.whisper.mode = Some(crate::config::WhisperMode::Remote);
                configs.push(config);
            }
            let mut remote = with_engine_language(base.clone(), "auto");
            remote.whisper.mode = Some(crate::config::WhisperMode::Remote);
            configs.push(remote);
        }
        if engine == TranscriptionEngine::Soniox {
            for hints in [vec![], vec!["en"], vec!["hu"], vec!["en", "de"]] {
                let mut config = base.clone();
                config.soniox.as_mut().unwrap().language_hints =
                    hints.into_iter().map(str::to_string).collect();
                configs.push(config);
            }
        }
        configs
    }

    /// The models that count as English with nothing set anywhere, engine by
    /// engine. This list is the whole of change (1): adding a model to it is
    /// a decision, so it is spelled out rather than derived.
    const ENGLISH_ONLY: &[(TranscriptionEngine, &[&str])] = &[
        (
            TranscriptionEngine::Whisper,
            &["tiny.en", "base.en", "small.en", "medium.en"],
        ),
        (
            TranscriptionEngine::Parakeet,
            &[
                "parakeet-tdt-0.6b-v2",
                "parakeet-tdt-0.6b-v2-int8",
                "parakeet-unified-en-0.6b",
            ],
        ),
        (TranscriptionEngine::Moonshine, &["base", "tiny"]),
        (TranscriptionEngine::Paraformer, &["paraformer-en"]),
        (
            TranscriptionEngine::Gguf,
            &["moonshine-streaming-small-Q8_0.gguf"],
        ),
        (
            TranscriptionEngine::OpenVino,
            &[
                "tiny.en-int4",
                "tiny.en-int8",
                "tiny.en-fp16",
                "base.en-int4",
                "base.en-int8",
                "base.en-fp16",
                "small.en-int4",
                "small.en-int8",
                "small.en-fp16",
                "medium.en-int4",
                "medium.en-int8",
                "medium.en-fp16",
            ],
        ),
    ];

    fn english_only(engine: TranscriptionEngine, model: &str) -> bool {
        ENGLISH_ONLY
            .iter()
            .any(|(e, models)| *e == engine && models.contains(&model))
    }

    /// The engine's language key on automatic detection, where it has one.
    fn on_automatic(config: Config) -> Config {
        match config.engine {
            TranscriptionEngine::Whisper
            | TranscriptionEngine::Gguf
            | TranscriptionEngine::SenseVoice
            | TranscriptionEngine::OpenVino => with_engine_language(config, "auto"),
            // Cohere has no automatic setting; the rest have no key.
            _ => config,
        }
    }

    #[test]
    fn an_english_only_model_counts_as_english_with_nothing_set() {
        for engine in ENGINES {
            for model in models_of(engine) {
                let config = on_automatic(config_for(engine, &model));
                let expected = english_only(engine, &model);
                assert_eq!(
                    config.model_language() == Some("en"),
                    expected,
                    "{} / {model}",
                    engine.name()
                );
                if engine == TranscriptionEngine::Cohere {
                    // Always has a language of its own (default en).
                    assert_eq!(config.text_language(), Some("en"));
                    continue;
                }
                assert_eq!(
                    is_english(config.text_language()),
                    expected,
                    "{} / {model}",
                    engine.name()
                );
                if expected {
                    assert_eq!(
                        config.text_language_source(),
                        Some(("en", TextLanguageSource::Model))
                    );
                }
            }
        }
        // Every name in the list is a real catalog entry, so a rename in the
        // registry cannot quietly empty it.
        for (engine, models) in ENGLISH_ONLY {
            let catalog = crate::model_catalog::model_catalog(engine.name());
            for model in *models {
                assert!(catalog.contains(model), "{} / {model}", engine.name());
            }
        }
    }

    /// The other names a registry model answers to, and the models that only
    /// write some other language.
    #[test]
    fn model_language_follows_the_registry_not_the_spelling() {
        let language = |engine, model: &str| config_for(engine, model).model_language();
        use TranscriptionEngine::*;

        // Directory names and short names of the same checkpoint agree.
        assert_eq!(language(Moonshine, "moonshine-base"), Some("en"));
        assert_eq!(language(Moonshine, "moonshine-tiny"), Some("en"));
        assert_eq!(language(Paraformer, "en"), Some("en"));
        assert_eq!(language(OpenVino, "base.en"), Some("en"));
        assert_eq!(
            language(OpenVino, "openvino-whisper-base.en-int8-ov"),
            Some("en")
        );

        // One language, and it is not English.
        assert_eq!(language(Moonshine, "base-ja"), Some("ja"));
        assert_eq!(language(Moonshine, "base-zh"), Some("zh"));
        assert_eq!(language(Moonshine, "tiny-ko"), Some("ko"));
        assert_eq!(language(Moonshine, "tiny-ar"), Some("ar"));
        assert_eq!(language(Gguf, "moonshine-base-ja-Q8_0.gguf"), Some("ja"));
        assert_eq!(language(Gguf, "gigaam-v3-e2e-rnnt-Q8_0.gguf"), Some("ru"));

        // Multilingual, whatever the name suggests.
        for (engine, model) in [
            (Whisper, "large-v3-turbo"),
            (Parakeet, "parakeet-tdt-0.6b-v3"),
            (Parakeet, "parakeet-tdt-0.6b-v3-Q8_0.gguf"),
            (Parakeet, "parakeet-ultra-Q8_0.gguf"),
            (Paraformer, "paraformer-zh"),
            (SenseVoice, "sensevoice-small"),
            (Dolphin, "dolphin-base"),
            (Omnilingual, "omnilingual-300m"),
            (Gguf, "Qwen3-ASR-0.6B-Q8_0.gguf"),
            (Gguf, "nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf"),
            (Gguf, "SenseVoiceSmall-Q8_0.gguf"),
            (OpenVino, "large-v3-int8"),
            (OpenVino, "base-int8"),
            (Cohere, "cohere-transcribe-03-2026-Q4_K_M.gguf"),
            (Soniox, "stt-rt-v4"),
        ] {
            assert_eq!(language(engine, model), None, "{model}");
        }

        // Nothing is read out of a name the registry does not hold: a path
        // to a file called like an English model, an English-looking name
        // nobody registered, a distil export.
        for (engine, model) in [
            (Whisper, "/models/ggml-base.en.bin"),
            (Whisper, "ggml-base.en.bin"),
            (Whisper, "distil-small.en"),
            (Parakeet, "/models/parakeet-tdt-0.6b-v2"),
            (Parakeet, "parakeet-tdt-0.6b-v2-custom"),
            (Parakeet, "my-parakeet-en-model"),
            (Moonshine, "/models/moonshine-base"),
            (Paraformer, "/models/paraformer-en"),
            (Gguf, "/models/moonshine-streaming-small-Q8_0.gguf"),
            (OpenVino, "/models/openvino-whisper-base.en-int8-ov"),
            (OpenVino, "distil-large-v3-int8"),
        ] {
            assert_eq!(language(engine, model), None, "{model}");
        }

        // A remote Whisper server runs its own model; ours says nothing.
        let mut remote = config_for(Whisper, "base.en");
        remote.whisper.mode = Some(crate::config::WhisperMode::Remote);
        assert_eq!(remote.model_language(), None);
        let mut cli = config_for(Whisper, "base.en");
        cli.whisper.mode = Some(crate::config::WhisperMode::Cli);
        assert_eq!(cli.model_language(), Some("en"));
    }

    /// With nothing declared, the language text cleanup sees is what it saw
    /// before, for every engine, model and engine language setting, with two
    /// exceptions and no others: a single-language model on automatic
    /// detection now has its language, and the OpenVINO engine's language
    /// key now counts as Whisper's always did.
    #[test]
    fn nothing_declared_changes_only_single_language_models_and_openvino() {
        let mut checked = 0;
        for engine in ENGINES {
            for model in models_of(engine) {
                for config in engine_settings(engine, &model) {
                    assert_eq!(config.text.language, "auto");
                    let before = language_before(&config);
                    let now = config.text_language();
                    let context = format!("{} / {model} / {config:?}", engine.name());
                    checked += 1;

                    if engine == TranscriptionEngine::OpenVino {
                        let own = config.openvino.as_ref().unwrap().language.as_str();
                        let expected = match own {
                            "auto" => config.model_language(),
                            code => Some(code),
                        };
                        assert_eq!(now, expected, "{context}");
                        assert_eq!(before, None, "{context}");
                        continue;
                    }
                    match before {
                        // An engine language that was set is still the answer.
                        Some(code) => assert_eq!(now, Some(code), "{context}"),
                        None => {
                            let allowed = match &config.whisper.language {
                                crate::config::LanguageConfig::Multiple(set)
                                    if engine == TranscriptionEngine::Whisper =>
                                {
                                    config
                                        .model_language()
                                        .filter(|code| set.iter().any(|allowed| allowed == code))
                                }
                                _ => config.model_language(),
                            };
                            assert_eq!(now, allowed, "{context}");
                        }
                    }
                    // The engine's own answer is untouched everywhere.
                    assert_eq!(config.active_language(), before, "{context}");
                }
            }
        }
        assert!(checked > 300, "{checked} configurations");
    }

    /// The multilingual defaults, spelled out: automatic detection and
    /// nothing declared is still "unknown", so LARD stays off.
    #[test]
    fn a_multilingual_model_on_automatic_is_still_unknown() {
        use TranscriptionEngine::*;
        for (engine, model) in [
            (Whisper, "large-v3-turbo"),
            (Parakeet, "parakeet-tdt-0.6b-v3"),
            (Parakeet, "parakeet-ultra-Q8_0.gguf"),
            (SenseVoice, "sensevoice-small"),
            (Paraformer, "paraformer-zh"),
            (Dolphin, "dolphin-base"),
            (Omnilingual, "omnilingual-300m"),
            (Gguf, "Qwen3-ASR-0.6B-Q8_0.gguf"),
            (OpenVino, "large-v3-int8"),
            (Soniox, "stt-rt-v4"),
        ] {
            let config = on_automatic(config_for(engine, model));
            assert_eq!(config.text_language(), None, "{model}");
            assert_eq!(config.text_language_conflict(), None);
        }
    }

    #[test]
    fn a_declared_language_fills_in_for_automatic_detection() {
        for engine in ENGINES {
            if engine == TranscriptionEngine::Cohere {
                continue; // no automatic setting to fill in for
            }
            for model in models_of(engine) {
                let config = on_automatic(config_for(engine, &model));
                if config.model_language().is_some() {
                    continue;
                }
                for declared in ["en", "en-US", "en_GB", "pt-BR", "fr"] {
                    let config = declaring(config.clone(), declared);
                    let context = format!("{} / {model} / {declared}", engine.name());
                    // Soniox's default hints are hu and en.
                    let hinted_out =
                        engine == TranscriptionEngine::Soniox && !declared.starts_with("en");
                    if hinted_out {
                        assert_eq!(config.text_language(), None, "{context}");
                        assert!(config.text_language_conflict().is_some(), "{context}");
                        continue;
                    }
                    assert_eq!(
                        config.text_language_source(),
                        Some((declared, TextLanguageSource::Declared)),
                        "{context}"
                    );
                    assert_eq!(config.text_language_conflict(), None, "{context}");
                    // The declaration is about text: the engine still hears
                    // nothing about it.
                    assert_eq!(config.active_language(), None, "{context}");
                }
                for nothing in ["auto", "", "english", "e", "en us"] {
                    let config = declaring(config.clone(), nothing);
                    assert_eq!(config.text_language(), None, "{model} / {nothing:?}");
                }
            }
        }
    }

    /// Precedence, case by case: the engine's language, then a
    /// single-language model, then the declaration.
    #[test]
    fn engine_language_wins_then_the_model_then_the_declaration() {
        use TextLanguageSource::{Declared, Engine, Model};
        use TranscriptionEngine::*;
        let resolve = |config: &Config| {
            config
                .text_language_source()
                .map(|(code, source)| (code.to_string(), source))
        };
        let is = |code: &str, source| Some((code.to_string(), source));

        // 1. A set engine language beats a declaration that disagrees.
        for (engine, model, engine_language) in [
            (Whisper, "large-v3-turbo", "fr"),
            (Cohere, "cohere-transcribe-03-2026-Q4_K_M.gguf", "de"),
            (Gguf, "Qwen3-ASR-0.6B-Q8_0.gguf", "ja"),
            (SenseVoice, "sensevoice-small", "zh"),
            (OpenVino, "large-v3-int8", "pt"),
        ] {
            let config = declaring(
                with_engine_language(config_for(engine, model), engine_language),
                "en",
            );
            assert_eq!(resolve(&config), is(engine_language, Engine), "{model}");
            let conflict = config.text_language_conflict().expect("a contradiction");
            assert!(
                conflict.contains(&format!("treats dictation as {engine_language}")),
                "{conflict}"
            );
            assert!(
                conflict.contains(config.engine_language_key()),
                "{conflict}"
            );
        }
        // ...and one that agrees is nothing to report.
        let agreed = declaring(
            with_engine_language(config_for(Whisper, "large-v3-turbo"), "en"),
            "en-US",
        );
        assert_eq!(resolve(&agreed), is("en", Engine));
        assert_eq!(agreed.text_language_conflict(), None);

        // 2. A set engine language beats the model too: `.en` with French
        //    set is treated as before, as French.
        let config = with_engine_language(config_for(Whisper, "base.en"), "fr");
        assert_eq!(resolve(&config), is("fr", Engine));

        // 3. A single-language model beats the declaration.
        let config = declaring(config_for(Parakeet, "parakeet-tdt-0.6b-v2"), "fr");
        assert_eq!(resolve(&config), is("en", Model));
        assert!(config
            .text_language_conflict()
            .unwrap()
            .contains("parakeet-tdt-0.6b-v2 only writes en"));
        let config = declaring(config_for(Moonshine, "base-ja"), "en");
        assert_eq!(resolve(&config), is("ja", Model));
        let config = declaring(
            with_engine_language(config_for(Gguf, "gigaam-v3-e2e-rnnt-Q8_0.gguf"), "auto"),
            "en",
        );
        assert_eq!(resolve(&config), is("ru", Model));

        // 4. Otherwise the declaration.
        let config = declaring(config_for(Parakeet, "parakeet-tdt-0.6b-v3"), "en");
        assert_eq!(resolve(&config), is("en", Declared));

        // 5. A detection set that leaves the candidate out makes it unknown,
        //    whether the candidate is declared or the model's.
        let whisper_set = |model: &str, set: &[&str]| {
            let mut config = config_for(Whisper, model);
            config.whisper.language = crate::config::LanguageConfig::Multiple(
                set.iter().map(|code| code.to_string()).collect(),
            );
            config
        };
        assert_eq!(
            resolve(&declaring(whisper_set("large-v3", &["en", "fr"]), "en")),
            is("en", Declared)
        );
        assert_eq!(
            resolve(&declaring(whisper_set("large-v3", &["en", "fr"]), "en-GB")),
            is("en-GB", Declared)
        );
        let excluded = declaring(whisper_set("large-v3", &["fr", "de"]), "en");
        assert_eq!(resolve(&excluded), None);
        assert!(excluded
            .text_language_conflict()
            .unwrap()
            .contains("whisper.language limits detection"));
        assert_eq!(
            resolve(&whisper_set("base.en", &["en", "fr"])),
            is("en", Model)
        );
        assert_eq!(resolve(&whisper_set("base.en", &["fr", "de"])), None);

        let soniox = |hints: &[&str]| {
            let mut config = config_for(Soniox, "stt-rt-v4");
            config.soniox.as_mut().unwrap().language_hints =
                hints.iter().map(|code| code.to_string()).collect();
            declaring(config, "en")
        };
        assert_eq!(resolve(&soniox(&[])), is("en", Declared));
        assert_eq!(resolve(&soniox(&["hu", "en"])), is("en", Declared));
        assert_eq!(resolve(&soniox(&["hu"])), None);
    }

    /// The guarantee behind the precedence: whatever is declared and
    /// whichever model is selected, text the engine was told to write in
    /// another language never counts as English.
    #[test]
    fn a_non_english_engine_language_is_never_english() {
        for engine in ENGINES {
            for model in models_of(engine) {
                for config in engine_settings(engine, &model) {
                    let told_other = match config.engine_language() {
                        EngineLanguage::Fixed(code) => !is_english(Some(code)),
                        EngineLanguage::OneOf(codes) => {
                            !codes.iter().any(|code| is_english(Some(code)))
                        }
                        EngineLanguage::Open => false,
                    };
                    if !told_other {
                        continue;
                    }
                    for declared in ["auto", "en", "en-US", "EN", "english"] {
                        let config = declaring(config.clone(), declared);
                        assert!(
                            !is_english(config.text_language()),
                            "{} / {model} / {declared} / {:?}",
                            engine.name(),
                            config.text_language()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn elevenlabs_has_no_on_demand_model_loading() {
        let config = Config {
            engine: TranscriptionEngine::ElevenLabs,
            elevenlabs: Some(ElevenLabsConfig::default()),
            ..Config::default()
        };
        assert!(!config.on_demand_loading());
    }

    #[test]
    fn elevenlabs_names_its_configured_model() {
        let mut config = Config {
            engine: TranscriptionEngine::ElevenLabs,
            ..Config::default()
        };
        assert_eq!(config.model_name(), "scribe_v3");
        config.set_model_for(TranscriptionEngine::ElevenLabs, "scribe_v9_future");
        assert_eq!(config.model_name(), "scribe_v9_future");
        assert_eq!(
            config.elevenlabs.as_ref().map(|e| e.model.as_str()),
            Some("scribe_v9_future")
        );
    }
}
