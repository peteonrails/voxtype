//! Free-text notes per meeting, kept in `notes.md` in the meeting's folder.
//!
//! The meeting daemon never writes this file, so notes can be edited while a
//! meeting records without a per-chunk save overwriting them. The meeting
//! assistant reads them with [`load`] when it builds a prompt; per-line
//! annotations are on each segment as
//! [`TranscriptSegment::annotation`](crate::meeting::TranscriptSegment::annotation).

use crate::meeting::{MeetingConfig, MeetingId, MeetingStorage, StorageError};
use chrono::{DateTime, Utc};
use serde::Serialize;

/// A meeting's notes, as `meeting notes --json` prints them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeetingNotes {
    pub meeting_id: String,
    /// The notes, verbatim; empty when there are none
    pub notes: String,
    /// When notes.md was last written; `None` when there are no notes
    pub updated_at: Option<DateTime<Utc>>,
}

/// Load a meeting's notes.
pub fn load(config: &MeetingConfig, meeting_id: &MeetingId) -> Result<MeetingNotes, StorageError> {
    MeetingStorage::open(config.storage.clone())?.load_notes(meeting_id)
}
