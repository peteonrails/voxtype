//! Giving a meeting a new title: `voxtype meeting title`, and a title taken
//! from the calendar when a meeting starts.
//!
//! The title is stored in the index, metadata.json and meeting.json, and
//! transcript.md is rewritten under it. The meeting's folder keeps the name
//! it was created with. A meeting the daemon is running also hears about it
//! through the [`TRIGGER`] file, so it updates its own copy, the meeting
//! view, and the speaker names the new title gives.

use crate::meeting::storage::{MeetingStorage, StorageError};
use crate::meeting::MeetingId;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Trigger file in the runtime directory: a running meeting was retitled.
pub const TRIGGER: &str = "meeting_title";

/// What the trigger file holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Retitle {
    pub meeting_id: String,
    pub title: String,
    /// A call link to set when the meeting has none
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Why a title was refused.
#[derive(Debug, thiserror::Error)]
pub enum RetitleError {
    #[error("A meeting title can't be empty.")]
    Empty,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// `title` as stored: one line, trimmed. Empty is refused.
pub fn clean_title(title: &str) -> Result<String, RetitleError> {
    let one_line = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return Err(RetitleError::Empty);
    }
    Ok(one_line)
}

/// What a retitle changed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Retitled {
    pub meeting_id: String,
    pub title: String,
    pub previous_title: Option<String>,
    /// The call link, when this retitle set it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Store `title` for meeting `id`, and `url` when it has no link yet.
/// `rewrite_transcript` rewrites transcript.md under the new title; leave
/// it off for a meeting the daemon is running, which owns the transcript
/// and rewrites it itself.
pub fn retitle_stored(
    storage: &MeetingStorage,
    id: &MeetingId,
    title: &str,
    url: Option<&str>,
    rewrite_transcript: bool,
) -> Result<Retitled, RetitleError> {
    let title = clean_title(title)?;
    let mut metadata = storage
        .get_meeting(id)?
        .ok_or_else(|| StorageError::NotFound(id.to_string()))?;
    let previous_title = metadata.title.replace(title.clone());
    let url = match (&metadata.url, url) {
        (None, Some(u)) if !u.trim().is_empty() => {
            metadata.url = Some(u.trim().to_string());
            metadata.url.clone()
        }
        _ => None,
    };
    storage.update_meeting(&metadata)?;
    // transcript.md carries the title in its heading.
    if rewrite_transcript {
        match storage.load_transcript_all(id) {
            Ok(transcript) => storage.save_transcript(id, &transcript)?,
            Err(StorageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Retitled {
        meeting_id: id.to_string(),
        title,
        previous_title,
        url,
    })
}

/// Tell the daemon its running meeting was retitled.
pub fn write_trigger(runtime_dir: &Path, retitle: &Retitle) -> std::io::Result<()> {
    std::fs::create_dir_all(runtime_dir)?;
    let staged = runtime_dir.join(format!(".{TRIGGER}.{}", std::process::id()));
    std::fs::write(&staged, serde_json::to_string(retitle)?)?;
    std::fs::rename(staged, runtime_dir.join(TRIGGER))
}

/// Take the trigger the daemon was sent, if there is one.
pub fn take_trigger(runtime_dir: &Path) -> Option<Retitle> {
    let path = runtime_dir.join(TRIGGER);
    let json = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    serde_json::from_str(&json)
        .map_err(|e| tracing::warn!("Ignoring a malformed retitle request: {}", e))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::{MeetingMetadata, Transcript, TranscriptSegment};

    fn storage() -> (tempfile::TempDir, MeetingStorage) {
        let dir = tempfile::tempdir().unwrap();
        let storage = MeetingStorage::open(crate::meeting::StorageConfig {
            storage_path: dir.path().to_path_buf(),
            retain_audio: false,
            max_meetings: 0,
            my_name: None,
        })
        .unwrap();
        (dir, storage)
    }

    #[test]
    fn titles_are_one_trimmed_line_and_never_empty() {
        assert_eq!(
            clean_title("  Pete <> Chad \n weekly ").unwrap(),
            "Pete <> Chad weekly"
        );
        assert!(matches!(clean_title(""), Err(RetitleError::Empty)));
        assert!(matches!(clean_title(" \n\t"), Err(RetitleError::Empty)));
    }

    #[test]
    fn a_stored_meeting_takes_the_title_everywhere_but_its_folder() {
        let (_dir, storage) = storage();
        let mut metadata = MeetingMetadata::new(Some("Zoom call".into()));
        let folder = storage.create_meeting(&metadata).unwrap();
        metadata.storage_path = Some(folder.clone());
        let mut t = Transcript::new();
        t.add_segment(TranscriptSegment::new(0, 0, 1000, "Hi.".into(), 0));
        storage.save_transcript(&metadata.id, &t).unwrap();

        let done = retitle_stored(
            &storage,
            &metadata.id,
            "Pete <> Chad Bi-Weekly",
            Some("https://zoom.us/j/1"),
            true,
        )
        .unwrap();
        assert_eq!(done.previous_title.as_deref(), Some("Zoom call"));
        assert_eq!(done.url.as_deref(), Some("https://zoom.us/j/1"));
        let stored = storage.get_meeting(&metadata.id).unwrap().unwrap();
        assert_eq!(stored.title.as_deref(), Some("Pete <> Chad Bi-Weekly"));
        assert_eq!(stored.url.as_deref(), Some("https://zoom.us/j/1"));
        let json = std::fs::read_to_string(folder.join("metadata.json")).unwrap();
        assert!(json.contains("Pete <> Chad Bi-Weekly"));
        let md = std::fs::read_to_string(folder.join("transcript.md")).unwrap();
        assert!(md.starts_with("# Pete <> Chad Bi-Weekly"), "{md}");
        assert_eq!(storage.get_meeting_path(&metadata.id).unwrap(), folder);

        // A link the meeting already has is kept.
        let again =
            retitle_stored(&storage, &metadata.id, "Sync", Some("https://other"), false).unwrap();
        assert_eq!(again.url, None);
        let stored = storage.get_meeting(&metadata.id).unwrap().unwrap();
        assert_eq!(stored.url.as_deref(), Some("https://zoom.us/j/1"));

        assert!(matches!(
            retitle_stored(&storage, &metadata.id, "  ", None, true),
            Err(RetitleError::Empty)
        ));
    }

    #[test]
    fn the_trigger_round_trips_once() {
        let dir = tempfile::tempdir().unwrap();
        let r = Retitle {
            meeting_id: "abc".into(),
            title: "Pete <> Chad".into(),
            url: None,
        };
        write_trigger(dir.path(), &r).unwrap();
        assert_eq!(take_trigger(dir.path()), Some(r));
        assert_eq!(take_trigger(dir.path()), None);
        std::fs::write(dir.path().join(TRIGGER), "not json").unwrap();
        assert_eq!(take_trigger(dir.path()), None);
        assert!(!dir.path().join(TRIGGER).exists());
    }
}
