//! ElevenLabs Scribe engine configuration.

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

const DEFAULT_VAD_SILENCE_THRESHOLD_SECS: f32 = 1.5;
/// The batch model when `[elevenlabs] model` is unset.
pub const DEFAULT_ELEVENLABS_MODEL: &str = "scribe_v3";
/// The realtime model when `[elevenlabs] realtime_model` is unset.
pub const DEFAULT_ELEVENLABS_REALTIME_MODEL: &str = "scribe_v3_live";

/// ElevenLabs API region.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ElevenLabsRegion {
    /// Global API endpoint.
    #[default]
    Global,
    /// United States API endpoint.
    Us,
    /// European Union residency endpoint.
    Eu,
    /// India residency endpoint.
    India,
    /// Singapore residency endpoint.
    Singapore,
}

/// ElevenLabs dictation behavior.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum ElevenLabsMode {
    /// Upload the completed recording and return one transcript.
    Batch,
    /// Stream audio but type only committed transcript segments.
    #[default]
    Realtime,
    /// Stream audio and type provisional partial transcript extensions.
    Partials,
}

/// Strategy for committing ElevenLabs realtime transcript segments.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum ElevenLabsCommitStrategy {
    /// Let ElevenLabs commit after configured silence.
    #[default]
    Vad,
    /// Keep the segment open until Voxtype explicitly commits it.
    Manual,
}

/// ElevenLabs Scribe cloud transcription configuration.
/// Requires: cargo build --features elevenlabs
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ElevenLabsConfig {
    /// API key sent in the sensitive `xi-api-key` header.
    pub api_key: Option<String>,

    /// API endpoint region. Default: `global`.
    pub region: ElevenLabsRegion,

    /// Optional ISO 639-1 or ISO 639-3 language hint.
    #[serde(deserialize_with = "deserialize_language_code")]
    pub language_code: Option<String>,

    /// Dictation behavior. Default: `realtime`.
    pub mode: ElevenLabsMode,

    /// Speech-to-text model for batch transcription, passed to the API as
    /// given. Default: `scribe_v3`.
    pub model: String,

    /// Speech-to-text model for the realtime and partials modes, passed to
    /// the API as given. Default: `scribe_v3_live`.
    pub realtime_model: String,

    /// Realtime segment commit strategy. Default: `vad`.
    pub commit_strategy: ElevenLabsCommitStrategy,

    /// Remove provider-detected fillers and disfluencies. Default: false.
    pub no_verbatim: bool,

    /// Silence required before VAD commits a realtime segment. Default: 1.5 seconds.
    #[serde(deserialize_with = "deserialize_vad_silence_threshold_secs")]
    pub vad_silence_threshold_secs: f32,
}

impl ElevenLabsConfig {
    /// Set a normalized language hint, treating whitespace-only input as unset.
    pub fn set_language_code(&mut self, value: &str) {
        self.language_code = normalize_language_code(value);
    }

    /// Whether this mode uses the realtime WebSocket API.
    pub fn uses_realtime_api(&self) -> bool {
        self.mode != ElevenLabsMode::Batch
    }

    /// The model the selected mode sends to the API.
    pub fn active_model(&self) -> &str {
        if self.uses_realtime_api() {
            &self.realtime_model
        } else {
            &self.model
        }
    }

    /// Point the selected mode at `model`.
    pub fn set_active_model(&mut self, model: String) {
        if self.uses_realtime_api() {
            self.realtime_model = model;
        } else {
            self.model = model;
        }
    }

    /// Whether provisional transcript extensions should be typed.
    pub fn types_partial_transcripts(&self) -> bool {
        self.mode == ElevenLabsMode::Partials
    }

    /// Set and validate the VAD silence threshold.
    pub fn set_vad_silence_threshold_secs(&mut self, value: f32) -> Result<(), String> {
        validate_vad_silence_threshold_secs(value)?;
        self.vad_silence_threshold_secs = value;
        Ok(())
    }

