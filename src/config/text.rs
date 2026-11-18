//! Text processing configuration.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Text processing configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TextConfig {
    /// Apply the optional English-only LARD disfluency model. Never downloads at startup.
    #[serde(default)]
    pub lard_disfluency: bool,
    /// Local VXDF model; defaults to the XDG data directory.
    #[serde(default)]
    pub lard_model_path: Option<std::path::PathBuf>,
    /// Restore punctuation, sentence boundaries, and letter casing for
    /// engines whose output is plain uncased CTC text. Engines that already
    /// supply these features are left untouched.
    #[serde(default)]
    pub punctuation_casing: bool,

    /// Enable spoken punctuation conversion (e.g., "period" → ".")
    #[serde(default)]
    pub spoken_punctuation: bool,

    /// Custom word replacements (case-insensitive)
    /// Example: { "vox type" = "voxtype" }
    #[serde(default)]
    pub replacements: HashMap<String, String>,

    /// Smart auto-submit: say "submit" at the end of dictation to press Enter.
    /// The word "submit" is stripped from the output and Enter is pressed.
    #[serde(default)]
    pub smart_auto_submit: bool,

    /// Remove common filler words ("uh", "um", etc.) from transcribed text.
    /// Defaults to false to preserve existing behavior. The list is
    /// configurable via `filler_words`.
    #[serde(default)]
    pub filter_filler_words: bool,

    /// Collapse restart disfluencies: a self-interrupted phrase that the
    /// speaker restarts by repeating a word ("the independent tool. the
    /// independent booking tool"), back-to-back stutters, and explicit
    /// retractions ("scratch that", "I'm sorry"). Off by default: a false
    /// positive deletes words the speaker actually said.
    #[serde(default)]
    pub collapse_restarts: bool,

    /// Repair sentence boundaries the engine placed at a pause: join a
    /// fragment that was split off mid-sentence ("what are the other phases?
    /// we have here.") and give a structurally interrogative sentence its
    /// question mark. Off by default.
    #[serde(default)]
    pub repair_sentence_breaks: bool,

    /// Convert dictated quantities to written form: "twenty five dollars"
    /// becomes "$25", "three thirty p m" becomes "3:30 p.m.". Only sentences
    /// that look like they are about a quantity are touched, and a lone
    /// number word is never turned into a digit, so ordinary prose ("one of
    /// the things", "no one knows") is left alone. Off by default.
    #[serde(default)]
    pub format_numbers: bool,

    /// Words removed when `filter_filler_words` is true. Matched
    /// case-insensitively on word boundaries; surrounding punctuation and
    /// whitespace are cleaned up after removal.
    #[serde(default = "default_filler_words")]
    pub filler_words: Vec<String>,

    /// The language you dictate in, for text cleanup: a code such as "en",
    /// "en-US" or "pt-BR", or "auto" (the default) to declare nothing.
    ///
    /// Language-specific cleanup needs to know what language the text is in.
    /// Most of the time the engine says so: its own language setting, or a
    /// model that only writes one language. A multilingual model on
    /// automatic detection says nothing, and this is where you fill that in.
    /// It only informs text cleanup and is never sent to the engine. An
    /// engine language that is set, and a single-language model, both win
    /// over it: see `Config::text_language`.
    #[serde(
        default = "default_text_language",
        deserialize_with = "deserialize_language"
    )]
    pub language: String,
}

fn default_text_language() -> String {
    "auto".to_string()
}

fn deserialize_language<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    Ok(normalize_language(&String::deserialize(deserializer)?))
}

