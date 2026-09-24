//! Sentence-boundary repair.
//!
//! Engines punctuate at pauses, and a speaker who pauses mid-sentence gets a
//! boundary they did not mean:
//!
//! ```text
//! what are the other phases? we have here.
//! ```
//!
//! Two things went wrong there, and neither is fixable alone. The `?` landed
//! early, stranding "we have here." as a fragment; and the real question mark
//! belongs at the end. Joining the fragment and then re-deciding the terminal
//! mark recovers "what are the other phases we have here?".
//!
//! Both rules are heuristics standing in for a punctuation model (#696 stage
//! 3), so both are deliberately narrow.

use regex::Regex;
use std::sync::OnceLock;

/// Abbreviations whose full stop is not a sentence end.
const ABBREVIATIONS: &[&str] = &[
    "e.g", "i.e", "etc", "vs", "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "fig", "no",
    "approx", "dept", "est", "inc", "ltd", "co", "al", "cf", "p.m", "a.m",
];

/// Words that can open a question.
const WH_WORDS: &[&str] = &[
    "how", "what", "why", "when", "where", "who", "whom", "whose", "which",
];

/// Auxiliaries. A question inverts subject and auxiliary, which is what
/// separates "how might we ..." from "what we need is ...".
const AUXILIARIES: &[&str] = &[
    "am", "is", "are", "was", "were", "be", "been", "being", "do", "does", "did", "have", "has",
    "had", "can", "could", "shall", "should", "will", "would", "may", "might", "must",
];

fn word_of(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '.')
        .to_lowercase()
}

/// A boundary the engine did not believe in: it wrote a terminator but then
/// carried on in lower case. Engines capitalise after a boundary they mean,
/// so the inconsistency is the tell.
fn join_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(\S*?)([.!?])\s+(\p{Ll}[\p{L}']*)")
            .expect("BUG: join regex is a compile-time constant and must be valid")
    })
}

/// Join fragments split off by a premature sentence boundary.
pub fn join_premature_breaks(text: &str) -> String {
    let mut out = text.to_string();
    // Re-scan after each join: removing one boundary can expose another.
    for _ in 0..8 {
        let Some(m) = join_re().captures(&out) else {
            break;
        };
        let whole = m.get(0).expect("group 0 always matches");
        let before = m.get(1).map(|g| g.as_str()).unwrap_or_default();
        let terminator = m.get(2).expect("group 2 is required").as_str();
        let next = m.get(3).expect("group 3 is required").as_str();

        let stem = word_of(before);
        let abbreviation = ABBREVIATIONS.contains(&stem.as_str())
            || ABBREVIATIONS.contains(&format!("{}{}", stem, terminator).as_str());
        // A decimal ("3.30") or a lower-case proper noun ("iPhone") is not a
        // premature break.
        let decimal = terminator == "."
            && before.chars().last().is_some_and(|c| c.is_ascii_digit())
            && next.chars().next().is_some_and(|c| c.is_ascii_digit());
        let proper_noun = next.chars().skip(1).any(|c| c.is_uppercase());

        if abbreviation || decimal || proper_noun {
            // Leave it alone and look past it.
            let Some(rest) = out.get(whole.end()..) else {
                break;
            };
            if !join_re().is_match(rest) {
                break;
            }
            // Rebuild scanning from after this match by recursing on the tail.
            let head = out[..whole.end()].to_string();
            let tail = join_premature_breaks(rest);
            out = format!("{head}{tail}");
            break;
        }

        let replacement = format!("{before} {next}");
        out.replace_range(whole.start()..whole.end(), &replacement);
    }
    out
}

/// Whether a sentence is a question by structure: a wh-word or an auxiliary
/// opens it, and subject and auxiliary are inverted.
fn reads_as_question(sentence: &str) -> bool {
    let words: Vec<String> = sentence.split_whitespace().map(word_of).collect();
    let Some(first) = words.first() else {
        return false;
    };
    if AUXILIARIES.contains(&first.as_str()) {
        return true;
    }
    if WH_WORDS.contains(&first.as_str()) {
        // "how might we" inverts; "what we need is" does not.
        return words
            .get(1)
            .is_some_and(|w| AUXILIARIES.contains(&w.as_str()));
    }
    false
}

fn terminal_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[^.!?]+[.!?]|[^.!?]+$")
            .expect("BUG: terminal regex is a compile-time constant and must be valid")
    })
}

/// Give a structurally interrogative sentence its question mark.
pub fn restore_question_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for m in terminal_re().find_iter(text) {
        let chunk = m.as_str();
        if chunk.ends_with('.') && reads_as_question(chunk) {
            out.push_str(&chunk[..chunk.len() - 1]);
            out.push('?');
        } else {
            out.push_str(chunk);
        }
    }
    out
}

/// Repair premature sentence breaks, then re-decide terminal punctuation.
pub fn repair(text: &str) -> String {
    restore_question_marks(&join_premature_breaks(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reported_case() {
        assert_eq!(
            repair("what are the other phases? we have here."),
            "what are the other phases we have here?"
        );
    }

    #[test]
    fn interrogatives_get_a_question_mark() {
        assert_eq!(
            repair("How might we visually indicate that."),
            "How might we visually indicate that?"
        );
        assert_eq!(
            repair("What are some ways we might deal with punctuation."),
            "What are some ways we might deal with punctuation?"
        );
        assert_eq!(
            repair("Are we looking at bookings."),
            "Are we looking at bookings?"
        );
    }

    #[test]
    fn statements_that_start_with_a_wh_word_are_left_alone() {
        for s in [
            "What we need is a switch.",
            "How to install this.",
            "What a mess.",
            "When it lands we can ship.",
        ] {
            assert_eq!(repair(s), s, "must not question: {s}");
        }
    }

    #[test]
    fn abbreviations_are_not_premature_breaks() {
        for s in [
            "use a tool, e.g. voxtype for dictation",
            "the meeting is at 3.30 p.m. tomorrow",
        ] {
            assert_eq!(join_premature_breaks(s), s, "must not join: {s}");
        }
    }

    #[test]
    fn decimals_are_not_premature_breaks() {
        assert_eq!(
            join_premature_breaks("it costs 3.30 a month"),
            "it costs 3.30 a month"
        );
    }

    #[test]
    fn lowercase_proper_nouns_are_not_premature_breaks() {
        assert_eq!(
            join_premature_breaks("We shipped it. iPhone users benefit."),
            "We shipped it. iPhone users benefit."
        );
    }

    #[test]
    fn a_real_sentence_boundary_survives() {
        let s = "We shipped it. It works well.";
        assert_eq!(repair(s), s);
    }

    #[test]
    fn joining_does_not_disturb_ordinary_prose() {
        let s = "The preview shows sixty held lines, so it is a glitch.";
        assert_eq!(repair(s), s);
    }
}
