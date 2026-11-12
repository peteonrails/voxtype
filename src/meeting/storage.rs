//! Storage layer for meeting transcription
//!
//! Provides SQLite-based index for meeting metadata and filesystem
//! storage for transcripts and audio files.

use crate::meeting::data::{
    MeetingData, MeetingId, MeetingMetadata, MeetingStatus, SegmentAnnotation, Transcript,
    TranscriptSegment,
};
use crate::meeting::manifest::{Manifest, MeetingContext, MANIFEST_FILE};
use crate::meeting::notes::MeetingNotes;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;
use thiserror::Error;

/// Storage-related errors
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Meeting not found: {0}")]
    NotFound(String),

    #[error("Storage path not configured")]
    PathNotConfigured,

    #[error(
        "Speaker '{speaker}' is not in this meeting. Speakers in the transcript: {}",
        speaker_list(.available)
    )]
    UnknownSpeaker {
        speaker: String,
        available: Vec<String>,
    },

    #[error("A speaker label cannot be empty")]
    EmptyLabel,

    #[error("Segment {segment} is not in this meeting. {}", segment_range(*.first, *.last))]
    UnknownSegment {
        segment: u32,
        first: Option<u32>,
        last: Option<u32>,
    },

    #[error("A line's text cannot be empty; use `meeting delete-line` to remove it")]
    EmptyText,

    #[error("An annotation needs text, --highlight, or both")]
    EmptyAnnotation,

    #[error("Meeting ID '{0}' matches more than one meeting; give more of it")]
    AmbiguousId(String),
}

fn segment_range(first: Option<u32>, last: Option<u32>) -> String {
    match (first, last) {
        (Some(first), Some(last)) => format!("Valid segment ids: {first} to {last}."),
        _ => "It has no segments yet.".to_string(),
    }
}

/// The user's free-text notes in a meeting folder.
pub const NOTES_FILE: &str = "notes.md";

/// The readable transcript kept beside transcript.json.
pub const TRANSCRIPT_MARKDOWN_FILE: &str = "transcript.md";

/// Render the meeting as Markdown into its folder, atomically.
fn write_transcript_markdown(dir: &std::path::Path, data: &MeetingData) -> std::io::Result<()> {
    let options = crate::meeting::export::ExportOptions {
        include_timestamps: true,
        include_speakers: true,
        include_metadata: true,
        line_width: 0,
    };
    let markdown = crate::meeting::export::export_meeting(
        data,
        crate::meeting::export::ExportFormat::Markdown,
        &options,
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    let staged = dir.join(format!(
        ".{}.{}",
        TRANSCRIPT_MARKDOWN_FILE,
        std::process::id()
    ));
    std::fs::write(&staged, markdown)?;
    std::fs::rename(&staged, dir.join(TRANSCRIPT_MARKDOWN_FILE))
}

fn speaker_list(speakers: &[String]) -> String {
    if speakers.is_empty() {
        "none yet".to_string()
    } else {
        speakers.join(", ")
    }
}

/// How a meeting's per-line edits fared when its transcript was replaced.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SegmentEditCarry {
    /// Speaker moves, text edits and deleted lines moved to a new line
    pub edits_carried: usize,
    /// Those with no new line to go to
    pub edits_dropped: usize,
    /// Those removed because they were not to be kept
    pub edits_discarded: usize,
    pub annotations_carried: usize,
    pub annotations_dropped: usize,
}

/// Meeting storage configuration
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Base path for meeting storage
    /// "auto" will use ~/.local/share/voxtype/meetings/
    pub storage_path: PathBuf,
    /// Whether to retain audio files
    pub retain_audio: bool,
    /// Maximum number of meetings to keep (0 = unlimited)
    pub max_meetings: u32,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            storage_path: Self::default_storage_path(),
            retain_audio: false,
            max_meetings: 0,
        }
    }
}

impl StorageConfig {
    /// Get the default storage path
    pub fn default_storage_path() -> PathBuf {
        crate::config::Config::data_dir().join("meetings")
    }

    /// Get the database path
    pub fn db_path(&self) -> PathBuf {
        self.storage_path.join("index.db")
    }
}

/// Meeting storage manager
pub struct MeetingStorage {
    config: StorageConfig,
    conn: Connection,
}

impl MeetingStorage {
    /// Open or create meeting storage
    pub fn open(config: StorageConfig) -> Result<Self, StorageError> {
        // Ensure storage directory exists
        std::fs::create_dir_all(&config.storage_path)?;

        let db_path = config.db_path();
        let conn = Connection::open(&db_path)?;

        let storage = Self { config, conn };
        storage.init_schema()?;

        Ok(storage)
    }

    /// Initialize database schema
    fn init_schema(&self) -> Result<(), StorageError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meetings (
                id TEXT PRIMARY KEY,
                title TEXT,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                duration_secs INTEGER,
                status TEXT NOT NULL DEFAULT 'active',
                chunk_count INTEGER NOT NULL DEFAULT 0,
                storage_path TEXT,
                audio_retained INTEGER NOT NULL DEFAULT 0,
                model TEXT,
                synced_at INTEGER,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );

            CREATE INDEX IF NOT EXISTS idx_meetings_started_at ON meetings(started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_meetings_status ON meetings(status);

            -- Speaker labels for ML diarization (Phase 3)
            CREATE TABLE IF NOT EXISTS speaker_labels (
                meeting_id TEXT NOT NULL,
                speaker_num INTEGER NOT NULL,
                label TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                PRIMARY KEY (meeting_id, speaker_num),
                FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
            );

            -- Speaker labels keyed by the speaker as the transcript names it
            -- (You, Remote, SPEAKER_00, ...). Supersedes speaker_labels, whose
            -- integer key could not name the simple-diarization speakers; its
            -- rows are carried over once and the old table is left for
            -- downgrades.
            CREATE TABLE IF NOT EXISTS speaker_name_labels (
                meeting_id TEXT NOT NULL,
                speaker_key TEXT NOT NULL,
                label TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                PRIMARY KEY (meeting_id, speaker_key)
            );
            INSERT OR IGNORE INTO speaker_name_labels (meeting_id, speaker_key, label)
                SELECT meeting_id, printf('SPEAKER_%02d', speaker_num), label
                FROM speaker_labels;

            -- One transcript line moved to another speaker by hand, keyed by
            -- segment ID. Applied over whatever the diarizer assigned on every
            -- save and load, so a later chunk cannot move the line back.
            CREATE TABLE IF NOT EXISTS segment_speaker_overrides (
                meeting_id TEXT NOT NULL,
                segment_id INTEGER NOT NULL,
                speaker_key TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                PRIMARY KEY (meeting_id, segment_id)
            );

            -- A line's text as the user corrected it, and lines the user
            -- deleted (kept, so they can be restored). Applied like the
            -- speaker overrides above.
            CREATE TABLE IF NOT EXISTS segment_text_edits (
                meeting_id TEXT NOT NULL,
                segment_id INTEGER NOT NULL,
                text TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                PRIMARY KEY (meeting_id, segment_id)
            );
            CREATE TABLE IF NOT EXISTS segment_deletions (
                meeting_id TEXT NOT NULL,
                segment_id INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                PRIMARY KEY (meeting_id, segment_id)
            );

            -- A note or highlight the user attached to one line, applied
            -- like the speaker overrides above.
            CREATE TABLE IF NOT EXISTS segment_annotations (
                meeting_id TEXT NOT NULL,
                segment_id INTEGER NOT NULL,
                text TEXT NOT NULL,
                highlight INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                PRIMARY KEY (meeting_id, segment_id)
            );

            -- What a meeting was recorded with, fixed at its start, for
            -- meeting.json.
            CREATE TABLE IF NOT EXISTS meeting_context (
                meeting_id TEXT PRIMARY KEY,
                engine TEXT,
                model TEXT,
                diarization TEXT,
                voxtype_version TEXT
            );

            -- Figures `meeting list` shows without opening each transcript:
            -- the number of speakers as displayed (labels and moves applied),
            -- refreshed on every transcript save.
            CREATE TABLE IF NOT EXISTS meeting_stats (
                meeting_id TEXT PRIMARY KEY,
                speaker_count INTEGER NOT NULL
            );

            -- The meeting's summary (MeetingSummary as JSON) and what wrote it.
            CREATE TABLE IF NOT EXISTS meeting_summaries (
                meeting_id TEXT PRIMARY KEY,
                summary_json TEXT NOT NULL,
                generated_by TEXT,
                generated_at INTEGER NOT NULL
            );
            "#,
        )?;

