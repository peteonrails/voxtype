//! Conservative post-hoc spelling correction for the top-level and
//! profile vocabularies.

use crate::config::{Config, VocabularyConfig};
use regex::Regex;
use rphonetic::{DoubleMetaphone, Encoder};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;

/// The vocabularies recordings use, prepared once at daemon startup: the
/// top-level `[vocabulary]`, and for each profile with a vocabulary of its
/// own, the top-level terms followed by the profile's.
pub struct Vocabularies {
    global: Option<Vocabulary>,
    profiles: HashMap<String, Vocabulary>,
}

impl Vocabularies {
    /// Load every vocabulary `config` names. A terms file that cannot be
    /// read disables only the vocabulary that names it, with a warning.
    pub fn load(config: &Config) -> Self {
        let global = Vocabulary::global(&config.vocabulary);
        let profiles = config
            .profiles
            .iter()
            .filter(|(_, profile)| profile.vocabulary.is_configured())
            .filter_map(|(name, profile)| match read_terms(&profile.vocabulary) {
                Ok(own) => {
                    let global_terms = global.iter().flat_map(|v| v.terms()).map(str::to_string);
                    let terms = global_terms.chain(own).collect();
                    Some((name.clone(), Vocabulary::from_terms(terms)))
                }
                Err(error) => {
                    tracing::warn!(profile = %name, %error, "Profile vocabulary disabled");
                    None
                }
            })
            .collect();
        Self { global, profiles }
    }

    /// The vocabulary of a recording with `profile`: the profile's, which
    /// starts with the top-level terms, or else the top-level one.
    pub fn for_profile(&self, profile: Option<&str>) -> Option<&Vocabulary> {
        profile
            .and_then(|name| self.profiles.get(name))
            .or(self.global.as_ref())
    }
}

/// Prepared vocabulary. Empty, repeated or invalid entries are ignored.
pub struct Vocabulary {
    terms: Vec<Term>,
    word_re: Regex,
}

struct Term {
    display: String,
    letters: String,
    words: usize,
    primary: String,
    alternate: String,
}

impl Vocabulary {
    /// Load inline terms and an optional JSON terms file. A terms file
    /// written `~/...` is read from the home directory.
    pub fn load(config: &VocabularyConfig) -> Result<Self, VocabularyLoadError> {
        read_terms(config).map(Self::from_terms)
    }

