//! Compact token-level disfluency tagger.
//!
//! The tagger is deliberately separate from text rewriting. It labels byte
//! spans and lets the cleanup pipeline decide how to remove them, which keeps
//! model inference testable and prevents punctuation policy from leaking into
//! the model runtime.
//!
//! The model looks at one sentence at a time for a repair. Its strongest
//! evidence is the same word coming up again a few tokens later, and the
//! training data gave it no reason to care what sits in between. A speaker
//! who restarts does so inside a sentence, though, while a word that comes
//! back after a full stop is ordinary prose ("an old gray owl. The owl
//! said"). So neither the repetition features nor the spread from a labeled
//! token to its neighbours crosses a sentence boundary. This is a guard at
//! inference time: `scripts/train-lard-disfluency.py` still counts repeats
//! across the whole utterance, which is how the published weights were made.

use regex::Regex;
use thiserror::Error;

const MAGIC: &[u8; 8] = b"VXDFL001";
const HEADER_LEN: usize = 24;

#[derive(Debug, Error, PartialEq)]
pub enum TaggerError {
    #[error("disfluency model is truncated")]
    Truncated,
    #[error("unsupported disfluency model format")]
    Format,
    #[error("disfluency model has an invalid feature dimension: {0}")]
    Dimensions(u32),
    #[error("disfluency model size does not match its header")]
    Size,
    #[error("failed to build disfluency tokenizer: {0}")]
    Tokenizer(#[from] regex::Error),
}

/// One token span and the model's binary label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedToken<'a> {
    pub text: &'a str,
    pub start: usize,
    pub end: usize,
    pub disfluent: bool,
    /// Index of the sentence the token is in, counted from the start of the
    /// text. A sentence ends at `.`, `!` or `?` followed by whitespace or
    /// the end of the text.
    pub sentence: usize,
}

/// A token as the model sees it.
struct Token<'a> {
    raw: &'a str,
    word: String,
    sentence: usize,
}

/// A fixed-size hashed linear tagger trained from LARD token annotations.
#[derive(Debug)]
pub struct DisfluencyTagger {
    weights: Vec<i16>,
    threshold: f32,
    continuation: f32,
    scale: f32,
    tokenizer: Regex,
}

impl DisfluencyTagger {
    /// Load a VXDF model. Model bytes remain independently distributable and
    /// are not retained after their quantized weights have been decoded.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TaggerError> {
        if bytes.len() < HEADER_LEN {
            return Err(TaggerError::Truncated);
        }
        if &bytes[..8] != MAGIC {
            return Err(TaggerError::Format);
        }
        let dimensions = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if dimensions == 0 || !dimensions.is_power_of_two() {
            return Err(TaggerError::Dimensions(dimensions));
        }
        let expected = HEADER_LEN + dimensions as usize * 2;
        if bytes.len() != expected {
            return Err(TaggerError::Size);
        }
        let threshold = f32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let continuation = f32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let scale = f32::from_le_bytes(bytes[20..24].try_into().unwrap());
        if !threshold.is_finite()
            || !continuation.is_finite()
            || continuation > threshold
            || !scale.is_finite()
            || scale <= 0.0
        {
            return Err(TaggerError::Format);
        }
        let (pairs, _) = bytes[HEADER_LEN..].as_chunks::<2>();
        let weights = pairs.iter().copied().map(i16::from_le_bytes).collect();
        // Apostrophe suffixes are separate tokens in LARD (`that`, `'s`).
        // Keeping the same tokenization at inference is part of the format.
        let tokenizer = Regex::new(r"[\p{L}\p{M}]+|\p{N}+|['’][\p{L}\p{M}]+|[^\s]")?;
        Ok(Self {
            weights,
            threshold,
            continuation,
            scale,
            tokenizer,
        })
    }

    /// Label the tokens in `text`, retaining byte spans into the original.
    pub fn tag<'a>(&self, text: &'a str) -> Vec<TaggedToken<'a>> {
        let matches: Vec<_> = self.tokenizer.find_iter(text).collect();
        let tokens: Vec<_> = matches
            .iter()
            .zip(sentence_indices(&matches))
            .map(|(item, sentence)| Token {
                raw: item.as_str(),
                word: normalize(item.as_str()),
                sentence,
            })
            .collect();
        let scores: Vec<_> = (0..tokens.len())
            .map(|index| self.score(&tokens, index))
            .collect();
        let mut labels: Vec<_> = scores
            .iter()
            .map(|score| *score >= self.threshold)
            .collect();
        for anchor in labels
            .iter()
            .enumerate()
            .filter_map(|(index, label)| label.then_some(index))
            .collect::<Vec<_>>()
        {
            // The spread stays inside the anchor's sentence, so a restart
            // that opens a sentence cannot take the previous one's last
            // words or its full stop with it.
            let sentence = tokens[anchor].sentence;
            let mut left = anchor;
            while left > 0
                && tokens[left - 1].sentence == sentence
                && scores[left - 1] >= self.continuation
            {
                left -= 1;
                labels[left] = true;
            }
            let mut right = anchor + 1;
            while right < scores.len()
                && tokens[right].sentence == sentence
                && scores[right] >= self.continuation
            {
                labels[right] = true;
                right += 1;
            }
        }
        matches
            .into_iter()
            .enumerate()
            .map(|(index, item)| TaggedToken {
                text: item.as_str(),
                start: item.start(),
                end: item.end(),
                disfluent: labels[index],
                sentence: tokens[index].sentence,
            })
            .collect()
    }

    fn score(&self, tokens: &[Token], index: usize) -> f32 {
        let token = &tokens[index];
        let word = &token.word;
        let prev = index
            .checked_sub(1)
            .map_or("<s>", |i| tokens[i].word.as_str());
        let next = tokens.get(index + 1).map_or("</s>", |t| t.word.as_str());
        let raw_token = word.as_str();
        let position = if index < 2 {
            "start"
        } else if index + 2 >= tokens.len() {
            "end"
        } else {
            "mid"
        };
        let prefix: String = raw_token.chars().take(3).collect();
        let suffix: String = raw_token
            .chars()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let mut features = vec![
            "bias".to_string(),
            format!("w={word}"),
            format!("p={prev}"),
            format!("n={next}"),
            format!("pw={prev}|{word}"),
            format!("wn={word}|{next}"),
            format!("shape={}", shape(token.raw)),
            format!("pre={prefix}"),
            format!("suf={suffix}"),
            format!("pos={position}"),
        ];
        // A repeat only counts inside the token's own sentence. Every number
        // normalizes to "#", which made any two of them a repeat and cost
        // "between 10 and 20" its "10", so numbers are compared as written.
        let repeats = |other: &Token| {
            other.word == *word
                && other.sentence == token.sentence
                && (word != "#" || other.raw == token.raw)
        };
        for distance in 1..=8 {
            if index >= distance && repeats(&tokens[index - distance]) {
                features.push(format!("same-prev={distance}"));
            }
            if tokens.get(index + distance).is_some_and(repeats) {
                features.push(format!("same-next={distance}"));
            }
        }
        features
            .iter()
            .map(|feature| {
                let hashed = fnv1a(feature.as_bytes());
                let bucket = hashed as usize & (self.weights.len() - 1);
                let sign = if hashed & (1 << 63) == 0 { 1.0 } else { -1.0 };
                self.weights[bucket] as f32 * self.scale * sign
            })
            .sum()
    }
}