        // Columns added after the table first shipped. ALTER TABLE has no IF
        // NOT EXISTS, so ask first; older binaries ignore the extra column.
        let has_url = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('meetings') WHERE name = 'url'")?
            .exists([])?;
        if !has_url {
            self.conn
                .execute_batch("ALTER TABLE meetings ADD COLUMN url TEXT;")?;
        }
        let has_echo_cancel = self
            .conn
            .prepare(
                "SELECT 1 FROM pragma_table_info('meeting_context') WHERE name = 'echo_cancel'",
            )?
            .exists([])?;
        if !has_echo_cancel {
            self.conn
                .execute_batch("ALTER TABLE meeting_context ADD COLUMN echo_cancel TEXT;")?;
        }
        Ok(())
    }

    /// Create a new meeting
    pub fn create_meeting(&self, metadata: &MeetingMetadata) -> Result<PathBuf, StorageError> {
        // Create meeting directory
        let meeting_dir = self.config.storage_path.join(metadata.storage_dir_name());
        std::fs::create_dir_all(&meeting_dir)?;

        // Insert into database
        self.conn.execute(
            r#"
            INSERT INTO meetings (id, title, started_at, status, storage_path, audio_retained, model, url)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                metadata.id.to_string(),
                metadata.title,
                metadata.started_at.timestamp(),
                status_to_string(metadata.status),
                meeting_dir.to_string_lossy().to_string(),
                metadata.audio_retained as i32,
                metadata.model,
                metadata.url,
            ],
        )?;

        // Write initial metadata file
        let metadata_path = meeting_dir.join("metadata.json");
        let json = serde_json::to_string_pretty(metadata)?;
        std::fs::write(&metadata_path, json)?;

        Ok(meeting_dir)
    }

    /// Update meeting metadata
    pub fn update_meeting(&self, metadata: &MeetingMetadata) -> Result<(), StorageError> {
        self.conn.execute(
            r#"
            UPDATE meetings SET
                title = ?2,
                ended_at = ?3,
                duration_secs = ?4,
                status = ?5,
                chunk_count = ?6,
                audio_retained = ?7,
                model = ?8,
                synced_at = ?9,
                url = ?10
            WHERE id = ?1
            "#,
            params![
                metadata.id.to_string(),
                metadata.title,
                metadata.ended_at.map(|dt| dt.timestamp()),
                metadata.duration_secs.map(|d| d as i64),
                status_to_string(metadata.status),
                metadata.chunk_count as i32,
                metadata.audio_retained as i32,
                metadata.model,
                metadata.synced_at.map(|dt| dt.timestamp()),
                metadata.url,
            ],
        )?;

        // Update metadata file if storage path exists
        if let Some(ref path) = metadata.storage_path {
            let metadata_path = path.join("metadata.json");
            // A caller holding metadata from before the summary was written
            // must not drop it from the file.
            let json = match metadata.summary {
                None => {
                    let mut with_summary = metadata.clone();
                    with_summary.summary = self.get_summary(&metadata.id)?;
                    serde_json::to_string_pretty(&with_summary)?
                }
                Some(_) => serde_json::to_string_pretty(metadata)?,
            };
            std::fs::write(metadata_path, json)?;
            self.refresh_manifest(&metadata.id);
        }

        Ok(())
    }

    /// Mark any active/paused meetings as completed.
    /// Called on daemon startup to clean up meetings orphaned by a previous crash or restart.
    pub fn complete_stale_meetings(&self) -> Result<u32, StorageError> {
        let stale: Vec<String> = self
            .conn
            .prepare("SELECT id FROM meetings WHERE status IN ('active', 'paused')")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        let now = Utc::now().timestamp();
        let count = self.conn.execute(
            r#"
            UPDATE meetings SET
                status = 'completed',
                ended_at = ?1,
                duration_secs = (?1 - started_at)
            WHERE status IN ('active', 'paused')
            "#,
            params![now],
        )?;
        // Their manifests still say "active"; the audio a crash left
        // behind is already playable, so the manifest is all that is stale.
        for id in stale.iter().filter_map(|id| MeetingId::parse(id).ok()) {
            self.refresh_manifest(&id);
        }
        Ok(count as u32)
    }

    /// Get meeting by ID
    pub fn get_meeting(&self, id: &MeetingId) -> Result<Option<MeetingMetadata>, StorageError> {
        let result = self
            .conn
            .query_row(
                r#"
                SELECT id, title, started_at, ended_at, duration_secs, status,
                       chunk_count, storage_path, audio_retained, model, synced_at, url
                FROM meetings WHERE id = ?1
                "#,
                params![id.to_string()],
                |row| {
                    Ok(MeetingMetadata {
                        id: MeetingId::parse(&row.get::<_, String>(0)?).unwrap_or_default(),
                        title: row.get(1)?,
                        started_at: timestamp_to_datetime(row.get(2)?),
                        ended_at: row.get::<_, Option<i64>>(3)?.map(timestamp_to_datetime),
                        duration_secs: row.get::<_, Option<i64>>(4)?.map(|d| d as u64),
                        status: string_to_status(&row.get::<_, String>(5)?),
                        chunk_count: row.get::<_, i32>(6)? as u32,
                        storage_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
                        audio_retained: row.get::<_, i32>(8)? != 0,
                        model: row.get(9)?,
                        url: row.get(11)?,
                        summary: None,
                        synced_at: row.get::<_, Option<i64>>(10)?.map(timestamp_to_datetime),
                    })
                },
            )
            .optional()?;

        let mut result = result;
        if let Some(ref mut metadata) = result {
            metadata.summary = self.get_summary(id)?;
        }
        Ok(result)
    }

    /// Store a meeting's summary in the index and in metadata.json.
    pub fn save_summary(
        &self,
        meeting_id: &MeetingId,
        summary: &crate::meeting::data::MeetingSummary,
    ) -> Result<(), StorageError> {
        let mut metadata = self
            .get_meeting(meeting_id)?
            .ok_or_else(|| StorageError::NotFound(meeting_id.to_string()))?;
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO meeting_summaries
                (meeting_id, summary_json, generated_by, generated_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![
                meeting_id.to_string(),
                serde_json::to_string(summary)?,
                summary.model,
                summary.generated_at.timestamp(),
            ],
        )?;
        metadata.summary = Some(summary.clone());
        self.update_meeting(&metadata)
    }

    /// The meeting's stored summary, if it has one.
    pub fn get_summary(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<Option<crate::meeting::data::MeetingSummary>, StorageError> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT summary_json FROM meeting_summaries WHERE meeting_id = ?1",
                params![meeting_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// List meetings with optional limit
    pub fn list_meetings(&self, limit: Option<u32>) -> Result<Vec<MeetingMetadata>, StorageError> {
        let sql = if limit.is_some() {
            r#"
                SELECT id, title, started_at, ended_at, duration_secs, status,
                       chunk_count, storage_path, audio_retained, model, synced_at, url
                FROM meetings
                ORDER BY started_at DESC
                LIMIT ?1
                "#
        } else {
            r#"
                SELECT id, title, started_at, ended_at, duration_secs, status,
                       chunk_count, storage_path, audio_retained, model, synced_at, url
                FROM meetings
                ORDER BY started_at DESC
                "#
        };

        let mut stmt = self.conn.prepare(sql)?;
        let row_mapper = |row: &rusqlite::Row| {
            Ok(MeetingMetadata {
                id: MeetingId::parse(&row.get::<_, String>(0)?).unwrap_or_default(),
                title: row.get(1)?,
                started_at: timestamp_to_datetime(row.get(2)?),
                ended_at: row.get::<_, Option<i64>>(3)?.map(timestamp_to_datetime),
                duration_secs: row.get::<_, Option<i64>>(4)?.map(|d| d as u64),
                status: string_to_status(&row.get::<_, String>(5)?),
                chunk_count: row.get::<_, i32>(6)? as u32,
                storage_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
                audio_retained: row.get::<_, i32>(8)? != 0,
                model: row.get(9)?,
                url: row.get(11)?,
                summary: None,
                synced_at: row.get::<_, Option<i64>>(10)?.map(timestamp_to_datetime),
            })
        };

        let meetings = if let Some(limit) = limit {
            stmt.query_map(params![limit], row_mapper)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], row_mapper)?
                .collect::<Result<Vec<_>, _>>()?
        };

        Ok(meetings)
    }

    /// Meetings for `meeting list --json`: each one's metadata plus whether
    /// it has a summary and how many speakers it shows. Both come from the
    /// index; a meeting saved before the speaker count was kept is counted
    /// from its transcript once and remembered.
    pub fn list_meeting_entries(
        &self,
        limit: Option<u32>,
    ) -> Result<Vec<MeetingListEntry>, StorageError> {
        let meetings = self.list_meetings(limit)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT m.id,
                   EXISTS (SELECT 1 FROM meeting_summaries s WHERE s.meeting_id = m.id),
                   st.speaker_count
            FROM meetings m LEFT JOIN meeting_stats st ON st.meeting_id = m.id
            "#,
        )?;
        let known: std::collections::HashMap<String, (bool, Option<u32>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (row.get::<_, bool>(1)?, row.get::<_, Option<u32>>(2)?),
                ))
            })?
            .collect::<Result<_, _>>()?;

        let mut entries = Vec::with_capacity(meetings.len());
        for metadata in meetings {
            let (has_summary, count) = known
                .get(&metadata.id.to_string())
                .cloned()
                .unwrap_or((false, None));
            let speaker_count = match count {
                Some(n) => n,
                None => self.backfill_speaker_count(&metadata.id),
            };
            entries.push(MeetingListEntry {
                metadata,
                has_summary,
                speaker_count,
            });
        }
        Ok(entries)
    }

    /// Count a meeting's speakers from its transcript and remember it. A
    /// meeting without a transcript yet has none and is not remembered.
    fn backfill_speaker_count(&self, meeting_id: &MeetingId) -> u32 {
        match self.load_transcript(meeting_id) {
            Ok(transcript) => {
                let _ = self.store_speaker_count(meeting_id, &transcript);
                displayed_speaker_count(&transcript)
            }
            Err(_) => 0,
        }
    }

    fn store_speaker_count(
        &self,
        meeting_id: &MeetingId,
        transcript: &Transcript,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meeting_stats (meeting_id, speaker_count) VALUES (?1, ?2)",
            params![meeting_id.to_string(), displayed_speaker_count(transcript)],
        )?;
        Ok(())
    }

    /// Get the most recent meeting
    pub fn get_latest_meeting(&self) -> Result<Option<MeetingMetadata>, StorageError> {
        let meetings = self.list_meetings(Some(1))?;
        Ok(meetings.into_iter().next())
    }

    /// Save transcript to filesystem
    pub fn save_transcript(
        &self,
        meeting_id: &MeetingId,
        transcript: &Transcript,
    ) -> Result<(), StorageError> {
        let metadata = self
            .get_meeting(meeting_id)?
            .ok_or_else(|| StorageError::NotFound(meeting_id.to_string()))?;

        let storage_path = metadata
            .storage_path
            .clone()
            .ok_or(StorageError::PathNotConfigured)?;

        // Labels live in the index, not in the caller's transcript: the
        // daemon rewrites transcript.json after every chunk from its own
        // in-memory copy, which knows nothing of a `meeting label` run
        // mid-meeting. Applying them here keeps every write labelled.
        let mut edited = transcript.clone();
        self.apply_edits(meeting_id, &mut edited)?;
        let transcript_path = storage_path.join("transcript.json");
        std::fs::write(transcript_path, serde_json::to_string_pretty(&edited)?)?;

        // transcript.md beside it, so the folder reads without voxtype.
        // Deleted lines stay in transcript.json as tombstones but count for
        // nothing: not in the markdown, and not toward speaker_count.
        let notes = self.load_notes(meeting_id)?.notes;
        edited.remove_deleted();
        self.store_speaker_count(meeting_id, &edited)?;
        let data = MeetingData {
            metadata,
            transcript: edited,
            notes: (!notes.trim().is_empty()).then_some(notes),
        };
        if let Err(e) = write_transcript_markdown(&storage_path, &data) {
            tracing::warn!("Failed to write transcript.md: {}", e);
        }
        // Every hand edit (label, reassign, text, delete) saves through
        // here, and each can change meeting.json's speakers.
        self.refresh_manifest(meeting_id);

        Ok(())
    }

    /// Everything the user changed by hand, which lives in the index rather
    /// than in the daemon's in-memory transcript: speakers moved line by
    /// line, then speaker names, then annotations.
    pub fn apply_edits(
        &self,
        meeting_id: &MeetingId,
        transcript: &mut Transcript,
    ) -> Result<(), StorageError> {
        transcript.apply_speaker_overrides(&self.get_speaker_overrides(meeting_id)?);
        transcript.apply_speaker_labels(&self.get_speaker_labels(meeting_id)?);
        transcript.apply_annotations(&self.get_annotations(meeting_id)?);
        transcript.apply_text_edits(&self.get_text_edits(meeting_id)?);
        transcript.apply_deletions(&self.get_deletions(meeting_id)?);
        Ok(())
    }

    /// Load the transcript as voxtype shows it: every edit applied and
    /// deleted lines left out.
    pub fn load_transcript(&self, meeting_id: &MeetingId) -> Result<Transcript, StorageError> {
        let mut transcript = self.load_transcript_all(meeting_id)?;
        transcript.remove_deleted();
        Ok(transcript)
    }

    /// Load the transcript with deleted lines still in it, marked `deleted`.
    /// Anything that saves the transcript back must start from this, or the
    /// deleted lines would be lost for good.
    pub(crate) fn load_transcript_all(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<Transcript, StorageError> {
        let metadata = self
            .get_meeting(meeting_id)?
            .ok_or_else(|| StorageError::NotFound(meeting_id.to_string()))?;

        let storage_path = metadata
            .storage_path
            .ok_or(StorageError::PathNotConfigured)?;

        let transcript_path = storage_path.join("transcript.json");
        let json = std::fs::read_to_string(transcript_path)?;
        let mut transcript: Transcript = serde_json::from_str(&json)?;
        transcript.sort_chronologically();
        self.apply_edits(meeting_id, &mut transcript)?;

        Ok(transcript)
    }

    /// Load complete meeting data (metadata + transcript)
    pub fn load_meeting_data(&self, meeting_id: &MeetingId) -> Result<MeetingData, StorageError> {
        let metadata = self
            .get_meeting(meeting_id)?
            .ok_or_else(|| StorageError::NotFound(meeting_id.to_string()))?;

        let transcript = self.load_transcript(meeting_id).unwrap_or_default();
        let notes = self.load_notes(meeting_id)?.notes;

        Ok(MeetingData {
            metadata,
            transcript,
            notes: (!notes.trim().is_empty()).then_some(notes),
        })
    }

    /// Delete a meeting and its files
    pub fn delete_meeting(&self, meeting_id: &MeetingId) -> Result<(), StorageError> {
        // Get storage path before deleting from DB
        let metadata = self.get_meeting(meeting_id)?;

        // Delete from database
        self.conn.execute(
            "DELETE FROM meetings WHERE id = ?1",
            params![meeting_id.to_string()],
        )?;
        self.conn.execute(
            "DELETE FROM speaker_name_labels WHERE meeting_id = ?1",
            params![meeting_id.to_string()],
        )?;
        self.conn.execute(
            "DELETE FROM segment_speaker_overrides WHERE meeting_id = ?1",
            params![meeting_id.to_string()],
        )?;
        self.conn.execute(
            "DELETE FROM segment_annotations WHERE meeting_id = ?1",
            params![meeting_id.to_string()],
        )?;
        self.conn.execute(
            "DELETE FROM meeting_context WHERE meeting_id = ?1",
            params![meeting_id.to_string()],
        )?;
        self.conn.execute(
            "DELETE FROM meeting_summaries WHERE meeting_id = ?1",
            params![meeting_id.to_string()],
        )?;
        self.conn.execute(
            "DELETE FROM meeting_stats WHERE meeting_id = ?1",
            params![meeting_id.to_string()],
        )?;
        for table in ["segment_text_edits", "segment_deletions"] {
            self.conn.execute(
                &format!("DELETE FROM {table} WHERE meeting_id = ?1"),
                params![meeting_id.to_string()],
            )?;
        }

        // Delete files if storage path exists
        if let Some(metadata) = metadata {
            if let Some(path) = metadata.storage_path {
                if path.exists() {
                    std::fs::remove_dir_all(path)?;
                }
            }
        }

        Ok(())
    }

    /// Get the storage path for a meeting
    pub fn get_meeting_path(&self, meeting_id: &MeetingId) -> Result<PathBuf, StorageError> {
        let metadata = self
            .get_meeting(meeting_id)?
            .ok_or_else(|| StorageError::NotFound(meeting_id.to_string()))?;

        metadata.storage_path.ok_or(StorageError::PathNotConfigured)
    }

    /// Resolve a meeting ID from a string (supports "latest" alias)
    pub fn resolve_meeting_id(&self, id_str: &str) -> Result<MeetingId, StorageError> {
        if id_str == "latest" {
            let meeting = self
                .get_latest_meeting()?
                .ok_or_else(|| StorageError::NotFound("No meetings found".to_string()))?;
            Ok(meeting.id)
        } else if let Ok(id) = MeetingId::parse(id_str) {
            Ok(id)
        } else {
            self.resolve_meeting_id_prefix(id_str)
        }
    }

    /// A meeting by the start of its ID, as `meeting list` shows it.
    fn resolve_meeting_id_prefix(&self, prefix: &str) -> Result<MeetingId, StorageError> {
        let invalid = || StorageError::NotFound(format!("Invalid meeting ID: {}", prefix));
        if prefix.len() < 4 || !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            return Err(invalid());
        }
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM meetings WHERE id LIKE ?1 || '%' LIMIT 2")?;
        let ids = stmt
            .query_map(params![prefix.to_ascii_lowercase()], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        match ids.as_slice() {
            [only] => MeetingId::parse(only).map_err(|_| invalid()),
            [] => Err(StorageError::NotFound(format!(
                "No meeting with ID {}",
                prefix
            ))),
            _ => Err(StorageError::AmbiguousId(prefix.to_string())),
        }
    }

    /// Name a speaker in a meeting.
    ///
    /// `speaker` is matched against the speakers the transcript actually
    /// contains: the diarizer's ID (`You`, `Remote`, `SPEAKER_00`), the
    /// numeric shorthand `0` for `SPEAKER_00`, any of those ignoring case,
    /// or a name already assigned, which renames every speaker carrying it.
    /// Anything else is [`StorageError::UnknownSpeaker`], so a typo never
    /// reports success while changing nothing. Returns the speaker IDs that
    /// were labelled.
    pub fn set_speaker_label(
        &self,
        meeting_id: &MeetingId,
        speaker: &str,
        label: &str,
    ) -> Result<Vec<String>, StorageError> {
        let label = label.trim();
        if label.is_empty() {
            return Err(StorageError::EmptyLabel);
        }
        self.get_meeting(meeting_id)?
            .ok_or_else(|| StorageError::NotFound(meeting_id.to_string()))?;

        // A meeting that has not finished its first chunk has no file yet.
        let transcript = match self.load_transcript_all(meeting_id) {
            Ok(t) => t,
            Err(StorageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                Transcript::new()
            }
            Err(e) => return Err(e),
        };

        let keys = resolve_speaker_keys(&transcript, speaker);
        if keys.is_empty() {
            return Err(StorageError::UnknownSpeaker {
                speaker: speaker.to_string(),
                available: transcript.speaker_keys(),
            });
        }

        for key in &keys {
            self.conn.execute(
                r#"
                INSERT OR REPLACE INTO speaker_name_labels (meeting_id, speaker_key, label)
                VALUES (?1, ?2, ?3)
                "#,
                params![meeting_id.to_string(), key, label],
            )?;
        }

        // Rewrite the file so readers of transcript.json see the name too.
        self.save_transcript(meeting_id, &transcript)?;

        Ok(keys)
    }

    /// Move a speaker's label to a new speaker ID, for when the diarizer
    /// renames a speaker mid-meeting. A label already on `to` wins.
    pub fn rename_speaker_key(
        &self,
        meeting_id: &MeetingId,
        from: &str,
        to: &str,
    ) -> Result<(), StorageError> {
        let id = meeting_id.to_string();
        self.conn.execute(
            "UPDATE OR IGNORE speaker_name_labels SET speaker_key = ?3 \
             WHERE meeting_id = ?1 AND speaker_key = ?2",
            params![id, from, to],
        )?;
        self.conn.execute(
            "DELETE FROM speaker_name_labels WHERE meeting_id = ?1 AND speaker_key = ?2",
            params![id, from],
        )?;
        // A line moved to `from` by hand moves with it: it is the same person.
        self.conn.execute(
            "UPDATE segment_speaker_overrides SET speaker_key = ?3 \
             WHERE meeting_id = ?1 AND speaker_key = ?2",
            params![id, from, to],
        )?;
        Ok(())
    }

    /// Move one transcript line to another speaker.
    ///
    /// `speaker` is matched as `meeting label` matches it: a speaker ID in
    /// the transcript (`You`, `Remote 1`, `SPEAKER_00` or `0`), ignoring case,
    /// or a name already given with `meeting label`. Text that matches no one
    /// creates a new speaker with that name. The move is stored per segment
    /// and applied over the diarizer's choice on every save and load.
    ///
    /// When `rewrite` is false the transcript file is left for the running
    /// meeting daemon to rewrite, so this process cannot clobber a chunk the
    /// daemon saved in between. Returns the segment as it now reads.
    pub fn reassign_segment(
        &self,
        meeting_id: &MeetingId,
        segment_id: u32,
        speaker: &str,
        rewrite: bool,
    ) -> Result<TranscriptSegment, StorageError> {
        let speaker = speaker.trim();
        if speaker.is_empty() {
            return Err(StorageError::EmptyLabel);
        }
        let mut transcript = self.transcript_with_segment(meeting_id, segment_id)?;

        let key = match resolve_speaker_keys(&transcript, speaker)
            .into_iter()
            .next()
        {
            Some(key) => key,
            None => match self.speaker_key_named(meeting_id, speaker)? {
                Some(key) => key,
                None => {
                    let key = self.fresh_speaker_key(meeting_id, &transcript)?;
                    self.conn.execute(
                        "INSERT OR REPLACE INTO speaker_name_labels (meeting_id, speaker_key, label) \
                         VALUES (?1, ?2, ?3)",
                        params![meeting_id.to_string(), key, speaker],
                    )?;
                    key
                }
            },
        };

        self.conn.execute(
            "INSERT OR REPLACE INTO segment_speaker_overrides (meeting_id, segment_id, speaker_key) \
             VALUES (?1, ?2, ?3)",
            params![meeting_id.to_string(), segment_id, key],
        )?;
        self.finish_segment_change(meeting_id, &mut transcript, segment_id, rewrite)
    }

    /// Give a transcript line back to the speaker the diarizer assigned.
    pub fn reset_segment_speaker(
        &self,
        meeting_id: &MeetingId,
        segment_id: u32,
        rewrite: bool,
    ) -> Result<TranscriptSegment, StorageError> {
        let mut transcript = self.transcript_with_segment(meeting_id, segment_id)?;
        self.conn.execute(
            "DELETE FROM segment_speaker_overrides WHERE meeting_id = ?1 AND segment_id = ?2",
            params![meeting_id.to_string(), segment_id],
        )?;
        self.finish_segment_change(meeting_id, &mut transcript, segment_id, rewrite)
    }

    /// Attach a note or highlight to one line, replacing any it had.
    ///
    /// As with [`MeetingStorage::reassign_segment`], `rewrite` is false
    /// while the meeting daemon owns the transcript file. Returns the
    /// segment as it now reads.
    pub fn annotate_segment(
        &self,
        meeting_id: &MeetingId,
        segment_id: u32,
        text: &str,
        highlight: bool,
        rewrite: bool,
    ) -> Result<TranscriptSegment, StorageError> {
        let text = text.trim();
        if text.is_empty() && !highlight {
            return Err(StorageError::EmptyAnnotation);
        }
        let mut transcript = self.transcript_with_segment(meeting_id, segment_id)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO segment_annotations \
             (meeting_id, segment_id, text, highlight, updated_at) \
             VALUES (?1, ?2, ?3, ?4, strftime('%s', 'now'))",
            params![meeting_id.to_string(), segment_id, text, highlight],
        )?;
        self.finish_segment_change(meeting_id, &mut transcript, segment_id, rewrite)
    }

    /// Remove the note or highlight on one line.
    pub fn remove_annotation(
        &self,
        meeting_id: &MeetingId,
        segment_id: u32,
        rewrite: bool,
    ) -> Result<TranscriptSegment, StorageError> {
        let mut transcript = self.transcript_with_segment(meeting_id, segment_id)?;
        self.conn.execute(
            "DELETE FROM segment_annotations WHERE meeting_id = ?1 AND segment_id = ?2",
            params![meeting_id.to_string(), segment_id],
        )?;
        self.finish_segment_change(meeting_id, &mut transcript, segment_id, rewrite)
    }

    /// Replace one line's text with a correction. The transcribed text is
    /// kept and shown as `original_text`; [`MeetingStorage::reset_segment_text`]
    /// restores it. `rewrite` as for [`MeetingStorage::reassign_segment`].
    pub fn edit_segment_text(
        &self,
        meeting_id: &MeetingId,
        segment_id: u32,
        text: &str,
        rewrite: bool,
    ) -> Result<TranscriptSegment, StorageError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(StorageError::EmptyText);
        }
        let mut transcript = self.transcript_with_segment(meeting_id, segment_id)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO segment_text_edits (meeting_id, segment_id, text, updated_at) \
             VALUES (?1, ?2, ?3, strftime('%s', 'now'))",
            params![meeting_id.to_string(), segment_id, text],
        )?;
        self.finish_segment_change(meeting_id, &mut transcript, segment_id, rewrite)
    }

    /// Give a line back the text it was transcribed with.
    pub fn reset_segment_text(
        &self,
        meeting_id: &MeetingId,
        segment_id: u32,
        rewrite: bool,
    ) -> Result<TranscriptSegment, StorageError> {
        let mut transcript = self.transcript_with_segment(meeting_id, segment_id)?;
        self.conn.execute(
            "DELETE FROM segment_text_edits WHERE meeting_id = ?1 AND segment_id = ?2",
            params![meeting_id.to_string(), segment_id],
        )?;
        self.finish_segment_change(meeting_id, &mut transcript, segment_id, rewrite)
    }

    /// Hide one line. It stays in transcript.json, marked deleted, so
    /// [`MeetingStorage::restore_segment`] can bring it back; everything that
    /// reads the meeting through voxtype leaves it out. Returns the line,
    /// marked deleted.
    pub fn delete_segment(
        &self,
        meeting_id: &MeetingId,
        segment_id: u32,
        rewrite: bool,
    ) -> Result<TranscriptSegment, StorageError> {
        let mut transcript = self.transcript_with_segment(meeting_id, segment_id)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO segment_deletions (meeting_id, segment_id) VALUES (?1, ?2)",
            params![meeting_id.to_string(), segment_id],
        )?;
        self.finish_segment_change(meeting_id, &mut transcript, segment_id, rewrite)
    }

    /// Bring back a deleted line.
    pub fn restore_segment(
        &self,
        meeting_id: &MeetingId,
        segment_id: u32,
        rewrite: bool,
    ) -> Result<TranscriptSegment, StorageError> {
        let mut transcript = self.transcript_with_segment(meeting_id, segment_id)?;
        self.conn.execute(
            "DELETE FROM segment_deletions WHERE meeting_id = ?1 AND segment_id = ?2",
            params![meeting_id.to_string(), segment_id],
        )?;
        self.finish_segment_change(meeting_id, &mut transcript, segment_id, rewrite)
    }

    /// The user's text corrections: segment ID to text.
    pub fn get_text_edits(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<std::collections::HashMap<u32, String>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT segment_id, text FROM segment_text_edits WHERE meeting_id = ?1")?;
        let edits = stmt
            .query_map(params![meeting_id.to_string()], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
        Ok(edits)
    }

    /// The segment IDs the user deleted.
    pub fn get_deletions(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<std::collections::HashSet<u32>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT segment_id FROM segment_deletions WHERE meeting_id = ?1")?;
        let deleted = stmt
            .query_map(params![meeting_id.to_string()], |row| row.get::<_, u32>(0))?
            .collect::<Result<std::collections::HashSet<_>, _>>()?;
        Ok(deleted)
    }

    /// The user's notes and highlights: segment ID to annotation.
    pub fn get_annotations(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<std::collections::HashMap<u32, SegmentAnnotation>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT segment_id, text, highlight FROM segment_annotations WHERE meeting_id = ?1",
        )?;
        let annotations = stmt
            .query_map(params![meeting_id.to_string()], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    SegmentAnnotation {
                        text: row.get(1)?,
                        highlight: row.get(2)?,
                    },
                ))
            })?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
        Ok(annotations)
    }

    /// The meeting's notes.md. A meeting without notes has empty `notes`
    /// and no `updated_at`.
    pub fn load_notes(&self, meeting_id: &MeetingId) -> Result<MeetingNotes, StorageError> {
        let path = self.get_meeting_path(meeting_id)?.join(NOTES_FILE);
        match std::fs::read_to_string(&path) {
            Ok(notes) => Ok(MeetingNotes {
                meeting_id: meeting_id.to_string(),
                notes,
                updated_at: std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .map(DateTime::<Utc>::from),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(MeetingNotes {
                meeting_id: meeting_id.to_string(),
                notes: String::new(),
                updated_at: None,
            }),
            Err(e) => Err(e.into()),
        }
    }

    /// Replace the meeting's notes. The meeting daemon never writes
    /// notes.md, so this is safe while it records. Written to a temporary
    /// file and renamed into place, so a reader never sees half of it.
    pub fn set_notes(
        &self,
        meeting_id: &MeetingId,
        notes: &str,
    ) -> Result<MeetingNotes, StorageError> {
        let dir = self.get_meeting_path(meeting_id)?;
        let path = dir.join(NOTES_FILE);
        let staged = dir.join(format!(".{}.{}", NOTES_FILE, std::process::id()));
        std::fs::write(&staged, notes)?;
        std::fs::rename(&staged, &path)?;
        self.refresh_transcript_markdown(meeting_id);
        self.refresh_manifest(meeting_id);
        self.load_notes(meeting_id)
    }

    /// Add a paragraph to the end of the meeting's notes.
    pub fn append_notes(
        &self,
        meeting_id: &MeetingId,
        text: &str,
    ) -> Result<MeetingNotes, StorageError> {
        let mut notes = self.load_notes(meeting_id)?.notes;
        if !notes.is_empty() && !notes.ends_with('\n') {
            notes.push('\n');
        }
        notes.push_str(text.trim_end());
        notes.push('\n');
        self.set_notes(meeting_id, &notes)
    }

    /// Rewrite transcript.md from what is saved, after a change that does
    /// not go through save_transcript. Best effort: the next save writes
    /// it again anyway.
    fn refresh_transcript_markdown(&self, meeting_id: &MeetingId) {
        let result = self.load_meeting_data(meeting_id).and_then(|data| {
            let dir = self.get_meeting_path(meeting_id)?;
            write_transcript_markdown(&dir, &data).map_err(StorageError::Io)
        });
        if let Err(e) = result {
            tracing::warn!("Failed to refresh transcript.md: {}", e);
        }
    }

    /// Move the meeting's per-line edits from old segment IDs to new ones,
    /// after its transcript was replaced (`meeting retranscribe`).
    ///
    /// Annotations always follow `mapping`. Speaker moves, text edits and
    /// deleted lines follow it with `keep_edits`, and are dropped without.
    /// An edit whose line has no entry in `mapping`, or whose new line
    /// already took another edit of the same kind, is dropped. One
    /// transaction: a failure leaves the edits as they were.
    pub fn remap_segment_edits(
        &self,
        meeting_id: &MeetingId,
        mapping: &std::collections::HashMap<u32, u32>,
        keep_edits: bool,
    ) -> Result<SegmentEditCarry, StorageError> {
        // Moved rows are parked above every real ID first, so a new ID that
        // equals an old one not yet moved cannot collide with it.
        const PARKED: i64 = 1 << 40;
        let id = meeting_id.to_string();
        let tx = self.conn.unchecked_transaction()?;
        let remap = |table: &str| -> Result<(usize, usize), StorageError> {
            let olds: Vec<i64> = tx
                .prepare(&format!(
                    "SELECT segment_id FROM {table} WHERE meeting_id = ?1"
                ))?
                .query_map(params![id], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            let mut carried = 0;
            for old in olds {
                if let Some(&new) = u32::try_from(old).ok().and_then(|o| mapping.get(&o)) {
                    carried += tx.execute(
                        &format!(
                            "UPDATE OR IGNORE {table} SET segment_id = ?3 \
                             WHERE meeting_id = ?1 AND segment_id = ?2"
                        ),
                        params![id, old, new as i64 + PARKED],
                    )?;
                }
            }
            let dropped = tx.execute(
                &format!("DELETE FROM {table} WHERE meeting_id = ?1 AND segment_id < ?2"),
                params![id, PARKED],
            )?;
            tx.execute(
                &format!(
                    "UPDATE {table} SET segment_id = segment_id - ?2 \
                     WHERE meeting_id = ?1 AND segment_id >= ?2"
                ),
                params![id, PARKED],
            )?;
            Ok((carried, dropped))
        };

        let mut carry = SegmentEditCarry::default();
        for table in [
            "segment_speaker_overrides",
            "segment_text_edits",
            "segment_deletions",
        ] {
            if keep_edits {
                let (carried, dropped) = remap(table)?;
                carry.edits_carried += carried;
                carry.edits_dropped += dropped;
            } else {
                carry.edits_discarded += tx.execute(
                    &format!("DELETE FROM {table} WHERE meeting_id = ?1"),
                    params![id],
                )?;
            }
        }
        let (carried, dropped) = remap("segment_annotations")?;
        carry.annotations_carried = carried;
        carry.annotations_dropped = dropped;
        tx.commit()?;
        Ok(carry)
    }

    /// Record what the meeting is recorded with, and rewrite its manifest.
    pub fn set_context(
        &self,
        meeting_id: &MeetingId,
        context: &MeetingContext,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO meeting_context
                (meeting_id, engine, model, diarization, voxtype_version, echo_cancel)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                meeting_id.to_string(),
                context.engine,
                context.model,
                context.diarization,
                context.voxtype_version,
                context.echo_cancel,
            ],
        )?;
        self.refresh_manifest(meeting_id);
        Ok(())
    }

    /// What the meeting was recorded with; empty for meetings recorded
    /// before voxtype kept it.
    pub fn get_context(&self, meeting_id: &MeetingId) -> Result<MeetingContext, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"
                SELECT engine, model, diarization, voxtype_version, echo_cancel
                FROM meeting_context WHERE meeting_id = ?1
                "#,
                params![meeting_id.to_string()],
                |row| {
                    Ok(MeetingContext {
                        engine: row.get(0)?,
                        model: row.get(1)?,
                        diarization: row.get(2)?,
                        voxtype_version: row.get(3)?,
                        echo_cancel: row.get(4)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Write the meeting's `meeting.json` from what is saved, and return
    /// its path.
    pub fn write_manifest(&self, meeting_id: &MeetingId) -> Result<PathBuf, StorageError> {
        let metadata = self
            .get_meeting(meeting_id)?
            .ok_or_else(|| StorageError::NotFound(meeting_id.to_string()))?;
        let dir = metadata
            .storage_path
            .clone()
            .ok_or(StorageError::PathNotConfigured)?;
        let transcript = match self.load_transcript(meeting_id) {
            Ok(t) => t,
            Err(StorageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                Transcript::new()
            }
            Err(e) => return Err(e),
        };
        let manifest = Manifest::build(
            &dir,
            &metadata,
            &self.get_context(meeting_id)?,
            &transcript.speaker_keys(),
            &self.get_speaker_labels(meeting_id)?,
        );
        manifest.write(&dir)?;
        Ok(dir.join(MANIFEST_FILE))
    }

    /// Rewrite `meeting.json` after a change. Best effort: a failure is
    /// logged, and the next change writes it again.
    pub fn refresh_manifest(&self, meeting_id: &MeetingId) {
        if let Err(e) = self.write_manifest(meeting_id) {
            tracing::warn!("Failed to write meeting.json for {}: {}", meeting_id, e);
        }
    }

    /// Lines moved to another speaker by hand: segment ID to speaker ID.
    pub fn get_speaker_overrides(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<std::collections::HashMap<u32, String>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT segment_id, speaker_key FROM segment_speaker_overrides WHERE meeting_id = ?1",
        )?;
        let overrides = stmt
            .query_map(params![meeting_id.to_string()], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
        Ok(overrides)
    }

    /// The meeting's transcript, or an error naming the valid segment IDs
    /// when it has no segment `segment_id`.
    fn transcript_with_segment(
        &self,
        meeting_id: &MeetingId,
        segment_id: u32,
    ) -> Result<Transcript, StorageError> {
        self.get_meeting(meeting_id)?
            .ok_or_else(|| StorageError::NotFound(meeting_id.to_string()))?;
        let transcript = match self.load_transcript_all(meeting_id) {
            Ok(t) => t,
            Err(StorageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                Transcript::new()
            }
            Err(e) => return Err(e),
        };
        if !transcript.segments.iter().any(|s| s.id == segment_id) {
            return Err(StorageError::UnknownSegment {
                segment: segment_id,
                first: transcript.segments.iter().map(|s| s.id).min(),
                last: transcript.segments.iter().map(|s| s.id).max(),
            });
        }
        Ok(transcript)
    }

    fn finish_segment_change(
        &self,
        meeting_id: &MeetingId,
        transcript: &mut Transcript,
        segment_id: u32,
        rewrite: bool,
    ) -> Result<TranscriptSegment, StorageError> {
        if rewrite {
            self.save_transcript(meeting_id, transcript)?;
        }
        self.apply_edits(meeting_id, transcript)?;
        Ok(transcript
            .segments
            .iter()
            .find(|s| s.id == segment_id)
            .cloned()
            .expect("checked by transcript_with_segment"))
    }

    /// A speaker ID given `name` with `meeting label`, even if none of its
    /// lines are left in the transcript.
    fn speaker_key_named(
        &self,
        meeting_id: &MeetingId,
        name: &str,
    ) -> Result<Option<String>, StorageError> {
        let mut keys: Vec<String> = self
            .get_speaker_labels(meeting_id)?
            .into_iter()
            .filter(|(_, label)| label.eq_ignore_ascii_case(name))
            .map(|(key, _)| key)
            .collect();
        keys.sort();
        Ok(keys.into_iter().next())
    }

    /// `Person N` for the lowest N no speaker, label or override uses. No
    /// diarizer emits that form, so it cannot collide with a later chunk.
    fn fresh_speaker_key(
        &self,
        meeting_id: &MeetingId,
        transcript: &Transcript,
    ) -> Result<String, StorageError> {
        let mut used: std::collections::HashSet<String> =
            transcript.speaker_keys().into_iter().collect();
        used.extend(
            transcript
                .segments
                .iter()
                .filter_map(|s| s.original_speaker.clone()),
        );
        used.extend(self.get_speaker_labels(meeting_id)?.into_keys());
        used.extend(self.get_speaker_overrides(meeting_id)?.into_values());
        Ok((1..)
            .map(|n| format!("Person {n}"))
            .find(|key| !used.contains(key))
            .expect("unbounded"))
    }

    /// All speaker labels for a meeting, keyed by speaker ID.
    pub fn get_speaker_labels(
        &self,
        meeting_id: &MeetingId,
    ) -> Result<std::collections::HashMap<String, String>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT speaker_key, label FROM speaker_name_labels WHERE meeting_id = ?1")?;

        let labels = stmt
            .query_map(params![meeting_id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;

        Ok(labels)
    }
}

/// A meeting as `meeting list --json` lists it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MeetingListEntry {
    #[serde(flatten)]
    pub metadata: MeetingMetadata,
    /// Whether a summary is saved for the meeting
    pub has_summary: bool,
    /// Distinct speakers as displayed, with labels and moved lines applied;
    /// two speakers given the same name count once
    pub speaker_count: u32,
}

/// Distinct displayed speakers among lines that say something.
fn displayed_speaker_count(transcript: &Transcript) -> u32 {
    let mut names: Vec<String> = transcript
        .segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| s.speaker_display())
        .collect();
    names.sort();
    names.dedup();
    names.len() as u32
}

/// Speaker IDs in `transcript` that `speaker` names, most specific match first.
fn resolve_speaker_keys(transcript: &Transcript, speaker: &str) -> Vec<String> {
    let speaker = speaker.trim();
    let keys = transcript.speaker_keys();

    if let Some(key) = keys.iter().find(|k| k.as_str() == speaker) {
        return vec![key.clone()];
    }
    if let Ok(num) = speaker.trim_start_matches("SPEAKER_").parse::<u32>() {
        let key = format!("SPEAKER_{:02}", num);
        if keys.contains(&key) {
            return vec![key];
        }
    }
    if let Some(key) = keys.iter().find(|k| k.eq_ignore_ascii_case(speaker)) {
        return vec![key.clone()];
    }

    let mut by_name: Vec<String> = transcript
        .segments
        .iter()
        .filter(|s| {
            s.speaker_label
                .as_deref()
                .is_some_and(|l| l.eq_ignore_ascii_case(speaker))
        })
        .map(|s| s.speaker_key())
        .collect();
    by_name.sort();
    by_name.dedup();
    by_name
}

// Helper functions for status serialization
fn status_to_string(status: MeetingStatus) -> &'static str {
    match status {
        MeetingStatus::Active => "active",
        MeetingStatus::Paused => "paused",
        MeetingStatus::Completed => "completed",
        MeetingStatus::Cancelled => "cancelled",
    }
}

fn string_to_status(s: &str) -> MeetingStatus {
    match s {
        "active" => MeetingStatus::Active,
        "paused" => MeetingStatus::Paused,
        "completed" => MeetingStatus::Completed,
        "cancelled" => MeetingStatus::Cancelled,
        _ => MeetingStatus::Active,
    }
}

fn timestamp_to_datetime(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_storage() -> (MeetingStorage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig {
            storage_path: temp_dir.path().to_path_buf(),
            retain_audio: false,
            max_meetings: 0,
        };
        let storage = MeetingStorage::open(config).unwrap();
        (storage, temp_dir)
    }

    /// A stored meeting with one line per (speaker id, text).
    fn meeting_with_lines(storage: &MeetingStorage, lines: &[(&str, &str)]) -> MeetingId {
        let mut metadata = MeetingMetadata::new(Some("Stats".into()));
        metadata.storage_path = Some(storage.create_meeting(&metadata).unwrap());
        let mut t = Transcript::new();
        for (i, (speaker, text)) in lines.iter().enumerate() {
            let mut seg = TranscriptSegment::new(
                i as u32,
                i as u64 * 1000,
                i as u64 * 1000 + 900,
                text.to_string(),
                0,
            );
            seg.speaker_id = Some(speaker.to_string());
            t.add_segment(seg);
        }
        storage.save_transcript(&metadata.id, &t).unwrap();
        metadata.id
    }

    fn entry(storage: &MeetingStorage, id: &MeetingId) -> (bool, u32) {
        let e = storage
            .list_meeting_entries(None)
            .unwrap()
            .into_iter()
            .find(|e| e.metadata.id == *id)
            .unwrap();
        (e.has_summary, e.speaker_count)
    }

    #[test]
    fn list_entries_count_displayed_speakers_and_flag_summaries() {
        let (storage, _dir) = create_test_storage();
        let id = meeting_with_lines(
            &storage,
            &[
                ("You", "hi"),
                ("Remote 1", "hello"),
                ("Remote 2", "hey"),
                ("Remote 3", "   "),
            ],
        );
        // A speaker with only an empty line does not count.
        assert_eq!(entry(&storage, &id), (false, 3));

        // Two speakers given one name display, and count, as one.
        storage.set_speaker_label(&id, "Remote 1", "Dana").unwrap();
        storage.set_speaker_label(&id, "Remote 2", "Dana").unwrap();
        assert_eq!(entry(&storage, &id), (false, 2));

        // Moving the only "You" line to Dana leaves one speaker.
        storage.reassign_segment(&id, 0, "Dana", true).unwrap();
        assert_eq!(entry(&storage, &id), (false, 1));

        let summary = crate::meeting::data::MeetingSummary {
            summary: "s".into(),
            key_points: vec![],
            action_items: vec![],
            decisions: vec![],
            chapters: vec![],
            generated_at: Utc::now(),
            model: None,
        };
        storage.save_summary(&id, &summary).unwrap();
        assert_eq!(entry(&storage, &id), (true, 1));

        let json = serde_json::to_value(storage.list_meeting_entries(None).unwrap()).unwrap();
        assert_eq!(json[0]["has_summary"], true);
        assert_eq!(json[0]["speaker_count"], 1);
        assert_eq!(json[0]["id"], id.to_string());
        assert!(
            json[0].get("title").is_some(),
            "metadata stays flat: {json}"
        );
    }

    #[test]
    fn a_deleted_line_does_not_count_its_speaker() {
        let (storage, _dir) = create_test_storage();
        let id = meeting_with_lines(&storage, &[("You", "hi"), ("Remote", "hello")]);
        assert_eq!(entry(&storage, &id), (false, 2));

        // Deleting Remote's only line leaves You alone, even though the
        // tombstone stays in transcript.json.
        storage.delete_segment(&id, 1, true).unwrap();
        assert_eq!(entry(&storage, &id), (false, 1));

        storage.restore_segment(&id, 1, true).unwrap();
        assert_eq!(entry(&storage, &id), (false, 2));
    }

    #[test]
    fn list_entries_read_the_index_not_the_transcript() {
        let (storage, _dir) = create_test_storage();
        let id = meeting_with_lines(&storage, &[("You", "a"), ("Remote", "b")]);
        let path = storage
            .get_meeting_path(&id)
            .unwrap()
            .join("transcript.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(entry(&storage, &id), (false, 2));
    }

    #[test]
    fn list_entries_backfill_meetings_saved_before_the_count() {
        let (storage, _dir) = create_test_storage();
        let id = meeting_with_lines(&storage, &[("You", "a"), ("Remote", "b")]);
        storage
            .conn
            .execute("DELETE FROM meeting_stats", [])
            .unwrap();
        assert_eq!(entry(&storage, &id), (false, 2));
        let stored: u32 = storage
            .conn
            .query_row(
                "SELECT speaker_count FROM meeting_stats WHERE meeting_id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, 2);

        // A meeting with no transcript yet has none, and nothing is stored.
        let mut fresh = MeetingMetadata::new(None);
        fresh.storage_path = Some(storage.create_meeting(&fresh).unwrap());
        assert_eq!(entry(&storage, &fresh.id), (false, 0));
    }

    #[test]
    fn test_create_and_get_meeting() {
        let (storage, _temp) = create_test_storage();

        let metadata = MeetingMetadata::new(Some("Test Meeting".to_string()));
        let meeting_id = metadata.id;

        storage.create_meeting(&metadata).unwrap();

        let loaded = storage.get_meeting(&meeting_id).unwrap().unwrap();
        assert_eq!(loaded.title, Some("Test Meeting".to_string()));
    }

    #[test]
    fn test_list_meetings() {
        let (storage, _temp) = create_test_storage();

        let metadata1 = MeetingMetadata::new(Some("Meeting 1".to_string()));
        let metadata2 = MeetingMetadata::new(Some("Meeting 2".to_string()));

        storage.create_meeting(&metadata1).unwrap();
        storage.create_meeting(&metadata2).unwrap();

        let meetings = storage.list_meetings(None).unwrap();
        assert_eq!(meetings.len(), 2);
    }

    #[test]
    fn test_list_meetings_with_limit() {
        let (storage, _temp) = create_test_storage();

        for i in 0..5 {
            let metadata = MeetingMetadata::new(Some(format!("Meeting {}", i)));
            storage.create_meeting(&metadata).unwrap();
        }

        let meetings = storage.list_meetings(Some(2)).unwrap();
        assert_eq!(meetings.len(), 2);
    }

    #[test]
    fn test_update_meeting() {
        let (storage, _temp) = create_test_storage();

        let mut metadata = MeetingMetadata::new(Some("Original Title".to_string()));
        let meeting_id = metadata.id;

        storage.create_meeting(&metadata).unwrap();

        metadata.title = Some("Updated Title".to_string());
        metadata.complete();
        storage.update_meeting(&metadata).unwrap();

        let loaded = storage.get_meeting(&meeting_id).unwrap().unwrap();
        assert_eq!(loaded.title, Some("Updated Title".to_string()));
        assert_eq!(loaded.status, MeetingStatus::Completed);
    }

    #[test]
    fn test_save_and_load_transcript() {
        let (storage, _temp) = create_test_storage();

        let mut metadata = MeetingMetadata::new(Some("Test".to_string()));
        let meeting_id = metadata.id;

        let path = storage.create_meeting(&metadata).unwrap();
        metadata.storage_path = Some(path);
        storage.update_meeting(&metadata).unwrap();

        let mut transcript = Transcript::new();
        transcript.add_segment(crate::meeting::data::TranscriptSegment::new(
            0,
            0,
            1000,
            "Hello world".to_string(),
            0,
        ));

        storage.save_transcript(&meeting_id, &transcript).unwrap();

        let loaded = storage.load_transcript(&meeting_id).unwrap();
        assert_eq!(loaded.segments.len(), 1);
        assert_eq!(loaded.segments[0].text, "Hello world");
    }

    #[test]
    fn test_delete_meeting() {
        let (storage, _temp) = create_test_storage();

        let metadata = MeetingMetadata::new(Some("Test".to_string()));
        let meeting_id = metadata.id;

        storage.create_meeting(&metadata).unwrap();
        assert!(storage.get_meeting(&meeting_id).unwrap().is_some());

        storage.delete_meeting(&meeting_id).unwrap();
        assert!(storage.get_meeting(&meeting_id).unwrap().is_none());
    }

    #[test]
    fn test_resolve_latest() {
        let (storage, _temp) = create_test_storage();

        let metadata = MeetingMetadata::new(Some("Latest".to_string()));
        let expected_id = metadata.id;

        storage.create_meeting(&metadata).unwrap();

        let resolved = storage.resolve_meeting_id("latest").unwrap();
        assert_eq!(resolved, expected_id);
    }

    #[test]
    fn test_get_latest_empty() {
        let (storage, _temp) = create_test_storage();
        assert!(storage.get_latest_meeting().unwrap().is_none());
    }

    #[test]
    fn test_resolve_meeting_id_by_uuid() {
        let (storage, _temp) = create_test_storage();
        let metadata = MeetingMetadata::new(Some("Test".to_string()));
        let id = metadata.id;
        storage.create_meeting(&metadata).unwrap();

        let resolved = storage.resolve_meeting_id(&id.to_string()).unwrap();
        assert_eq!(resolved, id);
    }

    #[test]
    fn test_resolve_meeting_id_invalid() {
        let (storage, _temp) = create_test_storage();
        let result = storage.resolve_meeting_id("not-a-uuid");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_latest_no_meetings() {
        let (storage, _temp) = create_test_storage();
        let result = storage.resolve_meeting_id("latest");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_meeting_path() {
        let (storage, _temp) = create_test_storage();
        let mut metadata = MeetingMetadata::new(Some("Path Test".to_string()));
        let id = metadata.id;

        let path = storage.create_meeting(&metadata).unwrap();
        metadata.storage_path = Some(path.clone());
        storage.update_meeting(&metadata).unwrap();

        let retrieved_path = storage.get_meeting_path(&id).unwrap();
        assert_eq!(retrieved_path, path);
    }

    #[test]
    fn test_get_meeting_path_not_found() {
        let (storage, _temp) = create_test_storage();
        let id = MeetingId::new();
        let result = storage.get_meeting_path(&id);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_meeting_data() {
        let (storage, _temp) = create_test_storage();
        let mut metadata = MeetingMetadata::new(Some("Data Test".to_string()));
        let id = metadata.id;

        let path = storage.create_meeting(&metadata).unwrap();
        metadata.storage_path = Some(path);
        storage.update_meeting(&metadata).unwrap();

        let mut transcript = Transcript::new();
        transcript.add_segment(crate::meeting::data::TranscriptSegment::new(
            0,
            0,
            2000,
            "Test segment".to_string(),
            0,
        ));
        storage.save_transcript(&id, &transcript).unwrap();

        let data = storage.load_meeting_data(&id).unwrap();
        assert_eq!(data.metadata.title, Some("Data Test".to_string()));
        assert_eq!(data.transcript.segments.len(), 1);
        assert_eq!(data.transcript.segments[0].text, "Test segment");
    }

    #[test]
    fn test_load_meeting_data_no_transcript() {
        let (storage, _temp) = create_test_storage();
        let mut metadata = MeetingMetadata::new(Some("No Transcript".to_string()));
        let id = metadata.id;

        let path = storage.create_meeting(&metadata).unwrap();
        metadata.storage_path = Some(path);
        storage.update_meeting(&metadata).unwrap();

        let data = storage.load_meeting_data(&id).unwrap();
        assert!(data.transcript.segments.is_empty());
    }

    #[test]
    fn test_delete_meeting_removes_files() {
        let (storage, _temp) = create_test_storage();
        let metadata = MeetingMetadata::new(Some("Delete Test".to_string()));
        let id = metadata.id;

        let path = storage.create_meeting(&metadata).unwrap();
        assert!(path.exists());

        storage.delete_meeting(&id).unwrap();
        assert!(!path.exists());
        assert!(storage.get_meeting(&id).unwrap().is_none());
    }

    #[test]
    fn test_status_roundtrip() {
        assert_eq!(
            string_to_status(status_to_string(MeetingStatus::Active)),
            MeetingStatus::Active
        );
        assert_eq!(
            string_to_status(status_to_string(MeetingStatus::Paused)),
            MeetingStatus::Paused
        );
        assert_eq!(
            string_to_status(status_to_string(MeetingStatus::Completed)),
            MeetingStatus::Completed
        );
        assert_eq!(
            string_to_status(status_to_string(MeetingStatus::Cancelled)),
            MeetingStatus::Cancelled
        );
    }

    #[test]
    fn test_status_unknown_defaults_to_active() {
        assert_eq!(string_to_status("unknown"), MeetingStatus::Active);
        assert_eq!(string_to_status(""), MeetingStatus::Active);
    }

    #[test]
    fn test_storage_config_default() {
        let config = StorageConfig::default();
        assert!(!config.retain_audio);
        assert_eq!(config.max_meetings, 0);
    }

    #[test]
    fn test_storage_config_db_path() {
        let config = StorageConfig {
            storage_path: PathBuf::from("/tmp/test-meetings"),
            retain_audio: false,
            max_meetings: 0,
        };
        assert_eq!(
            config.db_path(),
            PathBuf::from("/tmp/test-meetings/index.db")
        );
    }

    #[test]
    fn test_list_meetings_empty() {
        let (storage, _temp) = create_test_storage();
        let meetings = storage.list_meetings(None).unwrap();
        assert!(meetings.is_empty());
    }

    #[test]
    fn test_list_meetings_limit_zero() {
        let (storage, _temp) = create_test_storage();
        for i in 0..3 {
            let metadata = MeetingMetadata::new(Some(format!("Meeting {}", i)));
            storage.create_meeting(&metadata).unwrap();
        }
        let meetings = storage.list_meetings(Some(0)).unwrap();
        assert!(meetings.is_empty());
    }

    #[test]
    fn test_get_meeting_not_found() {
        let (storage, _temp) = create_test_storage();
        let id = MeetingId::new();
        let result = storage.get_meeting(&id).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_nonexistent_meeting() {
        let (storage, _temp) = create_test_storage();
        let id = MeetingId::new();
        // Should not error - just does nothing
        let result = storage.delete_meeting(&id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_save_transcript_meeting_not_found() {
        let (storage, _temp) = create_test_storage();
        let id = MeetingId::new();
        let transcript = Transcript::new();
        let result = storage.save_transcript(&id, &transcript);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_transcript_meeting_not_found() {
        let (storage, _temp) = create_test_storage();
        let id = MeetingId::new();
        let result = storage.load_transcript(&id);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_meeting_data_not_found() {
        let (storage, _temp) = create_test_storage();
        let id = MeetingId::new();
        let result = storage.load_meeting_data(&id);
        assert!(result.is_err());
    }

    /// A stored meeting whose transcript has the given (speaker_id, source) segments.
    fn meeting_with_speakers(
        storage: &MeetingStorage,
        speakers: &[(Option<&str>, crate::meeting::data::AudioSource)],
    ) -> (MeetingId, Transcript) {
        let metadata = MeetingMetadata::new(Some("Labels".to_string()));
        let id = metadata.id;
        storage.create_meeting(&metadata).unwrap();
        let mut transcript = Transcript::new();
        for (i, (speaker_id, source)) in speakers.iter().enumerate() {
            let mut seg = crate::meeting::data::TranscriptSegment::new(
                i as u32,
                i as u64 * 1000,
                i as u64 * 1000 + 900,
                format!("line {}", i),
                0,
            );
            seg.source = *source;
            seg.speaker_id = speaker_id.map(str::to_string);
            transcript.add_segment(seg);
        }
        storage.save_transcript(&id, &transcript).unwrap();
        (id, transcript)
    }

    fn displayed(storage: &MeetingStorage, id: &MeetingId) -> Vec<String> {
        storage
            .load_transcript(id)
            .unwrap()
            .segments
            .iter()
            .map(|s| s.speaker_display())
            .collect()
    }

    #[test]
    fn simple_diarization_speakers_can_be_labelled() {
        use crate::meeting::data::AudioSource::{Loopback, Microphone};
        let (storage, _temp) = create_test_storage();
        let (id, _) = meeting_with_speakers(
            &storage,
            &[(Some("You"), Microphone), (Some("Remote"), Loopback)],
        );

        let keys = storage.set_speaker_label(&id, "Remote", "Dana").unwrap();
        assert_eq!(keys, vec!["Remote".to_string()]);
        storage.set_speaker_label(&id, "You", "Pete").unwrap();

        assert_eq!(displayed(&storage, &id), vec!["Pete", "Dana"]);
    }

    #[test]
    fn undiarized_tracks_are_labelled_by_source() {
        use crate::meeting::data::AudioSource::{Loopback, Microphone};
        let (storage, _temp) = create_test_storage();
        let (id, _) = meeting_with_speakers(&storage, &[(None, Microphone), (None, Loopback)]);

        storage.set_speaker_label(&id, "remote", "Dana").unwrap();
        assert_eq!(displayed(&storage, &id), vec!["You", "Dana"]);
    }

    #[test]
    fn labelling_a_speaker_not_in_the_meeting_is_an_error() {
        use crate::meeting::data::AudioSource::{Loopback, Microphone};
        let (storage, _temp) = create_test_storage();
        let (id, _) = meeting_with_speakers(
            &storage,
            &[(Some("You"), Microphone), (Some("Remote"), Loopback)],
        );

        let err = storage
            .set_speaker_label(&id, "SPEAKER_01", "Dana")
            .unwrap_err();
        match err {
            StorageError::UnknownSpeaker { speaker, available } => {
                assert_eq!(speaker, "SPEAKER_01");
                assert_eq!(available, vec!["Remote".to_string(), "You".to_string()]);
            }
            other => panic!("expected UnknownSpeaker, got {:?}", other),
        }
        assert!(storage.get_speaker_labels(&id).unwrap().is_empty());
        assert_eq!(displayed(&storage, &id), vec!["You", "Remote"]);
    }

    #[test]
    fn labelling_before_the_first_chunk_is_an_error() {
        let (storage, _temp) = create_test_storage();
        let metadata = MeetingMetadata::new(None);
        storage.create_meeting(&metadata).unwrap();
        assert!(matches!(
            storage.set_speaker_label(&metadata.id, "You", "Pete"),
            Err(StorageError::UnknownSpeaker { .. })
        ));
    }

    #[test]
    fn empty_label_is_rejected() {
        use crate::meeting::data::AudioSource::Microphone;
        let (storage, _temp) = create_test_storage();
        let (id, _) = meeting_with_speakers(&storage, &[(Some("You"), Microphone)]);
        assert!(matches!(
            storage.set_speaker_label(&id, "You", "  "),
            Err(StorageError::EmptyLabel)
        ));
    }

    #[test]
    fn ml_speaker_ids_accept_numeric_shorthand() {
        use crate::meeting::data::AudioSource::Microphone;
        let (storage, _temp) = create_test_storage();
        let (id, _) = meeting_with_speakers(
            &storage,
            &[
                (Some("SPEAKER_00"), Microphone),
                (Some("SPEAKER_01"), Microphone),
            ],
        );

        storage.set_speaker_label(&id, "1", "Dana").unwrap();
        storage
            .set_speaker_label(&id, "SPEAKER_00", "Alice")
            .unwrap();
        storage
            .set_speaker_label(&id, "SPEAKER_00", "Carol")
            .unwrap();

        assert_eq!(displayed(&storage, &id), vec!["Carol", "Dana"]);
    }

    /// The daemon rewrites transcript.json from its in-memory transcript after
    /// every chunk and again on stop. A label set in between must survive.
    #[test]
    fn label_set_mid_meeting_survives_later_saves() {
        use crate::meeting::data::AudioSource::{Loopback, Microphone};
        let (storage, _temp) = create_test_storage();
        let (id, mut in_memory) = meeting_with_speakers(
            &storage,
            &[(Some("You"), Microphone), (Some("Remote"), Loopback)],
        );

        storage.set_speaker_label(&id, "Remote", "Dana").unwrap();

        // Next chunk, then stop: both saves come from the unlabelled copy.
        let mut seg =
            crate::meeting::data::TranscriptSegment::new(2, 3000, 3900, "more".to_string(), 1);
        seg.source = Loopback;
        seg.speaker_id = Some("Remote".to_string());
        in_memory.add_segment(seg);
        storage.save_transcript(&id, &in_memory).unwrap();
        storage.save_transcript(&id, &in_memory).unwrap();

        assert_eq!(displayed(&storage, &id), vec!["You", "Dana", "Dana"]);

        // And the file itself carries the name for readers that skip the index.
        let path = storage
            .get_meeting_path(&id)
            .unwrap()
            .join("transcript.json");
        let on_disk: Transcript =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(on_disk.segments[2].speaker_label.as_deref(), Some("Dana"));
    }

    #[test]
    fn speakers_given_the_same_name_render_as_one() {
        use crate::meeting::data::AudioSource::Microphone;
        let (storage, _temp) = create_test_storage();
        let (id, _) = meeting_with_speakers(
            &storage,
            &[
                (Some("SPEAKER_00"), Microphone),
                (Some("SPEAKER_02"), Microphone),
                (Some("SPEAKER_01"), Microphone),
            ],
        );

        storage
            .set_speaker_label(&id, "SPEAKER_00", "Dana")
            .unwrap();
        storage
            .set_speaker_label(&id, "SPEAKER_02", "Dana")
            .unwrap();

        let meeting = storage.load_meeting_data(&id).unwrap();
        assert_eq!(meeting.transcript.speakers(), vec!["Dana", "SPEAKER_01"]);

        let md = crate::meeting::export::export_meeting(
            &meeting,
            crate::meeting::export::ExportFormat::Markdown,
            &crate::meeting::export::ExportOptions {
                include_speakers: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(md.matches("### Dana").count(), 1, "{}", md);

        // Renaming by the shared name renames both speakers.
        let keys = storage
            .set_speaker_label(&id, "Dana", "Dana Smith")
            .unwrap();
        assert_eq!(
            keys,
            vec!["SPEAKER_00".to_string(), "SPEAKER_02".to_string()]
        );
        assert_eq!(
            displayed(&storage, &id),
            vec!["Dana Smith", "Dana Smith", "SPEAKER_01"]
        );
    }

    #[test]
    fn labels_from_the_numeric_table_carry_over() {
        use crate::meeting::data::AudioSource::Microphone;
        let temp = TempDir::new().unwrap();
        let config = StorageConfig {
            storage_path: temp.path().to_path_buf(),
            retain_audio: false,
            max_meetings: 0,
        };
        let id = {
            let storage = MeetingStorage::open(config.clone()).unwrap();
            let (id, _) = meeting_with_speakers(&storage, &[(Some("SPEAKER_03"), Microphone)]);
            storage
                .conn
                .execute(
                    "INSERT INTO speaker_labels (meeting_id, speaker_num, label) VALUES (?1, 3, 'Old')",
                    params![id.to_string()],
                )
                .unwrap();
            id
        };

        let storage = MeetingStorage::open(config).unwrap();
        assert_eq!(displayed(&storage, &id), vec!["Old"]);
    }

    /// Pete's case: a line clustered as "Remote 1" ("Primagen") was someone else.
    fn hybrid_meeting(storage: &MeetingStorage) -> (MeetingId, Transcript) {
        use crate::meeting::data::AudioSource::{Loopback, Microphone};
        let (id, transcript) = meeting_with_speakers(
            storage,
            &[
                (Some("You"), Microphone),
                (Some("Remote 1"), Loopback),
                (Some("Remote 1"), Loopback),
                (Some("Remote 2"), Loopback),
            ],
        );
        storage
            .set_speaker_label(&id, "Remote 1", "Primagen")
            .unwrap();
        (id, transcript)
    }

    fn speakers(storage: &MeetingStorage, id: &MeetingId) -> Vec<String> {
        displayed(storage, id)
    }

    #[test]
    fn reassign_moves_one_line_to_an_existing_speaker() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);

        let seg = storage.reassign_segment(&id, 2, "remote 2", true).unwrap();
        assert_eq!(seg.id, 2);
        assert_eq!(seg.speaker_key(), "Remote 2");
        assert_eq!(seg.original_speaker.as_deref(), Some("Remote 1"));
        assert_eq!(
            speakers(&storage, &id),
            vec!["You", "Primagen", "Remote 2", "Remote 2"]
        );
    }

    #[test]
    fn reassign_matches_a_speaker_by_name() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);
        storage.set_speaker_label(&id, "Remote 2", "Dana").unwrap();

        storage.reassign_segment(&id, 1, "dana", true).unwrap();
        assert_eq!(
            speakers(&storage, &id),
            vec!["You", "Dana", "Primagen", "Dana"]
        );
    }

    #[test]
    fn reassign_to_an_unknown_name_creates_a_speaker() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);

        let seg = storage.reassign_segment(&id, 2, "Casey", true).unwrap();
        assert_eq!(seg.speaker_key(), "Person 1");
        assert_eq!(seg.speaker_display(), "Casey");
        // A second new name gets its own key; the first name is reused.
        let seg = storage.reassign_segment(&id, 3, "Robin", true).unwrap();
        assert_eq!(seg.speaker_key(), "Person 2");
        let seg = storage.reassign_segment(&id, 0, "casey", true).unwrap();
        assert_eq!(seg.speaker_key(), "Person 1");
        assert_eq!(
            speakers(&storage, &id),
            vec!["Casey", "Primagen", "Casey", "Robin"]
        );
    }

    #[test]
    fn a_new_speaker_key_skips_keys_in_use() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = meeting_with_speakers(
            &storage,
            &[(
                Some("Person 1"),
                crate::meeting::data::AudioSource::Loopback,
            )],
        );
        let seg = storage.reassign_segment(&id, 0, "Casey", true).unwrap();
        assert_eq!(seg.speaker_key(), "Person 2");
    }

    #[test]
    fn reassign_reset_restores_the_diarizers_speaker() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);
        storage.reassign_segment(&id, 2, "Casey", true).unwrap();

        let seg = storage.reset_segment_speaker(&id, 2, true).unwrap();
        assert_eq!(seg.speaker_key(), "Remote 1");
        assert_eq!(seg.original_speaker, None);
        assert_eq!(
            speakers(&storage, &id),
            vec!["You", "Primagen", "Primagen", "Remote 2"]
        );
        assert!(storage.get_speaker_overrides(&id).unwrap().is_empty());
    }

    #[test]
    fn reassign_an_unknown_segment_names_the_valid_range() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);
        let err = storage.reassign_segment(&id, 99, "You", true).unwrap_err();
        assert!(matches!(
            err,
            StorageError::UnknownSegment {
                segment: 99,
                first: Some(0),
                last: Some(3)
            }
        ));
        assert_eq!(
            err.to_string(),
            "Segment 99 is not in this meeting. Valid segment ids: 0 to 3."
        );
        assert!(storage.reset_segment_speaker(&id, 99, true).is_err());
    }

    /// The daemon rewrites transcript.json from its own copy after every
    /// chunk. The move has to survive that and show up on load and in the
    /// file itself.
    #[test]
    fn reassign_survives_the_daemons_next_save() {
        let (storage, _temp) = create_test_storage();
        let (id, in_memory) = hybrid_meeting(&storage);
        // Live meeting: the CLI stores the move without touching the file.
        storage.reassign_segment(&id, 2, "Casey", false).unwrap();

        storage.save_transcript(&id, &in_memory).unwrap();
        assert_eq!(speakers(&storage, &id)[2], "Casey");

        let path = storage
            .get_meeting_path(&id)
            .unwrap()
            .join("transcript.json");
        let on_disk: Transcript =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(on_disk.segments[2].speaker_key(), "Person 1");
        assert_eq!(
            on_disk.segments[2].original_speaker.as_deref(),
            Some("Remote 1")
        );
        assert_eq!(on_disk.segments[2].speaker_label.as_deref(), Some("Casey"));
    }

    #[test]
    fn an_override_follows_a_speaker_rename() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = meeting_with_speakers(
            &storage,
            &[
                (Some("Remote"), crate::meeting::data::AudioSource::Loopback),
                (Some("You"), crate::meeting::data::AudioSource::Microphone),
            ],
        );
        storage.reassign_segment(&id, 1, "Remote", true).unwrap();

        storage
            .rename_speaker_key(&id, "Remote", "Remote 1")
            .unwrap();
        assert_eq!(
            storage
                .get_speaker_overrides(&id)
                .unwrap()
                .get(&1)
                .map(String::as_str),
            Some("Remote 1")
        );
    }

    #[test]
    fn deleting_a_meeting_drops_its_overrides() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);
        storage.reassign_segment(&id, 2, "Casey", true).unwrap();
        storage.delete_meeting(&id).unwrap();
        assert!(storage.get_speaker_overrides(&id).unwrap().is_empty());
    }

    #[test]
    fn meeting_ids_resolve_from_a_unique_prefix() {
        let (storage, _temp) = create_test_storage();
        let metadata = MeetingMetadata::new(None);
        storage.create_meeting(&metadata).unwrap();
        let full = metadata.id.to_string();

        assert_eq!(storage.resolve_meeting_id(&full[..8]).unwrap(), metadata.id);
        assert!(storage.resolve_meeting_id("abc").is_err());
        assert!(storage.resolve_meeting_id("zzzzzzzz").is_err());
    }

    #[test]
    fn annotate_attaches_a_note_and_highlight_to_one_line() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);

        let seg = storage
            .annotate_segment(&id, 2, " check this number ", true, true)
            .unwrap();
        assert_eq!(
            seg.annotation,
            Some(SegmentAnnotation {
                text: "check this number".to_string(),
                highlight: true
            })
        );
        // Replacing it, then a bare highlight.
        storage
            .annotate_segment(&id, 2, "other", false, true)
            .unwrap();
        let seg = storage.annotate_segment(&id, 3, "", true, true).unwrap();
        assert_eq!(seg.annotation.unwrap().text, "");

        let loaded = storage.load_transcript(&id).unwrap();
        let notes: Vec<_> = loaded
            .segments
            .iter()
            .map(|s| {
                s.annotation
                    .as_ref()
                    .map(|a| (a.text.as_str(), a.highlight))
            })
            .collect();
        assert_eq!(
            notes,
            vec![None, None, Some(("other", false)), Some(("", true))]
        );

        let seg = storage.remove_annotation(&id, 2, true).unwrap();
        assert_eq!(seg.annotation, None);
        assert_eq!(storage.get_annotations(&id).unwrap().len(), 1);
    }

    #[test]
    fn annotate_rejects_nothing_and_unknown_segments() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);
        assert!(matches!(
            storage.annotate_segment(&id, 1, "  ", false, true),
            Err(StorageError::EmptyAnnotation)
        ));
        assert!(matches!(
            storage.annotate_segment(&id, 99, "x", false, true),
            Err(StorageError::UnknownSegment { segment: 99, .. })
        ));
    }

    #[test]
    fn an_annotation_survives_the_daemons_next_save() {
        let (storage, _temp) = create_test_storage();
        let (id, in_memory) = hybrid_meeting(&storage);
        storage
            .annotate_segment(&id, 1, "important", true, false)
            .unwrap();
        storage.save_transcript(&id, &in_memory).unwrap();
        assert_eq!(
            storage.load_transcript(&id).unwrap().segments[1]
                .annotation
                .as_ref()
                .map(|a| a.text.as_str()),
            Some("important")
        );
    }

    #[test]
    fn notes_load_set_and_append() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);

        let empty = storage.load_notes(&id).unwrap();
        assert_eq!(empty.notes, "");
        assert_eq!(empty.updated_at, None);

        let notes = storage.append_notes(&id, "First thought").unwrap();
        assert_eq!(notes.notes, "First thought\n");
        assert!(notes.updated_at.is_some());
        let notes = storage.append_notes(&id, "Second thought").unwrap();
        assert_eq!(notes.notes, "First thought\nSecond thought\n");

        storage.set_notes(&id, "# Agenda\n- budget").unwrap();
        let notes = storage.append_notes(&id, "- travel").unwrap();
        assert_eq!(notes.notes, "# Agenda\n- budget\n- travel\n");
        assert_eq!(notes.meeting_id, id.to_string());

        let config = crate::meeting::MeetingConfig {
            storage: storage.config.clone(),
            ..Default::default()
        };
        assert_eq!(crate::meeting::notes::load(&config, &id).unwrap(), notes);
    }

    /// The daemon saves the transcript after every chunk; notes written in
    /// between must survive, and land at the top of transcript.md.
    #[test]
    fn notes_survive_transcript_saves_and_reach_transcript_md() {
        let (storage, _temp) = create_test_storage();
        let (id, in_memory) = hybrid_meeting(&storage);
        storage.set_notes(&id, "Ask about the budget").unwrap();
        storage
            .annotate_segment(&id, 2, "check this", false, false)
            .unwrap();
        storage.save_transcript(&id, &in_memory).unwrap();

        assert_eq!(
            storage.load_notes(&id).unwrap().notes,
            "Ask about the budget"
        );
        let data = storage.load_meeting_data(&id).unwrap();
        assert_eq!(data.notes.as_deref(), Some("Ask about the budget"));

        let md = std::fs::read_to_string(
            storage
                .get_meeting_path(&id)
                .unwrap()
                .join(TRANSCRIPT_MARKDOWN_FILE),
        )
        .unwrap();
        let notes_at = md.find("## Notes\n\nAsk about the budget").expect(&md);
        assert!(notes_at < md.find("## Transcript").unwrap(), "{md}");
        assert!(md.contains("line 2\n\n> Note: check this\n"), "{md}");
    }

    #[test]
    fn deleting_a_meeting_drops_its_annotations() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);
        storage.annotate_segment(&id, 2, "x", false, true).unwrap();
        storage.delete_meeting(&id).unwrap();
        assert!(storage.get_annotations(&id).unwrap().is_empty());
    }

    #[test]
    fn a_meetings_link_round_trips() {
        let (storage, _temp) = create_test_storage();
        let mut metadata = MeetingMetadata::new(None);
        metadata.url = Some("https://meet.example.com/x".to_string());
        storage.create_meeting(&metadata).unwrap();
        let loaded = storage.get_meeting(&metadata.id).unwrap().unwrap();
        assert_eq!(loaded.url.as_deref(), Some("https://meet.example.com/x"));
        assert_eq!(
            storage.list_meetings(None).unwrap()[0].url.as_deref(),
            Some("https://meet.example.com/x")
        );
    }

    /// An index from before meeting links gains the column on open.
    #[test]
    fn an_index_without_the_url_column_is_upgraded() {
        let temp = TempDir::new().unwrap();
        {
            let conn = Connection::open(temp.path().join("index.db")).unwrap();
            conn.execute_batch(
                "CREATE TABLE meetings (id TEXT PRIMARY KEY, title TEXT, started_at INTEGER NOT NULL, \
                 ended_at INTEGER, duration_secs INTEGER, status TEXT NOT NULL DEFAULT 'active', \
                 chunk_count INTEGER NOT NULL DEFAULT 0, storage_path TEXT, \
                 audio_retained INTEGER NOT NULL DEFAULT 0, model TEXT, synced_at INTEGER, \
                 created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')));\
                 INSERT INTO meetings (id, started_at) VALUES ('6f1c2a5e-0000-4000-8000-000000000001', 0);",
            )
            .unwrap();
        }
        let storage = MeetingStorage::open(StorageConfig {
            storage_path: temp.path().to_path_buf(),
            retain_audio: false,
            max_meetings: 0,
        })
        .unwrap();
        let old = storage.list_meetings(None).unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].url, None);
        // Opening again does not try to add it twice.
        drop(storage);
        MeetingStorage::open(StorageConfig {
            storage_path: temp.path().to_path_buf(),
            retain_audio: false,
            max_meetings: 0,
        })
        .unwrap();
    }

    fn raw_file(storage: &MeetingStorage, id: &MeetingId) -> Transcript {
        let path = storage
            .get_meeting_path(id)
            .unwrap()
            .join("transcript.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn edit_replaces_a_lines_text_and_reset_restores_it() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);

        let seg = storage
            .edit_segment_text(&id, 2, " We ship Friday. ", true)
            .unwrap();
        assert_eq!(seg.text, "We ship Friday.");
        assert_eq!(seg.original_text.as_deref(), Some("line 2"));
        assert_eq!(
            storage.load_transcript(&id).unwrap().segments[2].text,
            "We ship Friday."
        );

        let seg = storage.reset_segment_text(&id, 2, true).unwrap();
        assert_eq!(seg.text, "line 2");
        assert_eq!(seg.original_text, None);

        assert!(matches!(
            storage.edit_segment_text(&id, 2, "  ", true),
            Err(StorageError::EmptyText)
        ));
        assert!(matches!(
            storage.edit_segment_text(&id, 9, "x", true),
            Err(StorageError::UnknownSegment { segment: 9, .. })
        ));
    }

    #[test]
    fn an_edit_survives_the_daemons_next_save() {
        let (storage, _temp) = create_test_storage();
        let (id, in_memory) = hybrid_meeting(&storage);
        storage
            .edit_segment_text(&id, 1, "Corrected", false)
            .unwrap();
        storage.save_transcript(&id, &in_memory).unwrap();
        let line = &storage.load_transcript(&id).unwrap().segments[1];
        assert_eq!(line.text, "Corrected");
        assert_eq!(line.original_text.as_deref(), Some("line 1"));
    }

    /// A deleted line is gone from everything that reads the meeting, but
    /// kept in the file so it can come back, even across later saves.
    #[test]
    fn delete_hides_a_line_everywhere_but_keeps_it_for_restore() {
        let (storage, _temp) = create_test_storage();
        let (id, in_memory) = hybrid_meeting(&storage);

        let seg = storage.delete_segment(&id, 2, true).unwrap();
        assert!(seg.deleted);
        let shown: Vec<u32> = storage
            .load_transcript(&id)
            .unwrap()
            .segments
            .iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(shown, vec![0, 1, 3]);
        assert_eq!(
            storage
                .load_meeting_data(&id)
                .unwrap()
                .transcript
                .segments
                .len(),
            3
        );
        let md = std::fs::read_to_string(
            storage
                .get_meeting_path(&id)
                .unwrap()
                .join(TRANSCRIPT_MARKDOWN_FILE),
        )
        .unwrap();
        assert!(!md.contains("line 2"), "{md}");

        // The daemon's next save, and other edits, keep the tombstone.
        storage.save_transcript(&id, &in_memory).unwrap();
        storage.set_speaker_label(&id, "You", "Pete").unwrap();
        storage.reassign_segment(&id, 3, "You", true).unwrap();
        let file = raw_file(&storage, &id);
        assert_eq!(file.segments.len(), 4);
        assert!(file.segments[2].deleted);
        assert_eq!(storage.load_transcript(&id).unwrap().segments.len(), 3);

        let seg = storage.restore_segment(&id, 2, true).unwrap();
        assert!(!seg.deleted);
        assert_eq!(seg.text, "line 2");
        assert_eq!(storage.load_transcript(&id).unwrap().segments.len(), 4);
        assert!(!raw_file(&storage, &id).segments[2].deleted);
    }

    #[test]
    fn deleting_a_meeting_drops_its_edits_and_deletions() {
        let (storage, _temp) = create_test_storage();
        let (id, _) = hybrid_meeting(&storage);
        storage.edit_segment_text(&id, 1, "x", true).unwrap();
        storage.delete_segment(&id, 2, true).unwrap();
        storage.delete_meeting(&id).unwrap();
        assert!(storage.get_text_edits(&id).unwrap().is_empty());
        assert!(storage.get_deletions(&id).unwrap().is_empty());
    }

    #[test]
    fn test_speaker_labels_nonexistent_meeting() {
        let (storage, _temp) = create_test_storage();
        let id = MeetingId::new();
        let result = storage.set_speaker_label(&id, "You", "Alice");
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[test]
    fn test_get_speaker_labels_empty() {
        let (storage, _temp) = create_test_storage();
        let metadata = MeetingMetadata::new(Some("No Labels".to_string()));
        let id = metadata.id;
        storage.create_meeting(&metadata).unwrap();

        let labels = storage.get_speaker_labels(&id).unwrap();
        assert!(labels.is_empty());
    }

    #[test]
    fn test_create_meeting_creates_directory() {
        let (storage, _temp) = create_test_storage();
        let metadata = MeetingMetadata::new(Some("Dir Test".to_string()));
        let path = storage.create_meeting(&metadata).unwrap();
        assert!(path.exists());
        assert!(path.is_dir());
        // Should also write metadata.json
        assert!(path.join("metadata.json").exists());
    }

    #[test]
    fn test_list_meetings_ordered_by_start_time() {
        let (storage, _temp) = create_test_storage();

        // Create meetings with different started_at timestamps
        let mut metadata1 = MeetingMetadata::new(Some("First".to_string()));
        metadata1.started_at = chrono::Utc.timestamp_opt(1000000, 0).single().unwrap();
        storage.create_meeting(&metadata1).unwrap();

        let mut metadata2 = MeetingMetadata::new(Some("Second".to_string()));
        metadata2.started_at = chrono::Utc.timestamp_opt(2000000, 0).single().unwrap();
        storage.create_meeting(&metadata2).unwrap();

        let meetings = storage.list_meetings(None).unwrap();
        assert_eq!(meetings.len(), 2);
        // Ordered by started_at DESC, so Second should be first
        assert_eq!(meetings[0].title, Some("Second".to_string()));
        assert_eq!(meetings[1].title, Some("First".to_string()));
    }

    #[test]
    fn test_timestamp_to_datetime_invalid() {
        // A very old timestamp should still produce a DateTime
        let dt = timestamp_to_datetime(0);
        assert_eq!(dt.timestamp(), 0);
    }
}
