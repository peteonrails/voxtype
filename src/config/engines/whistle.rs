//! Whistle engine configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Languages accepted by the Whistle model.
pub const WHISTLE_LANGUAGES: &[&str] = &["en", "de", "fr", "es", "it", "nl", "pl"];

/// Whistle CPU speech-to-text configuration.
/// Requires: `cargo build --features whistle` and the Cactus Needle native runtime.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhistleConfig {
    /// Model file name in Voxtype's models directory, or an explicit path.
    #[serde(default = "default_model")]
    pub model: String,

    /// Optional forced language. Missing means automatic language detection.
    #[serde(default)]
    pub language: Option<String>,

    /// Optional words or phrases passed to Whistle's decoder keyword biasing.
    #[serde(default)]
    pub keywords: Vec<String>,

    /// Optional path to libneedle (libneedle.so, libneedle.dylib, or DLL).
    /// When omitted, the platform loader's normal search path is used.
    #[serde(default)]
    pub runtime: Option<PathBuf>,
}

fn default_model() -> String {
    "whistle.cact".to_string()
}

impl Default for WhistleConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            language: None,
            keywords: Vec::new(),
            runtime: None,
        }
    }
}

impl WhistleConfig {
    /// Validate an explicitly forced language, if any.
    pub fn validate_language(&self) -> anyhow::Result<()> {
        if let Some(language) = self.language.as_deref() {
            if !WHISTLE_LANGUAGES.contains(&language) {
                anyhow::bail!(
                    "unsupported Whistle language '{}'; supported codes: {}",
                    language,
                    WHISTLE_LANGUAGES.join(", ")
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_auto_detection_and_no_keywords() {
        let config = WhistleConfig::default();
        assert_eq!(config.model, "whistle.cact");
        assert_eq!(config.language, None);
        assert!(config.keywords.is_empty());
    }

    #[test]
    fn validates_supported_and_unsupported_languages() {
        for language in WHISTLE_LANGUAGES {
            WhistleConfig {
                language: Some((*language).to_string()),
                ..WhistleConfig::default()
            }
            .validate_language()
            .unwrap();
        }
        let error = WhistleConfig {
            language: Some("ja".to_string()),
            ..WhistleConfig::default()
        }
        .validate_language()
        .unwrap_err();
        assert!(error.to_string().contains("en, de, fr, es, it, nl, pl"));
    }

    #[test]
    fn parses_whistle_config() {
        #[derive(Deserialize)]
        struct Wrapper {
            whistle: WhistleConfig,
        }
        let parsed: Wrapper = toml::from_str(
            r#"
                [whistle]
                language = "fr"
                keywords = ["Voxtype", "NixOS"]
            "#,
        )
        .unwrap();
        assert_eq!(parsed.whistle.language.as_deref(), Some("fr"));
        assert_eq!(parsed.whistle.keywords, ["Voxtype", "NixOS"]);
    }
}
