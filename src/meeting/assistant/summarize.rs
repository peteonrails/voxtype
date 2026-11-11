//! Meeting summaries through the assistant: `meeting summarize` and the
//! automatic summary when a meeting stops.
//!
//! The result is a [`MeetingSummary`] (overview, key points, decisions,
//! action items with owners as speaker names, chapters with timestamps),
//! stored in SQLite and metadata.json, and written as `summary.md` in the
//! meeting folder.

use super::{
    build_context, choose_backend, meeting_folder, AssistantError, AssistantSettings, Backend,
    Overrides,
};
use crate::meeting::data::{MeetingData, MeetingId, MeetingSummary};
use crate::meeting::storage::{MeetingStorage, StorageConfig};
use crate::meeting::summary::{self, parse_summary_response, summary_to_markdown};
use serde::Serialize;
use std::path::PathBuf;

pub const SUMMARY_FILE: &str = "summary.md";

/// A summary and where it came from.
#[derive(Debug, Clone, Serialize)]
pub struct SummaryOutcome {
    pub meeting_id: String,
    /// `claude`, `codex`, `local:<model>`, `ollama` or `remote`.
    pub agent: String,
    pub agent_name: String,
    /// `summary.md`, when the meeting has a folder.
    pub path: Option<PathBuf>,
    pub summary: MeetingSummary,
}

/// Does `[meeting.summary] backend` name one of the older summarizers?
fn legacy_backend(settings: &AssistantSettings) -> bool {
    matches!(settings.summary_backend.as_str(), "local" | "remote")
}

/// Summarize `meeting` with the assistant `backend`.
pub fn summarize_with(
    settings: &AssistantSettings,
    meeting: &MeetingData,
    backend: &Backend,
) -> Result<MeetingSummary, AssistantError> {
    if meeting.transcript.segments.is_empty() {
        return Err(AssistantError::EmptyTranscript);
    }
    let ctx = build_context(settings, meeting, None, backend)?;
    let response = backend.complete(&ctx.summary(), true, settings.timeout, &mut |_| {})?;
    parse_summary_response(&response, Some(backend.id()))
        .map_err(|e| AssistantError::Parse(e.to_string()))
}

/// Summarize the meeting `id` (or `latest`), store it and write summary.md.
pub fn summarize_meeting(
    settings: &AssistantSettings,
    storage_config: &StorageConfig,
    id: &str,
    overrides: &Overrides,
) -> Result<SummaryOutcome, AssistantError> {
    let storage = MeetingStorage::open(storage_config.clone())
        .map_err(|e| AssistantError::Storage(format!("Failed to open storage: {e}")))?;
    let meeting_id = storage
        .resolve_meeting_id(id)
        .map_err(|e| AssistantError::Storage(format!("Meeting not found: {e}")))?;
    let meeting = storage
        .load_meeting_data(&meeting_id)
        .map_err(|e| AssistantError::Storage(format!("Failed to load meeting: {e}")))?;

    let (summary, agent, agent_name) =
        if legacy_backend(settings) && overrides.agent.is_none() && !overrides.local {
            let summarizer = summary::create_summarizer(&settings.summary).ok_or_else(|| {
                AssistantError::Local(format!(
                    "[meeting.summary] backend = \"{}\" is not fully configured",
                    settings.summary_backend
                ))
            })?;
            if meeting.transcript.segments.is_empty() {
                return Err(AssistantError::EmptyTranscript);
            }
            let s = summarizer
                .summarize(&meeting)
                .map_err(|e| AssistantError::Local(e.to_string()))?;
            (
                s,
                summarizer.name().to_string(),
                summarizer.name().to_string(),
            )
        } else {
            let backend = choose_backend(settings, overrides)?;
            let s = summarize_with(settings, &meeting, &backend)?;
            (s, backend.id(), backend.display_name())
        };

    let path = save(&storage, &meeting, &meeting_id, &summary, &agent_name)?;
    Ok(SummaryOutcome {
        meeting_id: meeting_id.to_string(),
        agent,
        agent_name,
        path,
        summary,
    })
}

/// Store `summary` and write summary.md. Returns the file's path.
fn save(
    storage: &MeetingStorage,
    meeting: &MeetingData,
    meeting_id: &MeetingId,
    summary: &MeetingSummary,
    agent_name: &str,
) -> Result<Option<PathBuf>, AssistantError> {
    storage
        .save_summary(meeting_id, summary)
        .map_err(|e| AssistantError::Storage(format!("Failed to save the summary: {e}")))?;
    let Some(folder) = meeting_folder(meeting) else {
        return Ok(None);
    };
    let path = folder.join(SUMMARY_FILE);
    let md = format!(
        "# {}\n\n_{} · summarized by {} on {}_\n\n{}",
        meeting.metadata.display_title(),
        meeting
            .metadata
            .started_at
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M"),
        agent_name,
        summary
            .generated_at
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M"),
        summary_to_markdown(summary)
    );
    std::fs::write(&path, md)
        .map_err(|e| AssistantError::Storage(format!("Failed to write {}: {e}", path.display())))?;
    Ok(Some(path))
}

/// What the automatic summary reports, for meeting.sock.
#[derive(Debug, Clone)]
pub enum SummaryEvent {
    Summarizing {
        meeting_id: String,
    },
    Ready(Box<SummaryOutcome>),
    Failed {
        meeting_id: String,
        code: &'static str,
        message: String,
    },
}

/// The summary run after a meeting stops.
pub struct AutoSummary {
    pub settings: AssistantSettings,
    pub storage: StorageConfig,
    pub meeting_id: MeetingId,
}