    /// Validate values that can also be constructed programmatically.
    pub fn validate(&self) -> Result<(), String> {
        validate_vad_silence_threshold_secs(self.vad_silence_threshold_secs)
    }
}

impl Default for ElevenLabsConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            region: ElevenLabsRegion::Global,
            language_code: None,
            mode: ElevenLabsMode::Realtime,
            model: DEFAULT_ELEVENLABS_MODEL.to_string(),
            realtime_model: DEFAULT_ELEVENLABS_REALTIME_MODEL.to_string(),
            commit_strategy: ElevenLabsCommitStrategy::Vad,
            no_verbatim: false,
            vad_silence_threshold_secs: default_vad_silence_threshold_secs(),
        }
    }
}

impl std::fmt::Debug for ElevenLabsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ElevenLabsConfig")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("region", &self.region)
            .field("language_code", &self.language_code)
            .field("mode", &self.mode)
            .field("model", &self.model)
            .field("realtime_model", &self.realtime_model)
            .field("commit_strategy", &self.commit_strategy)
            .field("no_verbatim", &self.no_verbatim)
            .field(
                "vad_silence_threshold_secs",
                &self.vad_silence_threshold_secs,
            )
            .finish()
    }
}

fn default_vad_silence_threshold_secs() -> f32 {
    DEFAULT_VAD_SILENCE_THRESHOLD_SECS
}

fn validate_vad_silence_threshold_secs(value: f32) -> Result<(), String> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err("ElevenLabs vad_silence_threshold_secs must be a positive finite number".to_string())
    }
}

fn deserialize_vad_silence_threshold_secs<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f32::deserialize(deserializer)?;
    validate_vad_silence_threshold_secs(value).map_err(D::Error::custom)?;
    Ok(value)
}

