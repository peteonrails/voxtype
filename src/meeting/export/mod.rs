//! Export functionality for meeting transcriptions
//!
//! Provides exporters for various output formats.

pub mod json;
pub mod markdown;
pub mod srt;
pub mod txt;
pub mod vtt;

use crate::meeting::data::MeetingData;
use thiserror::Error;

/// Export format types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Plain text
    Text,
    /// Markdown
    Markdown,
    /// JSON
    Json,
    /// SRT subtitles (Phase 2)
    Srt,
    /// VTT subtitles (Phase 2)
    Vtt,
}

impl ExportFormat {
    /// Parse format from string name
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "text" | "txt" => Some(ExportFormat::Text),
            "markdown" | "md" => Some(ExportFormat::Markdown),
            "json" => Some(ExportFormat::Json),
            "srt" => Some(ExportFormat::Srt),
            "vtt" => Some(ExportFormat::Vtt),
            _ => None,
        }
    }

    /// Get file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Text => "txt",
            ExportFormat::Markdown => "md",
            ExportFormat::Json => "json",
            ExportFormat::Srt => "srt",
            ExportFormat::Vtt => "vtt",
        }
    }

    /// Get all supported format names
    pub fn all_names() -> &'static [&'static str] {
        &["text", "txt", "markdown", "md", "json", "srt", "vtt"]
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportFormat::Text => write!(f, "text"),
            ExportFormat::Markdown => write!(f, "markdown"),
            ExportFormat::Json => write!(f, "json"),
            ExportFormat::Srt => write!(f, "srt"),
            ExportFormat::Vtt => write!(f, "vtt"),
        }
    }
}

/// Export errors
#[derive(Error, Debug)]
pub enum ExportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Format not supported: {0}")]
    UnsupportedFormat(String),
}

/// One line describing an annotation: "Note: text", "Highlight: text", or
/// "Highlight" alone.
pub fn annotation_line(annotation: &crate::meeting::data::SegmentAnnotation) -> String {
    match (annotation.highlight, annotation.text.is_empty()) {
        (true, true) => "Highlight".to_string(),
        (true, false) => format!("Highlight: {}", annotation.text),
        (false, _) => format!("Note: {}", annotation.text),
    }
}

/// Export options
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Include timestamps
    pub include_timestamps: bool,
    /// Include speaker labels
    pub include_speakers: bool,
    /// Include metadata header
    pub include_metadata: bool,
    /// Line width for wrapping (0 = no wrap)
    pub line_width: usize,
}

/// Trait for meeting exporters
pub trait Exporter: Send + Sync {
    /// Export meeting data to a string
    fn export(&self, meeting: &MeetingData, options: &ExportOptions)
        -> Result<String, ExportError>;

    /// Get the format name
    fn format(&self) -> ExportFormat;
}

/// Export meeting data to string in the specified format
pub fn export_meeting(
    meeting: &MeetingData,
    format: ExportFormat,
    options: &ExportOptions,
) -> Result<String, ExportError> {
    let exporter: Box<dyn Exporter> = match format {
        ExportFormat::Text => Box::new(txt::TextExporter),
        ExportFormat::Markdown => Box::new(markdown::MarkdownExporter),
        ExportFormat::Json => Box::new(json::JsonExporter),
        ExportFormat::Srt => Box::new(srt::SrtExporter),
        ExportFormat::Vtt => Box::new(vtt::VttExporter),
    };

    exporter.export(meeting, options)
}

