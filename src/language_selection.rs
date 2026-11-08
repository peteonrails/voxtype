//! Conservative quick-language controls for known checkpoints, not engine-wide promises.
use crate::config::{Config, LanguageConfig, TranscriptionEngine, WhisperMode};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LanguageMenu {
    pub context: String,
    pub current: String,
    pub choices: Vec<(&'static str, &'static str)>,
    pub explanation: String,
}

impl LanguageMenu {
    pub fn for_config(config: &Config) -> Self {
        let mut menu = Self {
            context: format!("{} / {}", config.engine.name(), config.model_name()),
            current: config.active_language().unwrap_or("auto").to_string(),
            explanation: "Language selection is unavailable for this checkpoint".into(),
            ..Self::default()
        };
        let basename = Path::new(config.model_name())
            .file_name()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        match config.engine {
            TranscriptionEngine::Cohere
                if crate::model_catalog::model_catalog("cohere").contains(&basename) =>
            {
                menu.choices = crate::config::COHERE_LANGUAGES
                    .iter()
                    .map(|&code| {
                        (
                            code,
                            whisper_rs::get_lang_id(code)
                                .and_then(whisper_rs::get_lang_str_full)
                                .unwrap_or(code),
                        )
                    })
                    .collect();
                menu.explanation =
                    "Explicit language hint; Cohere has no supported Auto control".into();
            }
            TranscriptionEngine::Whisper
                if config.whisper.effective_mode() == WhisperMode::Local =>
            {
                let name = basename.strip_prefix("ggml-").unwrap_or(basename);
                let name = name.strip_suffix(".bin").unwrap_or(name);
                if let Some(english_only) = crate::setup::model::whisper_model_english_only(name) {
                    if english_only {
                        menu.current = "en".into();
                        menu.explanation = "English only (fixed checkpoint)".into();
                    } else {
                        menu.choices.push(("auto", "Auto"));
                        for id in 0..=whisper_rs::get_lang_max_id() {
                            if let (Some(code), Some(label)) = (
                                whisper_rs::get_lang_str(id),
                                whisper_rs::get_lang_str_full(id),
                            ) {
                                // Cantonese was added with the v3 vocabulary.
                                if code != "yue" || name.starts_with("large-v3") {
                                    menu.choices.push((code, label));
                                }
                            }
                        }
                        menu.current = config.whisper.language.as_vec().join(",");
                        menu.explanation = "Multilingual Whisper; applies to next dictation".into();
                    }
                }
            }
            TranscriptionEngine::Parakeet => {
                menu.current = if basename.contains("-v2") || basename.contains("-en-") {
                    "en"
                } else if basename.contains("-v3") {
                    "automatic"
                } else {
                    "model-controlled"
                }
                .into();
                menu.explanation =
                    "Checkpoint determines language; no selectable language hint".into();
            }
            _ => {}
        }
        menu
    }
}

