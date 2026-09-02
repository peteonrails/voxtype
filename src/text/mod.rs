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

/// Default filler entries that are ordinary vocabulary in a given language.
///
/// Kept deliberately short: only words that are common enough to matter and
/// unambiguous enough to be sure about. `um` is the Portuguese masculine
/// indefinite article and the German preposition "around"/"at"; filtering it
/// silently mangles ordinary sentences in both.
fn filler_collisions(language: Option<&str>) -> &'static [&'static str] {
    match language.map(|l| l.split(['-', '_']).next().unwrap_or(l)) {
        Some("pt") => &["um"],
        Some("de") => &["um"],
        _ => &[],
    }
}

impl TextProcessor {
    /// Create a new text processor from configuration
    /// Build a processor for a language-agnostic context.
    ///
    /// Prefer [`new_for_language`](Self::new_for_language) where the active
    /// engine's language is known, so filler filtering can avoid words that
    /// are ordinary vocabulary rather than disfluencies.
    pub fn new(config: &TextConfig) -> Self {
        Self::new_for_language(config, None)
    }

    /// Build a processor that knows which language it is filtering.
    ///
    /// The default filler list is English disfluencies, and two of them are
    /// real words elsewhere: `um` is the masculine indefinite article in
    /// Portuguese and a common preposition in German. Filtering those turned
    /// "Faça um commit" into "Faça commit" with no way for the user to see
    /// why (#566).
    ///
    /// Only the built-in default is narrowed. A user who lists filler words
    /// explicitly gets exactly what they asked for.
    pub fn new_for_language(config: &TextConfig, language: Option<&str>) -> Self {
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
        let effective_fillers: Vec<String> =
            if config.filler_words == crate::config::text::default_filler_words() {
                let collisions = filler_collisions(language);
                config
                    .filler_words
                    .iter()
                    .filter(|w| !collisions.contains(&w.to_lowercase().as_str()))
                    .cloned()
                    .collect()
            } else {
                config.filler_words.clone()
            };

        let filler_re = if config.filter_filler_words && !effective_fillers.is_empty() {
            let alternation = effective_fillers
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
        let (converted, sentence_starts) = convert_spoken_punctuation(text);
        let capitalised = capitalise_after_terminators(&converted, &sentence_starts);

        clean_punctuation_spacing(&capitalised)
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

/// Where a converted symbol sits in relation to the words around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attachment {
    /// `clean_punctuation_spacing` decides the spacing.
    Free,
    /// Attaches to the word that follows, like an opening quote.
    Opening,
    /// Attaches to the word before, like a closing quote.
    Closing,
    /// Ends a sentence: attaches to the word before, and the next letter is
    /// capitalised.
    Terminator,
}

/// Spoken phrases and the symbols they convert to.
///
/// Order matters where one phrase is a prefix of another: the longer phrase
/// must come first, or only its prefix is matched.
const PUNCTUATION_MAP: &[(&str, &str, Attachment)] = {
    use Attachment::{Closing, Free, Opening, Terminator};

    &[
        // Multi-word phrases first
        ("full stop", ".", Terminator),
        ("question mark", "?", Terminator),
        ("exclamation mark", "!", Terminator),
        ("exclamation point", "!", Terminator),
        ("open parenthesis", "(", Opening),
        ("close parenthesis", ")", Closing),
        ("open paren", "(", Opening),
        ("close paren", ")", Closing),
        ("open bracket", "[", Opening),
        ("close bracket", "]", Closing),
        ("open brace", "{", Opening),
        ("close brace", "}", Closing),
        ("at sign", "@", Free),
        ("at symbol", "@", Free),
        ("dollar sign", "$", Free),
        ("percent sign", "%", Free),
        ("plus sign", "+", Free),
        ("equals sign", "=", Free),
        ("forward slash", "/", Free),
        ("single quote", "'", Free),
        ("double quote", "\"", Free),
        ("new paragraph", "\n\n", Free),
        ("new line", "\n", Free),
        // Single words
        ("period", ".", Terminator),
        ("comma", ",", Free),
        ("colon", ":", Free),
        ("semicolon", ";", Free),
        ("quote", "\"", Opening),
        ("unquote", "\"", Closing),
        ("dash", "-", Free),
        ("hyphen", "-", Free),
        ("underscore", "_", Free),
        ("hash", "#", Free),
        ("hashtag", "#", Free),
        ("percent", "%", Free),
        ("ampersand", "&", Free),
        ("asterisk", "*", Free),
        ("plus", "+", Free),
        ("equals", "=", Free),
        ("slash", "/", Free),
        ("backslash", "\\", Free),
        ("pipe", "|", Free),
        ("tilde", "~", Free),
        ("backtick", "`", Free),
        ("tab", "\t", Free),
    ]
};

/// Characters a sentence can open with before its first letter.
const SENTENCE_OPENERS: [char; 5] = ['"', '\'', '(', '[', '{'];

type SpokenMatcher = (Regex, HashMap<String, (&'static str, Attachment)>);

/// The regex that finds every spoken phrase, and the symbol and attachment for
/// each one.
///
/// The optional punctuation around a phrase is captured rather than baked in,
/// so one pattern serves every attachment and the caller decides what to keep.
fn spoken_matcher() -> Option<&'static SpokenMatcher> {
    static MATCHER: std::sync::OnceLock<Option<SpokenMatcher>> = std::sync::OnceLock::new();

    MATCHER
        .get_or_init(|| {
            let phrases = PUNCTUATION_MAP
                .iter()
                .map(|(phrase, _, _)| regex::escape(phrase))
                .collect::<Vec<_>>()
                .join("|");
            let pattern = format!(
                r"(?i)(?P<lead>[,;:]?[ \t]*)\b(?P<phrase>{phrases})\b[.,!?;:]?(?P<trail>[ \t]*)"
            );
            let symbols = PUNCTUATION_MAP
                .iter()
                .map(|(phrase, symbol, attachment)| (phrase.to_lowercase(), (*symbol, *attachment)))
                .collect();

            Some((Regex::new(&pattern).ok()?, symbols))
        })
        .as_ref()
}

/// Convert every spoken punctuation phrase, and report where in the result a
/// sentence terminator was inserted.
///
/// Engines that punctuate for themselves, such as Parakeet, decorate the spoken
/// command words too: "full stop" arrives as "Full stop.", and a phrase at a
/// clause boundary picks up a connector, as in "world, full stop." or
/// "..., unquote,". Each match therefore takes in one punctuation character
/// after the phrase, and for a symbol that attaches leftwards one connector
/// before it, so the engine's decoration of the spoken word does not survive
/// alongside the symbol the user asked for.
///
/// Every phrase is matched in one pass. Converting them one phrase at a time
/// would let a symbol inserted by an earlier phrase be eaten as decoration by a
/// later one, so "unquote full stop" would lose its full stop.
fn convert_spoken_punctuation(text: &str) -> (String, Vec<usize>) {
    let Some((regex, symbols)) = spoken_matcher() else {
        return (text.to_string(), Vec::new());
    };

    let mut result = String::with_capacity(text.len());
    let mut sentence_starts = Vec::new();
    let mut converted_to = 0;

    for captures in regex.captures_iter(text) {
        let (Some(whole), Some(phrase)) = (captures.get(0), captures.name("phrase")) else {
            continue;
        };
        let Some((symbol, attachment)) = symbols.get(&phrase.as_str().to_lowercase()) else {
            continue;
        };

        result.push_str(&text[converted_to..whole.start()]);
        if !matches!(attachment, Attachment::Closing | Attachment::Terminator) {
            result.push_str(captures.name("lead").map_or("", |lead| lead.as_str()));
        }
        result.push_str(symbol);
        if matches!(attachment, Attachment::Terminator) {
            sentence_starts.push(result.len());
        }
        if !matches!(attachment, Attachment::Opening) {
            result.push_str(captures.name("trail").map_or("", |trail| trail.as_str()));
        }
        converted_to = whole.end();
    }
    result.push_str(&text[converted_to..]);

    (result, sentence_starts)
}

/// Capitalise the first letter after each inserted sentence terminator.
///
/// The engine had no way to know a sentence ended where the user dictated one,
/// so its casing for the following word is wrong. Only text after a terminator
/// this conversion inserted is touched.
fn capitalise_after_terminators(text: &str, sentence_starts: &[usize]) -> String {
    let mut letters = std::collections::HashSet::new();
    for &start in sentence_starts {
        let Some(rest) = text.get(start..) else {
            continue;
        };
        for (offset, character) in rest.char_indices() {
            if character.is_whitespace() || SENTENCE_OPENERS.contains(&character) {
                continue;
            }
            if character.is_lowercase() {
                letters.insert(start + offset);
            }
            break;
        }
    }

    if letters.is_empty() {
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len());
    for (index, character) in text.char_indices() {
        if letters.contains(&index) {
            result.extend(character.to_uppercase());
        } else {
            result.push(character);
        }
    }

    result
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

    /// #566: "um" is the masculine indefinite article in Portuguese, so the
    /// English filler list silently turned "Faça um commit" into
    /// "Faça commit" with nothing to tell the user why.
    #[test]
    fn portuguese_keeps_um() {
        let config = TextConfig {
            filter_filler_words: true,
            ..Default::default()
        };
        let pt = TextProcessor::new_for_language(&config, Some("pt"));
        assert_eq!(pt.process("Faça um commit"), "Faça um commit");

        // German uses it as a preposition; same protection.
        let de = TextProcessor::new_for_language(&config, Some("de"));
        assert_eq!(de.process("Gehen wir um die Ecke"), "Gehen wir um die Ecke");
    }

    /// The protection must not weaken English, where "um" is the canonical
    /// filler and removing it is the whole point of the feature.
    #[test]
    fn english_still_drops_um() {
        let config = TextConfig {
            filter_filler_words: true,
            ..Default::default()
        };
        let en = TextProcessor::new_for_language(&config, Some("en"));
        assert_eq!(en.process("so um let us commit"), "so let us commit");

        // Unknown or auto-detected language keeps the historical behaviour.
        let unknown = TextProcessor::new_for_language(&config, None);
        assert_eq!(unknown.process("so um let us commit"), "so let us commit");
    }

    /// An explicit list is the user's instruction, not our default, so it is
    /// honoured verbatim even where it collides.
    #[test]
    fn explicit_filler_list_is_not_narrowed() {
        let config = TextConfig {
            filter_filler_words: true,
            filler_words: vec!["um".to_string()],
            ..Default::default()
        };
        let pt = TextProcessor::new_for_language(&config, Some("pt"));
        assert_eq!(pt.process("Faça um commit"), "Faça commit");
    }

    /// Regional tags must resolve to their base language.
    #[test]
    fn regional_variants_resolve_to_base_language() {
        assert_eq!(filler_collisions(Some("pt-BR")), &["um"]);
        assert_eq!(filler_collisions(Some("pt_PT")), &["um"]);
        assert!(filler_collisions(Some("en-US")).is_empty());
    }
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
    fn test_spoken_punctuation_absorbs_engine_punctuation() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        // Raw Parakeet output for "hello world full stop quote this is a
        // really lovely piece of software unquote full stop". The engine
        // punctuates and capitalises the spoken command words itself.
        assert_eq!(
            processor.process(
                "Hello World Full Stop. Quote This is a really lovely piece of software, unquote. Full stop."
            ),
            "Hello World. \"This is a really lovely piece of software\"."
        );
    }

    #[test]
    fn test_spoken_punctuation_absorbs_a_connector_before_a_terminator() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        // The same dictation as the previous test, transcribed on a later run.
        // Parakeet put its comma at the clause boundary before each spoken
        // command word this time, and left "this" lowercase because it did not
        // know a sentence ended there.
        assert_eq!(
            processor.process(
                "Hello world, full stop. Quote, this is a really lovely piece of software, unquote, full stop."
            ),
            "Hello world. \"This is a really lovely piece of software\"."
        );
    }

    #[test]
    fn test_spoken_terminator_capitalises_the_next_sentence() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(
            processor.process("one thing period another thing"),
            "one thing. Another thing"
        );
        // Casing elsewhere in the engine's text is left alone.
        assert_eq!(
            processor.process("one thing. another thing"),
            "one thing. another thing"
        );
    }

    #[test]
    fn test_spoken_punctuation_period_absorbs_engine_punctuation() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("Stop period. Next"), "Stop. Next");
    }

    #[test]
    fn test_spoken_punctuation_words_in_sequence_keep_both_symbols() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        // Each phrase absorbs only the character touching it, so dictating two
        // punctuation words in a row still produces both symbols.
        assert_eq!(processor.process("full stop comma"), ".,");
        assert_eq!(processor.process("Full stop. Comma."), ".,");
    }

    #[test]
    fn test_spoken_quotes_attach_to_the_quoted_words() {
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(
            processor.process("he said quote hello there unquote and left"),
            "he said \"hello there\" and left"
        );
        // "double quote" is a longer phrase and still converts on its own.
        assert_eq!(processor.process("a double quote b"), "a \" b");
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
