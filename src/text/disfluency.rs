//! Compact token-level disfluency tagger.
//!
//! The tagger is deliberately separate from text rewriting. It labels byte
//! spans and lets the cleanup pipeline decide how to remove them, which keeps
//! model inference testable and prevents punctuation policy from leaking into
//! the model runtime.

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
        let words: Vec<_> = matches
            .iter()
            .map(|item| normalize(item.as_str()))
            .collect();
        let scores: Vec<_> = matches
            .iter()
            .enumerate()
            .map(|(index, item)| self.score(&words, index, item.as_str()))
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
            let mut left = anchor;
            while left > 0 && scores[left - 1] >= self.continuation {
                left -= 1;
                labels[left] = true;
            }
            let mut right = anchor + 1;
            while right < scores.len() && scores[right] >= self.continuation {
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
            })
            .collect()
    }

    fn score(&self, words: &[String], index: usize, original: &str) -> f32 {
        let word = &words[index];
        let prev = index.checked_sub(1).map_or("<s>", |i| words[i].as_str());
        let next = words.get(index + 1).map_or("</s>", String::as_str);
        let raw_token = word.as_str();
        let position = if index < 2 {
            "start"
        } else if index + 2 >= words.len() {
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
            format!("shape={}", shape(original)),
            format!("pre={prefix}"),
            format!("suf={suffix}"),
            format!("pos={position}"),
        ];
        for distance in 1..=8 {
            if index >= distance && words[index - distance] == *word {
                features.push(format!("same-prev={distance}"));
            }
            if words.get(index + distance) == Some(word) {
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
}
