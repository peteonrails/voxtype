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
    /// Whether smart auto-submit is enabled
    smart_auto_submit: bool,
    /// Pre-compiled regex for submit trigger detection
    submit_re: Regex,
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

        // Use (?:^|\s) instead of \b so that hyphenated forms like "pre-submit"
        // do not trigger: a hyphen satisfies \b but not (?:^|\s).
        let submit_re = Regex::new(r"(?i)(?:^|\s)submit[.!?,;]*\s*$")
            .expect("BUG: submit regex is a compile-time constant and must be valid");

        Self {
            spoken_punctuation: config.spoken_punctuation,
            replacements,
            smart_auto_submit: config.smart_auto_submit,
            submit_re,
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

    /// Check if text ends with the submit trigger word.
    ///
    /// Returns `(stripped_text, should_submit)`. Handles trailing punctuation (e.g.,
    /// "submit." from spoken punctuation) and is case-insensitive.
    ///
    /// `cli_override` allows the caller to force enable (`Some(true)`) or disable
    /// (`Some(false)`) detection, overriding the config value. `None` uses the config.
    pub fn detect_submit(&self, text: &str, cli_override: Option<bool>) -> (String, bool) {
        let enabled = cli_override.unwrap_or(self.smart_auto_submit);
        if !enabled {
            return (text.to_string(), false);
        }

        // Match "submit" preceded by start-of-string or whitespace (not hyphens),
        // optionally followed by punctuation. Leading whitespace in the match is
        // consumed by replace(); trim_end() cleans any remaining trailing space.
        if self.submit_re.is_match(text) {
            let stripped = self.submit_re.replace(text, "").trim_end().to_string();
            (stripped, true)
        } else {
            (text.to_string(), false)
        }
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
            smart_auto_submit: false,
        }
    }

    fn make_config_with_submit(spoken_punctuation: bool) -> TextConfig {
        TextConfig {
            spoken_punctuation,
            replacements: HashMap::new(),
            smart_auto_submit: true,
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

    #[test]
    fn test_detect_submit_basic() {
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        let (text, submit) = processor.detect_submit("hello world submit", None);
        assert_eq!(text, "hello world");
        assert!(submit);
    }

    #[test]
    fn test_detect_submit_with_period() {
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        // spoken punctuation may add a period after "submit"
        let (text, submit) = processor.detect_submit("hello world submit.", None);
        assert_eq!(text, "hello world");
        assert!(submit);
    }

    #[test]
    fn test_detect_submit_with_exclamation() {
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        let (text, submit) = processor.detect_submit("hello world submit!", None);
        assert_eq!(text, "hello world");
        assert!(submit);
    }

    #[test]
    fn test_detect_submit_uppercase() {
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        let (text, submit) = processor.detect_submit("SUBMIT", None);
        assert_eq!(text, "");
        assert!(submit);
    }

    #[test]
    fn test_detect_submit_in_middle_no_match() {
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        let (text, submit) = processor.detect_submit("Submit this please", None);
        assert_eq!(text, "Submit this please");
        assert!(!submit);
    }

    #[test]
    fn test_detect_submit_partial_word_no_match() {
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        let (text, submit) = processor.detect_submit("submitted", None);
        assert_eq!(text, "submitted");
        assert!(!submit);
    }

    #[test]
    fn test_detect_submit_hyphenated_prefix_no_match() {
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        // "pre-submit" ends with "submit" but hyphen is not a word boundary we
        // accept: saying "I need to pre-submit" should not fire auto-submit.
        let (text, submit) = processor.detect_submit("I need to pre-submit", None);
        assert_eq!(text, "I need to pre-submit");
        assert!(!submit);
    }

    #[test]
    fn test_detect_submit_disabled() {
        let config = make_config(false, &[]);
        let processor = TextProcessor::new(&config);

        let (text, submit) = processor.detect_submit("hello world submit", None);
        assert_eq!(text, "hello world submit");
        assert!(!submit);
    }

    #[test]
    fn test_detect_submit_cli_override_enable() {
        // Config has smart_auto_submit=false, but CLI forces it on
        let config = make_config(false, &[]);
        let processor = TextProcessor::new(&config);

        let (text, submit) = processor.detect_submit("hello world submit", Some(true));
        assert_eq!(text, "hello world");
        assert!(submit);
    }

    #[test]
    fn test_detect_submit_cli_override_disable() {
        // Config has smart_auto_submit=true, but CLI forces it off
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        let (text, submit) = processor.detect_submit("hello world submit", Some(false));
        assert_eq!(text, "hello world submit");
        assert!(!submit);
    }
}
