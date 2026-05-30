//! Configuration loading and types for voxtype
//!
//! Configuration is loaded in layers:
//! 1. Built-in defaults
//! 2. Config file (~/.config/voxtype/config.toml)
//! 3. Environment variables (VOXTYPE_*)
//! 4. CLI arguments (highest priority)

mod audio;
mod default_config;
pub mod engines;
mod hotkey;
mod language;
mod load;
mod meeting;
mod notification;
mod output;
mod parse;
mod profile;
mod root;
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
pub use load::{load_config, save_config};
pub use meeting::{
    MeetingAudioConfig, MeetingConfig, MeetingDiarizationConfig, MeetingSummaryConfig,
};
pub use notification::NotificationConfig;
pub use output::{
    default_language_to_layout, AppliedLanguageXkbHint, FileMode, OutputConfig, OutputDriver,
    OutputMode,
};
pub use parse::parse_config_with_defaults;
pub use profile::{PostProcessConfig, Profile};
pub use root::Config;
pub use status::{ResolvedIcons, StatusConfig, StatusIconOverrides};
pub use text::TextConfig;
pub use vad::{VadBackend, VadConfig};
pub use whisper::{WhisperConfig, WhisperMode};

pub(super) fn default_true() -> bool {
    true
}
