//! Data structures for meeting transcription
//!
//! Defines the core data types for meetings, transcripts, and segments.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Unique identifier for a meeting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MeetingId(pub Uuid);

impl MeetingId {
    /// Generate a new unique meeting ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Parse from a string
    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl Default for MeetingId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MeetingId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for MeetingId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Audio source for speaker attribution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioSource {
    /// User's microphone (local speaker)
    Microphone,
    /// System audio loopback (remote participants)
    Loopback,
    /// Unknown source
    Unknown,
}

impl Default for AudioSource {
    fn default() -> Self {
        AudioSource::Unknown
    }
}

impl std::fmt::Display for AudioSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioSource::Microphone => write!(f, "You"),
            AudioSource::Loopback => write!(f, "Remote"),
            AudioSource::Unknown => write!(f, "Unknown"),
        }
    }
}

/// A single transcript segment with timing and speaker info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    /// Unique ID for this segment
    pub id: u32,
    /// Start time in milliseconds from meeting start
    pub start_ms: u64,
    /// End time in milliseconds from meeting start
    pub end_ms: u64,
    /// Transcribed text content
    pub text: String,
    /// Audio source (mic or loopback)
    pub source: AudioSource,
    /// Speaker ID (for diarization, Phase 3)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_id: Option<String>,
    /// Human-assigned speaker label (Phase 3)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_label: Option<String>,
    /// Confidence score (0.0 - 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Chunk number this segment belongs to
    pub chunk_id: u32,
}

impl TranscriptSegment {
    /// Create a new transcript segment
    pub fn new(id: u32, start_ms: u64, end_ms: u64, text: String, chunk_id: u32) -> Self {
        Self {
            id,
            start_ms,
            end_ms,
            text,
            source: AudioSource::Unknown,
            speaker_id: None,
            speaker_label: None,
            confidence: None,
            chunk_id,
        }
    }

    /// Duration of this segment in milliseconds
    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }

    /// Get the display name for the speaker
    pub fn speaker_display(&self) -> String {
        if let Some(ref label) = self.speaker_label {
            label.clone()
        } else if let Some(ref id) = self.speaker_id {
            id.clone()
        } else {
            self.source.to_string()
        }
    }

    /// Format timestamp as HH:MM:SS
    pub fn format_timestamp(&self) -> String {
        let secs = self.start_ms / 1000;
        let hours = secs / 3600;
        let minutes = (secs % 3600) / 60;
        let seconds = secs % 60;
        if hours > 0 {
            format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
        } else {
            format!("{:02}:{:02}", minutes, seconds)
        }
    }
}

/// Complete transcript for a meeting
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Transcript {
    /// Ordered list of transcript segments
    pub segments: Vec<TranscriptSegment>,
    /// Total number of chunks processed
    pub total_chunks: u32,
}

impl Transcript {
    /// Create a new empty transcript
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a segment to the transcript
    pub fn add_segment(&mut self, segment: TranscriptSegment) {
        self.segments.push(segment);
    }

    /// Get the full text without speaker labels
    pub fn plain_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Get the full text with speaker labels
    pub fn text_with_speakers(&self) -> String {
        let mut result = String::new();
        let mut last_speaker = String::new();

        for segment in &self.segments {
            let speaker = segment.speaker_display();
            if speaker != last_speaker {
                if !result.is_empty() {
                    result.push_str("\n\n");
                }
                result.push_str(&format!("**{}**: ", speaker));
                last_speaker = speaker;
            } else {
                result.push(' ');
            }
            result.push_str(&segment.text);
        }
        result
    }

    /// Total duration in milliseconds
    pub fn duration_ms(&self) -> u64 {
        self.segments
            .iter()
            .map(|s| s.end_ms)
            .max()
            .unwrap_or(0)
    }

    /// Word count
    pub fn word_count(&self) -> usize {
        self.segments
            .iter()
            .map(|s| s.text.split_whitespace().count())
            .sum()
    }

    /// Get segments for a specific speaker
    pub fn segments_by_speaker(&self, speaker: &str) -> Vec<&TranscriptSegment> {
        self.segments
            .iter()
            .filter(|s| s.speaker_display() == speaker)
            .collect()
    }

    /// Get unique speakers
    pub fn speakers(&self) -> Vec<String> {
        let mut speakers: Vec<String> = self
            .segments
            .iter()
            .map(|s| s.speaker_display())
            .collect();
        speakers.sort();
        speakers.dedup();
        speakers
    }
}

/// Meeting status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeetingStatus {
    /// Meeting is in progress
    Active,
    /// Meeting is paused
    Paused,
    /// Meeting has ended
    Completed,
    /// Meeting was cancelled/abandoned
    Cancelled,
}

impl Default for MeetingStatus {
    fn default() -> Self {
        MeetingStatus::Active
    }
}

/// Metadata for a meeting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingMetadata {
    /// Unique meeting ID
    pub id: MeetingId,
    /// User-provided title
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// When the meeting started
    pub started_at: DateTime<Utc>,
    /// When the meeting ended
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    /// Duration in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u64>,
    /// Meeting status
    pub status: MeetingStatus,
    /// Number of audio chunks
    pub chunk_count: u32,
    /// Storage path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_path: Option<PathBuf>,
    /// Whether audio was retained
    pub audio_retained: bool,
    /// Whisper model used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Summary (Phase 5)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<MeetingSummary>,
    /// Remote sync status (Phase 4)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<DateTime<Utc>>,
}

