//! Markdown exporter for meeting transcriptions

use crate::meeting::data::MeetingData;
use crate::meeting::export::{ExportError, ExportFormat, ExportOptions, Exporter};

/// Markdown exporter
pub struct MarkdownExporter;

impl Exporter for MarkdownExporter {
    fn format(&self) -> ExportFormat {
        ExportFormat::Markdown
    }

    fn export(
        &self,
        meeting: &MeetingData,
        options: &ExportOptions,
    ) -> Result<String, ExportError> {
        let mut output = String::new();

        // Title
        output.push_str(&format!("# {}\n\n", meeting.metadata.display_title()));

        // Metadata
        if options.include_metadata {
            output.push_str("## Meeting Info\n\n");
            output.push_str(&format!(
                "- **Date:** {}\n",
                meeting.metadata.started_at.format("%Y-%m-%d %H:%M UTC")
            ));
            if let Some(duration) = meeting.metadata.duration_secs {
                let hours = duration / 3600;
                let mins = (duration % 3600) / 60;
                let secs = duration % 60;
                if hours > 0 {
                    output.push_str(&format!("- **Duration:** {}h {}m {}s\n", hours, mins, secs));
                } else {
                    output.push_str(&format!("- **Duration:** {}m {}s\n", mins, secs));
                }
            }
            output.push_str(&format!(
                "- **Word Count:** {}\n",
                meeting.transcript.word_count()
            ));
            output.push_str(&format!(
                "- **Segments:** {}\n",
                meeting.transcript.segments.len()
            ));

            let speakers = meeting.transcript.speakers();
            if !speakers.is_empty() {
                output.push_str(&format!("- **Speakers:** {}\n", speakers.join(", ")));
            }

            output.push('\n');
        }

        // Summary (if available, Phase 5)
        if let Some(ref summary) = meeting.metadata.summary {
            output.push_str("## Summary\n\n");
            output.push_str(&summary.summary);
            output.push_str("\n\n");

            if !summary.key_points.is_empty() {
                output.push_str("### Key Points\n\n");
                for point in &summary.key_points {
                    output.push_str(&format!("- {}\n", point));
                }
                output.push('\n');
            }

            if !summary.action_items.is_empty() {
                output.push_str("### Action Items\n\n");
                for item in &summary.action_items {
                    let checkbox = if item.completed { "[x]" } else { "[ ]" };
                    let assignee = item
                        .assignee
                        .as_ref()
                        .map(|a| format!(" (@{})", a))
                        .unwrap_or_default();
                    output.push_str(&format!(
                        "- {} {}{}\n",
                        checkbox, item.description, assignee
                    ));
                }
                output.push('\n');
            }

            if !summary.decisions.is_empty() {
                output.push_str("### Decisions\n\n");
                for decision in &summary.decisions {
                    output.push_str(&format!("- {}\n", decision));
                }
                output.push('\n');
            }
        }

        // Transcript
        output.push_str("## Transcript\n\n");

        let mut last_speaker = String::new();

        for segment in &meeting.transcript.segments {
            if options.include_speakers {
                let speaker = segment.speaker_display();
                if speaker != last_speaker {
                    if !last_speaker.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&format!("### {}\n\n", speaker));
                    last_speaker = speaker;
                }
            }

            if options.include_timestamps {
                output.push_str(&format!("*[{}]* ", segment.format_timestamp()));
            }

            output.push_str(&segment.text);
            output.push_str("\n\n");
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::{AudioSource, TranscriptSegment};

    fn create_test_meeting() -> MeetingData {
        let mut meeting = MeetingData::new(Some("Weekly Standup".to_string()));

        let mut seg1 = TranscriptSegment::new(0, 0, 5000, "Good morning everyone.".to_string(), 0);
        seg1.source = AudioSource::Microphone;

        let mut seg2 = TranscriptSegment::new(1, 5000, 10000, "Hey, good morning!".to_string(), 0);
        seg2.source = AudioSource::Loopback;

        meeting.transcript.add_segment(seg1);
        meeting.transcript.add_segment(seg2);
        meeting
    }

    #[test]
    fn test_markdown_export_basic() {
        let meeting = create_test_meeting();
        let exporter = MarkdownExporter;
        let options = ExportOptions::default();

        let output = exporter.export(&meeting, &options).unwrap();
        assert!(output.starts_with("# Weekly Standup"));
        assert!(output.contains("## Transcript"));
    }

    #[test]
    fn test_markdown_export_with_metadata() {
        let meeting = create_test_meeting();
        let exporter = MarkdownExporter;
        let options = ExportOptions {
            include_metadata: true,
            ..Default::default()
        };

        let output = exporter.export(&meeting, &options).unwrap();
        assert!(output.contains("## Meeting Info"));
        assert!(output.contains("**Date:**"));
        assert!(output.contains("**Word Count:**"));
    }

    #[test]
    fn test_markdown_export_with_speakers() {
        let meeting = create_test_meeting();
        let exporter = MarkdownExporter;
        let options = ExportOptions {
            include_speakers: true,
            ..Default::default()
        };

        let output = exporter.export(&meeting, &options).unwrap();
        assert!(output.contains("### You"));
        assert!(output.contains("### Remote"));
    }

    #[test]
    fn test_markdown_export_with_timestamps() {
        let meeting = create_test_meeting();
        let exporter = MarkdownExporter;
        let options = ExportOptions {
            include_timestamps: true,
            ..Default::default()
        };

        let output = exporter.export(&meeting, &options).unwrap();
        assert!(output.contains("*[00:00]*"));
        assert!(output.contains("*[00:05]*"));
    }
}