/// Whether `tag` is shaped like a language code: a primary subtag of two or
/// three letters, optionally followed by region or script subtags (`en`,
/// `en-US`, `pt_BR`, `yue`). It says nothing about whether the language
/// exists.
pub fn is_language_tag(tag: &str) -> bool {
    let mut parts = tag.split(['-', '_']);
    let primary = parts.next().unwrap_or_default();
    (2..=3).contains(&primary.len())
        && primary.bytes().all(|byte| byte.is_ascii_alphabetic())
        && parts.all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

/// `text.language` as it is stored: trimmed, with `auto` and the primary
/// subtag in lower case ("EN-US" becomes "en-US"), because every reader
/// compares the primary subtag exactly. Anything that is not shaped like a
/// language code is returned trimmed and otherwise untouched, so the config
/// still shows what was typed; [`TextConfig::declared_language`] ignores it.
pub fn normalize_language(raw: &str) -> String {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("auto") {
        return "auto".to_string();
    }
    if !is_language_tag(raw) {
        return raw.to_string();
    }
    let split = raw.find(['-', '_']).unwrap_or(raw.len());
    format!("{}{}", raw[..split].to_ascii_lowercase(), &raw[split..])
}

impl TextConfig {
    /// Set `language` from something the user typed (a flag, an environment
    /// variable), normalized as the config file's value is. `Err` carries
    /// the reason for a value that is neither `auto` nor a language code,
    /// and leaves the field alone.
    pub fn set_language(&mut self, raw: &str) -> Result<(), String> {
        let language = normalize_language(raw);
        if language != "auto" && !is_language_tag(&language) {
            return Err(format!(
                "'{language}' is not a language code. Use one such as en, en-US or pt-BR, or auto."
            ));
        }
        self.language = language;
        Ok(())
    }

    /// The dictation language the user declared, or `None` for `auto`, an
    /// empty value and anything that is not a language code.
    pub fn declared_language(&self) -> Option<&str> {
        let language = self.language.as_str();
        (language != "auto" && is_language_tag(language)).then_some(language)
    }

    /// Whether `language` holds something that is neither `auto` nor a
    /// language code, which [`Self::declared_language`] ignores.
    pub fn language_is_malformed(&self) -> bool {
        !matches!(self.language.as_str(), "" | "auto") && !is_language_tag(&self.language)
    }
}

impl Default for TextConfig {
    fn default() -> Self {
        Self {
            lard_disfluency: false,
            lard_model_path: None,
            punctuation_casing: false,
            spoken_punctuation: false,
            replacements: HashMap::new(),
            smart_auto_submit: false,
            filter_filler_words: true,
            collapse_restarts: false,
            repair_sentence_breaks: false,
            format_numbers: false,
            filler_words: default_filler_words(),
            language: default_text_language(),
        }
    }
}

/// Default filler-word list. Conservative: single-syllable disfluencies only.
/// Multi-word phrases like "you know" or "sort of" are too aggressive for a
/// default and can be added via the `filler_words` config.
pub(crate) fn default_filler_words() -> Vec<String> {
    vec![
        "uh".to_string(),
        "um".to_string(),
        "er".to_string(),
        "ah".to_string(),
        "eh".to_string(),
        "hmm".to_string(),
        "hm".to_string(),
        "mm".to_string(),
        "mhm".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml: &str) -> TextConfig {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn language_defaults_to_auto_and_declares_nothing() {
        for config in [
            TextConfig::default(),
            parse(""),
            parse("language = \"auto\""),
        ] {
            assert_eq!(config.language, "auto");
            assert_eq!(config.declared_language(), None);
            assert!(!config.language_is_malformed());
        }
    }

    #[test]
    fn declared_language_is_normalized_on_load() {
        for (written, stored) in [
            ("en", "en"),
            ("EN", "en"),
            (" en ", "en"),
            ("en-US", "en-US"),
            ("EN-us", "en-us"),
            ("pt_BR", "pt_BR"),
            ("yue", "yue"),
            ("AUTO", "auto"),
        ] {
            let config = parse(&format!("language = {written:?}"));
            assert_eq!(config.language, stored, "{written:?}");
            assert!(!config.language_is_malformed(), "{written:?}");
        }
        assert_eq!(parse("language = \"EN\"").declared_language(), Some("en"));
        assert_eq!(
            parse("language = \"en-US\"").declared_language(),
            Some("en-US")
        );
    }

    /// A value that is not a language code declares nothing. It must not
    /// take the rest of `[text]` down with it either: replacements live in
    /// the same table.
    #[test]
    fn malformed_language_is_kept_but_declares_nothing() {
        for written in ["english", "e", "en us", "en-", "", "1234"] {
            let config = parse(&format!(
                "language = {written:?}\nspoken_punctuation = true"
            ));
            assert_eq!(config.declared_language(), None, "{written:?}");
            assert_eq!(config.language_is_malformed(), !written.is_empty());
            assert!(config.spoken_punctuation);
        }
    }
}
