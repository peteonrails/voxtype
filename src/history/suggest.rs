//! Suggest a text replacement from a correction.
//!
//! When someone corrects "cone ogres" to "CODEOWNERS" in history, the same
//! mishearing will come back. The correction already says what the rule
//! should be, so the history views offer to add it to `[text.replacements]`.
//! Only a small, contained change is offered: a rewrite of the sentence is
//! not a mishearing, and an insertion or deletion has nothing to key on.

use serde::Serialize;
use std::collections::HashMap;

/// Longest dictated phrase a rule is offered for, in words.
const MAX_FROM_WORDS: usize = 4;
/// Longest replacement a rule is offered for, in words.
const MAX_TO_WORDS: usize = 6;

/// A replacement worth offering: `from` as it was typed, `to` as corrected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleSuggestion {
    pub from: String,
    pub to: String,
}

/// The replacement a correction from `before` to `after` implies, if it
/// implies exactly one. `existing` is `[text.replacements]`, keyed by the
/// dictated form; a rule already there is not offered again.
pub fn suggest_rule(
    before: &str,
    after: &str,
    existing: &HashMap<String, String>,
) -> Option<RuleSuggestion> {
    let old: Vec<&str> = before.split_whitespace().collect();
    let new: Vec<&str> = after.split_whitespace().collect();

    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old_span = &old[prefix..old.len() - suffix];
    let new_span = &new[prefix..new.len() - suffix];
    if old_span.is_empty() || new_span.is_empty() {
        return None;
    }
    if old_span.len() > MAX_FROM_WORDS || new_span.len() > MAX_TO_WORDS {
        return None;
    }

    let (from, to) = peel_shared_punctuation(&old_span.join(" "), &new_span.join(" "));
    if from.chars().filter(|c| c.is_alphanumeric()).count() < 3 || to.is_empty() {
        return None;
    }
    if from == to {
        return None;
    }
    if from.to_lowercase() == to.to_lowercase() {
        // Rules match without regard to case. "github" to "GitHub" is a
        // rule; capitalising the first word of a sentence is not.
        let starts_sentence = prefix == 0 || old[prefix - 1].ends_with(['.', '!', '?']);
        if starts_sentence && only_first_letter_differs(&from, &to) {
            return None;
        }
    }
    let known = existing
        .iter()
        .any(|(k, v)| k.to_lowercase() == from.to_lowercase() && *v == to);
    if known {
        return None;
    }
    Some(RuleSuggestion { from, to })
}

/// Drop punctuation both spans share at their edges ("ogres," and
/// "CODEOWNERS,"), so the rule is about the words. Punctuation only one side
/// has stays: it may be part of the correction ("e.g.", "C++").
fn peel_shared_punctuation(from: &str, to: &str) -> (String, String) {
    let is_edge = |c: char| c.is_ascii_punctuation();
    let (mut from, mut to) = (from, to);
    while let (Some(a), Some(b)) = (from.chars().next_back(), to.chars().next_back()) {
        if a != b || !is_edge(a) {
            break;
        }
        from = &from[..from.len() - a.len_utf8()];
        to = &to[..to.len() - b.len_utf8()];
    }
    while let (Some(a), Some(b)) = (from.chars().next(), to.chars().next()) {
        if a != b || !is_edge(a) {
            break;
        }
        from = &from[a.len_utf8()..];
        to = &to[b.len_utf8()..];
    }
    (from.trim().to_string(), to.trim().to_string())
}

fn only_first_letter_differs(from: &str, to: &str) -> bool {
    let mut a = from.chars();
    let mut b = to.chars();
    match (a.next(), b.next()) {
        (Some(x), Some(y)) => x != y && a.as_str() == b.as_str(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggest(before: &str, after: &str) -> Option<(String, String)> {
        suggest_rule(before, after, &HashMap::new()).map(|s| (s.from, s.to))
    }

    #[test]
    fn a_misheard_phrase_becomes_a_rule() {
        assert_eq!(
            suggest(
                "Add me to the cone ogres file, please.",
                "Add me to the CODEOWNERS file, please."
            ),
            Some(("cone ogres".into(), "CODEOWNERS".into()))
        );
    }

    #[test]
    fn punctuation_both_sides_share_is_left_out_of_the_rule() {
        assert_eq!(
            suggest("We use vox type.", "We use Voxtype."),
            Some(("vox type".into(), "Voxtype".into()))
        );
        assert_eq!(
            suggest("Ask (erin) first", "Ask (Aaron) first"),
            Some(("erin".into(), "Aaron".into()))
        );
    }

    #[test]
    fn punctuation_only_the_correction_has_is_kept() {
        assert_eq!(
            suggest("Written in see plus plus today", "Written in C++ today"),
            Some(("see plus plus".into(), "C++".into()))
        );
    }

    #[test]
    fn insertions_deletions_and_rewrites_are_not_rules() {
        assert_eq!(suggest("Send it today", "Send it to Erin today"), None);
        assert_eq!(suggest("Okay so send it today", "Send it today"), None);
        assert_eq!(
            suggest(
                "I think we should probably look at this again later",
                "Let's revisit this on Monday with the whole team"
            ),
            None
        );
        assert_eq!(suggest("Same text.", "Same text."), None);
    }

    #[test]
    fn brand_capitals_are_rules_but_sentence_capitals_are_not() {
        assert_eq!(
            suggest("Push it to github now", "Push it to GitHub now"),
            Some(("github".into(), "GitHub".into()))
        );
        assert_eq!(suggest("history is on.", "History is on."), None);
        assert_eq!(
            suggest("It works. history is on.", "It works. History is on."),
            None
        );
        // Mid-sentence, a capital is a name.
        assert_eq!(
            suggest("Tell erin about it", "Tell Erin about it"),
            Some(("erin".into(), "Erin".into()))
        );
    }

    #[test]
    fn tiny_words_and_known_rules_are_not_offered() {
        assert_eq!(suggest("Go to it", "Go do it"), None);
        let mut existing = HashMap::new();
        existing.insert("Cone Ogres".to_string(), "CODEOWNERS".to_string());
        assert_eq!(
            suggest_rule("the cone ogres file", "the CODEOWNERS file", &existing),
            None
        );
        existing.insert("cone ogres".to_string(), "Code Owners".to_string());
        existing.remove("Cone Ogres");
        assert!(suggest_rule("the cone ogres file", "the CODEOWNERS file", &existing).is_some());
    }
}
