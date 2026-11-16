//! One dictation, as one line of a history day file.
//!
//! The line format is versioned by `schema`. Fields a newer voxtype adds are
//! kept through `extra` when an older one rewrites the line (to delete a
//! neighbour or set a correction), so a downgrade never strips them.

use chrono::{DateTime, FixedOffset, Local, NaiveDate};
use serde::{Deserialize, Serialize};

/// The line format this build writes.
pub const SCHEMA_VERSION: u32 = 1;

/// How a dictation ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The text reached the target (typed, pasted, copied or written).
    #[default]
    Delivered,
    /// Transcribed, but every output method failed.
    OutputFailed,
    /// The engine returned an error, or its task panicked.
    TranscriptionFailed,
    /// Voice activity detection heard no speech.
    NoSpeech,
    /// The recording was shorter than voxtype transcribes.
    TooShort,
    /// The engine returned no text.
    Empty,
    /// Cancelled before the text was delivered.
    Cancelled,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Delivered => "delivered",
            Outcome::OutputFailed => "output_failed",
            Outcome::TranscriptionFailed => "transcription_failed",
            Outcome::NoSpeech => "no_speech",
            Outcome::TooShort => "too_short",
            Outcome::Empty => "empty",
            Outcome::Cancelled => "cancelled",
        }
    }

    /// Whether this outcome put text somewhere.
    pub fn delivered(self) -> bool {
        self == Outcome::Delivered
    }
}

/// What ended the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EndedBy {
    /// The hotkey was released, or `record stop`/`toggle` ran.
    #[default]
    Released,
    /// `audio.max_duration_secs` cut it off.
    Timeout,
    /// A silence timeout ended an externally triggered recording.
    Silence,
    Cancelled,
}

/// What started the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// The built-in hotkey listener.
    #[default]
    Hotkey,
    /// `voxtype record start`/`toggle`, usually from a compositor binding.
    Record,
    Tray,
    Api,
}

/// How the text was delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutputInfo {
    /// `type`, `paste`, `clipboard` or `file`.
    pub mode: String,
    /// The driver that delivered the text (`wtype`, `dotool`, ...), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    /// Whether the first choice failed and a later one was used.
    #[serde(default)]
    pub fell_back: bool,
}

/// One automatic change between `raw_text` and `text`, at word granularity:
/// `from` was replaced by `to`. An empty `from` is an insertion, an empty
/// `to` a deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    pub from: String,
    pub to: String,
}

/// The word-level edits that turned `raw` into `text`, from the same diff
/// the transcription notification shows with `show_edits`.
pub fn edits_between(raw: &str, text: &str) -> Vec<Edit> {
    use crate::text::diff::{word_diff, SpanKind};
    let mut edits: Vec<Edit> = Vec::new();
    let mut pending: Option<Edit> = None;
    for span in word_diff(raw, text) {
        match span.kind {
            SpanKind::Same => {
                if let Some(edit) = pending.take() {
                    edits.push(edit);
                }
            }
            SpanKind::Deleted => {
                let edit = pending.get_or_insert(Edit {
                    from: String::new(),
                    to: String::new(),
                });
                join(&mut edit.from, &span.text);
            }
            SpanKind::Inserted => {
                let edit = pending.get_or_insert(Edit {
                    from: String::new(),
                    to: String::new(),
                });
                join(&mut edit.to, &span.text);
            }
        }
    }
    edits.extend(pending);
    edits
}

fn join(into: &mut String, words: &str) {
    if !into.is_empty() {
        into.push(' ');
    }
    into.push_str(words);
}

