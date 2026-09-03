//! Restart and repetition disfluency removal.
//!
//! Speakers who interrupt themselves usually restart the phrase by repeating
//! a word they already said, and the ASR engine plants a sentence boundary at
//! the interruption point because the speaker paused there:
//!
//! ```text
//! ... and the independent tool. the independent booking migration tool
//!         ^^^^^^^^^^^^^^^^^^^^^^  reparandum + false boundary
//!                                 ^^^^^^^^^^^^^^^^ repair restarts with a repeat
//! ```
//!
//! Deleting the reparandum and its boundary yields the sentence the speaker
//! meant. This module implements that, plus the simpler adjacent-repetition
//! case ("the the the") and explicit editing phrases ("I'm sorry", "scratch
//! that").
//!
//! Everything here is deliberately conservative: a false positive silently
//! deletes words the user said, which is worse than leaving a disfluency in.

use regex::Regex;
use std::sync::OnceLock;

/// Longest reparandum we will delete, in tokens. A self-correction that runs
/// longer than this is more likely to be a real sentence we would be eating.
const MAX_REPARANDUM_TOKENS: usize = 8;

/// Longest repeated anchor we try to match.
const MAX_ANCHOR_TOKENS: usize = 4;

/// A single-token anchor must be at least this long once normalized. Short
/// repeats across a sentence boundary are usually legitimate ("Send it to
/// Bob. Bob will know.").
const MIN_SINGLE_ANCHOR_LEN: usize = 4;

/// Guard passes so a pathological input cannot loop forever.
const MAX_PASSES: usize = 12;

/// Words too common to trust as a single-token restart anchor. A lone "this"
/// or "there" repeating across a boundary is ordinary English, not a restart.
const FUNCTION_WORDS: &[&str] = &[
    "that", "this", "these", "those", "they", "them", "their", "there", "then", "than", "with",
    "what", "when", "which", "while", "would", "could", "should", "have", "has", "had", "been",
    "were", "was", "will", "your", "yours", "ours", "from", "into", "just", "also", "only", "some",
    "such", "here", "does", "did", "not", "and", "but", "for", "the", "you", "our", "its",
];

/// Adjacent repeats that are ordinary English rather than a stutter.
const LEGITIMATE_DOUBLES: &[&str] = &["had", "that", "very", "no", "yes", "ha", "so"];

/// Editing phrases that explicitly mark the preceding clause as retracted.
/// Ordered longest-first so "i am sorry" wins over "sorry".
const EDITING_PHRASES: &[&str] = &[
    "scratch that",
    "i am sorry",
    "i'm sorry",
    "strike that",
    "i meant to say",
    "or rather",
    "i mean",
];

/// Connectives kept at the head of a clause when its body is retracted, so
/// "..., so X. I'm sorry, Y" keeps the "so" and becomes "..., so Y".
const KEPT_CONNECTIVES: &[&str] = &["so", "and", "but", "because", "or", "then"];

fn word_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[\p{Alphabetic}\p{Nd}][\p{Alphabetic}\p{Nd}'\u{2019}_-]*")
            .expect("BUG: word regex is a compile-time constant and must be valid")
    })
}

/// A word occurrence with its byte span in the source string.
#[derive(Debug, Clone)]
struct Tok {
    start: usize,
    end: usize,
    lower: String,
    /// Plural-insensitive form, so "line" and "Lines" compare equal.
    stem: String,
}

fn stem_of(lower: &str) -> String {
    // Only strip a single trailing "s", and only when enough stem remains, so
    // "is" and "as" survive intact while "lines" folds onto "line".
    if lower.len() >= 4 {
        if let Some(rest) = lower.strip_suffix('s') {
            if rest.len() >= 3 && !rest.ends_with('s') {
                return rest.to_string();
            }
        }
    }
    lower.to_string()
}

fn tokenize(text: &str) -> Vec<Tok> {
    word_re()
        .find_iter(text)
        .map(|m| {
            let lower = m.as_str().to_lowercase();
            Tok {
                start: m.start(),
                end: m.end(),
                stem: stem_of(&lower),
                lower,
            }
        })
        .collect()
}

/// Whether the gap between two tokens carries a sentence terminator.
fn gap_is_sentence_boundary(text: &str, a: &Tok, b: &Tok) -> bool {
    text[a.end..b.start].contains(['.', '?', '!'])
}

/// Whether the gap carries any clause-or-stronger break.
fn gap_is_clause_break(text: &str, a: &Tok, b: &Tok) -> bool {
    text[a.end..b.start].contains(['.', '?', '!', ',', ';', ':'])
}

