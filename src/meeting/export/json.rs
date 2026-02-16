//! JSON exporter for meeting transcriptions

use crate::meeting::data::MeetingData;
use crate::meeting::export::{ExportError, ExportFormat, ExportOptions, Exporter};
use serde::Serialize;

/// JSON exporter
pub struct JsonExporter;

/// Exported meeting structure for JSON
#[derive(Serialize)]
struct ExportedMeeting {
    metadata: ExportedMetadata,
    transcript: ExportedTranscript,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<ExportedSummary>,
}

#[derive(Serialize)]
struct ExportedMetadata {
    id: String,
    title: Option<String>,
    #[serde(rename = "startedAt")]
    started_at: String,
    #[serde(rename = "endedAt", skip_serializing_if = "Option::is_none")]
    ended_at: Option<String>,
    #[serde(rename = "durationSecs", skip_serializing_if = "Option::is_none")]
    duration_secs: Option<u64>,
    status: String,
    #[serde(rename = "chunkCount")]
    chunk_count: u32,
}

#[derive(Serialize)]
struct ExportedTranscript {
    segments: Vec<ExportedSegment>,
    #[serde(rename = "totalChunks")]
    total_chunks: u32,
    #[serde(rename = "wordCount")]
    word_count: usize,
    #[serde(rename = "durationMs")]
    duration_ms: u64,
    speakers: Vec<String>,
}

#[derive(Serialize)]
struct ExportedSegment {
    id: u32,
    #[serde(rename = "startMs")]
    start_ms: u64,
    #[serde(rename = "endMs")]
    end_ms: u64,
    text: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    speaker: Option<String>,
    #[serde(rename = "chunkId")]
    chunk_id: u32,
}

#[derive(Serialize)]
struct ExportedSummary {
    summary: String,
    #[serde(rename = "keyPoints")]
    key_points: Vec<String>,
    #[serde(rename = "actionItems")]
    action_items: Vec<ExportedActionItem>,
    decisions: Vec<String>,
    #[serde(rename = "generatedAt")]
    generated_at: String,
}

#[derive(Serialize)]
struct ExportedActionItem {
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    assignee: Option<String>,
    #[serde(rename = "dueDate", skip_serializing_if = "Option::is_none")]
    due_date: Option<String>,
    completed: bool,
}

impl Exporter for JsonExporter {
    fn format(&self) -> ExportFormat {
        ExportFormat::Json
    }

    fn export(
        &self,
        meeting: &MeetingData,
        _options: &ExportOptions,
    ) -> Result<String, ExportError> {
        let exported = ExportedMeeting {
            metadata: ExportedMetadata {
                id: meeting.metadata.id.to_string(),
                title: meeting.metadata.title.clone(),
                started_at: meeting.metadata.started_at.to_rfc3339(),
                ended_at: meeting.metadata.ended_at.map(|dt| dt.to_rfc3339()),
                duration_secs: meeting.metadata.duration_secs,
                status: format!("{:?}", meeting.metadata.status).to_lowercase(),
                chunk_count: meeting.metadata.chunk_count,
            },
            transcript: ExportedTranscript {
                segments: meeting
                    .transcript
                    .segments
                    .iter()
                    .map(|s| ExportedSegment {
                        id: s.id,
                        start_ms: s.start_ms,
                        end_ms: s.end_ms,
                        text: s.text.clone(),
                        source: format!("{:?}", s.source).to_lowercase(),
                        speaker: s.speaker_label.clone().or_else(|| s.speaker_id.clone()),
                        chunk_id: s.chunk_id,
                    })
                    .collect(),
                total_chunks: meeting.transcript.total_chunks,
                word_count: meeting.transcript.word_count(),
                duration_ms: meeting.transcript.duration_ms(),
                speakers: meeting.transcript.speakers(),
            },
            summary: meeting.metadata.summary.as_ref().map(|s| ExportedSummary {
                summary: s.summary.clone(),
                key_points: s.key_points.clone(),
                action_items: s
                    .action_items
                    .iter()
                    .map(|a| ExportedActionItem {
                        description: a.description.clone(),
                        assignee: a.assignee.clone(),
                        due_date: a.due_date.clone(),
                        completed: a.completed,
                    })
                    .collect(),
                decisions: s.decisions.clone(),
                generated_at: s.generated_at.to_rfc3339(),
            }),
        };

        serde_json::to_string_pretty(&exported)
            .map_err(|e| ExportError::Serialization(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::TranscriptSegment;

    fn create_test_meeting() -> MeetingData {
        let mut meeting = MeetingData::new(Some("Test Meeting".to_string()));
        meeting.transcript.add_segment(TranscriptSegment::new(
            0,
            0,
            5000,
            "Hello world.".to_string(),
            0,
        ));
        meeting
    }

    #[test]
    fn test_json_export() {
        let meeting = create_test_meeting();
        let exporter = JsonExporter;
        let options = ExportOptions::default();

        let output = exporter.export(&meeting, &options).unwrap();

        // Parse and verify structure
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert!(parsed["metadata"]["id"].is_string());
        assert_eq!(parsed["metadata"]["title"].as_str(), Some("Test Meeting"));
        assert!(parsed["transcript"]["segments"].is_array());
        assert_eq!(
            parsed["transcript"]["segments"][0]["text"].as_str(),
            Some("Hello world.")
        );
    }

    #[test]
    fn test_json_export_valid_json() {
        let meeting = create_test_meeting();
        let exporter = JsonExporter;
        let options = ExportOptions::default();

        let output = exporter.export(&meeting, &options).unwrap();

        // Should be valid JSON
        let result: Result<serde_json::Value, _> = serde_json::from_str(&output);
        assert!(result.is_ok());
    }

    #[test]
    fn test_json_export_roundtrip() {
        let meeting = create_test_meeting();
        let exporter = JsonExporter;
        let options = ExportOptions::default();

        let output = exporter.export(&meeting, &options).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

        // Verify key fields
        assert_eq!(parsed["transcript"]["wordCount"].as_u64(), Some(2));
        assert_eq!(
            parsed["transcript"]["segments"].as_array().unwrap().len(),
            1
        );
    }
}
