//! License management and Pro feature gating
//!
//! Voxtype offers both free and Pro features. This module handles
//! checking which features require a Pro license and whether the
//! current installation has one.

use thiserror::Error;

/// Pro features that require a license
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProFeature {
    /// Large whisper models (large-v3, large-v3-turbo)
    LargeModels,
    /// GPU isolation mode for memory management
    GpuIsolation,
    /// Remote transcription backend
    RemoteBackend,
    /// Meeting transcription mode (Phase 1)
    MeetingMode,
    /// Meeting speaker diarization (Phase 3)
    MeetingDiarization,
    /// Meeting remote sync to corporate server (Phase 4)
    MeetingRemoteSync,
    /// Meeting AI summarization (Phase 5)
    MeetingSummarization,
}

impl ProFeature {
    /// Human-readable name for the feature
    pub fn name(&self) -> &'static str {
        match self {
            ProFeature::LargeModels => "Large Models",
            ProFeature::GpuIsolation => "GPU Isolation",
            ProFeature::RemoteBackend => "Remote Backend",
            ProFeature::MeetingMode => "Meeting Mode",
            ProFeature::MeetingDiarization => "Meeting Diarization",
            ProFeature::MeetingRemoteSync => "Meeting Remote Sync",
            ProFeature::MeetingSummarization => "Meeting Summarization",
        }
    }

    /// Description of the feature
    pub fn description(&self) -> &'static str {
        match self {
            ProFeature::LargeModels => {
                "Access to large-v3 and large-v3-turbo models for higher accuracy"
            }
            ProFeature::GpuIsolation => {
                "Subprocess-based GPU memory isolation for battery-efficient operation"
            }
            ProFeature::RemoteBackend => "Send audio to a remote Whisper server for transcription",
            ProFeature::MeetingMode => "Continuous meeting transcription with chunked processing",
            ProFeature::MeetingDiarization => {
                "ML-based speaker identification with voice fingerprinting"
            }
            ProFeature::MeetingRemoteSync => {
                "Sync meetings to a corporate server for centralized storage"
            }
            ProFeature::MeetingSummarization => {
                "AI-generated summaries with action items and key points"
            }
        }
    }
}

/// Errors related to license validation
#[derive(Error, Debug)]
pub enum LicenseError {
    #[error("Pro license required for {feature}.\n  Get a license at https://voxtype.io/pro")]
    ProRequired { feature: String },

    #[error("Invalid license key")]
    InvalidKey,

    #[error("License expired")]
    Expired,

    #[error("License file not found: {0}")]
    NotFound(String),

    #[error("License validation failed: {0}")]
    ValidationFailed(String),
}

/// License status for the current installation
#[derive(Debug, Clone, Default)]
pub struct License {
    /// Whether this is a Pro license
    pub is_pro: bool,
    /// License key (if any)
    pub key: Option<String>,
    /// Email associated with license
    pub email: Option<String>,
    /// Expiration timestamp (if any)
    pub expires_at: Option<i64>,
}

impl License {
    /// Load license from the default location
    ///
    /// License file is stored at ~/.config/voxtype/license.toml
    pub fn load() -> Self {
        Self::load_from_path(Self::default_path())
    }

    /// Default path for the license file
    fn default_path() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("", "", "voxtype")
            .map(|dirs| dirs.config_dir().join("license.toml"))
    }

    /// Load license from a specific path
    fn load_from_path(path: Option<std::path::PathBuf>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };

        if !path.exists() {
            tracing::debug!("No license file found at {:?}", path);
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                #[derive(serde::Deserialize)]
                struct LicenseFile {
                    key: Option<String>,
                    email: Option<String>,
                }

                match toml::from_str::<LicenseFile>(&contents) {
                    Ok(file) => {
                        // Validate the license key
                        if let Some(ref key) = file.key {
                            if Self::validate_key(key) {
                                tracing::info!("Pro license loaded");
                                return Self {
                                    is_pro: true,
                                    key: Some(key.clone()),
                                    email: file.email,
                                    expires_at: None, // TODO: Extract from key
                                };
                            } else {
                                tracing::warn!("Invalid license key");
                            }
                        }
                        Self::default()
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse license file: {}", e);
                        Self::default()
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read license file: {}", e);
                Self::default()
            }
        }
    }

    /// Validate a license key format
    ///
    /// In the future, this will perform cryptographic validation.
    /// For now, we just check the format.
    fn validate_key(key: &str) -> bool {
        // Expected format: VOXPRO-XXXX-XXXX-XXXX-XXXX
        let parts: Vec<&str> = key.split('-').collect();
        if parts.len() != 5 {
            return false;
        }
        if parts[0] != "VOXPRO" {
            return false;
        }
        // Each part after prefix should be 4 alphanumeric characters
        for part in &parts[1..] {
            if part.len() != 4 || !part.chars().all(|c| c.is_ascii_alphanumeric()) {
                return false;
            }
        }
        true
    }

    /// Check if a specific Pro feature is available
    pub fn has_feature(&self, _feature: ProFeature) -> bool {
        // For development/testing, allow enabling features via environment variable
        if std::env::var("VOXTYPE_PRO_ENABLED").is_ok() {
            return true;
        }

        self.is_pro
    }

    /// Require a Pro feature, returning an error if not available
    pub fn require_feature(&self, feature: ProFeature) -> Result<(), LicenseError> {
        if self.has_feature(feature) {
            Ok(())
        } else {
            Err(LicenseError::ProRequired {
                feature: feature.name().to_string(),
            })
        }
    }

    /// Check if the license is expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            now > expires_at
        } else {
            false
        }
    }
}

