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
    /// Whether filler-word filtering is enabled
    filter_filler_words: bool,
    /// Pre-compiled regex matching any configured filler word.
    /// `None` when the filter is disabled or the list is empty so the hot
    /// path can early-out without touching regex.
    filler_re: Option<Regex>,
    /// Pre-compiled regex matching duplicate spaces left behind after
    /// removing fillers. Compiled once even when the filter is off so
    /// rebuilding the processor stays cheap.
    filler_space_re: Regex,
    /// Pre-compiled regex matching " ," / " ." / " ;" / " ?" etc. left
    /// behind when a filler precedes attached punctuation.
    filler_punct_re: Regex,
    /// Pre-compiled regex matching duplicated punctuation like ", ," that
    /// can appear after removing back-to-back fillers around commas.
    filler_dup_punct_re: Regex,
    /// Pre-compiled regex matching a connector punctuation (",;:") that ends
    /// up directly before a sentence terminator (".!?") after filler removal,
    /// e.g. "hello world, uh." -> "hello world,." -> "hello world.".
    filler_connector_before_term_re: Regex,
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

        // Build a single alternation of all filler words. Word boundaries
        // (\b) ensure "um" is removed without touching "umbrella" or "summer".
        let filler_re = if config.filter_filler_words && !config.filler_words.is_empty() {
            let alternation = config
                .filler_words
                .iter()
                .filter(|w| !w.trim().is_empty())
                .map(|w| regex::escape(w.trim()))
                .collect::<Vec<_>>()
                .join("|");
            if alternation.is_empty() {
                None
            } else {
                let pattern = format!(r"(?i)\b(?:{})\b", alternation);
                Regex::new(&pattern).ok()
            }
        } else {
            None
        };

        let filler_space_re = Regex::new(r" {2,}")
            .expect("BUG: whitespace regex is a compile-time constant and must be valid");
        let filler_punct_re = Regex::new(r" +([,.;:!?])")
            .expect("BUG: punctuation regex is a compile-time constant and must be valid");
        let filler_dup_punct_re = Regex::new(r"([,;:])(\s*[,;:])+").expect(
            "BUG: duplicate-punctuation regex is a compile-time constant and must be valid",
        );
        let filler_connector_before_term_re = Regex::new(r"[,;:]+(\s*)([.!?])").expect(
            "BUG: connector-before-terminator regex is a compile-time constant and must be valid",
        );

        Self {
            spoken_punctuation: config.spoken_punctuation,
            replacements,
            smart_auto_submit: config.smart_auto_submit,
            submit_re,
            filter_filler_words: config.filter_filler_words,
            filler_re,
            filler_space_re,
            filler_punct_re,
            filler_dup_punct_re,
            filler_connector_before_term_re,
        }
    }

    /// Process text by applying all enabled transformations
    pub fn process(&self, text: &str) -> String {
        let mut result = text.to_string();

        // Filter filler words first, on the raw transcription. Running before
        // word_replacements lets users override the default list (e.g. by
        // mapping "um" to itself) without needing to disable the filter.
        if self.filter_filler_words {
            result = self.apply_filler_filter(&result);
        }

        // Apply replacements first so phrases containing spoken punctuation words
        // (e.g. "slash pr" → "/pr") match before those words are converted to
        // punctuation characters.
        if !self.replacements.is_empty() {
            result = self.apply_replacements(&result);
        }

        if self.spoken_punctuation {
            result = self.apply_spoken_punctuation(&result);
        }

        // Apply replacements again to catch patterns that only became matchable
        // after spoken punctuation conversion.
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
            // After stripping "submit", also remove trailing connector punctuation
            // (commas, semicolons) that would otherwise dangle at end of text.
            // Sentence-ending punctuation (. ! ?) is preserved.
            let stripped = self
                .submit_re
                .replace(text, "")
                .trim_end_matches(|c: char| c.is_whitespace() || c == ',' || c == ';')
                .to_string();
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

    /// Remove filler words and clean up the punctuation/whitespace they leave
    /// behind. Examples:
    ///   "Well, um, I think"  -> "Well, I think"
    ///   "uh hello"           -> "hello"
    ///   "I think, uh."       -> "I think."
    ///   "um uh hello"        -> "hello"
    fn apply_filler_filter(&self, text: &str) -> String {
        let Some(re) = &self.filler_re else {
            return text.to_string();
        };

        // Replace each filler with a single space so the input
        // "um, hello" becomes " , hello" and we can fold whitespace below.
        let mut result = re.replace_all(text, " ").into_owned();

        // Collapse "<space><punct>" to "<punct>" so " , hello" -> ", hello".
        result = self.filler_punct_re.replace_all(&result, "$1").into_owned();

        // Collapse runs like ",," or ", ," that appear when fillers sit
        // between commas/semicolons/colons.
        result = self
            .filler_dup_punct_re
            .replace_all(&result, "$1")
            .into_owned();

        // A connector ("," ";" ":") sitting directly before a sentence
        // terminator (".!?") is dropped: "hello world, uh." starts as
        // "hello world,." and should become "hello world.".
        result = self
            .filler_connector_before_term_re
            .replace_all(&result, "$2")
            .into_owned();

        // Collapse multiple spaces left behind to a single space.
        result = self.filler_space_re.replace_all(&result, " ").into_owned();

        // Trim leading/trailing whitespace and dangling connector punctuation
        // produced when fillers appeared at the start/end of the utterance.
        result
            .trim()
            .trim_start_matches([',', ';', ':'])
            .trim_start()
            .trim_end_matches([',', ';', ':'])
            .to_string()
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
            ..Default::default()
        }
    }

    fn make_config_with_submit(spoken_punctuation: bool) -> TextConfig {
        TextConfig {
            spoken_punctuation,
            replacements: HashMap::new(),
            smart_auto_submit: true,
            ..Default::default()
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
    fn test_pipeline_spoken_punctuation_then_detect_submit() {
        // Simulates the full daemon pipeline: user says "hello world comma submit"
        // process() converts "comma" → "," then detect_submit() strips ", submit"
        let config = TextConfig {
            spoken_punctuation: true,
            replacements: HashMap::new(),
            smart_auto_submit: true,
            ..Default::default()
        };
        let processor = TextProcessor::new(&config);

        let processed = processor.process("hello world comma submit");
        let (text, submit) = processor.detect_submit(&processed, None);
        assert_eq!(text, "hello world");
        assert!(submit);
    }

    #[test]
    fn test_pipeline_spoken_punctuation_period_then_detect_submit() {
        // Simulates: user says "hello world period submit"
        // process() converts "period" → "." then detect_submit() strips " submit"
        // The period on the prior sentence is preserved.
        let config = TextConfig {
            spoken_punctuation: true,
            replacements: HashMap::new(),
            smart_auto_submit: true,
            ..Default::default()
        };
        let processor = TextProcessor::new(&config);

        let processed = processor.process("hello world period submit");
        let (text, submit) = processor.detect_submit(&processed, None);
        assert_eq!(text, "hello world.");
        assert!(submit);
    }

    #[test]
    fn test_detect_submit_strips_trailing_comma() {
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        // "hello world, submit" - spoken punctuation may produce a comma before
        // "submit"; the dangling comma should be stripped from the result.
        let (text, submit) = processor.detect_submit("hello world, submit", None);
        assert_eq!(text, "hello world");
        assert!(submit);
    }

    #[test]
    fn test_detect_submit_strips_trailing_semicolon() {
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        let (text, submit) = processor.detect_submit("hello world; submit", None);
        assert_eq!(text, "hello world");
        assert!(submit);
    }

    #[test]
    fn test_detect_submit_preserves_trailing_period() {
        let config = make_config_with_submit(false);
        let processor = TextProcessor::new(&config);

        // A sentence ending in ". submit" should keep the period on the prior sentence.
        let (text, submit) = processor.detect_submit("hello world. submit", None);
        assert_eq!(text, "hello world.");
        assert!(submit);
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

    #[test]
    fn test_replacements_match_spoken_words_before_punctuation() {
        // "slash pr" should match the replacement before "slash" is converted to "/"
        let config = make_config(true, &[("slash pr", "/pr")]);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("slash pr"), "/pr");
    }

    #[test]
    fn test_replacements_with_multiple_spoken_punctuation_words() {
        // "dash dash" should match the replacement before each "dash" is converted to "-"
        let config = make_config(true, &[("dash dash", "--")]);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("dash dash"), "--");
    }

    fn make_filler_config(enabled: bool, words: Option<Vec<&str>>) -> TextConfig {
        let filler_words = match words {
            Some(words) => words.into_iter().map(String::from).collect(),
            None => TextConfig::default().filler_words,
        };
        TextConfig {
            filter_filler_words: enabled,
            filler_words,
            ..Default::default()
        }
    }

    #[test]
    fn test_filler_filter_enabled_by_default() {
        // Filler-word filtering ships on by default. Existing users who want
        // the old behavior must opt out via `filter_filler_words = false`.
        let config = TextConfig::default();
        assert!(config.filter_filler_words);

        let processor = TextProcessor::new(&config);
        assert_eq!(processor.process("um hello"), "hello");
    }

    #[test]
    fn test_filler_filter_default_list() {
        // Sanity-check the documented default list.
        let config = TextConfig::default();
        assert_eq!(
            config.filler_words,
            vec!["uh", "um", "er", "ah", "eh", "hmm", "hm", "mm", "mhm"]
        );
    }

    #[test]
    fn test_filler_filter_enabled_basic() {
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("um hello world"), "hello world");
        assert_eq!(processor.process("hello uh world"), "hello world");
        assert_eq!(processor.process("hello world um"), "hello world");
    }

    #[test]
    fn test_filler_filter_case_insensitive() {
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("UM hello"), "hello");
        assert_eq!(processor.process("Um hello"), "hello");
        assert_eq!(processor.process("Hmm I see"), "I see");
    }

    #[test]
    fn test_filler_filter_respects_word_boundaries() {
        // The classic edge case: "um" inside "umbrella" must not be removed.
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("umbrella"), "umbrella");
        assert_eq!(processor.process("an umbrella"), "an umbrella");
        assert_eq!(processor.process("summer"), "summer");
        assert_eq!(processor.process("hummingbird"), "hummingbird");
        assert_eq!(processor.process("erase the file"), "erase the file");
    }

    #[test]
    fn test_filler_filter_punctuation_cleanup_mid_sentence() {
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        // The canonical example from the brief.
        assert_eq!(processor.process("Well, um, I think"), "Well, I think");
    }

    #[test]
    fn test_filler_filter_punctuation_cleanup_start() {
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("um, hello world"), "hello world");
        assert_eq!(processor.process("uh hello world"), "hello world");
    }

    #[test]
    fn test_filler_filter_punctuation_cleanup_end() {
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("hello world, um"), "hello world");
        assert_eq!(processor.process("hello world, uh."), "hello world.");
    }

    #[test]
    fn test_filler_filter_back_to_back_fillers() {
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("um uh hello"), "hello");
        // Back-to-back fillers between commas collapse to a single comma:
        // "hello [um], [uh], world" -> "hello, world". This matches the
        // canonical "Well, um, I think" -> "Well, I think" treatment.
        assert_eq!(processor.process("hello um, uh, world"), "hello, world");
        assert_eq!(processor.process("um, uh, well"), "well");
    }

    #[test]
    fn test_filler_filter_preserves_sentence_punctuation() {
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        // Sentence-final punctuation must survive even when a filler sits
        // immediately before it.
        assert_eq!(processor.process("hello um."), "hello.");
        assert_eq!(processor.process("hello um!"), "hello!");
        assert_eq!(processor.process("hello um?"), "hello?");
    }

    #[test]
    fn test_filler_filter_custom_list() {
        // Override the default list. "um" should now be preserved while
        // "like" and "you know" are stripped.
        let config = make_filler_config(true, Some(vec!["like", "you know"]));
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("um like hello"), "um hello");
        assert_eq!(processor.process("hello you know world"), "hello world");
    }

    #[test]
    fn test_filler_filter_empty_list_is_noop() {
        // An empty list with the flag enabled should leave text untouched
        // rather than panic when building the regex.
        let config = make_filler_config(true, Some(vec![]));
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("um hello"), "um hello");
    }

    #[test]
    fn test_filler_filter_runs_before_replacements() {
        // If a user maps "uh" to "uhhh" via word_replacements, the filler
        // filter strips "uh" first, so the replacement sees clean input.
        let mut config = make_filler_config(true, None);
        config
            .replacements
            .insert("hello".to_string(), "HELLO".to_string());
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("um hello uh world"), "HELLO world");
    }

    #[test]
    fn test_filler_filter_with_spoken_punctuation() {
        // Pipeline interaction: filler is removed first, then "period" -> ".".
        let mut config = make_filler_config(true, None);
        config.spoken_punctuation = true;
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("well um I think period"), "well I think.");
    }
}