/// Persist one canonical engine key and return the next-utterance configuration.
pub fn persist(active: &Config, path: &Path, language: &str) -> Result<Config, String> {
    let menu = LanguageMenu::for_config(active);
    if !menu.choices.iter().any(|choice| choice.0 == language) {
        return Err(format!(
            "Unsupported language '{language}' for {}. {}",
            menu.context, menu.explanation
        ));
    }
    let disk: Config = toml::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if disk.engine != active.engine || disk.model_name() != active.model_name() {
        return Err("The configured engine/model differs from the running daemon. Restart explicitly before changing its language.".into());
    }
    let table = active.engine.name();
    let mut editor =
        crate::tui::ConfigEditor::load_from_path(path.to_path_buf()).map_err(|e| e.to_string())?;
    editor.set_string(table, "language", language);
    editor.save().map_err(|e| e.to_string())?;
    let readback =
        crate::tui::ConfigEditor::load_from_path(path.to_path_buf()).map_err(|e| e.to_string())?;
    if readback.get_string(table, "language").as_deref() != Some(language) {
        return Err("Language save failed readback; runtime unchanged".into());
    }
    let mut next = active.clone();
    match next.engine {
        TranscriptionEngine::Whisper => {
            next.whisper.language = LanguageConfig::Single(language.into())
        }
        TranscriptionEngine::Cohere => {
            next.cohere.get_or_insert_with(Default::default).language = language.into()
        }
        _ => return Err("This engine has no supported language selector".into()),
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_persists_comments_and_other_engine_without_mutating_active() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"# keep
engine = "cohere"
[cohere]
model = "cohere-transcribe-03-2026-Q4_K_M.gguf"
language = "en" # hint
gguf_backend = "vulkan"
[whisper]
language = "de"
[text]
disfluency_removal = false
"#,
        )
        .unwrap();
        let active: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let next = persist(&active, &path, "fr").unwrap();
        assert_eq!(active.active_language(), Some("en"));
        assert_eq!(next.active_language(), Some("fr"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep"));
        assert!(text.contains("# hint"));
        assert!(text.contains("language = \"de\""));
        assert_eq!(next.cohere.unwrap().gguf_backend, "vulkan");
    }

    #[test]
    fn every_canonical_whisper_checkpoint_uses_metadata_and_unknown_distil_is_read_only() {
        let mut config = Config::default();
        for name in crate::model_catalog::model_catalog("whisper") {
            config.whisper.model = name.into();
            let menu = LanguageMenu::for_config(&config);
            match crate::setup::model::whisper_model_english_only(name) {
                Some(true) => {
                    assert!(menu.choices.is_empty(), "{name}");
                    assert_eq!(menu.current, "en");
                }
                Some(false) => assert!(menu.choices.iter().any(|c| c.0 == "auto"), "{name}"),
                None => panic!("Canonical checkpoint {name} has no metadata"),
            }
        }
        // These do not belong to the local Whisper catalog. In particular,
        // English-only distil variants must never inherit a universal list.
        for name in [
            "distil-large-v2",
            "distil-large-v3",
            "distil-medium.en",
            "/models/ggml-distil-large-v3.bin",
            "distil-small.en",
        ] {
            config.whisper.model = name.into();
            assert!(
                LanguageMenu::for_config(&config).choices.is_empty(),
                "{name}"
            );
        }
    }

    #[test]
    fn engine_checkpoint_filtering_and_auto_are_conservative() {
        let mut cfg = Config::default();
        cfg.whisper.model = "base".into();
        let multilingual = LanguageMenu::for_config(&cfg);
        assert!(multilingual.choices.iter().any(|c| c.0 == "auto"));
        assert!(!multilingual.choices.iter().any(|c| c.0 == "yue"));
        cfg.whisper.model = "large-v3".into();
        assert!(LanguageMenu::for_config(&cfg)
            .choices
            .iter()
            .any(|c| c.0 == "yue"));
        for model in ["base.en", "custom.bin", "unknown"] {
            cfg.whisper.model = model.into();
            assert!(LanguageMenu::for_config(&cfg).choices.is_empty());
        }
        cfg.whisper.model = "base".into();
        cfg.whisper.mode = Some(WhisperMode::Remote);
        assert!(LanguageMenu::for_config(&cfg).choices.is_empty());
        cfg.set_model_for(
            TranscriptionEngine::Parakeet,
            "parakeet-tdt-0.6b-v3-Q8_0.gguf",
        );
        let menu = LanguageMenu::for_config(&cfg);
        assert!(menu.choices.is_empty());
        assert!(menu.explanation.contains("no selectable"));
        cfg.set_model_for(TranscriptionEngine::Cohere, "/models/custom.gguf");
        assert!(LanguageMenu::for_config(&cfg).choices.is_empty());
    }

    #[test]
    fn invalid_fixed_language_and_save_failure_leave_state_unchanged() {
        let mut cfg = Config::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# fixed checkpoint\n[whisper]\nmodel = \"base.en\"\n",
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(persist(&cfg, &path, "fr").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        cfg.whisper.model = "base".into();
        assert!(persist(&cfg, &path, "fr").unwrap_err().contains("differs"));
        assert!(persist(&cfg, &dir.path().join("missing"), "fr").is_err());
        assert_eq!(cfg.active_language(), Some("en"));
    }

    #[test]
    fn known_cohere_offers_exact_supported_hints_without_auto() {
        let mut config = Config::default();
        config.set_model_for(
            TranscriptionEngine::Cohere,
            "cohere-transcribe-03-2026-Q4_K_M.gguf",
        );
        let menu = LanguageMenu::for_config(&config);
        assert_eq!(menu.choices.len(), 14);
        assert!(menu.choices.iter().any(|c| c.0 == "fr"));
        assert!(!menu.choices.iter().any(|c| c.0 == "auto"));
    }
}