/// Export meeting data to a file
pub fn export_meeting_to_file(
    meeting: &MeetingData,
    format: ExportFormat,
    options: &ExportOptions,
    path: &std::path::Path,
) -> Result<(), ExportError> {
    let content = export_meeting(meeting, format, options)?;
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::{AudioSource, TranscriptSegment};

    /// A conversation as the daemon records it: per chunk, the microphone
    /// track's segments arrive before the loopback track's.
    fn conversation() -> MeetingData {
        let mut meeting = MeetingData::new(Some("Turns".to_string()));
        let lines = [
            (0, 2_000, AudioSource::Microphone, "Can you hear me?"),
            (1, 14_000, AudioSource::Microphone, "Great, let's start."),
            (2, 6_000, AudioSource::Loopback, "Yes, loud and clear."),
            (3, 21_000, AudioSource::Loopback, "Sounds good."),
        ];
        for (id, start, source, text) in lines {
            let mut seg = TranscriptSegment::new(id, start, start + 3_000, text.to_string(), 0);
            seg.source = source;
            meeting.transcript.add_segment(seg);
        }
        meeting
    }

    #[test]
    fn exports_carry_notes_on_top_and_annotations_under_their_line() {
        let mut meeting = conversation();
        meeting.notes = Some("Budget review\n- travel is high\n".to_string());
        meeting.transcript.segments[1].annotation = Some(crate::meeting::SegmentAnnotation {
            text: "follow up".to_string(),
            highlight: true,
        });
        meeting.transcript.segments[2].annotation = Some(crate::meeting::SegmentAnnotation {
            text: "check".to_string(),
            highlight: false,
        });
        let options = ExportOptions {
            include_speakers: true,
            ..Default::default()
        };

        let md = export_meeting(&meeting, ExportFormat::Markdown, &options).unwrap();
        assert!(
            md.contains("## Notes\n\nBudget review\n- travel is high\n\n"),
            "{md}"
        );
        assert!(md.find("## Notes").unwrap() < md.find("## Transcript").unwrap());
        assert!(
            md.contains("Yes, loud and clear.\n\n> **Highlight: follow up**\n"),
            "{md}"
        );
        assert!(
            md.contains("Great, let's start.\n\n> Note: check\n"),
            "{md}"
        );

        let txt = export_meeting(&meeting, ExportFormat::Text, &options).unwrap();
        assert!(txt.starts_with("Notes:\nBudget review"), "{txt}");
        assert!(
            txt.contains("Yes, loud and clear.\n  > Highlight: follow up\n"),
            "{txt}"
        );

        let json: serde_json::Value =
            serde_json::from_str(&export_meeting(&meeting, ExportFormat::Json, &options).unwrap())
                .unwrap();
        assert_eq!(json["notes"], "Budget review\n- travel is high\n");
        assert_eq!(
            json["transcript"]["segments"][1]["annotation"]["highlight"],
            true
        );
        assert!(json["transcript"]["segments"][0]
            .get("annotation")
            .is_none());
    }

    /// Bug: exports listed all of You, then all of Remote, per chunk.
    #[test]
    fn every_format_keeps_turn_order_and_per_line_timestamps() {
        let meeting = conversation();
        let options = ExportOptions {
            include_timestamps: true,
            include_speakers: true,
            ..Default::default()
        };
        let in_order = [
            "Can you hear me?",
            "loud and clear",
            "let's start",
            "Sounds good",
        ];

        for format in [
            ExportFormat::Text,
            ExportFormat::Markdown,
            ExportFormat::Json,
            ExportFormat::Srt,
            ExportFormat::Vtt,
        ] {
            let out = export_meeting(&meeting, format, &options).unwrap();
            let positions: Vec<usize> = in_order
                .iter()
                .map(|line| {
                    out.find(line)
                        .unwrap_or_else(|| panic!("{format}: {line} missing"))
                })
                .collect();
            assert!(
                positions.windows(2).all(|w| w[0] < w[1]),
                "{format} lost turn order:\n{out}"
            );
        }

        let text = export_meeting(&meeting, ExportFormat::Text, &options).unwrap();
        for stamp in ["[00:02]", "[00:06]", "[00:14]", "[00:21]"] {
            assert!(text.contains(stamp), "text export missing {stamp}:\n{text}");
        }
        let md = export_meeting(&meeting, ExportFormat::Markdown, &options).unwrap();
        for stamp in ["*[00:02]*", "*[00:06]*", "*[00:14]*", "*[00:21]*"] {
            assert!(md.contains(stamp), "markdown export missing {stamp}:\n{md}");
        }
        let srt = export_meeting(&meeting, ExportFormat::Srt, &options).unwrap();
        assert!(srt.contains("00:00:06,000 --> 00:00:09,000\n[Remote] Yes, loud and clear."));
        let vtt = export_meeting(&meeting, ExportFormat::Vtt, &options).unwrap();
        assert!(vtt.contains("00:00:14.000 --> 00:00:17.000\n<v You>Great, let's start."));
    }

    #[test]
    fn test_format_from_str() {
        assert_eq!(ExportFormat::parse("text"), Some(ExportFormat::Text));
        assert_eq!(ExportFormat::parse("txt"), Some(ExportFormat::Text));
        assert_eq!(
            ExportFormat::parse("markdown"),
            Some(ExportFormat::Markdown)
        );
        assert_eq!(ExportFormat::parse("md"), Some(ExportFormat::Markdown));
        assert_eq!(ExportFormat::parse("json"), Some(ExportFormat::Json));
        assert_eq!(ExportFormat::parse("invalid"), None);
    }

    #[test]
    fn test_format_extension() {
        assert_eq!(ExportFormat::Text.extension(), "txt");
        assert_eq!(ExportFormat::Markdown.extension(), "md");
        assert_eq!(ExportFormat::Json.extension(), "json");
        assert_eq!(ExportFormat::Srt.extension(), "srt");
        assert_eq!(ExportFormat::Vtt.extension(), "vtt");
    }

    #[test]
    fn test_format_display() {
        assert_eq!(ExportFormat::Text.to_string(), "text");
        assert_eq!(ExportFormat::Markdown.to_string(), "markdown");
        assert_eq!(ExportFormat::Json.to_string(), "json");
        assert_eq!(ExportFormat::Srt.to_string(), "srt");
        assert_eq!(ExportFormat::Vtt.to_string(), "vtt");
    }

    #[test]
    fn test_format_from_str_case_insensitive() {
        assert_eq!(ExportFormat::parse("TEXT"), Some(ExportFormat::Text));
        assert_eq!(
            ExportFormat::parse("Markdown"),
            Some(ExportFormat::Markdown)
        );
        assert_eq!(ExportFormat::parse("JSON"), Some(ExportFormat::Json));
        assert_eq!(ExportFormat::parse("SRT"), Some(ExportFormat::Srt));
        assert_eq!(ExportFormat::parse("VTT"), Some(ExportFormat::Vtt));
    }

    #[test]
    fn test_all_names() {
        let names = ExportFormat::all_names();
        assert!(names.contains(&"text"));
        assert!(names.contains(&"txt"));
        assert!(names.contains(&"markdown"));
        assert!(names.contains(&"md"));
        assert!(names.contains(&"json"));
        assert!(names.contains(&"srt"));
        assert!(names.contains(&"vtt"));
    }

    #[test]
    fn test_export_meeting_text() {
        use crate::meeting::data::{MeetingData, TranscriptSegment};

        let mut meeting = MeetingData::new(Some("Test".to_string()));
        meeting
            .transcript
            .add_segment(TranscriptSegment::new(0, 0, 1000, "Hello".to_string(), 0));

        let result = export_meeting(&meeting, ExportFormat::Text, &ExportOptions::default());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("Hello"));
    }

    #[test]
    fn test_export_meeting_srt() {
        use crate::meeting::data::MeetingData;

        let meeting = MeetingData::new(Some("Test".to_string()));
        let result = export_meeting(&meeting, ExportFormat::Srt, &ExportOptions::default());
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_meeting_vtt() {
        use crate::meeting::data::MeetingData;

        let meeting = MeetingData::new(Some("Test".to_string()));
        let result = export_meeting(&meeting, ExportFormat::Vtt, &ExportOptions::default());
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_options_default() {
        let opts = ExportOptions::default();
        assert!(!opts.include_timestamps);
        assert!(!opts.include_speakers);
        assert!(!opts.include_metadata);
        assert_eq!(opts.line_width, 0);
    }
}
