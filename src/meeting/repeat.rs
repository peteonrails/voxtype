//! Collapsing a speech engine's repetition loops.
//!
//! On faint or ambiguous audio a decoder can lock onto a phrase and repeat
//! it until the window ends: Pete's 1.3.0 call had "I don't know what
//! you're talking about." six times in one line, from the faint echo of
//! the other side. Nobody says a whole phrase three or more times back to
//! back, so such a run is collapsed to one instance. Short repeats people
//! do say ("No, no, no.", "Yes. Yes.") are left alone: a phrase must be at
//! least [`MIN_PHRASE_WORDS`] words and repeat [`MIN_REPEATS`] times.

use crate::meeting::echo::normalize_word;

/// The shortest phrase, in words, treated as a loop.
pub const MIN_PHRASE_WORDS: usize = 3;
/// How many times in a row it has to come.
pub const MIN_REPEATS: usize = 3;

/// `text` with every run of a phrase of [`MIN_PHRASE_WORDS`] or more words
/// repeated [`MIN_REPEATS`] or more times back to back reduced to its first
/// instance, or `None` when there is none. Words compare without case or
/// punctuation; the first instance keeps its own.
pub fn collapse(text: &str) -> Option<String> {
    let mut tokens: Vec<&str> = text.split_whitespace().collect();
    let mut changed = false;
    while let Some((at, len, repeats)) = find_loop(&tokens) {
        tokens.drain(at + len..at + len * repeats);
        changed = true;
    }
    changed.then(|| tokens.join(" "))
}

/// The first run, as (start, phrase length, repeats), preferring at each
/// start the phrase whose run covers the most words.
fn find_loop(tokens: &[&str]) -> Option<(usize, usize, usize)> {
    let words: Vec<String> = tokens.iter().map(|t| normalize_word(t)).collect();
    for at in 0..words.len() {
        let mut best: Option<(usize, usize)> = None;
        for len in MIN_PHRASE_WORDS..=(words.len() - at) / MIN_REPEATS {
            let phrase = &words[at..at + len];
            if phrase.iter().all(String::is_empty) {
                continue;
            }
            let mut repeats = 1;
            while at + len * (repeats + 1) <= words.len()
                && words[at + len * repeats..at + len * (repeats + 1)] == *phrase
            {
                repeats += 1;
            }
            if repeats >= MIN_REPEATS && best.is_none_or(|(l, r)| len * repeats > l * r) {
                best = Some((len, repeats));
            }
        }
        if let Some((len, repeats)) = best {
            return Some((at, len, repeats));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decoder_loop_collapses_to_one_instance() {
        let line = "I don't know what you're talking about. ".repeat(6);
        assert_eq!(
            collapse(line.trim()).as_deref(),
            Some("I don't know what you're talking about.")
        );
        // Text around the loop stays.
        assert_eq!(
            collapse("Well, we ship it. We ship it. We ship it. Then we rest.").as_deref(),
            Some("Well, we ship it. Then we rest.")
        );
        // Punctuation and case may vary between the copies.
        assert_eq!(
            collapse("Thanks for the update, thanks for the update. Thanks for the update!")
                .as_deref(),
            Some("Thanks for the update,")
        );
    }

    #[test]
    fn short_or_twice_said_repeats_stay() {
        for line in [
            "No, no.",
            "Yes. Yes.",
            "No, no, no.",
            "Yeah yeah yeah yeah.",
            "Ship it now. Ship it now.",
            "We said that we said that.",
            "",
        ] {
            assert_eq!(collapse(line), None, "{line:?}");
        }
    }
}