/// One dictation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub schema: u32,
    /// `YYYYMMDD-HHMMSS-xxxxxx`: the local start time and a random suffix.
    /// The date part names the day file the entry lives in.
    pub id: String,
    pub ts_start: DateTime<FixedOffset>,
    pub ts_end: DateTime<FixedOffset>,

    // What was said
    /// Exactly what the engine returned.
    #[serde(default)]
    pub raw_text: String,
    /// What was output, after replacements, cleanup and post-processing.
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    // How it was transcribed
    #[serde(default)]
    pub engine: String,
    #[serde(default)]
    pub model: String,
    /// Where inference ran, such as `cpu`, `vulkan` or `cuda`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// The binary variant, such as `avx2` or `onnx-migraphx`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default)]
    pub audio_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcribe_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_load_ms: Option<u64>,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub voxtype_version: String,

    // Corrections
    /// The automatic changes from `raw_text` to `text`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<Edit>,
    /// The user's own correction, set later from a history surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrected_text: Option<String>,

    // How it was delivered
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputInfo>,
    /// Whether the text reached its target.
    #[serde(default)]
    pub delivered: bool,
    #[serde(default)]
    pub outcome: Outcome,
    /// What went wrong, for an outcome that is not `delivered`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default)]
    pub auto_submit: bool,
    #[serde(default)]
    pub ended_by: EndedBy,

    // Context
    /// The focused app's class or app ID, such as `kitty` or `firefox`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Only with `history.record_window_title`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_device: Option<String>,

    // Bookkeeping
    /// The saved audio, once audio retention exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_ref: Option<String>,

    /// Fields this build does not know, kept as they were.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl HistoryEntry {
    /// A blank entry for a dictation that started at `ts_start`, with a
    /// fresh ID.
    pub fn new(ts_start: DateTime<Local>) -> Self {
        let start = ts_start.fixed_offset();
        Self {
            schema: SCHEMA_VERSION,
            id: new_id(&ts_start),
            ts_start: start,
            ts_end: start,
            raw_text: String::new(),
            text: String::new(),
            language: None,
            engine: String::new(),
            model: String::new(),
            backend: None,
            variant: None,
            audio_ms: 0,
            transcribe_ms: None,
            model_load_ms: None,
            streaming: false,
            voxtype_version: crate::cli::VERSION.to_string(),
            edits: Vec::new(),
            corrected_text: None,
            output: None,
            delivered: false,
            outcome: Outcome::Delivered,
            error: None,
            trigger: Trigger::Hotkey,
            profile: None,
            auto_submit: false,
            ended_by: EndedBy::Released,
            app: None,
            window_title: None,
            input_device: None,
            audio_ref: None,
            extra: serde_json::Map::new(),
        }
    }

    /// The day file this entry belongs in: the date in its ID, which is the
    /// local date it started.
    pub fn day(&self) -> Option<NaiveDate> {
        day_of_id(&self.id)
    }

    /// The best text there is: the correction, else what was output, else
    /// what the engine heard.
    pub fn best_text(&self) -> &str {
        if let Some(c) = self.corrected_text.as_deref() {
            return c;
        }
        if !self.text.is_empty() {
            return &self.text;
        }
        &self.raw_text
    }

    pub fn set_outcome(&mut self, outcome: Outcome, error: Option<String>) {
        self.outcome = outcome;
        self.delivered = outcome.delivered();
        self.error = error;
    }
}

/// A fresh entry ID for a dictation that started at `start`.
pub fn new_id(start: &DateTime<Local>) -> String {
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!("{}-{}", start.format("%Y%m%d-%H%M%S"), &random[..6])
}

/// The local date an entry ID names, if it is well formed.
pub fn day_of_id(id: &str) -> Option<NaiveDate> {
    let date = id.get(..8)?;
    if !date.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    NaiveDate::parse_from_str(date, "%Y%m%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn ids_name_their_day() {
        let start = Local.with_ymd_and_hms(2026, 11, 14, 9, 5, 7).unwrap();
        let id = new_id(&start);
        assert!(id.starts_with("20261114-090507-"), "{id}");
        assert_eq!(id.len(), "20261114-090507-".len() + 6);
        assert_eq!(day_of_id(&id), NaiveDate::from_ymd_opt(2026, 11, 14));
        assert_ne!(id, new_id(&start));
        assert_eq!(day_of_id("garbage"), None);
        assert_eq!(day_of_id("2026111x-1"), None);
    }

    #[test]
    fn edits_pair_deletions_with_what_replaced_them() {
        let edits = edits_between(
            "um one of the things we need costs twenty five dollars",
            "One of the things we need costs $25.",
        );
        assert_eq!(
            edits,
            vec![
                Edit {
                    from: "um one".into(),
                    to: "One".into()
                },
                Edit {
                    from: "twenty five dollars".into(),
                    to: "$25.".into()
                },
            ]
        );
        assert!(edits_between("same words", "same words").is_empty());
        assert_eq!(
            edits_between("hello", "hello there"),
            vec![Edit {
                from: String::new(),
                to: "there".into()
            }]
        );
    }

    #[test]
    fn a_line_round_trips_and_keeps_unknown_fields() {
        let mut e = HistoryEntry::new(Local.with_ymd_and_hms(2026, 11, 14, 9, 0, 0).unwrap());
        e.raw_text = "hello".into();
        e.text = "Hello.".into();
        e.set_outcome(Outcome::Delivered, None);
        let line = serde_json::to_string(&e).unwrap();
        assert!(!line.contains('\n'));
        let back: HistoryEntry = serde_json::from_str(&line).unwrap();
        assert_eq!(back, e);

        let mut v: serde_json::Value = serde_json::from_str(&line).unwrap();
        v["future_field"] = serde_json::json!({"x": 1});
        let back: HistoryEntry = serde_json::from_value(v).unwrap();
        let again = serde_json::to_string(&back).unwrap();
        assert!(again.contains("\"future_field\":{\"x\":1}"), "{again}");
    }

    #[test]
    fn failed_entries_say_so() {
        let mut e = HistoryEntry::new(Local::now());
        e.set_outcome(Outcome::NoSpeech, Some("no speech detected".into()));
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["delivered"], false);
        assert_eq!(v["outcome"], "no_speech");
        assert_eq!(v["error"], "no speech detected");
        assert_eq!(v["schema"], SCHEMA_VERSION);
    }

    #[test]
    fn best_text_prefers_the_correction() {
        let mut e = HistoryEntry::new(Local::now());
        e.raw_text = "raw".into();
        assert_eq!(e.best_text(), "raw");
        e.text = "out".into();
        assert_eq!(e.best_text(), "out");
        e.corrected_text = Some("fixed".into());
        assert_eq!(e.best_text(), "fixed");
    }
}
