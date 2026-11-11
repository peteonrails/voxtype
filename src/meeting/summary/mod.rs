//! AI-powered meeting summarization
//!
//! Generates summaries, action items, key decisions, and other
//! structured insights from meeting transcripts.
//!
//! # Backends
//!
//! - **Local**: Uses Ollama for local LLM inference
//! - **Remote**: Uses a remote API endpoint for summarization
//! - **Disabled**: Summarization disabled

pub mod local;
pub mod remote;

use crate::meeting::data::{ActionItem, Chapter, MeetingData, MeetingSummary};
use chrono::Utc;
use serde::Deserialize;
use thiserror::Error;

/// Summary-related errors
#[derive(Error, Debug)]
pub enum SummaryError {
    #[error("Summarizer not configured")]
    NotConfigured,

    #[error("LLM request failed: {0}")]
    Request(String),

    #[error("Failed to parse LLM response: {0}")]
    Parse(String),

    #[error("Transcript is empty")]
    EmptyTranscript,

    #[error("Ollama not available at {0}")]
    OllamaUnavailable(String),
}

/// Format a MeetingSummary as markdown
pub fn summary_to_markdown(summary: &MeetingSummary) -> String {
    let mut output = String::new();

    if !summary.summary.is_empty() {
        output.push_str("## Summary\n\n");
        output.push_str(&summary.summary);
        output.push_str("\n\n");
    }

    if !summary.key_points.is_empty() {
        output.push_str("## Key Points\n\n");
        for point in &summary.key_points {
            output.push_str(&format!("- {}\n", point));
        }
        output.push('\n');
    }

    if !summary.action_items.is_empty() {
        output.push_str("## Action Items\n\n");
        for item in &summary.action_items {
            let assignee = item
                .assignee
                .as_ref()
                .map(|a| format!(" ({})", a))
                .unwrap_or_default();
            let checkbox = if item.completed { "[x]" } else { "[ ]" };
            output.push_str(&format!(
                "- {} {}{}\n",
                checkbox, item.description, assignee
            ));
        }
        output.push('\n');
    }

    if !summary.decisions.is_empty() {
        output.push_str("## Decisions\n\n");
        for decision in &summary.decisions {
            output.push_str(&format!("- {}\n", decision));
        }
        output.push('\n');
    }

    if !summary.chapters.is_empty() {
        output.push_str("## Chapters\n\n");
        for chapter in &summary.chapters {
            output.push_str(&format!(
                "- [{}] {}\n",
                crate::meeting::assistant::prompt::ts(chapter.start_ms),
                chapter.title
            ));
        }
        output.push('\n');
    }

    output
}

/// Summarization configuration
#[derive(Debug, Clone)]
pub struct SummaryConfig {
    /// Backend to use: "local", "remote", or "disabled"
    pub backend: String,

    /// Ollama URL for local backend
    pub ollama_url: String,

    /// Ollama model name
    pub ollama_model: String,

    /// Remote API endpoint
    pub remote_endpoint: Option<String>,

    /// Remote API key
    pub remote_api_key: Option<String>,

    /// Request timeout in seconds
    pub timeout_secs: u64,
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            backend: "disabled".to_string(),
            ollama_url: "http://localhost:11434".to_string(),
            ollama_model: "llama3.2".to_string(),
            remote_endpoint: None,
            remote_api_key: None,
            timeout_secs: 120,
        }
    }
}

/// Trait for summarization backends
pub trait Summarizer: Send + Sync {
    /// Generate a summary from meeting data
    fn summarize(&self, meeting: &MeetingData) -> Result<MeetingSummary, SummaryError>;

    /// Get the backend name
    fn name(&self) -> &'static str;

    /// Check if the backend is available
    fn is_available(&self) -> bool;
}

/// Create a summarizer based on configuration
pub fn create_summarizer(config: &SummaryConfig) -> Option<Box<dyn Summarizer>> {
    match config.backend.as_str() {
        "local" => Some(Box::new(local::OllamaSummarizer::new(config))),
        "remote" => {
            if config.remote_endpoint.is_some() {
                Some(Box::new(remote::RemoteSummarizer::new(config)))
            } else {
                tracing::warn!("Remote summarizer requires remote_endpoint to be set");
                None
            }
        }
        "disabled" | "" => None,
        _ => {
            tracing::warn!("Unknown summarizer backend '{}', disabling", config.backend);
            None
        }
    }
}