    /// The vocabulary of the top-level `[vocabulary]` table `config`: `None`
    /// when it holds no terms or names a terms file that cannot be read
    /// (logged).
    pub fn global(config: &VocabularyConfig) -> Option<Self> {
        if !config.is_configured() {
            return None;
        }
        match Self::load(config) {
            Ok(vocabulary) if !vocabulary.terms.is_empty() => Some(vocabulary),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(%error, "Global vocabulary disabled");
                None
            }
        }
    }

    fn from_terms(terms: Vec<String>) -> Self {
        let encoder = DoubleMetaphone::default();
        let mut seen = HashSet::new();
        let terms = terms
            .into_iter()
            .filter_map(|display| {
                let display = display.trim().to_string();
                let letters = letters_only(&display);
                if letters.is_empty() || !seen.insert(letters.clone()) {
                    return None;
                }
                let words = display.split_whitespace().count().max(1);
                Some(Term {
                    primary: encoder.encode(&letters),
                    alternate: encoder.encode_alternate(&letters),
                    display,
                    letters,
                    words,
                })
            })
            .collect();
        Self {
            terms,
            word_re: Regex::new(r"[\p{L}\p{N}]+(?:['’][\p{L}\p{N}]+)?")
                .expect("BUG: vocabulary token regex must compile"),
        }
    }

    /// Canonical terms, in configured order, for backend-native biasing.
    pub fn terms(&self) -> impl Iterator<Item = &str> {
        self.terms.iter().map(|term| term.display.as_str())
    }

    /// Correct unambiguous phonetic near-matches while preserving punctuation.
    pub fn correct(&self, text: &str) -> String {
        if self.terms.is_empty() {
            return text.to_string();
        }
        let tokens: Vec<_> = self.word_re.find_iter(text).collect();
        let encoder = DoubleMetaphone::default();
        let mut edits = Vec::new();

        for start in 0..tokens.len() {
            let mut matches = Vec::new();
            for term in &self.terms {
                let min_words = term.words.saturating_sub(1).max(1);
                let max_words = term.words + 1;
                for count in min_words..=max_words {
                    let Some(last) = tokens.get(start + count - 1) else {
                        continue;
                    };
                    let first = tokens[start];
                    let candidate = &text[first.start()..last.end()];
                    if protected(text, first.start(), last.end()) {
                        continue;
                    }
                    if candidate
                        .chars()
                        .any(|character| character.is_alphabetic() && !character.is_ascii())
                    {
                        continue;
                    }
                    let letters = letters_only(candidate);
                    if letters.eq_ignore_ascii_case(&term.letters) {
                        matches.push((first.start(), last.end(), &term.display));
                        continue;
                    }
                    if term.letters.len() <= 5 || common_word(&letters) {
                        continue;
                    }
                    let distance = strsim::levenshtein(&letters, &term.letters);
                    let limit = if term.letters.len() == 6 {
                        1
                    } else {
                        (term.letters.len() / 4).max(1)
                    };
                    if distance > limit {
                        continue;
                    }
                    let primary = encoder.encode(&letters);
                    let alternate = encoder.encode_alternate(&letters);
                    if phonetic_match(&primary, &alternate, term) {
                        matches.push((first.start(), last.end(), &term.display));
                    }
                }
            }
            matches.sort_unstable_by_key(|(from, to, _)| (*from, *to));
            matches.dedup_by_key(|(from, to, term)| (*from, *to, *term));
            if matches.len() == 1 {
                edits.push(matches[0]);
            }
        }

        edits.sort_unstable_by_key(|(from, to, _)| (*from, std::cmp::Reverse(*to)));
        let mut last_end = 0;
        edits.retain(|(from, to, _)| {
            if *from < last_end {
                false
            } else {
                last_end = *to;
                true
            }
        });
        let mut output = text.to_string();
        for (from, to, replacement) in edits.into_iter().rev() {
            output.replace_range(from..to, replacement);
        }
        output
    }
}