/// Global license state
///
/// Call `init_license()` early in application startup to load the license.
/// Then use `license()` to access the current license state.
static LICENSE: std::sync::OnceLock<License> = std::sync::OnceLock::new();

/// Initialize the license system
///
/// Should be called once at application startup.
pub fn init_license() -> &'static License {
    LICENSE.get_or_init(License::load)
}

/// Get the current license
///
/// Panics if `init_license()` has not been called.
pub fn license() -> &'static License {
    LICENSE
        .get()
        .expect("License not initialized. Call init_license() first.")
}

/// Check if a Pro feature is available
///
/// Convenience function that combines init and check.
pub fn has_pro_feature(feature: ProFeature) -> bool {
    init_license().has_feature(feature)
}

/// Require a Pro feature, returning an error if not available
///
/// Convenience function for feature gating.
pub fn require_pro_feature(feature: ProFeature) -> Result<(), LicenseError> {
    init_license().require_feature(feature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_license_key_validation_valid() {
        assert!(License::validate_key("VOXPRO-ABCD-1234-EFGH-5678"));
        assert!(License::validate_key("VOXPRO-0000-0000-0000-0000"));
        assert!(License::validate_key("VOXPRO-ZZZZ-YYYY-XXXX-WWWW"));
    }

    #[test]
    fn test_license_key_validation_invalid() {
        assert!(!License::validate_key(""));
        assert!(!License::validate_key("VOXPRO"));
        assert!(!License::validate_key("VOXPRO-ABCD"));
        assert!(!License::validate_key("VOXFRE-ABCD-1234-EFGH-5678")); // Wrong prefix
        assert!(!License::validate_key("VOXPRO-ABC-1234-EFGH-5678")); // Too short
        assert!(!License::validate_key("VOXPRO-ABCDE-1234-EFGH-5678")); // Too long
        assert!(!License::validate_key("VOXPRO-AB!D-1234-EFGH-5678")); // Invalid char
    }

    #[test]
    fn test_default_license_is_not_pro() {
        let license = License::default();
        assert!(!license.is_pro);
        assert!(license.key.is_none());
    }

    #[test]
    fn test_pro_feature_names() {
        assert_eq!(ProFeature::MeetingMode.name(), "Meeting Mode");
        assert_eq!(ProFeature::MeetingDiarization.name(), "Meeting Diarization");
    }

    #[test]
    fn test_non_pro_license_denies_features() {
        let license = License::default();
        // Without VOXTYPE_PRO_ENABLED env var, should not have features
        std::env::remove_var("VOXTYPE_PRO_ENABLED");
        assert!(!license.has_feature(ProFeature::MeetingMode));
    }

    #[test]
    fn test_pro_license_grants_features() {
        let license = License {
            is_pro: true,
            key: Some("VOXPRO-TEST-TEST-TEST-TEST".to_string()),
            email: None,
            expires_at: None,
        };
        assert!(license.has_feature(ProFeature::MeetingMode));
        assert!(license.has_feature(ProFeature::MeetingDiarization));
    }

    #[test]
    fn test_require_feature_error() {
        let license = License::default();
        std::env::remove_var("VOXTYPE_PRO_ENABLED");
        let result = license.require_feature(ProFeature::MeetingMode);
        assert!(result.is_err());
    }
}