/// The sentence each token belongs to.
///
/// A sentence ends at `.`, `!` or `?` once whitespace or the end of the text
/// follows, with any closing quote or bracket in between (`"Done!" he`). A
/// mark with more text straight after it does not end one, which keeps
/// "1.2" and "e.g., this" whole. An ellipsis does end one: the cautious
/// reading, since a wrong deletion costs more than a restart left in.
fn sentence_indices(matches: &[regex::Match]) -> Vec<usize> {
    let mut indices = Vec::with_capacity(matches.len());
    let mut sentence = 0;
    // A terminator has been seen with nothing but closers after it.
    let mut closed = false;
    let mut previous_end = 0;
    for item in matches {
        let token = item.as_str();
        let terminator = matches!(token, "." | "!" | "?");
        if item.start() > previous_end {
            if closed {
                sentence += 1;
            }
            closed = false;
        } else if !terminator && !matches!(token, "\"" | "'" | ")" | "]" | "\u{201D}" | "\u{2019}")
        {
            closed = false;
        }
        indices.push(sentence);
        closed |= terminator;
        previous_end = item.end();
    }
    indices
}

fn normalize(token: &str) -> String {
    if token.chars().all(|char| char.is_numeric()) {
        "#".to_string()
    } else {
        token.to_lowercase().replace('’', "'")
    }
}

fn shape(token: &str) -> String {
    let mut result = String::new();
    for char in token.chars() {
        let kind = if char.is_alphabetic() {
            'A'
        } else if char.is_numeric() {
            '9'
        } else {
            char
        };
        if !result.ends_with(kind) {
            result.push(kind);
        }
    }
    result
}

