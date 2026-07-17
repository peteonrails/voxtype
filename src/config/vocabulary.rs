//! Unified vocabulary configuration.
//!
//! One list of proper nouns / jargon, injected into every transcription
//! engine's biasing mechanism (Deepgram keyterms, Whisper initial_prompt,
//! Soniox context terms) and exposed to the post-process command via the
//! VOXTYPE_VOCABULARY environment variable.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct VocabularyConfig {
    /// Inline vocabulary terms (proper names, jargon, product names).
    #[serde(default)]
    pub terms: Vec<String>,

    /// Path to a JSON file containing a list of terms
    /// (`["term1", "term2", ...]`). Tilde-expanded. Loaded once at daemon
    /// startup and merged after `terms`. A configured-but-unreadable file
    /// is a startup error, not a silent skip.
    #[serde(default)]
    pub terms_file: Option<PathBuf>,
}

impl VocabularyConfig {
    /// True when the user configured any vocabulary source.
    pub fn is_configured(&self) -> bool {
        !self.terms.is_empty() || self.terms_file.is_some()
    }

    /// Merge inline + file terms: trimmed, empty entries skipped,
    /// deduplicated case-sensitively, first-seen order preserved.
    pub fn resolve_terms(&self) -> Result<Vec<String>, String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut push = |t: &str| {
            let trimmed = t.trim();
            if trimmed.is_empty() {
                return;
            }
            if seen.insert(trimmed.to_string()) {
                out.push(trimmed.to_string());
            }
        };
        for t in &self.terms {
            push(t);
        }
        if let Some(path) = &self.terms_file {
            let path = expand_tilde(path);
            let bytes = std::fs::read(&path).map_err(|e| {
                format!("vocabulary terms_file unreadable ({}): {e}", path.display())
            })?;
            let parsed: Vec<String> = serde_json::from_slice(&bytes).map_err(|e| {
                format!(
                    "vocabulary terms_file must be a JSON array of strings ({}): {e}",
                    path.display()
                )
            })?;
            for t in &parsed {
                push(t);
            }
        }
        Ok(out)
    }
}

fn expand_tilde(path: &Path) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn default_is_unconfigured_and_empty() {
        let cfg = VocabularyConfig::default();
        assert!(!cfg.is_configured());
        assert_eq!(cfg.resolve_terms().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn inline_terms_trimmed_and_deduped_in_order() {
        let cfg = VocabularyConfig {
            terms: vec![
                "  voxtype ".into(),
                "Hyprland".into(),
                "voxtype".into(),
                "".into(),
            ],
            terms_file: None,
        };
        assert_eq!(cfg.resolve_terms().unwrap(), vec!["voxtype", "Hyprland"]);
    }

    #[test]
    fn file_terms_merged_after_inline() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, r#"["jj", "Hyprland", "Sisyphus"]"#).unwrap();
        let cfg = VocabularyConfig {
            terms: vec!["Hyprland".into()],
            terms_file: Some(f.path().to_path_buf()),
        };
        assert_eq!(
            cfg.resolve_terms().unwrap(),
            vec!["Hyprland", "jj", "Sisyphus"]
        );
    }

    #[test]
    fn missing_file_is_an_error() {
        let cfg = VocabularyConfig {
            terms: vec![],
            terms_file: Some(PathBuf::from("/nonexistent/vocab.json")),
        };
        let err = cfg.resolve_terms().unwrap_err();
        assert!(err.contains("unreadable"), "got: {err}");
    }

    #[test]
    fn malformed_file_is_an_error() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, r#"{{"not": "an array"}}"#).unwrap();
        let cfg = VocabularyConfig {
            terms: vec![],
            terms_file: Some(f.path().to_path_buf()),
        };
        let err = cfg.resolve_terms().unwrap_err();
        assert!(err.contains("JSON array"), "got: {err}");
    }

    #[test]
    fn tilde_expansion_uses_home() {
        let expanded = expand_tilde(Path::new("~/vocab.json"));
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expanded, PathBuf::from(home).join("vocab.json"));
    }
}
