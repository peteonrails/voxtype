//! Meeting lines from an engine's word times.
//!
//! Engines that time every word (Parakeet through parakeet-rs or through
//! transcribe.cpp) give meeting mode more than one line per utterance: a line
//! per sentence, and a new line after a pause, each starting at its first word.

use super::TimedSegment;

/// One word and when it was spoken, in seconds from the start of the clip.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedWord {
    pub text: String,
    pub start_secs: f32,
    pub end_secs: f32,
}

/// Silence between one word's end and the next word's start that ends a line.
/// Shorter than the 0.7 s pause meeting mode splits utterances at, so a pause
/// the audio level cannot show (a noisy room, the other side talking) still
/// breaks the line.
pub const LINE_PAUSE_SECS: f32 = 0.5;

/// Lines from timed words: a new line after a word ending a sentence (. ? !)
/// and after a pause of [`LINE_PAUSE_SECS`]. A line runs from its first word's
/// start to its last word's end; one with no letters or digits is dropped.
pub fn lines_from_words(words: &[TimedWord]) -> Vec<TimedSegment> {
    let mut lines = Vec::new();
    let mut text = String::new();
    let mut start = 0.0f32;
    for (i, word) in words.iter().enumerate() {
        let piece = word.text.trim();
        if piece.is_empty() {
            continue;
        }
        if text.is_empty() {
            start = word.start_secs;
        } else if !piece.chars().all(|c| c.is_ascii_punctuation()) {
            text.push(' ');
        }
        text.push_str(piece);
        let sentence_end = piece.ends_with(['.', '?', '!']);
        let pause = words
            .get(i + 1)
            .is_some_and(|next| next.start_secs - word.end_secs >= LINE_PAUSE_SECS);
        if sentence_end || pause || i + 1 == words.len() {
            if text.chars().any(char::is_alphanumeric) {
                lines.push(TimedSegment {
                    text: std::mem::take(&mut text),
                    start_secs: start,
                    end_secs: word.end_secs.max(start),
                });
            }
            text.clear();
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(spec: &[(&str, f32, f32)]) -> Vec<TimedWord> {
        spec.iter()
            .map(|&(text, start_secs, end_secs)| TimedWord {
                text: text.to_string(),
                start_secs,
                end_secs,
            })
            .collect()
    }

    fn spans(lines: &[TimedSegment]) -> Vec<(&str, f32, f32)> {
        lines
            .iter()
            .map(|l| (l.text.as_str(), l.start_secs, l.end_secs))
            .collect()
    }

    #[test]
    fn sentences_and_pauses_end_lines() {
        let w = words(&[
            ("Okay,", 0.40, 0.70),
            ("let's", 0.72, 0.95),
            ("start.", 0.96, 1.40),
            ("The", 1.52, 1.60),
            ("build", 1.62, 1.90),
            ("passed", 1.92, 2.30),
            ("so", 3.10, 3.20),
            ("3.5", 3.25, 3.70),
            ("ships?", 3.72, 4.10),
        ]);
        assert_eq!(
            spans(&lines_from_words(&w)),
            vec![
                ("Okay, let's start.", 0.40, 1.40),
                ("The build passed", 1.52, 2.30),
                ("so 3.5 ships?", 3.10, 4.10),
            ]
        );
    }

    #[test]
    fn short_gaps_and_wordless_lines() {
        // A 0.4 s gap is not a pause; punctuation rows join without a space.
        let w = words(&[("yes", 0.0, 0.3), ("right", 0.7, 1.0), (".", 1.0, 1.05)]);
        assert_eq!(
            spans(&lines_from_words(&w)),
            vec![("yes right.", 0.0, 1.05)]
        );
        assert!(lines_from_words(&words(&[("...", 0.0, 0.2)])).is_empty());
        assert!(lines_from_words(&words(&[(" ", 0.0, 0.2)])).is_empty());
        assert!(lines_from_words(&[]).is_empty());
    }
}
