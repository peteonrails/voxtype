//! WebVTT subtitle export format
//!
//! Generates WebVTT subtitle files with optional speaker labels and styling.

use super::{ExportError, ExportFormat, ExportOptions, Exporter};
use crate::meeting::data::MeetingData;

/// VTT exporter
pub struct VttExporter;

impl Exporter for VttExporter {
    fn export(
        &self,
        meeting: &MeetingData,
        options: &ExportOptions,
    ) -> Result<String, ExportError> {
        let mut output = String::from("WEBVTT\n");

        // Add metadata if requested
        if options.include_metadata {
            output.push_str(&format!(
                "NOTE\nMeeting: {}\nDate: {}\n",
                meeting.metadata.display_title(),
                meeting.metadata.started_at.format("%Y-%m-%d %H:%M:%S")
            ));
            if let Some(duration) = meeting.metadata.duration_secs {
                output.push_str(&format!("Duration: {}s\n", duration));
            }
            output.push('\n');
        } else {
            output.push('\n');
        }

        for (i, segment) in meeting.transcript.segments.iter().enumerate() {
            // Optional cue identifier
            output.push_str(&format!("cue-{}\n", i + 1));

            // Timestamps: 00:00:00.000 --> 00:00:00.000
            let start = format_vtt_time(segment.start_ms);
            let end = format_vtt_time(segment.end_ms);
            output.push_str(&format!("{} --> {}\n", start, end));

            // Text with optional speaker (VTT supports <v> voice spans)
            if options.include_speakers {
                let speaker = segment.speaker_display();
                if !speaker.is_empty() && speaker != "Unknown" {
                    output.push_str(&format!("<v {}>{}\n", speaker, segment.text));
                } else {
                    output.push_str(&format!("{}\n", segment.text));
                }
            } else {
                output.push_str(&format!("{}\n", segment.text));
            }

            // Blank line between cues
            output.push('\n');
        }

        Ok(output)
    }

    fn format(&self) -> ExportFormat {
        ExportFormat::Vtt
    }
}

/// Format milliseconds as VTT timestamp (HH:MM:SS.mmm)
fn format_vtt_time(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    let millis = ms % 1000;

    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_vtt_time() {
        assert_eq!(format_vtt_time(0), "00:00:00.000");
        assert_eq!(format_vtt_time(1500), "00:00:01.500");
        assert_eq!(format_vtt_time(65000), "00:01:05.000");
        assert_eq!(format_vtt_time(3661500), "01:01:01.500");
    }

    #[test]
    fn test_vtt_export_header() {
        use crate::meeting::data::MeetingData;

        let meeting = MeetingData::new(Some("Test".to_string()));
        let exporter = VttExporter;
        let options = ExportOptions::default();
        let output = exporter.export(&meeting, &options).unwrap();

        assert!(output.starts_with("WEBVTT\n"));
    }

    #[test]
    fn test_vtt_export_with_speakers() {
        use crate::meeting::data::{MeetingData, TranscriptSegment};

        let mut meeting = MeetingData::new(Some("Test".to_string()));
        let mut seg = TranscriptSegment::new(1, 0, 2000, "Hello world".to_string(), 0);
        seg.speaker_label = Some("Alice".to_string());
        meeting.transcript.add_segment(seg);

        let exporter = VttExporter;
        let options = ExportOptions {
            include_speakers: true,
            ..Default::default()
        };
        let output = exporter.export(&meeting, &options).unwrap();

        assert!(output.contains("<v Alice>Hello world"));
    }

    #[test]
    fn test_vtt_export_with_metadata() {
        use crate::meeting::data::MeetingData;

        let meeting = MeetingData::new(Some("Important Meeting".to_string()));
        let exporter = VttExporter;
        let options = ExportOptions {
            include_metadata: true,
            ..Default::default()
        };
        let output = exporter.export(&meeting, &options).unwrap();

        assert!(output.contains("NOTE"));
        assert!(output.contains("Meeting: Important Meeting"));
    }
}