/// Generate the prompt for summarization
pub fn generate_prompt(meeting: &MeetingData) -> String {
    let mut prompt = String::from(
        r#"Analyze the following meeting transcript and provide a structured summary.

Format your response as JSON with this structure:
{
  "summary": "2-3 sentence summary of the meeting",
  "key_points": ["point 1", "point 2"],
  "action_items": [{"description": "task description", "assignee": "person or null", "due_date": "date or null"}],
  "decisions": ["decision 1", "decision 2"]
}

"#,
    );

    if let Some(ref title) = meeting.metadata.title {
        prompt.push_str(&format!("Meeting Title: {}\n", title));
    }

    prompt.push_str(&format!(
        "Date: {}\n\n",
        meeting.metadata.started_at.format("%Y-%m-%d %H:%M")
    ));

    prompt.push_str("## Transcript\n\n");

    for segment in &meeting.transcript.segments {
        let speaker = segment.speaker_display();
        if !speaker.is_empty() && speaker != "Unknown" {
            prompt.push_str(&format!("{}: {}\n", speaker, segment.text));
        } else {
            prompt.push_str(&format!("{}\n", segment.text));
        }
    }

    prompt.push_str("\n## End of Transcript\n\nProvide the JSON summary:");

    prompt
}

/// Parse JSON summary from LLM response
pub fn parse_summary_response(
    response: &str,
    model: Option<String>,
) -> Result<MeetingSummary, SummaryError> {
    // Try to extract JSON from the response
    let json_str = extract_json(response).ok_or_else(|| {
        SummaryError::Parse(format!(
            "No valid JSON found in response: {}",
            &response[..response.len().min(200)]
        ))
    })?;

    // Parse the JSON - use intermediate struct to match LLM output
    #[derive(Deserialize)]
    struct RawSummary {
        summary: Option<String>,
        overview: Option<String>, // The assistant prompt asks for "overview"
        key_points: Option<Vec<String>>,
        action_items: Option<Vec<RawActionItem>>,
        decisions: Option<Vec<String>>,
        chapters: Option<Vec<RawChapter>>,
    }

    #[derive(Deserialize)]
    struct RawActionItem {
        description: Option<String>,
        task: Option<String>, // Some LLMs use "task" instead of "description"
        assignee: Option<String>,
        owner: Option<String>, // The assistant prompt asks for "owner"
        due_date: Option<String>,
        due: Option<String>, // Alternative name
    }

    #[derive(Deserialize)]
    struct RawChapter {
        start: Option<serde_json::Value>,
        start_ms: Option<u64>,
        title: Option<String>,
    }

    // "null" as a string is a model's way of saying no owner or no date.
    let present = |v: Option<String>| v.filter(|s| !s.trim().is_empty() && s.trim() != "null");

    let raw: RawSummary =
        serde_json::from_str(json_str).map_err(|e| SummaryError::Parse(e.to_string()))?;

    Ok(MeetingSummary {
        summary: raw.overview.or(raw.summary).unwrap_or_default(),
        key_points: raw.key_points.unwrap_or_default(),
        action_items: raw
            .action_items
            .unwrap_or_default()
            .into_iter()
            .map(|item| ActionItem {
                description: item.description.or(item.task).unwrap_or_default(),
                assignee: present(item.owner.or(item.assignee)),
                due_date: present(item.due_date.or(item.due)),
                completed: false,
            })
            .filter(|item| !item.description.trim().is_empty())
            .collect(),
        decisions: raw.decisions.unwrap_or_default(),
        chapters: raw
            .chapters
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| {
                let start_ms = c.start_ms.or_else(|| match c.start? {
                    serde_json::Value::String(s) => crate::meeting::assistant::prompt::parse_ts(
                        s.trim_matches(|c| c == '[' || c == ']'),
                    ),
                    serde_json::Value::Number(n) => n.as_u64().map(|secs| secs * 1000),
                    _ => None,
                })?;
                Some(Chapter {
                    start_ms,
                    title: c.title.filter(|t| !t.trim().is_empty())?,
                })
            })
            .collect(),
        generated_at: Utc::now(),
        model,
    })
}

