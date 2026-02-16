//! Storage layer for meeting transcription
//!
//! Provides SQLite-based index for meeting metadata and filesystem
//! storage for transcripts and audio files.

use crate::meeting::data::{MeetingData, MeetingId, MeetingMetadata, MeetingStatus, Transcript};
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
        directories::ProjectDirs::from("", "", "voxtype")
            .map(|dirs| dirs.data_dir().join("meetings"))
            .unwrap_or_else(|| PathBuf::from("~/.local/share/voxtype/meetings"))
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
            "#,
        )?;
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
            INSERT INTO meetings (id, title, started_at, status, storage_path, audio_retained, model)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                metadata.id.to_string(),
                metadata.title,
                metadata.started_at.timestamp(),
                status_to_string(metadata.status),
                meeting_dir.to_string_lossy().to_string(),
                metadata.audio_retained as i32,
                metadata.model,
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
                synced_at = ?9
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
            ],
        )?;

        // Update metadata file if storage path exists
        if let Some(ref path) = metadata.storage_path {
            let metadata_path = path.join("metadata.json");
            let json = serde_json::to_string_pretty(metadata)?;
            std::fs::write(metadata_path, json)?;
        }

        Ok(())
    }

    /// Get meeting by ID
    pub fn get_meeting(&self, id: &MeetingId) -> Result<Option<MeetingMetadata>, StorageError> {
        let result = self
            .conn
            .query_row(
                r#"
                SELECT id, title, started_at, ended_at, duration_secs, status,
                       chunk_count, storage_path, audio_retained, model, synced_at
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
                        storage_path: row
                            .get::<_, Option<String>>(7)?
                            .map(PathBuf::from),
                        audio_retained: row.get::<_, i32>(8)? != 0,
                        model: row.get(9)?,
                        summary: None,
                        synced_at: row.get::<_, Option<i64>>(10)?.map(timestamp_to_datetime),
                    })
                },
            )
            .optional()?;

        Ok(result)
    }

    /// List meetings with optional limit
    pub fn list_meetings(&self, limit: Option<u32>) -> Result<Vec<MeetingMetadata>, StorageError> {
        let sql = if let Some(limit) = limit {
            format!(
                r#"
                SELECT id, title, started_at, ended_at, duration_secs, status,
                       chunk_count, storage_path, audio_retained, model, synced_at
                FROM meetings
                ORDER BY started_at DESC
                LIMIT {}
                "#,
                limit
            )
        } else {
            r#"
                SELECT id, title, started_at, ended_at, duration_secs, status,
                       chunk_count, storage_path, audio_retained, model, synced_at
                FROM meetings
                ORDER BY started_at DESC
                "#
            .to_string()
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let meetings = stmt
            .query_map([], |row| {
                Ok(MeetingMetadata {
                    id: MeetingId::parse(&row.get::<_, String>(0)?).unwrap_or_default(),
                    title: row.get(1)?,
                    started_at: timestamp_to_datetime(row.get(2)?),
                    ended_at: row.get::<_, Option<i64>>(3)?.map(timestamp_to_datetime),
                    duration_secs: row.get::<_, Option<i64>>(4)?.map(|d| d as u64),
                    status: string_to_status(&row.get::<_, String>(5)?),
                    chunk_count: row.get::<_, i32>(6)? as u32,
                    storage_path: row
                        .get::<_, Option<String>>(7)?
                        .map(PathBuf::from),
                    audio_retained: row.get::<_, i32>(8)? != 0,
                    model: row.get(9)?,
                    summary: None,
                    synced_at: row.get::<_, Option<i64>>(10)?.map(timestamp_to_datetime),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(meetings)
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
            .ok_or(StorageError::PathNotConfigured)?;

        let transcript_path = storage_path.join("transcript.json");
        let json = serde_json::to_string_pretty(transcript)?;
        std::fs::write(transcript_path, json)?;

        Ok(())
    }

    /// Load transcript from filesystem
    pub fn load_transcript(&self, meeting_id: &MeetingId) -> Result<Transcript, StorageError> {
        let metadata = self
            .get_meeting(meeting_id)?
            .ok_or_else(|| StorageError::NotFound(meeting_id.to_string()))?;

        let storage_path = metadata
            .storage_path
            .ok_or(StorageError::PathNotConfigured)?;

        let transcript_path = storage_path.join("transcript.json");
        let json = std::fs::read_to_string(transcript_path)?;
        let transcript: Transcript = serde_json::from_str(&json)?;

        Ok(transcript)
    }

    /// Load complete meeting data (metadata + transcript)
    pub fn load_meeting_data(&self, meeting_id: &MeetingId) -> Result<MeetingData, StorageError> {
        let metadata = self
            .get_meeting(meeting_id)?
            .ok_or_else(|| StorageError::NotFound(meeting_id.to_string()))?;

        let transcript = self.load_transcript(meeting_id).unwrap_or_default();

        Ok(MeetingData {
            metadata,
            transcript,
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
        } else {
            MeetingId::parse(id_str)
                .map_err(|_| StorageError::NotFound(format!("Invalid meeting ID: {}", id_str)))
        }
    }
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
}
