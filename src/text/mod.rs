//! Text processing module
//!
//! Provides post-transcription text transformations including:
//! - Spoken punctuation conversion (e.g., "period" → ".")
//! - Custom word replacements

pub mod diff;
pub mod disfluency;
#[cfg(feature = "itn")]
pub mod itn;
#[cfg(feature = "punctuation-casing")]
pub mod punctuation;
pub mod restarts;
pub mod sentences;
pub mod vocabulary;

use crate::config::TextConfig;
use regex::Regex;
use std::collections::HashMap;
use vocabulary::Vocabulary;

/// Text processor that applies transformations to transcribed text
pub struct TextProcessor {
    disfluency_tagger: Option<disfluency::DisfluencyTagger>,
    /// Resident classifier used only for engines with plain CTC output.
    #[cfg(feature = "punctuation-casing")]
    punctuation_caser: Option<punctuation::PunctuationCaser>,
    /// Whether spoken punctuation is enabled
    spoken_punctuation: bool,
    /// Marks that take no space before them, for the active language
    tight_punctuation: &'static [char],
    /// Custom word replacements (lowercase key → replacement value)
    replacements: HashMap<String, String>,
    /// Whether smart auto-submit is enabled
    smart_auto_submit: bool,
    /// Pre-compiled regex for submit trigger detection
    submit_re: Regex,
    /// Whether filler-word filtering is enabled
    filter_filler_words: bool,
    /// Whether restart/stutter collapsing is enabled
    collapse_restarts: bool,
    /// Whether premature sentence breaks are repaired
    repair_sentence_breaks: bool,
    /// Whether dictated quantities are converted to written form.
    /// Only read when the `itn` feature is compiled in.
    #[cfg_attr(not(feature = "itn"), allow(dead_code))]
    format_numbers: bool,
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

/// Build a filler pattern that also matches a held pronunciation.
///
/// Engines transcribe a drawn-out filler with the last sound repeated -
/// "uhhhh", "ummm", "hmmm" - and an exact-word pattern misses every one of
/// them, which is the common case: a clipped "uh" barely registers in a
/// transcript, a held one is what litters it.
///
/// Only the final character is allowed to repeat, so "um" matches "ummm"
/// without "umbrella" or "summer" ever matching.
fn elongated_filler_pattern(word: &str) -> String {
    // "err" is an ordinary English verb, so "er" keeps matching exactly.
    if word.eq_ignore_ascii_case("er") {
        return regex::escape(word);
    }
    let mut chars = word.chars().collect::<Vec<_>>();
    match chars.pop() {
        Some(last) if last.is_alphabetic() => {
            let head: String = chars.into_iter().collect();
            format!(
                "{}{}+",
                regex::escape(&head),
                regex::escape(&last.to_string())
            )
        }
        _ => regex::escape(word),
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
        Self::build(config, language, false)
    }

    /// Build a processor for finished output from the active transcription
    /// configuration. Punctuation restoration is activated only when both
    /// the user opted in and that exact engine configuration emits plain CTC
    /// output (for example SenseVoice with its own ITN disabled).
    pub fn new_for_config(
        config: &TextConfig,
        language: Option<&str>,
        root: &crate::config::Config,
    ) -> Self {
        Self::build(config, language, root.needs_punctuation_casing())
    }

    fn build(config: &TextConfig, language: Option<&str>, engine_needs_punctuation: bool) -> Self {
        #[cfg(not(feature = "punctuation-casing"))]
        if config.punctuation_casing && engine_needs_punctuation {
            tracing::warn!(
                "Punctuation/casing was requested, but this build does not include the \
                 punctuation-casing feature; keeping engine output unchanged"
            );
        }

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
                .map(|w| elongated_filler_pattern(w.trim()))
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
        // Built from the language's tight set, not a fixed class: in French
        // a space before ? ! : ; is correct, so removing filler must not
        // close it up either.
        let tight = tight_punctuation(language);
        let filler_class: String = ",.;:!?".chars().filter(|c| tight.contains(c)).collect();
        let filler_punct_re = Regex::new(&format!(" +([{}])", filler_class))
            .expect("BUG: punctuation class is built from a known-safe char set");
        let filler_dup_punct_re = Regex::new(r"([,;:])(\s*[,;:])+").expect(
            "BUG: duplicate-punctuation regex is a compile-time constant and must be valid",
        );
        let filler_connector_before_term_re = Regex::new(r"[,;:]+(\s*)([.!?])").expect(
            "BUG: connector-before-terminator regex is a compile-time constant and must be valid",
        );

        let disfluency_tagger = if config.lard_disfluency {
            if language.and_then(|l| l.split(['-', '_']).next()) == Some("en") {
                let path = config
                    .lard_model_path
                    .clone()
                    .unwrap_or_else(crate::setup::lard::model_path);
                match std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        disfluency::DisfluencyTagger::from_bytes(&bytes).map_err(|e| e.to_string())
                    }) {
                    Ok(model) => Some(model),
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error,
                            "LARD cleanup unavailable; preserving text. Run voxtype setup lard --source <verified-model.vxdf>");
                        None
                    }
                }
            } else {
                tracing::warn!(
                    "LARD cleanup requires an explicit English language; preserving text"
                );
                None
            }
        } else {
            None
        };

        Self {
            disfluency_tagger,
            #[cfg(feature = "punctuation-casing")]
            punctuation_caser: load_punctuation_caser(
                config.punctuation_casing,
                engine_needs_punctuation,
                language,
            ),
            spoken_punctuation: config.spoken_punctuation,
            tight_punctuation: tight_punctuation(language),
            replacements,
            smart_auto_submit: config.smart_auto_submit,
            submit_re,
            filter_filler_words: config.filter_filler_words,
            collapse_restarts: config.collapse_restarts,
            repair_sentence_breaks: config.repair_sentence_breaks,
            format_numbers: config.format_numbers,
            filler_re,
            filler_space_re,
            filler_punct_re,
            filler_dup_punct_re,
            filler_connector_before_term_re,
        }
    }

    /// Process text by applying all enabled transformations
    pub fn process(&self, text: &str) -> String {
        self.process_with_vocabulary(text, None)
    }

    /// Process text with the vocabulary selected by the active profile.
    pub fn process_with_vocabulary(&self, text: &str, vocabulary: Option<&Vocabulary>) -> String {
        let mut result = text.to_string();

        // Vocabulary is the first cleanup stage so every later stage sees the
        // canonical spelling. User replacements still run at the end and can
        // deliberately override a vocabulary result.
        if let Some(vocabulary) = vocabulary {
            result = vocabulary.correct(&result);
        }

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

        // Restart collapsing keys on sentence boundaries, so it has to run
        // after spoken punctuation has turned a dictated "period" into a real
        // ".". Running it earlier would make every boundary invisible for
        // anyone using spoken_punctuation. Filler removal has already gone,
        // so a filler mid-stutter ("the uh the point") cannot hide the repeat
        // either, and the replacements pass below still gets the last word.
        if let Some(tagger) = &self.disfluency_tagger {
            let tokens = tagger.tag(&result);
            let mut cleaned = String::with_capacity(result.len());
            let mut cursor = 0;
            for token in tokens.iter().filter(|token| token.disfluent) {
                cleaned.push_str(&result[cursor..token.start]);
                cursor = token.end;
                while result[cursor..].starts_with(char::is_whitespace) {
                    cursor += result[cursor..].chars().next().unwrap().len_utf8();
                }
            }
            cleaned.push_str(&result[cursor..]);
            // A model that labels everything must never erase a dictation.
            if !cleaned.trim().is_empty() {
                // Segment separators belong to the streaming protocol. Do not
                // trim them or adjacent fluent Final events will join words.
                result = cleaned;
            }
        }

        if self.collapse_restarts {
            result = restarts::collapse_restarts(&result);
        }

        // Plain CTC engines emit lowercase words without sentence marks. The
        // model is loaded only for those engines, so native punctuation from
        // Whisper, Parakeet TDT, Cohere, and remote APIs is never discarded.
        #[cfg(feature = "punctuation-casing")]
        if let Some(model) = &self.punctuation_caser {
            match model.restore(&result) {
                Ok(restored) => result = restored,
                Err(error) => tracing::warn!(
                    "Punctuation/casing failed; keeping the engine transcript unchanged: {error}"
                ),
            }
        }

        // Boundary repair runs after disfluency removal, so a restart that
        // straddled the bad boundary is already gone, and before number
        // formatting, whose own sentence splitting wants real sentences.
        if self.repair_sentence_breaks {
            result = sentences::repair(&result);
        }

        // Number formatting runs on text that is already de-disfluent and
        // punctuated, so its sentence-level quantity check sees real
        // sentences, and still ahead of the replacements pass below.
        #[cfg(feature = "itn")]
        if self.format_numbers {
            result = itn::apply(&result);
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
            ("open parenthesis", ATTACH_OPEN_PAREN),
            ("close parenthesis", ATTACH_CLOSE_PAREN),
            ("open paren", ATTACH_OPEN_PAREN),
            ("close paren", ATTACH_CLOSE_PAREN),
            ("open bracket", ATTACH_OPEN_BRACKET),
            ("close bracket", ATTACH_CLOSE_BRACKET),
            ("open brace", ATTACH_OPEN_BRACE),
            ("close brace", ATTACH_CLOSE_BRACE),
            ("at sign", ATTACH_AT),
            ("at symbol", ATTACH_AT),
            ("dollar sign", ATTACH_DOLLAR),
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
            ("hash", ATTACH_HASH),
            ("hashtag", ATTACH_HASH),
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
        result = clean_punctuation_spacing(&result, self.tight_punctuation);

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

        // Fluent streaming deltas carry their own separators. With no filler
        // there is nothing to clean up, including whitespace or punctuation.
        if !re.is_match(text) {
            return text.to_string();
        }

        // Mark each filler, remembering whether Whisper capitalized it: a
        // capitalized filler usually started a sentence, and its capital
        // belongs to the word that now starts it.
        let marked = re.replace_all(text, |caps: &regex::Captures| {
            if caps[0].starts_with(char::is_uppercase) {
                FILLER_UPPER.to_string()
            } else {
                FILLER_LOWER.to_string()
            }
        });

        // Then splice out each run of fillers ("um", "uh, um,") by looking
        // at the punctuation on either side of it as a whole.
        let mut result = remove_filler_runs(&marked);

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
        let cleaned = result
            .trim()
            .trim_start_matches([',', ';', ':'])
            .trim_start()
            .trim_end_matches([',', ';', ':']);
        if cleaned.is_empty() {
            return String::new();
        }
        // Only discard separators created by removing a filler, not the
        // original segment boundary carried by a streaming backend.
        let leading = &text[..text.len() - text.trim_start().len()];
        let trailing = &text[text.trim_end().len()..];
        format!("{leading}{cleaned}{trailing}")
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

#[cfg(feature = "punctuation-casing")]
fn load_punctuation_caser(
    enabled: bool,
    engine_needs_punctuation: bool,
    language: Option<&str>,
) -> Option<punctuation::PunctuationCaser> {
    if !enabled || !engine_needs_punctuation {
        return None;
    }
    if !punctuation::supports_language(language) {
        tracing::warn!(
            language = language.unwrap_or("unknown"),
            "Punctuation/casing is not available for this language; keeping engine output"
        );
        return None;
    }
    let model_dir = crate::config::Config::models_dir().join(punctuation::MODEL_DIR);
    match punctuation::PunctuationCaser::from_dir(&model_dir) {
        Ok(model) => {
            tracing::info!(path = %model_dir.display(), "Punctuation/casing model loaded");
            Some(model)
        }
        Err(error) => {
            tracing::warn!("Punctuation/casing disabled: {error}");
            None
        }
    }
}

/// Stand-ins for a removed filler while its surroundings are cleaned up.
/// Private-use code points, so they cannot collide with transcribed text.
const FILLER_UPPER: char = '\u{E000}';
const FILLER_LOWER: char = '\u{E001}';

fn is_filler_mark(c: char) -> bool {
    c == FILLER_UPPER || c == FILLER_LOWER
}

fn is_connector(c: char) -> bool {
    matches!(c, ',' | ';' | ':')
}

fn is_terminator(c: char) -> bool {
    matches!(c, '.' | '!' | '?')
}

/// True when `left` ends a sentence, allowing a closing quote or bracket
/// after the terminator: `month.` or `"Done!"`.
fn ends_sentence(left: &str) -> bool {
    let trimmed = left.trim_end_matches(['"', '\'', ')', ']', '\u{201D}', '\u{2019}']);
    trimmed.ends_with(is_terminator)
}

/// The text of the current sentence before a filler run: everything after
/// the last sentence terminator in `left`.
fn sentence_before(left: &str) -> &str {
    match left.rfind(is_terminator) {
        Some(i) => &left[i + 1..],
        None => left,
    }
}

/// The text of the current sentence after a filler run, up to and
/// including its terminator.
fn sentence_after(right: &str) -> &str {
    match right.find(is_terminator) {
        Some(i) => &right[..i],
        None => right,
    }
}

/// Remove every run of filler marks from `marked`, fixing the punctuation
/// each run leaves behind.
///
/// A run is one or more fillers separated only by spaces and connectors
/// (`um`, `uh, um,`). What happens to the punctuation depends on both sides:
///
/// - Sentence start: connectors after the run go, and a capitalized filler
///   hands its capital to the next word (`month. Um, starting` becomes
///   `month. Starting`).
/// - Before a terminator: the connector before the run goes
///   (`world, uh.` becomes `world.`).
/// - Between two commas mid-sentence: Whisper bracketed the hesitation
///   with them, so both go (`is, um, more` becomes `is more`). They stay as
///   one when they did real work: after a lone introductory word
///   (`Well, um, I think`) or in a sentence that has other commas, which
///   is usually a list (`eggs, um, milk, and bread`).
/// - A single connector on one side stays (`hello um, world` keeps its
///   comma).
fn remove_filler_runs(marked: &str) -> String {
    let chars: Vec<char> = marked.chars().collect();
    let mut out = String::with_capacity(marked.len());
    let mut i = 0;

    while i < chars.len() {
        if !is_filler_mark(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }

        // Extend the run across further fillers joined by spaces/connectors.
        let capitalize = chars[i] == FILLER_UPPER;
        let mut end = i + 1;
        let mut scan = end;
        while scan < chars.len() {
            if is_filler_mark(chars[scan]) {
                end = scan + 1;
            } else if !(chars[scan].is_whitespace() || is_connector(chars[scan])) {
                break;
            }
            scan += 1;
        }

        // Punctuation on the right: connectors, then whitespace.
        let mut right_start = end;
        let mut right_connector = None;
        while right_start < chars.len()
            && (chars[right_start].is_whitespace() || is_connector(chars[right_start]))
        {
            if is_connector(chars[right_start]) && right_connector.is_none() {
                right_connector = Some(chars[right_start]);
            }
            right_start += 1;
        }
        let right: String = chars[right_start..].iter().collect();

        // Punctuation on the left, already emitted into `out`.
        let left_trimmed_len = out.trim_end().len();
        out.truncate(left_trimmed_len);
        let left_connector = out.chars().last().filter(|c| is_connector(*c));

        let at_text_start = out.is_empty();
        let at_sentence_start = at_text_start || ends_sentence(&out);
        let at_text_end = right.trim().is_empty();
        let before_terminator = right.starts_with(is_terminator);

        if at_sentence_start {
            // Nothing to keep: drop right connectors, maybe capitalize.
        } else if at_text_end || before_terminator {
            if left_connector.is_some() {
                out.pop();
            }
        } else if let Some(lc) = left_connector {
            if right_connector.is_some() {
                let before = sentence_before(&out[..out.len() - lc.len_utf8()]).trim();
                let lone_intro_word = !before.is_empty() && !before.contains(char::is_whitespace);
                let other_commas = before.contains(',') || sentence_after(&right).contains(',');
                if !(lone_intro_word || other_commas) {
                    out.pop();
                }
            }
        } else if let Some(rc) = right_connector {
            out.push(rc);
        }

        if !out.is_empty() && !before_terminator && !at_text_end {
            out.push(' ');
        }

        let mut rest = right.chars();
        if capitalize && at_sentence_start {
            if let Some(first) = rest.next() {
                out.extend(first.to_uppercase());
            }
        }
        out.extend(rest);
        return finish_filler_runs(out);
    }

    out
}

/// Continue removing filler runs after the first one has been handled.
fn finish_filler_runs(text: String) -> String {
    if text.contains(is_filler_mark) {
        remove_filler_runs(&text)
    } else {
        text
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

/// Punctuation that takes no space before it, which is language-specific.
///
/// English closes up against `? ! : ;`. French does not: it requires a
/// (narrow, non-breaking) space before them, so "Bonjour ! Ça va ?" is
/// correct and stripping that space is a typographic error. Same defect
/// class as #566 - a language-blind rule applied to every transcription.
fn tight_punctuation(language: Option<&str>) -> &'static [char] {
    match language.map(|l| l.split(['-', '_']).next().unwrap_or(l)) {
        // French keeps its space before the "high" punctuation marks.
        Some("fr") => &['.', ',', ')', ']', '}'],
        _ => &['.', ',', '?', '!', ':', ';', ')', ']', '}'],
    }
}

/// Sentinel-wrapped symbols. Spoken punctuation emits these so the spacing
/// pass can tell "the user said 'dollar sign'" from "the engine wrote $25".
/// U+E000..U+E002 are private-use, so they cannot occur in real transcription.
const ATTACH_AT: &str = "\u{e000}@\u{e000}";
const ATTACH_DOLLAR: &str = "\u{e001}$\u{e001}";
const ATTACH_HASH: &str = "\u{e002}#\u{e002}";
const ATTACH_OPEN_PAREN: &str = "\u{e003}(\u{e003}";
const ATTACH_CLOSE_PAREN: &str = "\u{e004})\u{e004}";
const ATTACH_OPEN_BRACKET: &str = "\u{e005}[\u{e005}";
const ATTACH_CLOSE_BRACKET: &str = "\u{e006}]\u{e006}";
const ATTACH_OPEN_BRACE: &str = "\u{e007}{\u{e007}";
const ATTACH_CLOSE_BRACE: &str = "\u{e008}}\u{e008}";

/// Collapse whitespace either side of a sentinel-wrapped symbol, then drop
/// the sentinels. "hash include" -> "#include"; "cost $25" is untouched
/// because nothing wrapped that dollar sign.
fn glue_attached_symbols(text: &str) -> String {
    let mut result = text.to_string();
    for sentinel in [
        '\u{e000}', '\u{e001}', '\u{e002}', '\u{e003}', '\u{e004}', '\u{e005}', '\u{e006}',
        '\u{e007}', '\u{e008}',
    ] {
        let s = sentinel.to_string();
        // Whitespace outside the sentinels goes away; repeat so runs collapse.
        while result.contains(&format!(" {}", s)) {
            result = result.replace(&format!(" {}", s), &s);
        }
        while result.contains(&format!("{} ", s)) {
            result = result.replace(&format!("{} ", s), &s);
        }
        result = result.replace(&s, "");
    }
    result
}

/// Clean up spacing around punctuation marks
fn clean_punctuation_spacing(text: &str, tight: &[char]) -> String {
    let mut result = text.to_string();

    // Remove space before punctuation that shouldn't have it. Which marks
    // those are depends on the language: see tight_punctuation.
    for punct in tight {
        result = result.replace(&format!(" {}", punct), &punct.to_string());
    }

    // Glue #, @ and $ to their neighbours - but only the ones spoken
    // punctuation just inserted, which are the only ones wrapped in
    // sentinels. Stripping every " $" unconditionally also ate the space in
    // an engine's own output ("it will cost $25" -> "cost$25").
    result = glue_attached_symbols(&result);

    // Remove spaces around newlines and tabs
    result = result.replace(" \n", "\n");
    result = result.replace("\n ", "\n");
    result = result.replace(" \t", "\t");
    result = result.replace("\t ", "\t");

    result
}

#[cfg(test)]
mod tests {
    fn lard_processor(language: &str, model: &std::path::Path) -> super::TextProcessor {
        let config: crate::config::TextConfig = toml::from_str(&format!(
            "lard_disfluency = true\nlard_model_path = {:?}\nfilter_filler_words = false",
            model.to_string_lossy()
        ))
        .unwrap();
        super::TextProcessor::new_for_language(&config, Some(language))
    }

    #[test]
    fn lard_preserves_fluent_streaming_segment_spacing() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/models/lard-disfluency-v1.vxdf");
        let text = " Please book a hotel in Boston for Tuesday ";
        assert_eq!(lard_processor("en", &path).process(text), text);
    }

    #[test]
    fn lard_cleanup_is_opt_in_english_and_fail_open() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/models/lard-disfluency-v1.vxdf");
        let text = "I need a flight no I need a hotel";
        assert_eq!(
            lard_processor("en-US", &path).process(text),
            "I need a hotel"
        );
        assert_eq!(lard_processor("fr", &path).process(text), text);
        assert_eq!(lard_processor("auto", &path).process(text), text);
        assert_eq!(
            lard_processor("en", std::path::Path::new("/missing/lard.vxdf")).process(text),
            text
        );
        let corrupt = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(lard_processor("en", corrupt.path()).process(text), text);
        assert_eq!(
            super::TextProcessor::new(&crate::config::TextConfig {
                filter_filler_words: false,
                ..Default::default()
            })
            .process(text),
            text
        );
        assert_eq!(
            lard_processor("en", &path).process("Please book a hotel in Boston for Tuesday"),
            "Please book a hotel in Boston for Tuesday"
        );
    }

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

    /// #669/#581 regression gate: routing the streaming and transcribe-file
    /// paths through TextProcessor must not change what the batch path
    /// produces. Pins the exact output for a representative input that
    /// exercises replacements and spoken punctuation together.
    #[test]
    fn batch_pipeline_output_is_pinned() {
        let config = make_config(true, &[("vox type", "voxtype")]);
        let processor = TextProcessor::new(&config);

        assert_eq!(
            processor.process("hello comma I use vox type for dictation period"),
            "hello, I use voxtype for dictation."
        );
    }

    #[test]
    fn collapse_restarts_is_off_by_default() {
        let config = TextConfig::default();
        assert!(!config.collapse_restarts);
        let processor = TextProcessor::new(&config);
        let s = "in this migration? migration tool on the admin side?";
        assert_eq!(processor.process(s), s);
    }

    #[test]
    fn collapse_restarts_runs_in_the_pipeline_when_enabled() {
        let config = TextConfig {
            collapse_restarts: true,
            ..Default::default()
        };
        let processor = TextProcessor::new(&config);
        assert_eq!(
            processor.process("in this migration? migration tool on the admin side?"),
            "in this migration tool on the admin side?"
        );
    }

    #[test]
    fn restarts_see_boundaries_produced_by_spoken_punctuation() {
        // With spoken_punctuation on, the boundary arrives as the word
        // "period". Restart collapsing must run after the conversion or it
        // sees no boundary at all and does nothing.
        let config = TextConfig {
            collapse_restarts: true,
            spoken_punctuation: true,
            ..Default::default()
        };
        let processor = TextProcessor::new(&config);
        assert_eq!(
            processor.process(
                "are we looking at this migration period migration tool on the admin side period"
            ),
            "are we looking at this migration tool on the admin side."
        );
    }

    #[test]
    fn user_replacements_still_win_over_restart_collapsing() {
        // The user's rules run after restart collapsing, so they always get
        // the last word on the text.
        let mut config = TextConfig {
            collapse_restarts: true,
            ..Default::default()
        };
        config
            .replacements
            .insert("admin side".to_string(), "admin console".to_string());
        let processor = TextProcessor::new(&config);
        assert_eq!(
            processor.process("in this migration? migration tool on the admin side?"),
            "in this migration tool on the admin console?"
        );
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
    fn french_keeps_its_space_before_high_punctuation() {
        // French typography requires a space before ? ! : ; - stripping it
        // is an error, the same language-blind class of bug as #566.
        let config = TextConfig {
            spoken_punctuation: true,
            ..Default::default()
        };
        let fr = TextProcessor::new_for_language(&config, Some("fr"));
        assert_eq!(fr.process("Bonjour ! Ça va ?"), "Bonjour ! Ça va ?");
        assert_eq!(
            fr.process("Attention : c'est important"),
            "Attention : c'est important"
        );
        // A space before a full stop or comma is still wrong in French.
        assert_eq!(fr.process("Bonjour , le monde ."), "Bonjour, le monde.");
    }

    #[test]
    fn english_still_closes_up_high_punctuation() {
        let config = TextConfig {
            spoken_punctuation: true,
            ..Default::default()
        };
        let en = TextProcessor::new_for_language(&config, Some("en"));
        assert_eq!(en.process("Hello ! How are you ?"), "Hello! How are you?");
        // And the language-agnostic constructor keeps English behaviour.
        let any = TextProcessor::new(&config);
        assert_eq!(any.process("Hello ! How are you ?"), "Hello! How are you?");
    }

    #[test]
    fn prose_parentheses_keep_their_leading_space() {
        // Gluing the space before "(" is a coding convention ("function(").
        // Applied to prose it produced "the result(see below)".
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(
            processor.process("the result (see below) is fine"),
            "the result (see below) is fine"
        );
        assert_eq!(
            processor.process("we shipped it [finally] last week"),
            "we shipped it [finally] last week"
        );
    }

    #[test]
    fn engine_emitted_symbols_keep_their_spacing() {
        // Parakeet emits "$25" itself. The spoken-punctuation spacing pass
        // used to strip the space before any $, # or @, turning "it will
        // cost $25" into "cost$25".
        let config = make_config(true, &[]);
        let processor = TextProcessor::new(&config);

        assert_eq!(
            processor.process("It will cost $25 a month."),
            "It will cost $25 a month."
        );
        assert_eq!(
            processor.process("We have 50% coverage and #3 is open."),
            "We have 50% coverage and #3 is open."
        );
        assert_eq!(
            processor.process("Mail me at pete@tern.travel"),
            "Mail me at pete@tern.travel"
        );
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

        // A capitalized filler started the sentence, so the next word now does.
        assert_eq!(processor.process("UM hello"), "Hello");
        assert_eq!(processor.process("Um hello"), "Hello");
        assert_eq!(processor.process("Hmm I see"), "I see");
    }

    /// The punctuation a removed filler leaves behind, from real Whisper
    /// output (Kokoro clips through small.en) plus the edges around it.
    #[test]
    fn test_filler_removal_leaves_clean_punctuation() {
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        let cases = [
            // A sentence-initial filler after a full stop used to leave ".,"
            // and a lowercase sentence.
            (
                "It costs $25 a month. Um, starting at 3:30 p.m. on Friday.",
                "It costs $25 a month. Starting at 3:30 p.m. on Friday.",
            ),
            // Commas Whisper put around a mid-sentence hesitation both go.
            (
                "One of the things we need is, um, more testing.",
                "One of the things we need is more testing.",
            ),
            // Start and end of the text.
            ("Um, so we ship it.", "So we ship it."),
            ("We ship it, uh", "We ship it"),
            ("We ship it, uh.", "We ship it."),
            // Before a question mark.
            ("Can you send it, um?", "Can you send it?"),
            ("Is it ready um?", "Is it ready?"),
            // Chains of fillers.
            ("Uh, um, we should ship.", "We should ship."),
            ("It is, uh, um, done.", "It is done."),
            ("Done. Uh, um, next item.", "Done. Next item."),
            // Commas that did real work stay as one.
            ("Well, um, I think so.", "Well, I think so."),
            (
                "We need eggs, um, milk, and bread.",
                "We need eggs, milk, and bread.",
            ),
            // A lowercase filler mid-sentence changes no capitals.
            ("we should um ship it", "we should ship it"),
            ("Done. um next item.", "Done. next item."),
            // Closing quote before the sentence start.
            (
                "He said \"done.\" Um, then left.",
                "He said \"done.\" Then left.",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(processor.process(input), expected, "input: {input:?}");
        }
    }

    #[test]
    fn test_filler_removal_without_fillers_changes_nothing() {
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        for text in [
            "It costs $25 a month. Starting at 3:30 p.m. on Friday.",
            "Use e.g., this one. Then, that one!",
            "Well, I think so.",
            "summer, umbrella; erase: done.",
        ] {
            assert_eq!(processor.process(text), text);
        }
    }

    #[test]
    fn test_filler_removal_with_custom_non_english_fillers() {
        let config = make_filler_config(true, Some(vec!["euh", "bah"]));
        let processor = TextProcessor::new(&config);

        assert_eq!(
            processor.process("C'est prêt. Euh, on livre demain."),
            "C'est prêt. On livre demain."
        );
        assert_eq!(
            processor.process("Il faut, bah, plus de tests."),
            "Il faut plus de tests."
        );
    }

    #[test]
    fn filler_filter_catches_held_fillers() {
        // The case users actually report: a drawn-out filler while thinking.
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        assert_eq!(processor.process("uhhhh hello world"), "hello world");
        assert_eq!(processor.process("ummm hello world"), "hello world");
        assert_eq!(processor.process("uhh ok then"), "ok then");
        assert_eq!(processor.process("hmmm let me think"), "let me think");
        assert_eq!(processor.process("well ahhh I see"), "well I see");
    }

    #[test]
    fn elongation_does_not_swallow_real_words() {
        let config = make_filler_config(true, None);
        let processor = TextProcessor::new(&config);

        // "er" stays exact: to err is human.
        assert_eq!(processor.process("to err is human"), "to err is human");
        // The classic boundary cases still hold with elongation enabled.
        assert_eq!(processor.process("an umbrella"), "an umbrella");
        assert_eq!(
            processor.process("summer hummingbird"),
            "summer hummingbird"
        );
        assert_eq!(processor.process("ahead of time"), "ahead of time");
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
