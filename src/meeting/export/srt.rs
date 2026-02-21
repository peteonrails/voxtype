//! SRT (SubRip) subtitle export format
//!
//! Generates standard SRT subtitle files with optional speaker labels.

use super::{ExportError, ExportFormat, ExportOptions, Exporter};
use crate::meeting::data::MeetingData;

/// SRT exporter
pub struct SrtExporter;

impl Exporter for SrtExporter {
    fn export(
        &self,
        meeting: &MeetingData,
        options: &ExportOptions,
    ) -> Result<String, ExportError> {
        let mut output = String::new();
        let mut index = 1;

        for segment in &meeting.transcript.segments {
            // Sequence number
            output.push_str(&format!("{}\n", index));

            // Timestamps: 00:00:00,000 --> 00:00:00,000
            let start = format_srt_time(segment.start_ms);
            let end = format_srt_time(segment.end_ms);
            output.push_str(&format!("{} --> {}\n", start, end));

            // Text with optional speaker
            if options.include_speakers {
                let speaker = segment.speaker_display();
                if !speaker.is_empty() && speaker != "Unknown" {
                    output.push_str(&format!("[{}] {}\n", speaker, segment.text));
                } else {
                    output.push_str(&format!("{}\n", segment.text));
                }
            } else {
                output.push_str(&format!("{}\n", segment.text));
            }

            // Blank line between entries
            output.push('\n');
            index += 1;
        }

        Ok(output)
    }

    fn format(&self) -> ExportFormat {
        ExportFormat::Srt
    }
}

/// Format milliseconds as SRT timestamp (HH:MM:SS,mmm)
fn format_srt_time(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    let millis = ms % 1000;

    format!("{:02}:{:02}:{:02},{:03}", hours, minutes, seconds, millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_srt_time() {
        assert_eq!(format_srt_time(0), "00:00:00,000");
        assert_eq!(format_srt_time(1500), "00:00:01,500");
        assert_eq!(format_srt_time(65000), "00:01:05,000");
        assert_eq!(format_srt_time(3661500), "01:01:01,500");
    }

    #[test]
    fn test_srt_export() {
        use crate::meeting::data::{AudioSource, MeetingData, TranscriptSegment};

        let mut meeting = MeetingData::new(Some("Test".to_string()));
        meeting.transcript.add_segment(TranscriptSegment::new(
            1,
            0,
            2000,
            "Hello world".to_string(),
            0,
        ));
        meeting.transcript.segments[0].source = AudioSource::Microphone;

        meeting.transcript.add_segment(TranscriptSegment::new(
            2,
            2500,
            5000,
            "How are you".to_string(),
            0,
        ));
        meeting.transcript.segments[1].source = AudioSource::Loopback;

        let exporter = SrtExporter;
        let options = ExportOptions {
            include_speakers: true,
            ..Default::default()
        };
        let output = exporter.export(&meeting, &options).unwrap();

        assert!(output.contains("1\n"));
        assert!(output.contains("00:00:00,000 --> 00:00:02,000"));
        assert!(output.contains("[You] Hello world"));
        assert!(output.contains("[Remote] How are you"));
    }
}