impl AutoSummary {
    /// Summarize if auto-summary is on and the meeting has a transcript.
    /// Runs the work off the async runtime. A failure is reported through
    /// `emit` and the log, and never touches the transcript. Returns
    /// whether a summary was attempted.
    pub async fn run(self, emit: impl Fn(SummaryEvent) + Send + Sync + 'static) -> bool {
        let id = self.meeting_id.to_string();
        let settings = self.settings.clone();
        let storage = self.storage.clone();
        let check_id = id.clone();
        let wanted = tokio::task::spawn_blocking(move || {
            if !settings.auto_summary() {
                return false;
            }
            MeetingStorage::open(storage)
                .ok()
                .and_then(|s| {
                    let mid = s.resolve_meeting_id(&check_id).ok()?;
                    s.load_transcript(&mid).ok()
                })
                .is_some_and(|t| !t.segments.is_empty())
        })
        .await
        .unwrap_or(false);
        if !wanted {
            tracing::debug!("Auto-summary skipped for meeting {}", id);
            return false;
        }

        emit(SummaryEvent::Summarizing {
            meeting_id: id.clone(),
        });
        tracing::info!("Summarizing meeting {}", id);
        let settings = self.settings;
        let storage = self.storage;
        let run_id = id.clone();
        let result = tokio::task::spawn_blocking(move || {
            summarize_meeting(&settings, &storage, &run_id, &Overrides::default())
        })
        .await;
        match result {
            Ok(Ok(outcome)) => {
                tracing::info!("Meeting {} summarized by {}", id, outcome.agent_name);
                emit(SummaryEvent::Ready(Box::new(outcome)));
            }
            Ok(Err(e)) => {
                tracing::warn!("Meeting {} summary failed: {}", id, e);
                emit(SummaryEvent::Failed {
                    meeting_id: id,
                    code: e.code(),
                    message: format!("Summary failed: {e}"),
                });
            }
            Err(e) => {
                tracing::error!("Meeting {} summary task failed: {}", id, e);
                emit(SummaryEvent::Failed {
                    meeting_id: id,
                    code: "internal",
                    message: format!("Summary failed: {e}"),
                });
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Sandbox;
    use super::*;

    const REPLY: &str = r#"{"overview": "Steve and Pete agreed to ship Friday.",
        "key_points": ["Budget is 40k"],
        "decisions": ["Ship on Friday"],
        "action_items": [{"description": "Tag the release", "owner": "Steve", "due": "Friday"}],
        "chapters": [{"start": "00:41", "title": "Release date"}, {"start": "01:00", "title": "Budget"}]}"#;

    #[test]
    fn summarize_saves_summary_md_metadata_and_index() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&[], REPLY);
        let m = sb.meeting(
            "Release sync",
            &[
                (41, "Remote", "We ship on Friday."),
                (60, "You", "Budget is 40k."),
            ],
        );
        sb.label(&m, "Remote", "Steve");
        sb.label(&m, "You", "Pete");

        let outcome = summarize_meeting(
            &sb.settings(),
            &sb.storage_config(),
            "latest",
            &Overrides::default(),
        )
        .unwrap();
        assert_eq!(outcome.agent, "claude");
        assert_eq!(outcome.agent_name, "Claude");
        let s = &outcome.summary;
        assert_eq!(s.summary, "Steve and Pete agreed to ship Friday.");
        assert_eq!(s.action_items[0].assignee.as_deref(), Some("Steve"));
        assert_eq!(s.chapters.len(), 2);
        assert_eq!(s.model.as_deref(), Some("claude"));

        let prompt = sb.claude_stdin(0);
        assert!(prompt.contains("[00:41] Steve: We ship on Friday."));
        assert!(prompt.contains("Reply with only a JSON object"));

        let md = std::fs::read_to_string(outcome.path.unwrap()).unwrap();
        assert!(md.starts_with("# Release sync\n"));
        assert!(md.contains("summarized by Claude"));
        assert!(md.contains("## Chapters\n\n- [00:41] Release date\n- [01:00] Budget"));
        assert!(md.contains("[ ] Tag the release (Steve)"));

        let reloaded = sb.reload(&m);
        assert_eq!(
            reloaded.metadata.summary.as_ref().unwrap().chapters.len(),
            2
        );
        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(sb.folder(&m).join("metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(meta["summary"]["chapters"][1]["title"], "Budget");

        // A later metadata write that predates the summary keeps it.
        let storage = MeetingStorage::open(sb.storage_config()).unwrap();
        let mut stale = m.metadata.clone();
        stale.summary = None;
        storage.update_meeting(&stale).unwrap();
        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(sb.folder(&m).join("metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            meta["summary"]["summary"],
            "Steve and Pete agreed to ship Friday."
        );
    }

    #[test]
    fn an_unparseable_summary_is_an_error_and_saves_nothing() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&[], "I cannot help with that.");
        let m = sb.meeting("X", &[(1, "You", "hello")]);
        let err = summarize_meeting(
            &sb.settings(),
            &sb.storage_config(),
            "latest",
            &Overrides::default(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "parse");
        assert!(sb.reload(&m).metadata.summary.is_none());
        assert!(!sb.folder(&m).join(SUMMARY_FILE).exists());
    }

    #[test]
    fn an_empty_meeting_is_not_summarized() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.meeting("Empty", &[]);
        let err = summarize_meeting(
            &sb.settings(),
            &sb.storage_config(),
            "latest",
            &Overrides::default(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "empty_transcript");
        assert_eq!(sb.claude_calls(), 0);
    }
}
