//! Text processing module
//!
//! Provides post-transcription text transformations including:
//! - Spoken punctuation conversion (e.g., "period" → ".")
//! - Custom word replacements

use crate::config::TextConfig;
use regex::Regex;
use std::collections::HashMap;

/// Text processor that applies transformations to transcribed text
pub struct TextProcessor {
    /// Whether spoken punctuation is enabled
    spoken_punctuation: bool,
    /// Custom word replacements (lowercase key → replacement value)
    replacements: HashMap<String, String>,
}

impl TextProcessor {
    /// Create a new text processor from configuration
    pub fn new(config: &TextConfig) -> Self {
        // Normalize replacement keys to lowercase for case-insensitive matching
        let replacements = config
            .replacements
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.clone()))
            .collect();

        Self {
            spoken_punctuation: config.spoken_punctuation,
            replacements,
        }
    }

    /// Process text by applying all enabled transformations
    pub fn process(&self, text: &str) -> String {
        let mut result = text.to_string();

        // Apply spoken punctuation first (so user replacements can override if needed)
        if self.spoken_punctuation {
            result = self.apply_spoken_punctuation(&result);
        }

        // Apply custom replacements
        if !self.replacements.is_empty() {
            result = self.apply_replacements(&result);
        }

        result
    }

    /// Apply spoken punctuation conversions
    fn apply_spoken_punctuation(&self, text: &str) -> String {
        let mut result = text.to_string();

        // Order matters: longer phrases first to avoid partial matches
        // Using word boundaries to avoid replacing parts of words
        let punctuation_map: &[(&str, &str)] = &[
            // Multi-word phrases first
            ("question mark", "?"),
            ("exclamation mark", "!"),
            ("exclamation point", "!"),
            ("open parenthesis", "("),
            ("close parenthesis", ")"),
            ("open paren", "("),
            ("close paren", ")"),
            ("open bracket", "["),
            ("close bracket", "]"),
            ("open brace", "{"),
            ("close brace", "}"),
            ("at sign", "@"),
            ("at symbol", "@"),
            ("dollar sign", "$"),
            ("percent sign", "%"),
            ("plus sign", "+"),
            ("equals sign", "="),
            ("forward slash", "/"),
            ("single quote", "'"),
            ("double quote", "\""),
            ("new paragraph", "\n\n"),
            ("new line", "\n"),
            // Single words
            ("period", "."),
            ("comma", ","),
            ("colon", ":"),
            ("semicolon", ";"),
            ("dash", "-"),
            ("hyphen", "-"),
            ("underscore", "_"),
            ("hash", "#"),
            ("hashtag", "#"),
            ("percent", "%"),
            ("ampersand", "&"),
            ("asterisk", "*"),
            ("plus", "+"),
            ("equals", "="),
            ("slash", "/"),
            ("backslash", "\\"),
            ("pipe", "|"),
            ("tilde", "~"),
            ("backtick", "`"),
            ("tab", "\t"),
        ];

        for (phrase, symbol) in punctuation_map {
            result = replace_phrase_case_insensitive(&result, phrase, symbol);
        }

        // Clean up spacing around punctuation
        result = clean_punctuation_spacing(&result);

        result
    }

    /// Apply custom word replacements (case-insensitive)
    fn apply_replacements(&self, text: &str) -> String {
        let mut result = text.to_string();

        for (word, replacement) in &self.replacements {
            result = replace_phrase_case_insensitive(&result, word, replacement);
        }

        result
    }
}

/// Replace a word/phrase case-insensitively using regex for proper word boundaries
fn replace_phrase_case_insensitive(text: &str, from: &str, to: &str) -> String {
    // Escape regex special characters in the search phrase
    let escaped = regex::escape(from);

    // Build regex with word boundaries (case-insensitive)
    let pattern = format!(r"(?i)\b{}\b", escaped);

    match Regex::new(&pattern) {
        Ok(re) => re.replace_all(text, to).into_owned(),
        Err(_) => text.to_string(),
    }
}

/// Clean up spacing around punctuation marks
fn clean_punctuation_spacing(text: &str) -> String {
    let mut result = text.to_string();

    // Remove space before punctuation that shouldn't have it
    for punct in ['.', ',', '?', '!', ':', ';', ')', ']', '}'] {
        result = result.replace(&format!(" {}", punct), &punct.to_string());
    }

    // Remove space after opening brackets
    for punct in ['(', '[', '{'] {
        result = result.replace(&format!("{} ", punct), &punct.to_string());
    }

    // Remove space before opening brackets (for function calls, array access, etc.)
    for punct in ['(', '[', '{'] {
        result = result.replace(&format!(" {}", punct), &punct.to_string());
    }

    // Remove space before symbols that typically attach to the next word (email, hashtags, etc.)
    for sym in ['#', '@', '$'] {
        result = result.replace(&format!(" {}", sym), &sym.to_string());
    }

    // Remove space after symbols that typically attach to the next word
    for sym in ['#', '@', '$'] {
        result = result.replace(&format!("{} ", sym), &sym.to_string());
    }

    // Remove spaces around newlines and tabs
    result = result.replace(" \n", "\n");
    result = result.replace("\n ", "\n");
    result = result.replace(" \t", "\t");
    result = result.replace("\t ", "\t");

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(spoken_punctuation: bool, replacements: &[(&str, &str)]) -> TextConfig {
        TextConfig {
            spoken_punctuation,
            replacements: replacements
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn test_spoken_punctuation_basic() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("hello period"), "hello.");
        assert_eq!(processor.process("hello comma world"), "hello, world");
        assert_eq!(processor.process("what question mark"), "what?");
    }

    #[test]
    fn test_spoken_punctuation_multi_word() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("open paren test close paren"), "(test)");
        assert_eq!(processor.process("hello exclamation mark"), "hello!");
    }

    #[test]
    fn test_spoken_punctuation_case_insensitive() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("hello PERIOD"), "hello.");
        assert_eq!(processor.process("hello Period"), "hello.");
    }

    #[test]
    fn test_word_replacements() {
        let config = make_config(false, &[("vox type", "voxtype")]);
        let processor = TextProcessor::new(&config);

        assert_eq!(
            processor.process("I use vox type for dictation"),
            "I use voxtype for dictation"
        );
    }

    #[test]
    fn test_word_replacements_case_insensitive() {
        let config = make_config(false, &[("rust", "Rust")]);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("I love RUST"), "I love Rust");
        assert_eq!(processor.process("rust is great"), "Rust is great");
    }

    #[test]
    fn test_disabled_processing() {
        let config = make_config(false, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("hello period"), "hello period");
    }

    #[test]
    fn test_combined_processing() {
        let config = make_config(true, &[("voxtype", "Voxtype")]);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("I use voxtype period"), "I use Voxtype.");
    }

    #[test]
    fn test_developer_punctuation() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(
            processor.process("function open paren close paren"),
            "function()"
        );
        assert_eq!(
            processor.process("array open bracket close bracket"),
            "array[]"
        );
        assert_eq!(processor.process("hash include"), "#include");
        assert_eq!(processor.process("user at sign example"), "user@example");
    }

    #[test]
    fn test_newline_and_tab() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(
            processor.process("line one new line line two"),
            "line one\nline two"
        );
        assert_eq!(processor.process("col one tab col two"), "col one\tcol two");
    }
}
