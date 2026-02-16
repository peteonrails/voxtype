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
#[derive(Default)]
pub enum AudioSource {
    /// User's microphone (local speaker)
    Microphone,
    /// System audio loopback (remote participants)
    Loopback,
    /// Unknown source
    #[default]
    Unknown,
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
        self.segments.iter().map(|s| s.end_ms).max().unwrap_or(0)
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
        let mut speakers: Vec<String> = self.segments.iter().map(|s| s.speaker_display()).collect();
        speakers.sort();
        speakers.dedup();
        speakers
    }
}

/// Meeting status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum MeetingStatus {
    /// Meeting is in progress
    #[default]
    Active,
    /// Meeting is paused
    Paused,
    /// Meeting has ended
    Completed,
    /// Meeting was cancelled/abandoned
    Cancelled,
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
        self.title
            .clone()
            .unwrap_or_else(|| self.started_at.format("Meeting %Y-%m-%d %H:%M").to_string())
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
            format!(
                "{}-{}",
                date,
                self.id.0.to_string().split('-').next().unwrap_or("meeting")
            )
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
        transcript.add_segment(TranscriptSegment::new(
            2,
            1000,
            2000,
            "world".to_string(),
            0,
        ));
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

    #[test]
    fn test_meeting_id_parse_invalid() {
        let result = MeetingId::parse("not-a-uuid");
        assert!(result.is_err());
    }

    #[test]
    fn test_meeting_id_parse_empty() {
        let result = MeetingId::parse("");
        assert!(result.is_err());
    }

    #[test]
    fn test_meeting_id_from_str() {
        let id = MeetingId::new();
        let parsed: MeetingId = id.to_string().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_meeting_id_default_is_unique() {
        let id1 = MeetingId::default();
        let id2 = MeetingId::default();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_audio_source_default() {
        let source = AudioSource::default();
        assert_eq!(source, AudioSource::Unknown);
    }

    #[test]
    fn test_audio_source_display() {
        assert_eq!(format!("{}", AudioSource::Microphone), "You");
        assert_eq!(format!("{}", AudioSource::Loopback), "Remote");
        assert_eq!(format!("{}", AudioSource::Unknown), "Unknown");
    }

    #[test]
    fn test_transcript_empty() {
        let transcript = Transcript::new();
        assert_eq!(transcript.plain_text(), "");
        assert_eq!(transcript.word_count(), 0);
        assert_eq!(transcript.duration_ms(), 0);
        assert!(transcript.speakers().is_empty());
        assert!(transcript.segments.is_empty());
    }

    #[test]
    fn test_transcript_text_with_speakers() {
        let mut transcript = Transcript::new();
        let mut seg1 = TranscriptSegment::new(0, 0, 1000, "Hello".to_string(), 0);
        seg1.source = AudioSource::Microphone;
        let mut seg2 = TranscriptSegment::new(1, 1000, 2000, "Hi there".to_string(), 0);
        seg2.source = AudioSource::Loopback;
        transcript.add_segment(seg1);
        transcript.add_segment(seg2);

        let text = transcript.text_with_speakers();
        assert!(text.contains("**You**: Hello"));
        assert!(text.contains("**Remote**: Hi there"));
    }

    #[test]
    fn test_transcript_text_with_speakers_merges_consecutive() {
        let mut transcript = Transcript::new();
        let mut seg1 = TranscriptSegment::new(0, 0, 1000, "Hello".to_string(), 0);
        seg1.source = AudioSource::Microphone;
        let mut seg2 = TranscriptSegment::new(1, 1000, 2000, "world".to_string(), 0);
        seg2.source = AudioSource::Microphone;
        transcript.add_segment(seg1);
        transcript.add_segment(seg2);

        let text = transcript.text_with_speakers();
        // Same speaker should not repeat the label
        assert_eq!(text.matches("**You**").count(), 1);
    }

    #[test]
    fn test_transcript_segments_by_speaker() {
        let mut transcript = Transcript::new();
        let mut seg1 = TranscriptSegment::new(0, 0, 1000, "Hello".to_string(), 0);
        seg1.source = AudioSource::Microphone;
        let mut seg2 = TranscriptSegment::new(1, 1000, 2000, "Hi".to_string(), 0);
        seg2.source = AudioSource::Loopback;
        let mut seg3 = TranscriptSegment::new(2, 2000, 3000, "Bye".to_string(), 0);
        seg3.source = AudioSource::Microphone;
        transcript.add_segment(seg1);
        transcript.add_segment(seg2);
        transcript.add_segment(seg3);

        let you_segments = transcript.segments_by_speaker("You");
        assert_eq!(you_segments.len(), 2);
        let remote_segments = transcript.segments_by_speaker("Remote");
        assert_eq!(remote_segments.len(), 1);
    }

    #[test]
    fn test_transcript_speakers_unique_sorted() {
        let mut transcript = Transcript::new();
        let mut seg1 = TranscriptSegment::new(0, 0, 1000, "A".to_string(), 0);
        seg1.source = AudioSource::Loopback;
        let mut seg2 = TranscriptSegment::new(1, 1000, 2000, "B".to_string(), 0);
        seg2.source = AudioSource::Microphone;
        let mut seg3 = TranscriptSegment::new(2, 2000, 3000, "C".to_string(), 0);
        seg3.source = AudioSource::Loopback;
        transcript.add_segment(seg1);
        transcript.add_segment(seg2);
        transcript.add_segment(seg3);

        let speakers = transcript.speakers();
        assert_eq!(speakers.len(), 2);
        assert!(speakers.contains(&"You".to_string()));
        assert!(speakers.contains(&"Remote".to_string()));
    }

    #[test]
    fn test_segment_speaker_display_with_label() {
        let mut segment = TranscriptSegment::new(0, 0, 1000, "Test".to_string(), 0);
        segment.speaker_label = Some("Alice".to_string());
        assert_eq!(segment.speaker_display(), "Alice");
    }

    #[test]
    fn test_segment_speaker_display_with_id_no_label() {
        let mut segment = TranscriptSegment::new(0, 0, 1000, "Test".to_string(), 0);
        segment.speaker_id = Some("SPEAKER_00".to_string());
        assert_eq!(segment.speaker_display(), "SPEAKER_00");
    }

    #[test]
    fn test_segment_speaker_display_label_overrides_id() {
        let mut segment = TranscriptSegment::new(0, 0, 1000, "Test".to_string(), 0);
        segment.speaker_id = Some("SPEAKER_00".to_string());
        segment.speaker_label = Some("Bob".to_string());
        assert_eq!(segment.speaker_display(), "Bob");
    }

    #[test]
    fn test_segment_duration_zero() {
        let segment = TranscriptSegment::new(0, 5000, 5000, "".to_string(), 0);
        assert_eq!(segment.duration_ms(), 0);
    }

    #[test]
    fn test_segment_format_timestamp_zero() {
        let segment = TranscriptSegment::new(0, 0, 1000, "Test".to_string(), 0);
        assert_eq!(segment.format_timestamp(), "00:00");
    }

    #[test]
    fn test_segment_format_timestamp_minutes_only() {
        let segment = TranscriptSegment::new(0, 125000, 130000, "Test".to_string(), 0);
        assert_eq!(segment.format_timestamp(), "02:05");
    }

    #[test]
    fn test_meeting_metadata_cancel() {
        let mut metadata = MeetingMetadata::new(Some("Cancelled".to_string()));
        assert_eq!(metadata.status, MeetingStatus::Active);
        metadata.cancel();
        assert_eq!(metadata.status, MeetingStatus::Cancelled);
        assert!(metadata.ended_at.is_some());
    }

    #[test]
    fn test_meeting_metadata_storage_dir_no_title() {
        let mut metadata = MeetingMetadata::new(None);
        metadata.started_at = DateTime::parse_from_rfc3339("2024-06-15T09:00:00Z")
            .unwrap()
            .into();
        let dir_name = metadata.storage_dir_name();
        assert!(dir_name.starts_with("2024-06-15-"));
    }

    #[test]
    fn test_meeting_metadata_complete_sets_duration() {
        let mut metadata = MeetingMetadata::new(Some("Duration Test".to_string()));
        std::thread::sleep(std::time::Duration::from_millis(10));
        metadata.complete();
        assert!(metadata.duration_secs.is_some());
    }

    #[test]
    fn test_meeting_data_add_segment() {
        let mut meeting = MeetingData::new(Some("Test".to_string()));
        assert!(meeting.transcript.segments.is_empty());

        meeting.add_segment(TranscriptSegment::new(0, 0, 1000, "Hello".to_string(), 0));
        assert_eq!(meeting.transcript.segments.len(), 1);
    }

    #[test]
    fn test_meeting_data_complete_sets_chunk_count() {
        let mut meeting = MeetingData::new(Some("Test".to_string()));
        meeting.transcript.total_chunks = 5;
        meeting.complete();
        assert_eq!(meeting.metadata.chunk_count, 5);
    }

    #[test]
    fn test_meeting_status_default() {
        let status = MeetingStatus::default();
        assert_eq!(status, MeetingStatus::Active);
    }

    #[test]
    fn test_meeting_metadata_new_defaults() {
        let metadata = MeetingMetadata::new(None);
        assert!(metadata.title.is_none());
        assert!(metadata.ended_at.is_none());
        assert!(metadata.duration_secs.is_none());
        assert_eq!(metadata.status, MeetingStatus::Active);
        assert_eq!(metadata.chunk_count, 0);
        assert!(!metadata.audio_retained);
        assert!(metadata.model.is_none());
        assert!(metadata.summary.is_none());
        assert!(metadata.synced_at.is_none());
    }

    #[test]
    fn test_segment_serialization_roundtrip() {
        let mut segment = TranscriptSegment::new(0, 0, 5000, "Hello world".to_string(), 0);
        segment.source = AudioSource::Microphone;
        segment.speaker_id = Some("SPEAKER_00".to_string());
        segment.confidence = Some(0.95);

        let json = serde_json::to_string(&segment).unwrap();
        let deserialized: TranscriptSegment = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, 0);
        assert_eq!(deserialized.text, "Hello world");
        assert_eq!(deserialized.source, AudioSource::Microphone);
        assert_eq!(deserialized.speaker_id, Some("SPEAKER_00".to_string()));
    }

    #[test]
    fn test_transcript_duration_ms() {
        let mut transcript = Transcript::new();
        transcript.add_segment(TranscriptSegment::new(0, 0, 5000, "A".to_string(), 0));
        transcript.add_segment(TranscriptSegment::new(1, 5000, 12000, "B".to_string(), 1));
        assert_eq!(transcript.duration_ms(), 12000);
    }
}
