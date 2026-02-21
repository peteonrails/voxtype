//! Plain text exporter for meeting transcriptions

use crate::meeting::data::MeetingData;
use crate::meeting::export::{ExportError, ExportFormat, ExportOptions, Exporter};

/// Plain text exporter
pub struct TextExporter;

impl Exporter for TextExporter {
    fn format(&self) -> ExportFormat {
        ExportFormat::Text
    }

    fn export(
        &self,
        meeting: &MeetingData,
        options: &ExportOptions,
    ) -> Result<String, ExportError> {
        let mut output = String::new();

        // Metadata header
        if options.include_metadata {
            output.push_str(&meeting.metadata.display_title());
            output.push('\n');
            output.push_str(&format!(
                "Date: {}\n",
                meeting.metadata.started_at.format("%Y-%m-%d %H:%M")
            ));
            if let Some(duration) = meeting.metadata.duration_secs {
                let mins = duration / 60;
                let secs = duration % 60;
                output.push_str(&format!("Duration: {}:{:02}\n", mins, secs));
            }
            output.push_str(&format!("Words: {}\n", meeting.transcript.word_count()));
            output.push('\n');
            output.push_str(&"=".repeat(60));
            output.push_str("\n\n");
        }

        // Transcript
        let mut last_speaker = String::new();

        for segment in &meeting.transcript.segments {
            let mut line = String::new();

            // Timestamp
            if options.include_timestamps {
                line.push_str(&format!("[{}] ", segment.format_timestamp()));
            }

            // Speaker change
            if options.include_speakers {
                let speaker = segment.speaker_display();
                if speaker != last_speaker {
                    if !last_speaker.is_empty() {
                        output.push('\n');
                    }
                    line.push_str(&format!("{}:\n", speaker));
                    last_speaker = speaker;
                }
            }

            // Text
            line.push_str(&segment.text);

            // Word wrap if configured
            if options.line_width > 0 {
                output.push_str(&wrap_text(&line, options.line_width));
            } else {
                output.push_str(&line);
            }
            output.push('\n');
        }

        Ok(output)
    }
}

/// Simple word wrapping
fn wrap_text(text: &str, width: usize) -> String {
    if width == 0 || text.len() <= width {
        return text.to_string();
    }

    let mut result = String::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            current_line.push_str(word);
        } else if current_line.len() + 1 + word.len() <= width {
            current_line.push(' ');
            current_line.push_str(word);
        } else {
            result.push_str(&current_line);
            result.push('\n');
            current_line = word.to_string();
        }
    }

    if !current_line.is_empty() {
        result.push_str(&current_line);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::{MeetingMetadata, Transcript, TranscriptSegment};

    fn create_test_meeting() -> MeetingData {
        let mut meeting = MeetingData::new(Some("Test Meeting".to_string()));
        meeting.transcript.add_segment(TranscriptSegment::new(
            0,
            0,
            5000,
            "Hello world, this is a test.".to_string(),
            0,
        ));
        meeting.transcript.add_segment(TranscriptSegment::new(
            1,
            5000,
            10000,
            "This is the second segment.".to_string(),
            0,
        ));
        meeting
    }

    #[test]
    fn test_text_export_basic() {
        let meeting = create_test_meeting();
        let exporter = TextExporter;
        let options = ExportOptions::default();

        let output = exporter.export(&meeting, &options).unwrap();
        assert!(output.contains("Hello world"));
        assert!(output.contains("second segment"));
    }

    #[test]
    fn test_text_export_with_timestamps() {
        let meeting = create_test_meeting();
        let exporter = TextExporter;
        let options = ExportOptions {
            include_timestamps: true,
            ..Default::default()
        };

        let output = exporter.export(&meeting, &options).unwrap();
        assert!(output.contains("[00:00]"));
        assert!(output.contains("[00:05]"));
    }

    #[test]
    fn test_text_export_with_metadata() {
        let meeting = create_test_meeting();
        let exporter = TextExporter;
        let options = ExportOptions {
            include_metadata: true,
            ..Default::default()
        };

        let output = exporter.export(&meeting, &options).unwrap();
        assert!(output.contains("Test Meeting"));
        assert!(output.contains("Date:"));
    }

    #[test]
    fn test_wrap_text() {
        let text = "This is a long line that should be wrapped at a certain width.";
        let wrapped = wrap_text(text, 20);
        for line in wrapped.lines() {
            assert!(line.len() <= 20 || !line.contains(' '));
        }
    }

    #[test]
    fn test_wrap_text_no_wrap() {
        let text = "Short";
        assert_eq!(wrap_text(text, 80), "Short");
    }
}
