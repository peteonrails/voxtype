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