fn fnv1a(value: &[u8]) -> u64 {
    value.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &[u8] = include_bytes!("../../assets/models/lard-disfluency-v1.vxdf");

    #[test]
    fn rejects_invalid_models() {
        assert_eq!(
            DisfluencyTagger::from_bytes(b"short").unwrap_err(),
            TaggerError::Truncated
        );
        let mut bad = MODEL.to_vec();
        bad[0] = b'!';
        assert_eq!(
            DisfluencyTagger::from_bytes(&bad).unwrap_err(),
            TaggerError::Format
        );
        assert_eq!(
            DisfluencyTagger::from_bytes(&MODEL[..30]).unwrap_err(),
            TaggerError::Size
        );
    }

    #[test]
    fn preserves_original_byte_spans() {
        let tagger = DisfluencyTagger::from_bytes(MODEL).unwrap();
        let text = "That’s fine, thanks.";
        for token in tagger.tag(text) {
            assert_eq!(&text[token.start..token.end], token.text);
        }
    }

    #[test]
    fn tags_lard_style_reparandum() {
        let tagger = DisfluencyTagger::from_bytes(MODEL).unwrap();
        let tagged = tagger.tag("I need a flight no I need a hotel");
        let removed: Vec<_> = tagged
            .iter()
            .filter(|token| token.disfluent)
            .map(|token| token.text)
            .collect();
        assert_eq!(removed, ["I", "need", "a", "flight", "no"]);
    }

    #[test]
    fn leaves_fluent_regression_fixture_alone() {
        let tagger = DisfluencyTagger::from_bytes(MODEL).unwrap();
        let removed: Vec<_> = tagger
            .tag("Please book a hotel in Boston for Tuesday")
            .into_iter()
            .filter(|token| token.disfluent)
            .map(|token| token.text)
            .collect();
        assert!(removed.is_empty(), "unexpected labels: {removed:?}");
    }

    fn removed<'a>(tagger: &DisfluencyTagger, text: &'a str) -> Vec<&'a str> {
        tagger
            .tag(text)
            .into_iter()
            .filter(|token| token.disfluent)
            .map(|token| token.text)
            .collect()
    }

    /// The model's strongest feature is the same word coming up again within
    /// eight tokens. A full stop in between used to make no difference, so
    /// the first "owl" in "an old gray owl. The owl said" was labeled.
    #[test]
    fn a_word_repeated_in_the_next_sentence_is_not_a_restart() {
        let tagger = DisfluencyTagger::from_bytes(MODEL).unwrap();
        for text in [
            "It flew into the forest where it met an old gray owl. The owl said hello.",
            "Send it to Bob. Bob will know.",
            "It works! It works really well.",
            "Is it done? It is done.",
            "The owl sat. The owl said hello.",
            "Good. Good work everyone.",
        ] {
            let removed = removed(&tagger, text);
            assert!(removed.is_empty(), "{text}: unexpected labels {removed:?}");
        }
        // A closing quote does not hide the boundary before it.
        assert!(!removed(&tagger, "He said \"No.\" No one came.").contains(&"No"));
    }

    #[test]
    fn a_restart_inside_one_sentence_is_still_tagged() {
        let tagger = DisfluencyTagger::from_bytes(MODEL).unwrap();
        assert_eq!(removed(&tagger, "I want I want to go"), ["I", "want"]);
        assert_eq!(
            removed(&tagger, "Can you can you hear me? I can hear you."),
            ["Can", "you"]
        );
        assert_eq!(
            removed(&tagger, "We went to the the store. The store was closed."),
            ["the"]
        );
    }

    /// Numbers all normalize to "#", so the repetition features used to fire
    /// between any two of them.
    #[test]
    fn two_different_numbers_are_not_a_repeat() {
        let tagger = DisfluencyTagger::from_bytes(MODEL).unwrap();
        for text in [
            "between 10 and 20 people came",
            "It costs 4.50 dollars",
            "In 2025 and 2026 we shipped a lot.",
            "Chapter 3, page 47 of the book.",
        ] {
            let removed = removed(&tagger, text);
            assert!(removed.is_empty(), "{text}: unexpected labels {removed:?}");
        }
        assert_eq!(removed(&tagger, "a table for 2 for 2 people"), ["for", "2"]);
    }

    /// The spread from a labeled token used to run back over the previous
    /// sentence's full stop and take it too.
    #[test]
    fn a_restart_that_opens_a_sentence_leaves_the_one_before_alone() {
        let tagger = DisfluencyTagger::from_bytes(MODEL).unwrap();
        assert_eq!(
            removed(&tagger, "Okay. I want I want to go home. That is all."),
            ["I", "want"]
        );
    }

    #[test]
    fn sentences_end_at_a_terminator_followed_by_whitespace() {
        let tagger = DisfluencyTagger::from_bytes(MODEL).unwrap();
        let sentences = |text| -> Vec<(&str, usize)> {
            tagger
                .tag(text)
                .into_iter()
                .map(|token| (token.text, token.sentence))
                .collect()
        };
        assert_eq!(
            sentences("Hi. Go on! Why? Fine"),
            [
                ("Hi", 0),
                (".", 0),
                ("Go", 1),
                ("on", 1),
                ("!", 1),
                ("Why", 2),
                ("?", 2),
                ("Fine", 3)
            ]
        );
        // A decimal point and an abbreviation's stops have text after them.
        assert!(sentences("Version 1.2, e.g., is out")
            .iter()
            .all(|(_, sentence)| *sentence == 0));
        // A closing quote belongs to the sentence it closes.
        assert_eq!(
            sentences("\"Done!\" he said"),
            [
                ("\"", 0),
                ("Done", 0),
                ("!", 0),
                ("\"", 0),
                ("he", 1),
                ("said", 1)
            ]
        );
        assert_eq!(sentences("Wait... what").last(), Some(&("what", 1)));
    }
}