/// Re-case `repair` to match how `anchor` was written, so removing a false
/// boundary does not leave a stray capital mid-sentence.
fn recase_like(anchor: &str, repair: &str) -> Option<String> {
    let anchor_lower = anchor.chars().next()?.is_lowercase();
    let mut chars = repair.chars();
    let first = chars.next()?;
    if anchor_lower && first.is_uppercase() {
        // Leave acronyms ("API") alone; only fix Title-case words.
        if repair.chars().skip(1).any(|c| c.is_uppercase()) {
            return None;
        }
        let mut out = first.to_lowercase().to_string();
        out.push_str(chars.as_str());
        return Some(out);
    }
    None
}

/// Find one repeat-anchored restart and return the edit to apply:
/// `(delete_from, delete_to, optional replacement for the repair's first word)`.
fn find_restart(text: &str, toks: &[Tok]) -> Option<(usize, usize, Option<String>)> {
    for i in 0..toks.len().saturating_sub(1) {
        if !gap_is_sentence_boundary(text, &toks[i], &toks[i + 1]) {
            continue;
        }
        let repair_start = i + 1;
        let max_anchor = MAX_ANCHOR_TOKENS.min(toks.len() - repair_start).min(i + 1);

        // Prefer the longest anchor: "the independent" beats a bare "the".
        for len in (1..=max_anchor).rev() {
            // The anchor has to carry at least one content word. Without
            // this, "That is the plan. That is what we agreed." matches on
            // the bare function words "that is" and loses a whole sentence.
            let has_content = (0..len).any(|k| {
                let t = &toks[repair_start + k];
                t.stem.len() >= MIN_SINGLE_ANCHOR_LEN && !FUNCTION_WORDS.contains(&t.stem.as_str())
            });
            if !has_content {
                continue;
            }
            // The anchor must end at or before the boundary, and the span we
            // delete has to stay short.
            let highest_j = (i + 1).saturating_sub(len);
            let lowest_j = (i + 2).saturating_sub(MAX_REPARANDUM_TOKENS.min(i + 2));
            for j in (lowest_j..=highest_j).rev() {
                let matches = (0..len).all(|k| toks[j + k].stem == toks[repair_start + k].stem);
                if !matches {
                    continue;
                }
                // Never delete across a boundary that sits inside the
                // reparandum: that would eat a whole prior sentence.
                let crosses =
                    (j..i).any(|k| gap_is_sentence_boundary(text, &toks[k], &toks[k + 1]));
                if crosses {
                    continue;
                }
                let recased = recase_like(&text[toks[j].start..toks[j].end], {
                    &text[toks[repair_start].start..toks[repair_start].end]
                });
                return Some((toks[j].start, toks[repair_start].start, recased));
            }
        }
    }
    None
}

/// Find an explicit editing phrase and the retracted clause before it.
fn find_editing_phrase(text: &str, toks: &[Tok]) -> Option<(usize, usize)> {
    for phrase in EDITING_PHRASES {
        let words: Vec<&str> = phrase.split(' ').collect();
        for start in 0..toks.len() {
            if start + words.len() > toks.len() {
                break;
            }
            if !(0..words.len()).all(|k| toks[start + k].lower == words[k]) {
                continue;
            }
            // The phrase only retracts something if a clause precedes it.
            if start == 0 {
                continue;
            }
            // The retracted clause ends just before the phrase, so start the
            // walk there rather than at the phrase itself.
            let mut clause_start = start - 1;
            while clause_start > 0
                && !gap_is_clause_break(text, &toks[clause_start - 1], &toks[clause_start])
            {
                clause_start -= 1;
            }
            // Keep a leading connective: "..., so X. I'm sorry, Y" -> "..., so Y".
            if KEPT_CONNECTIVES.contains(&toks[clause_start].lower.as_str())
                && clause_start + 1 < start
            {
                clause_start += 1;
            }
            if start - clause_start > MAX_REPARANDUM_TOKENS {
                continue;
            }
            let end_tok = start + words.len() - 1;
            // Swallow the phrase's trailing comma and spacing.
            let mut cut_to = toks[end_tok].end;
            let tail: &str = &text[cut_to..];
            let consumed: usize = tail
                .char_indices()
                .take_while(|(_, c)| c.is_whitespace() || *c == ',' || *c == ':')
                .map(|(i, c)| i + c.len_utf8())
                .last()
                .unwrap_or(0);
            cut_to += consumed;
            return Some((toks[clause_start].start, cut_to));
        }
    }
    None
}