impl MeetingMetadata {
    /// Create new meeting metadata
    pub fn new(title: Option<String>) -> Self {
        Self {
            id: MeetingId::new(),
            title,
            started_at: Utc::now(),
            ended_at: None,
            duration_secs: None,
            status: MeetingStatus::Active,
            chunk_count: 0,
            storage_path: None,
            audio_retained: false,
            model: None,
            summary: None,
            synced_at: None,
        }
    }

    /// Mark the meeting as completed
    pub fn complete(&mut self) {
        self.ended_at = Some(Utc::now());
        self.status = MeetingStatus::Completed;
        if let Some(ended) = self.ended_at {
            self.duration_secs = Some((ended - self.started_at).num_seconds() as u64);
        }
    }

    /// Mark the meeting as cancelled
    pub fn cancel(&mut self) {
        self.ended_at = Some(Utc::now());
        self.status = MeetingStatus::Cancelled;
    }

    /// Get a display title (or fallback to date)
    pub fn display_title(&self) -> String {
        self.title.clone().unwrap_or_else(|| {
            self.started_at.format("Meeting %Y-%m-%d %H:%M").to_string()
        })
    }

    /// Generate the default storage directory name
    pub fn storage_dir_name(&self) -> String {
        let date = self.started_at.format("%Y-%m-%d").to_string();
        if let Some(ref title) = self.title {
            // Sanitize title for filesystem
            let safe_title: String = title
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else if c.is_whitespace() {
                        '-'
                    } else {
                        '_'
                    }
                })
                .collect();
            format!("{}-{}", date, safe_title.to_lowercase())
        } else {
            format!("{}-{}", date, self.id.0.to_string().split('-').next().unwrap_or("meeting"))
        }
    }
}

/// AI-generated meeting summary (Phase 5)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingSummary {
    /// Brief summary of the meeting
    pub summary: String,
    /// Key discussion points
    #[serde(default)]
    pub key_points: Vec<String>,
    /// Action items extracted
    #[serde(default)]
    pub action_items: Vec<ActionItem>,
    /// Decisions made
    #[serde(default)]
    pub decisions: Vec<String>,
    /// When the summary was generated
    pub generated_at: DateTime<Utc>,
    /// Model used to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// An action item from the meeting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionItem {
    /// Description of the action
    pub description: String,
    /// Assigned to (speaker name)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Due date (if mentioned)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    /// Completed status
    #[serde(default)]
    pub completed: bool,
}

/// Complete meeting data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingData {
    /// Meeting metadata
    pub metadata: MeetingMetadata,
    /// Transcript
    pub transcript: Transcript,
}

impl MeetingData {
    /// Create a new meeting
    pub fn new(title: Option<String>) -> Self {
        Self {
            metadata: MeetingMetadata::new(title),
            transcript: Transcript::new(),
        }
    }

    /// Add a transcript segment
    pub fn add_segment(&mut self, segment: TranscriptSegment) {
        self.transcript.add_segment(segment);
    }

    /// Complete the meeting
    pub fn complete(&mut self) {
        self.metadata.complete();
        self.metadata.chunk_count = self.transcript.total_chunks;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meeting_id_new() {
        let id1 = MeetingId::new();
        let id2 = MeetingId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_meeting_id_parse() {
        let id = MeetingId::new();
        let parsed = MeetingId::parse(&id.to_string()).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_transcript_segment() {
        let segment = TranscriptSegment::new(1, 0, 5000, "Hello world".to_string(), 0);
        assert_eq!(segment.duration_ms(), 5000);
        assert_eq!(segment.format_timestamp(), "00:00");
    }

    #[test]
    fn test_transcript_segment_timestamp_format() {
        let segment = TranscriptSegment::new(1, 3661000, 3665000, "Test".to_string(), 0);
        assert_eq!(segment.format_timestamp(), "01:01:01");
    }

    #[test]
    fn test_transcript_plain_text() {
        let mut transcript = Transcript::new();
        transcript.add_segment(TranscriptSegment::new(1, 0, 1000, "Hello".to_string(), 0));
        transcript.add_segment(TranscriptSegment::new(2, 1000, 2000, "world".to_string(), 0));
        assert_eq!(transcript.plain_text(), "Hello world");
    }

    #[test]
    fn test_transcript_word_count() {
        let mut transcript = Transcript::new();
        transcript.add_segment(TranscriptSegment::new(
            1,
            0,
            1000,
            "Hello world foo bar".to_string(),
            0,
        ));
        assert_eq!(transcript.word_count(), 4);
    }

    #[test]
    fn test_meeting_metadata_display_title() {
        let metadata = MeetingMetadata::new(Some("Team Standup".to_string()));
        assert_eq!(metadata.display_title(), "Team Standup");

        let metadata = MeetingMetadata::new(None);
        assert!(metadata.display_title().starts_with("Meeting"));
    }

    #[test]
    fn test_meeting_metadata_storage_dir_name() {
        let mut metadata = MeetingMetadata::new(Some("Team Standup!".to_string()));
        metadata.started_at = DateTime::parse_from_rfc3339("2024-01-15T10:30:00Z")
            .unwrap()
            .into();
        let dir_name = metadata.storage_dir_name();
        assert!(dir_name.starts_with("2024-01-15-team-standup_"));
    }

    #[test]
    fn test_meeting_complete() {
        let mut meeting = MeetingData::new(Some("Test".to_string()));
        assert_eq!(meeting.metadata.status, MeetingStatus::Active);

        meeting.complete();
        assert_eq!(meeting.metadata.status, MeetingStatus::Completed);
        assert!(meeting.metadata.ended_at.is_some());
    }
}
