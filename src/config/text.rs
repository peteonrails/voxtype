//! Text processing configuration.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Text processing configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TextConfig {
    /// Enable spoken punctuation conversion (e.g., "period" → ".")
    #[serde(default)]
    pub spoken_punctuation: bool,

    /// Custom word replacements (case-insensitive)
    /// Example: { "vox type" = "voxtype" }
    #[serde(default)]
    pub replacements: HashMap<String, String>,

    /// Smart auto-submit: say "submit" at the end of dictation to press Enter.
    /// The word "submit" is stripped from the output and Enter is pressed.
    #[serde(default)]
    pub smart_auto_submit: bool,

    /// Remove common filler words ("uh", "um", etc.) from transcribed text.
    /// Defaults to false to preserve existing behavior. The list is
    /// configurable via `filler_words`.
    #[serde(default)]
    pub filter_filler_words: bool,

    /// Collapse restart disfluencies: a self-interrupted phrase that the
    /// speaker restarts by repeating a word ("the independent tool. the
    /// independent booking tool"), back-to-back stutters, and explicit
    /// retractions ("scratch that", "I'm sorry"). Off by default: a false
    /// positive deletes words the speaker actually said.
    #[serde(default)]
    pub collapse_restarts: bool,

    /// Repair sentence boundaries the engine placed at a pause: join a
    /// fragment that was split off mid-sentence ("what are the other phases?
    /// we have here.") and give a structurally interrogative sentence its
    /// question mark. Off by default.
    #[serde(default)]
    pub repair_sentence_breaks: bool,

    /// Convert dictated quantities to written form: "twenty five dollars"
    /// becomes "$25", "three thirty p m" becomes "3:30 p.m.". Only sentences
    /// that look like they are about a quantity are touched, and a lone
    /// number word is never turned into a digit, so ordinary prose ("one of
    /// the things", "no one knows") is left alone. Off by default.
    #[serde(default)]
    pub format_numbers: bool,

    /// Words removed when `filter_filler_words` is true. Matched
    /// case-insensitively on word boundaries; surrounding punctuation and
    /// whitespace are cleaned up after removal.
    #[serde(default = "default_filler_words")]
    pub filler_words: Vec<String>,
}

impl Default for TextConfig {
    fn default() -> Self {
        Self {
            spoken_punctuation: false,
            replacements: HashMap::new(),
            smart_auto_submit: false,
            filter_filler_words: true,
            collapse_restarts: false,
            repair_sentence_breaks: false,
            format_numbers: false,
            filler_words: default_filler_words(),
        }
    }
}

/// Default filler-word list. Conservative: single-syllable disfluencies only.
/// Multi-word phrases like "you know" or "sort of" are too aggressive for a
/// default and can be added via the `filler_words` config.
pub(crate) fn default_filler_words() -> Vec<String> {
    vec![
        "uh".to_string(),
        "um".to_string(),
        "er".to_string(),
        "ah".to_string(),
        "eh".to_string(),
        "hmm".to_string(),
        "hm".to_string(),
        "mm".to_string(),
        "mhm".to_string(),
    ]
}