fn normalize_language_code(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn deserialize_language_code<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.as_deref().and_then(normalize_language_code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_configuration_contract() {
        let config = ElevenLabsConfig::default();
        assert_eq!(
            (
                config.api_key,
                config.region,
                config.language_code,
                config.mode,
                config.model.as_str(),
                config.realtime_model.as_str(),
                config.commit_strategy,
                config.no_verbatim,
                config.vad_silence_threshold_secs,
            ),
            (
                None,
                ElevenLabsRegion::Global,
                None,
                ElevenLabsMode::Realtime,
                "scribe_v3",
                "scribe_v3_live",
                ElevenLabsCommitStrategy::Vad,
                false,
                1.5,
            )
        );

        let deserialized: ElevenLabsConfig = toml::from_str("").unwrap();
        assert_eq!(
            (
                deserialized.api_key,
                deserialized.region,
                deserialized.language_code,
                deserialized.mode,
                deserialized.model.as_str(),
                deserialized.realtime_model.as_str(),
                deserialized.commit_strategy,
                deserialized.no_verbatim,
                deserialized.vad_silence_threshold_secs,
            ),
            (
                None,
                ElevenLabsRegion::Global,
                None,
                ElevenLabsMode::Realtime,
                "scribe_v3",
                "scribe_v3_live",
                ElevenLabsCommitStrategy::Vad,
                false,
                1.5,
            )
        );
    }

    #[test]
    fn parses_every_region_string() {
        let cases = [
            ("global", ElevenLabsRegion::Global),
            ("us", ElevenLabsRegion::Us),
            ("eu", ElevenLabsRegion::Eu),
            ("india", ElevenLabsRegion::India),
            ("singapore", ElevenLabsRegion::Singapore),
        ];

        for (value, expected) in cases {
            assert_eq!(value.parse::<ElevenLabsRegion>().unwrap(), expected);
            let parsed: ElevenLabsConfig =
                toml::from_str(&format!("region = \"{value}\"")).unwrap();
            assert_eq!(parsed.region, expected);
        }
    }

    #[test]
    fn rejects_invalid_region_string() {
        assert!("mars".parse::<ElevenLabsRegion>().is_err());
        assert!(toml::from_str::<ElevenLabsConfig>("region = \"mars\"").is_err());
    }

    #[test]
    fn trims_language_code_and_treats_empty_as_unset() {
        let config: ElevenLabsConfig = toml::from_str("language_code = \"  en  \"").unwrap();
        assert_eq!(config.language_code.as_deref(), Some("en"));

        let config: ElevenLabsConfig = toml::from_str("language_code = \"   \"").unwrap();
        assert_eq!(config.language_code, None);
    }

    #[test]
    fn mode_expresses_every_meaningful_runtime_state() {
        let cases = [
            ("batch", ElevenLabsMode::Batch, false, false),
            ("realtime", ElevenLabsMode::Realtime, true, false),
            ("partials", ElevenLabsMode::Partials, true, true),
        ];

        for (value, expected_mode, expected_streaming, expected_partials) in cases {
            let config: ElevenLabsConfig = toml::from_str(&format!("mode = \"{value}\"")).unwrap();
            assert_eq!(
                (
                    config.mode,
                    config.uses_realtime_api(),
                    config.types_partial_transcripts(),
                ),
                (expected_mode, expected_streaming, expected_partials),
            );
        }

        assert!(toml::from_str::<ElevenLabsConfig>("mode = \"continuous\"").is_err());
    }

    #[test]
    fn parses_and_rejects_commit_strategies() {
        for (value, expected) in [
            ("vad", ElevenLabsCommitStrategy::Vad),
            ("manual", ElevenLabsCommitStrategy::Manual),
        ] {
            let config: ElevenLabsConfig =
                toml::from_str(&format!("commit_strategy = \"{value}\"")).unwrap();
            assert_eq!(config.commit_strategy, expected);
        }

        assert!(toml::from_str::<ElevenLabsConfig>("commit_strategy = \"automatic\"").is_err());
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(toml::from_str::<ElevenLabsConfig>("unexpected = true").is_err());
    }

    #[test]
    fn validates_configurable_vad_silence_threshold() {
        let config: ElevenLabsConfig = toml::from_str("vad_silence_threshold_secs = 0.75").unwrap();
        assert_eq!(config.vad_silence_threshold_secs, 0.75);

        for invalid in ["0", "-0.1", "nan", "inf"] {
            let toml = format!("vad_silence_threshold_secs = {invalid}");
            assert!(
                toml::from_str::<ElevenLabsConfig>(&toml).is_err(),
                "accepted invalid threshold {invalid}"
            );
        }
    }

    #[test]
    fn serialization_includes_provider_behavior_settings() {
        let config: ElevenLabsConfig = toml::from_str(
            r#"
                mode = "partials"
                model = "scribe_v9_future"
                realtime_model = "scribe_v9_live"
                commit_strategy = "manual"
                no_verbatim = true
                vad_silence_threshold_secs = 0.75
            "#,
        )
        .unwrap();

        assert_eq!(
            (config.commit_strategy, config.no_verbatim),
            (ElevenLabsCommitStrategy::Manual, true)
        );
        let serialized = toml::to_string(&config).unwrap();
        assert!(serialized.contains("mode = \"partials\""));
        assert!(serialized.contains("model = \"scribe_v9_future\""));
        assert!(serialized.contains("realtime_model = \"scribe_v9_live\""));
        assert!(serialized.contains("commit_strategy = \"manual\""));
        assert!(serialized.contains("no_verbatim = true"));
        assert!(serialized.contains("vad_silence_threshold_secs = 0.75"));
    }

    #[test]
    fn debug_output_redacts_api_key() {
        let config = ElevenLabsConfig {
            api_key: Some("super-secret-key".to_string()),
            ..ElevenLabsConfig::default()
        };
        let debug = format!("{config:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret-key"));
    }
}