/// Collapse runs of the same word repeated back to back ("the the the").
fn collapse_adjacent(text: &str, toks: &[Tok]) -> Option<(usize, usize)> {
    for i in 0..toks.len().saturating_sub(1) {
        let a = &toks[i];
        let b = &toks[i + 1];
        if a.lower != b.lower || LEGITIMATE_DOUBLES.contains(&a.lower.as_str()) {
            continue;
        }
        // Only a bare space may separate a stutter; punctuation means the
        // speaker meant it.
        if text[a.end..b.start].chars().any(|c| !c.is_whitespace()) {
            continue;
        }
        return Some((a.start, b.start));
    }
    None
}

/// Remove restart, editing-phrase and stutter disfluencies from `text`.
pub fn collapse_restarts(text: &str) -> String {
    let mut out = text.to_string();

    for _ in 0..MAX_PASSES {
        let toks = tokenize(&out);
        if toks.len() < 2 {
            break;
        }

        if let Some((from, to)) = collapse_adjacent(&out, &toks) {
            out.replace_range(from..to, "");
            continue;
        }
        if let Some((from, to, recased)) = find_restart(&out, &toks) {
            if let Some(word) = recased {
                let repair = toks
                    .iter()
                    .find(|t| t.start == to)
                    .expect("BUG: repair token start came from this token list");
                out.replace_range(repair.start..repair.end, &word);
            }
            out.replace_range(from..to, "");
            continue;
        }
        if let Some((from, to)) = find_editing_phrase(&out, &toks) {
            out.replace_range(from..to, "");
            continue;
        }
        break;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three transcripts this feature was built from, pinned verbatim.
    #[test]
    fn real_dictation_restart_across_false_boundary() {
        let got = collapse_restarts(
            "In our cleanup tool, both the admin tool and the independent tool. \
             the independent booking migration tool that we're building for the advisors.",
        );
        assert_eq!(
            got,
            "In our cleanup tool, both the admin tool and the independent booking \
             migration tool that we're building for the advisors."
        );
    }

    #[test]
    fn real_dictation_single_word_anchor() {
        let got = collapse_restarts(
            "Are we looking at bookings or booking imports in this migration? \
             migration tool on the admin side?",
        );
        assert_eq!(
            got,
            "Are we looking at bookings or booking imports in this migration tool \
             on the admin side?"
        );
    }

    #[test]
    fn real_dictation_plural_variant_anchor_is_recased() {
        let got = collapse_restarts(
            "The preview shows me 60 held needs advisor consent line. Lines, so it is fine.",
        );
        assert_eq!(
            got,
            "The preview shows me 60 held needs advisor consent lines, so it is fine."
        );
    }

    #[test]
    fn real_dictation_editing_phrase_retracts_clause() {
        let got = collapse_restarts(
            "so it is not just a glitch. I'm sorry, it's not by design, it is a glitch.",
        );
        assert_eq!(got, "so it's not by design, it is a glitch.");
    }

    #[test]
    fn adjacent_stutter_collapses() {
        assert_eq!(
            collapse_restarts("so the the the point is"),
            "so the point is"
        );
        assert_eq!(collapse_restarts("I I I think so"), "I think so");
    }

    // --- guards: these must NOT be touched ---

    #[test]
    fn short_proper_noun_repeat_is_left_alone() {
        let s = "Send it to Bob. Bob will know.";
        assert_eq!(collapse_restarts(s), s);
    }

    #[test]
    fn pronoun_repeat_across_boundary_is_left_alone() {
        let s = "I like it. It is good.";
        assert_eq!(collapse_restarts(s), s);
    }

    #[test]
    fn function_word_repeat_across_boundary_is_left_alone() {
        let s = "That is the plan. That is what we agreed.";
        assert_eq!(collapse_restarts(s), s);
    }

    #[test]
    fn legitimate_doubles_survive() {
        assert_eq!(collapse_restarts("he had had enough"), "he had had enough");
        assert_eq!(
            collapse_restarts("it is very very good"),
            "it is very very good"
        );
    }

    #[test]
    fn setting_name_echo_is_left_alone() {
        // "Requires agency suppliers" is a literal setting name, not a restart:
        // there is no sentence boundary, so the restart rule must not fire.
        let s = "this host requires a Requires agency suppliers setting";
        assert_eq!(collapse_restarts(s), s);
    }

    #[test]
    fn plain_text_is_untouched() {
        let s = "The quick brown fox jumps over the lazy dog.";
        assert_eq!(collapse_restarts(s), s);
    }

    #[test]
    fn long_reparandum_is_not_eaten() {
        // A genuine repeated sentence opener far apart must not delete a clause.
        let s = "The migration ran overnight and finished without incident. \
                 The migration report is attached.";
        assert_eq!(collapse_restarts(s), s);
    }
}