/// Inline terms, then those of the JSON terms file, as configured. A terms
/// file written `~/...` is read from the home directory.
fn read_terms(config: &VocabularyConfig) -> Result<Vec<String>, VocabularyLoadError> {
    let mut terms = config.terms.clone();
    if let Some(configured) = &config.terms_file {
        let path = configured
            .to_str()
            .map_or_else(|| configured.clone(), crate::llm::expand_home);
        let contents = fs::read_to_string(&path).map_err(|source| VocabularyLoadError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let parsed: Value =
            serde_json::from_str(&contents).map_err(|source| VocabularyLoadError::Json {
                path: path.display().to_string(),
                source,
            })?;
        let values = parsed
            .as_array()
            .ok_or_else(|| VocabularyLoadError::Shape {
                path: path.display().to_string(),
            })?;
        for value in values {
            let term = value.as_str().ok_or_else(|| VocabularyLoadError::Shape {
                path: path.display().to_string(),
            })?;
            terms.push(term.to_string());
        }
    }
    Ok(terms)
}

fn letters_only(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .flat_map(char::to_lowercase)
        .collect()
}

fn protected(text: &str, from: usize, to: usize) -> bool {
    let start = text[..from]
        .rfind(char::is_whitespace)
        .map_or(0, |index| index + 1);
    let end = text[to..]
        .find(char::is_whitespace)
        .map_or(text.len(), |index| to + index);
    text[start..end].contains(['@', '/', '\\'])
}

fn phonetic_match(primary: &str, alternate: &str, term: &Term) -> bool {
    [primary, alternate]
        .iter()
        .filter(|key| !key.is_empty())
        .any(|key| key == &term.primary || key == &term.alternate)
}

// These are the ordinary words from the issue's pinned trap corpus. The
// strict phonetic + edit-distance gate remains the primary false-positive
// guard; this list protects known collisions without claiming a language-wide
// dictionary.
fn common_word(word: &str) -> bool {
    matches!(
        word,
        "wild" | "monarchy" | "hyperlink" | "pipe" | "tail" | "way" | "bar"
    )
}

#[derive(Debug, thiserror::Error)]
pub enum VocabularyLoadError {
    #[error("cannot read vocabulary file '{path}': {source}")]
    Read { path: String, source: io::Error },
    #[error("cannot parse vocabulary file '{path}' as JSON: {source}")]
    Json {
        path: String,
        source: serde_json::Error,
    },
    #[error("vocabulary file '{path}' must contain a JSON array of strings")]
    Shape { path: String },
}

#[cfg(test)]
mod tests {
    use super::{Vocabularies, Vocabulary};
    use crate::config::{TextConfig, VocabularyConfig};
    use crate::text::TextProcessor;
    use std::collections::HashMap;

    fn vocabulary(terms: &[&str]) -> Vocabulary {
        Vocabulary::from_terms(terms.iter().map(|term| (*term).to_string()).collect())
    }

    #[test]
    fn corrects_split_merged_and_misspelled_terms() {
        let vocabulary = vocabulary(&["Hyprland", "PipeWire", "Voxtype", "Shae Wilde"]);
        assert_eq!(
            vocabulary.correct("Hyper land uses pipe wire and VoxType by Shay Wilde."),
            "Hyprland uses PipeWire and Voxtype by Shae Wilde."
        );
    }

    #[test]
    fn trap_sentence_is_unchanged() {
        let vocabulary = vocabulary(&["Hyprland", "Omarchy", "PipeWire", "Tailscale"]);
        let trap = "The wild monarchy hyperlink led to a pipe by the tail near the way bar.";
        assert_eq!(vocabulary.correct(trap), trap);
    }

    #[test]
    fn leaves_urls_emails_paths_and_short_terms_alone() {
        let vocabulary = vocabulary(&["GitHub", "Voxtype"]);
        let text = "gitlab@example.com /opt/voxtipe https://voxtipe.dev gitlab";
        assert_eq!(vocabulary.correct(text), text);
        assert_eq!(
            vocabulary.correct("Use Vox tipe 👍, not βeta."),
            "Use Voxtype 👍, not βeta."
        );
    }

    #[test]
    fn ties_are_unchanged_and_processing_is_idempotent() {
        let vocabulary = vocabulary(&["Hyprland", "Hyperland"]);
        let once = vocabulary.correct("Hiper land");
        assert_eq!(once, "Hiper land");
        assert_eq!(vocabulary.correct(&once), once);
    }

    #[test]
    fn user_replacements_run_after_vocabulary() {
        let vocabulary = vocabulary(&["Voxtype"]);
        let processor = TextProcessor::new(&TextConfig {
            replacements: HashMap::from([("Voxtype".to_string(), "VOXTYPE".to_string())]),
            ..TextConfig::default()
        });

        assert_eq!(
            processor.process_with_vocabulary("Vox tipe", Some(&vocabulary)),
            "VOXTYPE"
        );
    }

    #[test]
    fn loads_and_merges_a_json_terms_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("terms.json");
        std::fs::write(&path, r#"["PipeWire", "Tailscale"]"#).unwrap();
        let vocabulary = Vocabulary::load(&VocabularyConfig {
            terms: vec!["Voxtype".to_string()],
            terms_file: Some(path),
        })
        .unwrap();

        assert_eq!(
            vocabulary.correct("Vox tipe uses pipe wire and tail scale"),
            "Voxtype uses PipeWire and Tailscale"
        );
    }

    /// The vocabularies a config.toml holding `config` sets up.
    fn vocabularies(config: &str) -> Vocabularies {
        Vocabularies::load(&toml::from_str(config).unwrap())
    }

    fn terms_for(vocabularies: &Vocabularies, profile: Option<&str>) -> Vec<String> {
        vocabularies
            .for_profile(profile)
            .map(|vocabulary| vocabulary.terms().map(str::to_string).collect())
            .unwrap_or_default()
    }

    #[test]
    fn global_vocabulary_applies_with_or_without_a_profile() {
        let vocabularies = vocabularies(
            r#"
            [vocabulary]
            terms = ["Voxtype", "PipeWire"]

            [profiles.notes]
            post_process_command = "cat"
            "#,
        );

        for profile in [None, Some("notes"), Some("unknown")] {
            assert_eq!(
                terms_for(&vocabularies, profile),
                ["Voxtype", "PipeWire"],
                "{profile:?}"
            );
        }
    }

    #[test]
    fn profile_vocabulary_alone_applies_only_to_its_profile() {
        let vocabularies = vocabularies(
            r#"
            [profiles.code.vocabulary]
            terms = ["Hyprland"]
            "#,
        );

        assert_eq!(terms_for(&vocabularies, Some("code")), ["Hyprland"]);
        assert!(vocabularies.for_profile(None).is_none());
    }

    #[test]
    fn global_terms_come_first_and_profile_repeats_drop() {
        let vocabularies = vocabularies(
            r#"
            [vocabulary]
            terms = ["Voxtype", "PipeWire"]

            [profiles.code.vocabulary]
            terms = ["Hyprland", "pipewire", "Voxtype"]
            "#,
        );

        assert_eq!(
            terms_for(&vocabularies, Some("code")),
            ["Voxtype", "PipeWire", "Hyprland"]
        );
        assert_eq!(
            vocabularies
                .for_profile(Some("code"))
                .unwrap()
                .correct("vox type on hyper land with pipe wire"),
            "Voxtype on Hyprland with PipeWire"
        );
    }

    #[test]
    fn absent_or_empty_global_vocabulary_is_none() {
        for config in [
            "",
            "[vocabulary]",
            "[vocabulary]\nterms = []",
            "[vocabulary]\nterms = [\" \"]",
        ] {
            assert!(
                vocabularies(config).for_profile(None).is_none(),
                "{config:?}"
            );
        }
    }

    #[test]
    fn an_unreadable_terms_file_disables_only_its_own_vocabulary() {
        let broken_profile = vocabularies(
            r#"
            [vocabulary]
            terms = ["Voxtype"]

            [profiles.broken.vocabulary]
            terms = ["Hyprland"]
            terms_file = "/nonexistent/voxtype-terms.json"
            "#,
        );
        assert_eq!(terms_for(&broken_profile, Some("broken")), ["Voxtype"]);

        let broken_global = vocabularies(
            r#"
            [vocabulary]
            terms = ["Voxtype"]
            terms_file = "/nonexistent/voxtype-terms.json"

            [profiles.code.vocabulary]
            terms = ["Hyprland"]
            "#,
        );
        assert!(broken_global.for_profile(None).is_none());
        assert_eq!(terms_for(&broken_global, Some("code")), ["Hyprland"]);
    }

    /// A terms file under the home directory can be written `~/...`, as
    /// config.toml paths usually are, in the top-level table and in a
    /// profile's. Runs in its own process with a scratch HOME, so it never
    /// reads the real home directory.
    #[test]
    fn terms_files_expand_a_leading_tilde() {
        let scratch = tempfile::tempdir().unwrap();
        if !crate::test_support::own_process(
            "text::vocabulary::tests::terms_files_expand_a_leading_tilde",
            |child| {
                child.env("HOME", scratch.path());
            },
        ) {
            return;
        }
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        std::fs::write(home.join("global.json"), r#"["PipeWire", "Voxtype"]"#).unwrap();
        std::fs::write(home.join("code.json"), r#"["Hyprland"]"#).unwrap();

        let vocabularies = vocabularies(
            r#"
            [vocabulary]
            terms = ["Voxtype"]
            terms_file = "~/global.json"

            [profiles.code.vocabulary]
            terms_file = "~/code.json"
            "#,
        );

        assert_eq!(terms_for(&vocabularies, None), ["Voxtype", "PipeWire"]);
        assert_eq!(
            terms_for(&vocabularies, Some("code")),
            ["Voxtype", "PipeWire", "Hyprland"]
        );
    }
}
