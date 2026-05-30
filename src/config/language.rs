//! Language configuration.

use serde::{Deserialize, Serialize};

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
