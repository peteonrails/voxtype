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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// A note or highlight attached to one transcript line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentAnnotation {
    /// The note; empty for a bare highlight
    pub text: String,
    /// Whether the line is highlighted
    pub highlight: bool,
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
    /// The speaker the diarizer assigned, when the line has been moved to
    /// another speaker by hand (`meeting reassign`). `speaker_id` then holds
    /// the chosen speaker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_speaker: Option<String>,
    /// A note or highlight the user attached to this line
    /// (`meeting annotate`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<SegmentAnnotation>,
    /// The text as transcribed, when the user corrected it with
    /// `meeting edit`; `text` then holds the correction
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_text: Option<String>,
    /// Hidden with `meeting delete-line`. Deleted lines stay in
    /// transcript.json so they can be restored, and are left out of
    /// everything that reads a meeting through voxtype.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
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
            original_speaker: None,
            annotation: None,
            original_text: None,
            deleted: false,
            confidence: None,
            chunk_id,
        }
    }

    /// Duration of this segment in milliseconds
    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }

    /// The speaker as the diarizer named it: the diarization ID when there
    /// is one, otherwise the track (`You` or `Remote`). This is what
    /// `meeting label` keys a human name on.
    pub fn speaker_key(&self) -> String {
        self.speaker_id
            .clone()
            .unwrap_or_else(|| self.source.to_string())
    }

    /// Get the display name for the speaker
    pub fn speaker_display(&self) -> String {
        self.speaker_label
            .clone()
            .unwrap_or_else(|| self.speaker_key())
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

    /// Add a segment, keeping the transcript in order of start time.
    ///
    /// The microphone and loopback tracks are transcribed one after the
    /// other for the same stretch of time, so appending would list all of
    /// one side and then all of the other. A segment goes after any that
    /// start at the same moment, so arrival order breaks ties.
    pub fn add_segment(&mut self, segment: TranscriptSegment) {
        let at = self
            .segments
            .partition_point(|s| s.start_ms <= segment.start_ms);
        self.segments.insert(at, segment);
    }

    /// Order segments by start time. Transcripts written before segments
    /// were kept in order list each chunk's tracks one after the other.
    pub fn sort_chronologically(&mut self) {
        self.segments.sort_by_key(|s| s.start_ms);
    }

    /// Remove echoed phrases from mic segments that match loopback transcripts.
    /// Works at the phrase level: finds runs of consecutive words in a mic segment
    /// that appear in any loopback segment and strips them out, keeping the user's
    /// actual speech. Returns the number of words removed.
    pub fn dedup_bleed_through(&mut self) -> usize {
        // Build a set of loopback n-grams for fast lookup.
        // We use 4-grams as the minimum match length to avoid false positives.
        let min_ngram = 4;

        let total_removed =
            self.dedup_short_echoes(min_ngram) + self.dedup_echoed_phrases(min_ngram);

        // A segment that was all echo is gone, not an empty line.
        self.segments.retain(|s| !s.text.trim().is_empty());

        total_removed
    }

    /// A microphone segment too short for the phrase match ("Sounds good.")
    /// is echo when a loopback segment overlapping it in time holds the same
    /// words in the same order. The overlap is what keeps a reply you make
    /// later in the same words.
    fn dedup_short_echoes(&mut self, min_ngram: usize) -> usize {
        const TOLERANCE_MS: u64 = 1000;
        let loopback: Vec<(u64, u64, Vec<String>)> = self
            .segments
            .iter()
            .filter(|s| s.source == AudioSource::Loopback)
            .map(|s| (s.start_ms, s.end_ms, normalize_for_dedup(&s.text)))
            .collect();

        let mut removed = 0;
        for seg in &mut self.segments {
            if seg.source != AudioSource::Microphone {
                continue;
            }
            let words = normalize_for_dedup(&seg.text);
            if words.is_empty() || words.len() >= min_ngram {
                continue;
            }
            let echoed = loopback.iter().any(|(start, end, said)| {
                seg.start_ms < end + TOLERANCE_MS
                    && *start < seg.end_ms + TOLERANCE_MS
                    && said.windows(words.len()).any(|w| w == words.as_slice())
            });
            if echoed {
                tracing::debug!("Stripping short echo: {:?}", seg.text);
                removed += words.len();
                seg.text.clear();
            }
        }
        removed
    }

    fn dedup_echoed_phrases(&mut self, min_ngram: usize) -> usize {
        let loopback_words: Vec<String> = self
            .segments
            .iter()
            .filter(|s| s.source == AudioSource::Loopback)
            .flat_map(|s| normalize_for_dedup(&s.text))
            .collect();

        if loopback_words.len() < min_ngram {
            return 0;
        }

        // Build set of all n-grams (4 to 8 words) from loopback
        let mut loopback_ngrams: std::collections::HashSet<Vec<&str>> =
            std::collections::HashSet::new();
        for n in min_ngram..=8.min(loopback_words.len()) {
            for window in loopback_words.windows(n) {
                loopback_ngrams.insert(window.iter().map(|s| s.as_str()).collect());
            }
        }

        let mut total_removed = 0;

        for seg in &mut self.segments {
            if seg.source != AudioSource::Microphone {
                continue;
            }

            let original_text = seg.text.clone();
            let words: Vec<&str> = original_text.split_whitespace().collect();
            if words.len() < min_ngram {
                continue;
            }

            // Find which words are part of echoed phrases.
            // A word is "echoed" if it's part of a consecutive run of >= min_ngram
            // words that appears in the loopback transcript.
            let mut keep = vec![true; words.len()];
            let normalized: Vec<String> = words
                .iter()
                .map(|w| {
                    w.to_lowercase()
                        .chars()
                        .filter(|c| c.is_alphanumeric())
                        .collect::<String>()
                })
                .collect();

            // Greedy: find longest matching runs first
            let mut i = 0;
            while i + min_ngram <= normalized.len() {
                // Try longest match first (up to 8 words)
                let max_n = 8.min(normalized.len() - i);
                let mut matched_len = 0;

                for n in (min_ngram..=max_n).rev() {
                    let ngram: Vec<&str> =
                        normalized[i..i + n].iter().map(|s| s.as_str()).collect();
                    if loopback_ngrams.contains(&ngram) {
                        matched_len = n;
                        break;
                    }
                }

                if matched_len > 0 {
                    // Try to extend the match beyond the ngram window
                    let mut end = i + matched_len;
                    while end < normalized.len() {
                        // Check if adding the next word still matches loopback
                        let check_start = end.saturating_sub(min_ngram - 1);
                        let check_len = end - check_start + 1;
                        if check_len >= min_ngram {
                            let ngram: Vec<&str> = normalized[check_start..=end]
                                .iter()
                                .map(|s| s.as_str())
                                .collect();
                            if loopback_ngrams.contains(&ngram) {
                                end += 1;
                                continue;
                            }
                        }
                        break;
                    }

                    for entry in keep.iter_mut().take(end).skip(i) {
                        *entry = false;
                    }
                    total_removed += end - i;

                    tracing::debug!(
                        "Stripping echoed phrase ({} words): {:?}",
                        end - i,
                        words[i..end].join(" ")
                    );

                    i = end;
                } else {
                    i += 1;
                }
            }

            // Reconstruct text from kept words
            if keep.iter().any(|&k| !k) {
                let new_text: String = words
                    .iter()
                    .zip(keep.iter())
                    .filter(|(_, &k)| k)
                    .map(|(&w, _)| w)
                    .collect::<Vec<_>>()
                    .join(" ");
                seg.text = new_text;
            }
        }

        total_removed
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

    /// Name speakers: `labels` maps a speaker ID ([`TranscriptSegment::speaker_key`])
    /// to a human name. Speakers given the same name display as one.
    pub fn apply_speaker_labels(&mut self, labels: &std::collections::HashMap<String, String>) {
        if labels.is_empty() {
            return;
        }
        for segment in &mut self.segments {
            if let Some(label) = labels.get(&segment.speaker_key()) {
                segment.speaker_label = Some(label.clone());
            }
        }
    }

    /// Move lines to the speakers chosen by hand: `overrides` maps a segment
    /// ID to a speaker ID. A line without an override goes back to the
    /// speaker the diarizer gave it. A line whose speaker changes loses its
    /// old name, so apply labels after this.
    pub fn apply_speaker_overrides(&mut self, overrides: &std::collections::HashMap<u32, String>) {
        for segment in &mut self.segments {
            let assigned = segment
                .original_speaker
                .take()
                .unwrap_or_else(|| segment.speaker_key());
            let target = overrides.get(&segment.id).unwrap_or(&assigned);
            if segment.speaker_key() != *target {
                segment.speaker_id = Some(target.clone());
                segment.speaker_label = None;
            }
            if *target != assigned {
                segment.original_speaker = Some(assigned);
            }
        }
    }

    /// Attach the user's notes and highlights: `annotations` maps a segment
    /// ID to its annotation, and a line missing from it has none.
    pub fn apply_annotations(
        &mut self,
        annotations: &std::collections::HashMap<u32, SegmentAnnotation>,
    ) {
        for segment in &mut self.segments {
            segment.annotation = annotations.get(&segment.id).cloned();
        }
    }

    /// Apply the user's corrections: `edits` maps a segment ID to its new
    /// text. A line without an edit goes back to what was transcribed.
    pub fn apply_text_edits(&mut self, edits: &std::collections::HashMap<u32, String>) {
        for segment in &mut self.segments {
            if let Some(original) = segment.original_text.take() {
                segment.text = original;
            }
            if let Some(edit) = edits.get(&segment.id) {
                if *edit != segment.text {
                    segment.original_text =
                        Some(std::mem::replace(&mut segment.text, edit.clone()));
                }
            }
        }
    }

    /// Mark the lines the user deleted; every other line is not deleted.
    pub fn apply_deletions(&mut self, deleted: &std::collections::HashSet<u32>) {
        for segment in &mut self.segments {
            segment.deleted = deleted.contains(&segment.id);
        }
    }

    /// Drop deleted lines, for anything that shows or reads the meeting.
    pub fn remove_deleted(&mut self) {
        self.segments.retain(|s| !s.deleted);
    }

    /// Rename a speaker ID across the transcript, including lines moved to
    /// or away from it by hand.
    pub fn rename_speaker(&mut self, from: &str, to: &str) -> usize {
        let mut renamed = 0;
        for segment in &mut self.segments {
            if segment.speaker_id.as_deref() == Some(from) {
                segment.speaker_id = Some(to.to_string());
                renamed += 1;
            }
            if segment.original_speaker.as_deref() == Some(from) {
                segment.original_speaker = Some(to.to_string());
                renamed += 1;
            }
        }
        renamed
    }

    /// Unique speaker IDs, before any human labels.
    pub fn speaker_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.segments.iter().map(|s| s.speaker_key()).collect();
        keys.sort();
        keys.dedup();
        keys
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
    /// The call's link, from `meeting start --url`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
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
            url: None,
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
    /// Topics in order, each starting at a transcript time
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<Chapter>,
    /// When the summary was generated
    pub generated_at: DateTime<Utc>,
    /// Model used to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// One topic of a meeting summary, starting at a transcript time
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Chapter {
    /// Where the topic begins, in milliseconds from meeting start
    pub start_ms: u64,
    /// Short title of the topic
    pub title: String,
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
    /// The user's free-text notes (notes.md), when there are any
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl MeetingData {
    /// Create a new meeting
    pub fn new(title: Option<String>) -> Self {
        Self {
            metadata: MeetingMetadata::new(title),
            transcript: Transcript::new(),
            notes: None,
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

/// Normalize text for dedup comparison: lowercase, strip punctuation, split into words
fn normalize_for_dedup(text: &str) -> Vec<String> {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect()
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

    fn segment(id: u32, start_ms: u64, source: AudioSource, chunk_id: u32) -> TranscriptSegment {
        let mut seg =
            TranscriptSegment::new(id, start_ms, start_ms + 1500, format!("s{}", id), chunk_id);
        seg.source = source;
        seg
    }

    /// The daemon transcribes a chunk's microphone track, then its loopback
    /// track. Turn order has to come from the timestamps, not arrival.
    #[test]
    fn transcript_interleaves_tracks_by_start_time() {
        let mut transcript = Transcript::new();
        // Chunk 0: mic then loopback.
        transcript.add_segment(segment(0, 1_000, AudioSource::Microphone, 0));
        transcript.add_segment(segment(1, 12_000, AudioSource::Microphone, 0));
        transcript.add_segment(segment(2, 5_000, AudioSource::Loopback, 1));
        transcript.add_segment(segment(3, 20_000, AudioSource::Loopback, 1));
        // Chunk 1.
        transcript.add_segment(segment(4, 31_000, AudioSource::Microphone, 2));
        transcript.add_segment(segment(5, 30_500, AudioSource::Loopback, 3));

        let order: Vec<u32> = transcript.segments.iter().map(|s| s.id).collect();
        assert_eq!(order, vec![0, 2, 1, 3, 5, 4]);
    }

    #[test]
    fn transcript_keeps_arrival_order_for_equal_starts() {
        let mut transcript = Transcript::new();
        transcript.add_segment(segment(0, 1_000, AudioSource::Microphone, 0));
        transcript.add_segment(segment(1, 1_000, AudioSource::Loopback, 1));
        let order: Vec<u32> = transcript.segments.iter().map(|s| s.id).collect();
        assert_eq!(order, vec![0, 1]);
    }

    #[test]
    fn sort_chronologically_reorders_a_chunk_grouped_transcript() {
        let mut transcript = Transcript::new();
        transcript.segments = vec![
            segment(0, 1_000, AudioSource::Microphone, 0),
            segment(1, 12_000, AudioSource::Microphone, 0),
            segment(2, 5_000, AudioSource::Loopback, 1),
        ];
        transcript.sort_chronologically();
        let order: Vec<u32> = transcript.segments.iter().map(|s| s.id).collect();
        assert_eq!(order, vec![0, 2, 1]);
    }

    fn said(
        id: u32,
        start_ms: u64,
        end_ms: u64,
        source: AudioSource,
        text: &str,
    ) -> TranscriptSegment {
        let mut seg = TranscriptSegment::new(id, start_ms, end_ms, text.to_string(), 0);
        seg.source = source;
        seg
    }

    fn lines(transcript: &Transcript) -> Vec<String> {
        transcript
            .segments
            .iter()
            .map(|s| format!("{}: {}", s.source, s.text))
            .collect()
    }

    #[test]
    fn dedup_strips_an_echoed_phrase_and_keeps_the_rest() {
        let mut t = Transcript::new();
        t.add_segment(said(
            0,
            1_000,
            6_000,
            AudioSource::Loopback,
            "We should ship the release on Friday.",
        ));
        t.add_segment(said(
            1,
            1_000,
            8_000,
            AudioSource::Microphone,
            "we should ship the release on Friday agreed",
        ));
        assert_eq!(t.dedup_bleed_through(), 7);
        assert_eq!(
            lines(&t),
            vec![
                "Remote: We should ship the release on Friday.",
                "You: agreed"
            ]
        );
    }

    /// Bug: a segment that was all echo stayed behind as an empty line.
    #[test]
    fn dedup_drops_a_segment_that_was_all_echo() {
        let mut t = Transcript::new();
        t.add_segment(said(
            0,
            1_000,
            6_000,
            AudioSource::Loopback,
            "We should ship the release on Friday.",
        ));
        t.add_segment(said(
            1,
            1_000,
            6_000,
            AudioSource::Microphone,
            "We should ship the release on Friday.",
        ));
        t.dedup_bleed_through();
        assert_eq!(
            lines(&t),
            vec!["Remote: We should ship the release on Friday."]
        );
    }

    /// Bug: echoes under four words ("Sounds good.") were never recognised.
    #[test]
    fn dedup_strips_a_short_echo_that_overlaps_the_remote_line() {
        let mut t = Transcript::new();
        t.add_segment(said(
            0,
            9_100,
            10_000,
            AudioSource::Loopback,
            "Sounds good.",
        ));
        t.add_segment(said(
            1,
            9_150,
            10_050,
            AudioSource::Microphone,
            "Sounds good.",
        ));
        t.dedup_bleed_through();
        assert_eq!(lines(&t), vec!["Remote: Sounds good."]);
    }

    #[test]
    fn dedup_keeps_a_short_reply_in_the_same_words_said_later() {
        let mut t = Transcript::new();
        t.add_segment(said(
            0,
            9_100,
            10_000,
            AudioSource::Loopback,
            "Sounds good.",
        ));
        t.add_segment(said(
            1,
            14_000,
            15_000,
            AudioSource::Microphone,
            "Sounds good.",
        ));
        t.dedup_bleed_through();
        assert_eq!(lines(&t), vec!["Remote: Sounds good.", "You: Sounds good."]);
    }

    fn spoken_by(id: u32, speaker: &str) -> TranscriptSegment {
        let mut seg =
            TranscriptSegment::new(id, id as u64 * 1000, id as u64 * 1000 + 900, "x".into(), 0);
        seg.speaker_id = Some(speaker.to_string());
        seg
    }

    fn keys(t: &Transcript) -> Vec<(String, Option<String>)> {
        t.segments
            .iter()
            .map(|s| (s.speaker_key(), s.original_speaker.clone()))
            .collect()
    }

    #[test]
    fn overrides_move_a_line_and_remember_the_diarizers_choice() {
        let mut t = Transcript::new();
        t.add_segment(spoken_by(0, "Remote 1"));
        t.add_segment(spoken_by(1, "Remote 1"));
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(1, "Remote 2".to_string());

        t.apply_speaker_overrides(&overrides);
        assert_eq!(
            keys(&t),
            vec![
                ("Remote 1".to_string(), None),
                ("Remote 2".to_string(), Some("Remote 1".to_string())),
            ]
        );

        // Applying again is a no-op; removing the override undoes it.
        t.apply_speaker_overrides(&overrides);
        assert_eq!(t.segments[1].original_speaker.as_deref(), Some("Remote 1"));
        t.apply_speaker_overrides(&std::collections::HashMap::new());
        assert_eq!(
            keys(&t),
            vec![
                ("Remote 1".to_string(), None),
                ("Remote 1".to_string(), None)
            ]
        );
    }

    #[test]
    fn a_moved_line_drops_its_old_speakers_name() {
        let mut t = Transcript::new();
        let mut seg = spoken_by(0, "Remote 1");
        seg.speaker_label = Some("Primagen".to_string());
        t.add_segment(seg);
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(0, "Remote 2".to_string());
        t.apply_speaker_overrides(&overrides);
        assert_eq!(t.segments[0].speaker_display(), "Remote 2");
    }

    #[test]
    fn an_override_to_the_diarizers_own_choice_is_no_move() {
        let mut t = Transcript::new();
        t.add_segment(spoken_by(0, "You"));
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(0, "You".to_string());
        t.apply_speaker_overrides(&overrides);
        assert_eq!(keys(&t), vec![("You".to_string(), None)]);
    }

    #[test]
    fn a_rename_reaches_moved_lines_on_both_sides() {
        let mut t = Transcript::new();
        t.add_segment(spoken_by(0, "Remote"));
        t.add_segment(spoken_by(1, "Remote"));
        t.add_segment(spoken_by(2, "Remote 2"));
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(1, "Person 1".to_string()); // moved away from Remote
        overrides.insert(2, "Remote".to_string()); // moved onto Remote
        t.apply_speaker_overrides(&overrides);

        t.rename_speaker("Remote", "Remote 1");
        assert_eq!(
            keys(&t),
            vec![
                ("Remote 1".to_string(), None),
                ("Person 1".to_string(), Some("Remote 1".to_string())),
                ("Remote 1".to_string(), Some("Remote 2".to_string())),
            ]
        );
    }

    #[test]
    fn annotations_attach_by_segment_id_and_come_off_again() {
        let mut t = Transcript::new();
        t.add_segment(spoken_by(0, "You"));
        t.add_segment(spoken_by(1, "Remote"));
        let mut annotations = std::collections::HashMap::new();
        let note = SegmentAnnotation {
            text: "check".to_string(),
            highlight: false,
        };
        annotations.insert(1, note.clone());
        t.apply_annotations(&annotations);
        assert_eq!(t.segments[0].annotation, None);
        assert_eq!(t.segments[1].annotation, Some(note));

        t.apply_annotations(&std::collections::HashMap::new());
        assert_eq!(t.segments[1].annotation, None);
        let json = serde_json::to_value(&t.segments[1]).unwrap();
        assert!(json.get("annotation").is_none());
    }

    #[test]
    fn text_edits_replace_a_line_and_remember_the_transcription() {
        let mut t = Transcript::new();
        t.add_segment(spoken_by(0, "You"));
        t.add_segment(spoken_by(1, "Remote"));
        let mut edits = std::collections::HashMap::new();
        edits.insert(1, "corrected".to_string());

        t.apply_text_edits(&edits);
        t.apply_text_edits(&edits);
        assert_eq!(t.segments[1].text, "corrected");
        assert_eq!(t.segments[1].original_text.as_deref(), Some("x"));
        assert_eq!(t.segments[0].original_text, None);

        t.apply_text_edits(&std::collections::HashMap::new());
        assert_eq!(t.segments[1].text, "x");
        assert_eq!(t.segments[1].original_text, None);

        // An edit that matches the transcription is no edit.
        edits.insert(1, "x".to_string());
        t.apply_text_edits(&edits);
        assert_eq!(t.segments[1].original_text, None);
    }

    #[test]
    fn deletions_mark_lines_and_remove_deleted_drops_them() {
        let mut t = Transcript::new();
        t.add_segment(spoken_by(0, "You"));
        t.add_segment(spoken_by(1, "Remote"));
        t.apply_deletions(&[1].into_iter().collect());
        assert!(t.segments[1].deleted);
        assert_eq!(
            serde_json::to_value(&t.segments[1]).unwrap()["deleted"],
            true
        );
        assert!(serde_json::to_value(&t.segments[0])
            .unwrap()
            .get("deleted")
            .is_none());

        let mut shown = t.clone();
        shown.remove_deleted();
        assert_eq!(shown.segments.len(), 1);

        t.apply_deletions(&std::collections::HashSet::new());
        assert!(!t.segments[1].deleted);
    }

    #[test]
    fn test_transcript_duration_ms() {
        let mut transcript = Transcript::new();
        transcript.add_segment(TranscriptSegment::new(0, 0, 5000, "A".to_string(), 0));
        transcript.add_segment(TranscriptSegment::new(1, 5000, 12000, "B".to_string(), 1));
        assert_eq!(transcript.duration_ms(), 12000);
    }
}