/// Extract JSON object from a string that may contain other text
fn extract_json(s: &str) -> Option<&str> {
    // Find the first { and last }
    let start = s.find('{')?;
    let end = s.rfind('}')?;

    if end > start {
        Some(&s[start..=end])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_simple() {
        let input = r#"Here is the summary: {"summary": "Test meeting"}"#;
        let json = extract_json(input).unwrap();
        assert_eq!(json, r#"{"summary": "Test meeting"}"#);
    }

    #[test]
    fn test_extract_json_nested() {
        let input = r#"{"a": {"b": 1}}"#;
        let json = extract_json(input).unwrap();
        assert_eq!(json, input);
    }

    #[test]
    fn test_parse_summary_response() {
        let response = r#"{"summary": "Brief meeting about X", "key_points": ["Point 1"], "action_items": [{"description": "Do thing", "assignee": "Alice"}], "decisions": ["Agreed on Y"]}"#;

        let summary = parse_summary_response(response, None).unwrap();
        assert_eq!(summary.summary, "Brief meeting about X");
        assert_eq!(summary.key_points.len(), 1);
        assert_eq!(summary.action_items.len(), 1);
        assert_eq!(summary.action_items[0].assignee, Some("Alice".to_string()));
    }

    #[test]
    fn test_parse_summary_response_with_task_field() {
        // Some LLMs use "task" instead of "description"
        let response = r#"{"summary": "Meeting summary", "action_items": [{"task": "Do task", "assignee": "Bob"}]}"#;

        let summary = parse_summary_response(response, Some("llama3.2".to_string())).unwrap();
        assert_eq!(summary.action_items[0].description, "Do task");
        assert_eq!(summary.model, Some("llama3.2".to_string()));
    }

    #[test]
    fn test_summary_to_markdown() {
        let summary = MeetingSummary {
            summary: "Test meeting summary".to_string(),
            key_points: vec!["Point 1".to_string(), "Point 2".to_string()],
            action_items: vec![ActionItem {
                description: "Do thing".to_string(),
                assignee: Some("Alice".to_string()),
                due_date: None,
                completed: false,
            }],
            decisions: vec!["Decision 1".to_string()],
            chapters: vec![Chapter {
                start_ms: 65_000,
                title: "Budget".to_string(),
            }],
            generated_at: Utc::now(),
            model: None,
        };

        let md = summary_to_markdown(&summary);
        assert!(md.contains("## Summary"));
        assert!(md.contains("Test meeting summary"));
        assert!(md.contains("## Action Items"));
        assert!(md.contains("[ ] Do thing (Alice)"));
        assert!(md.contains("## Chapters\n\n- [01:05] Budget"));
    }

    #[test]
    fn test_parse_assistant_summary_shape() {
        let response = r#"Sure: {"overview": "We planned the release.",
            "decisions": ["Ship Friday"],
            "action_items": [{"description": "Tag rc", "owner": "Steve", "due": "null"},
                             {"description": "", "owner": null}],
            "chapters": [{"start": "00:41", "title": "Release"}, {"start": "[1:02:03]", "title": "Later"},
                         {"start": "soon", "title": "Bad"}]}"#;
        let summary = parse_summary_response(response, Some("claude".to_string())).unwrap();
        assert_eq!(summary.summary, "We planned the release.");
        assert_eq!(summary.action_items.len(), 1);
        assert_eq!(summary.action_items[0].assignee.as_deref(), Some("Steve"));
        assert_eq!(summary.action_items[0].due_date, None);
        assert_eq!(
            summary.chapters,
            vec![
                Chapter {
                    start_ms: 41_000,
                    title: "Release".into()
                },
                Chapter {
                    start_ms: 3_723_000,
                    title: "Later".into()
                },
            ]
        );
    }

    #[test]
    fn test_default_config() {
        let config = SummaryConfig::default();
        assert_eq!(config.backend, "disabled");
        assert_eq!(config.ollama_url, "http://localhost:11434");
        assert_eq!(config.ollama_model, "llama3.2");
    }
}
