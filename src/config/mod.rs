//! Configuration loading and types for voxtype
//!
//! Configuration is loaded in layers:
//! 1. Built-in defaults
//! 2. Config file (~/.config/voxtype/config.toml)
//! 3. Environment variables (VOXTYPE_*)
//! 4. CLI arguments (highest priority)

mod audio;
mod default_config;
mod engines;
mod history;
mod hotkey;
mod language;
mod load;
mod meeting;
mod meeting_detect;
mod notification;
mod output;
mod parse;
mod profile;
mod root;
pub mod schema;
mod status;
mod streaming;
pub(crate) mod text;
mod vad;
mod whisper;

pub use audio::{AudioConfig, AudioFeedbackConfig, DeadInputConfig};
pub use default_config::{
    default_config_content, generated_config_content, new_config_content, DEFAULT_CONFIG,
};
pub use engines::{
    openvino_driver_packages, CohereConfig, DolphinConfig, ElevenLabsCommitStrategy,
    ElevenLabsConfig, ElevenLabsMode, ElevenLabsRegion, GgufConfig, MoonshineConfig,
    OmnilingualConfig, OpenVinoConfig, ParaformerConfig, ParakeetConfig, ParakeetModelType,
    SenseVoiceConfig, SonioxConfig, TranscriptionEngine, SONIOX_REGIONS,
};
pub use history::{
    parse_app_list as parse_history_app_list, HistoryConfig, HistoryOverrides,
    DEFAULT_EXCLUDE_APPS as DEFAULT_HISTORY_EXCLUDE_APPS,
};
pub use hotkey::{ActivationMode, HotkeyConfig};
pub use language::{LanguageConfig, COHERE_LANGUAGES};
pub use load::{load_config, resolve_config_file, save_config};
pub use meeting::{
    MeetingActionsConfig, MeetingAudioConfig, MeetingCalendarConfig, MeetingConfig,
    MeetingDiarizationConfig, MeetingShareConfig, MeetingStartOptions, MeetingSummaryConfig,
};
pub use meeting_detect::{
    default_apps as default_call_apps, CallAppPattern, CallEnd, MeetingDetectConfig,
    CALL_END_CHOICES,
};
pub use notification::NotificationConfig;
pub use output::{
    default_language_to_layout, AppliedLanguageXkbHint, FileMode, OutputConfig, OutputDriver,
    OutputMode,
};
pub use profile::{PostProcessConfig, Profile, VocabularyConfig};
pub(crate) use root::EngineLanguage;
pub use root::{ActiveGguf, Config, OmarchyConfig, TextLanguageSource, TrayConfig};
pub use status::{ResolvedIcons, StatusConfig, StatusIconOverrides};
pub use streaming::{StreamingConfig, StreamingRecoveryConfig};
pub use text::TextConfig;
pub use vad::{VadBackend, VadConfig};
pub use whisper::{WhisperConfig, WhisperMode};

pub(super) fn default_true() -> bool {
    true
}

pub(super) fn default_on_demand_loading() -> bool {
    false
}
