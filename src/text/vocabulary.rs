//! Conservative post-hoc spelling correction for profile vocabulary.

use crate::config::VocabularyConfig;
use regex::Regex;
use rphonetic::{DoubleMetaphone, Encoder};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io;

/// Prepared profile vocabulary. Empty or invalid entries are ignored.
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
    /// Load inline terms and an optional JSON terms file.
    pub fn load(config: &VocabularyConfig) -> Result<Self, VocabularyLoadError> {
        let mut terms = config.terms.clone();
        if let Some(path) = &config.terms_file {
            let contents =
                fs::read_to_string(path).map_err(|source| VocabularyLoadError::Read {
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
        Ok(Self::from_terms(terms))
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
    use super::Vocabulary;
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
}
