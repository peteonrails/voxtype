//! Dictation history (#209): a record of every dictation, kept as plain
//! JSONL day files with a derived full-text index.
//!
//! - [`entry`]: what one dictation records, one JSON line.
//! - [`store`]: the day files, their permissions, atomic rewrites,
//!   compression and retention.
//! - [`index`]: the contentless FTS5 search index beside the meetings'.
//! - [`recorder`]: the daemon's side, building and writing entries.
//! - [`focus`]: which app has focus, for `app` and `exclude_apps`.
//! - [`suggest`]: the text replacement a correction implies.

pub mod entry;
pub mod focus;
pub mod index;
pub mod recorder;
pub mod store;
pub mod suggest;

pub use entry::{edits_between, Edit, EndedBy, HistoryEntry, Outcome, OutputInfo, Trigger};
pub use index::{HistoryHit, HistoryIndex, HistoryQuery};
pub use store::{DayFile, HistoryStore};
pub use suggest::{suggest_rule, RuleSuggestion};
