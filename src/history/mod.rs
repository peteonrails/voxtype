//! Dictation history (#209): a record of every dictation, kept as plain
//! JSONL day files with a derived full-text index.
//!
//! - [`entry`]: what one dictation records, one JSON line.
//! - [`store`]: the day files, their permissions, atomic rewrites,
//!   compression and retention.

pub mod entry;
pub mod store;

pub use entry::{edits_between, Edit, EndedBy, HistoryEntry, Outcome, OutputInfo, Trigger};
pub use store::{DayFile, HistoryStore};
