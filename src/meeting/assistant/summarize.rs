//! Meeting summaries through the assistant: `meeting summarize` and the
//! automatic summary when a meeting stops.
//!
//! The result is a [`MeetingSummary`] (overview, key points, decisions,
//! action items with owners as speaker names, chapters with timestamps),
//! stored in SQLite and metadata.json, and written as `summary.md` in the
//! meeting folder.

use super::{
    build_context, choose_summary_backend, meeting_folder, summary_backend_without_llm,
    AssistantError, AssistantSettings, Backend, Overrides, SummaryBackend,
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
    /// The agent that failed first, when the summary came from the local
    /// model instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fell_back_from: Option<String>,
}

/// Run one of the older `[meeting.summary]` summarizers.
fn summarize_legacy(
    settings: &AssistantSettings,
    meeting: &MeetingData,
    kind: &str,
) -> Result<MeetingSummary, AssistantError> {
    let mut config = settings.summary.clone();
    config.backend = kind.to_string();
    let summarizer = summary::create_summarizer(&config).ok_or_else(|| {
        AssistantError::Local(format!(
            "[meeting.summary] backend = \"{kind}\" is not fully configured (remote_endpoint is missing)"
        ))
    })?;
    if meeting.transcript.segments.is_empty() {
        return Err(AssistantError::EmptyTranscript);
    }
    summarizer
        .summarize(meeting)
        .map_err(|e| AssistantError::Local(e.to_string()))
}

/// `error` as a log line at the default level may show it. An answer that
/// could not be read is quoted in its error, and the answer is the meeting
/// in summary: the log says only that it could not be read. The whole error
/// still reaches the meeting view and the command line, and debug logging.
fn error_for_log(error: &AssistantError) -> String {
    let message = error.to_string();
    if matches!(error, AssistantError::Parse(_)) || message.contains(summary::PARSE_FAILED) {
        "the model's answer could not be read (debug logging shows it)".to_string()
    } else {
        message
    }
}

/// Summarize with `backend`. An agent that fails may hand over to the local
/// model, but only when `[meeting.summary] backend = "local"` says the user
/// set it up, and never the other way: the local model failing never sends
/// the transcript to an agent.
///
/// The chosen `omarchy` or `openai` model failing (stopped mid-request, a
/// refused key) hands over to what the summary would have used without it,
/// the same as when it was not running to begin with.
fn run_summary(
    settings: &AssistantSettings,
    meeting: &MeetingData,
    backend: &SummaryBackend,
    overrides: &Overrides,
) -> Result<(MeetingSummary, SummaryBackend, Option<String>), AssistantError> {
    if let SummaryBackend::Assistant(b @ Backend::Llm(_)) = backend {
        return match summarize_with(settings, meeting, b) {
            Ok(s) => Ok((s, backend.clone(), None)),
            Err(e @ (AssistantError::EmptyTranscript | AssistantError::Storage(_))) => Err(e),
            Err(e) => {
                tracing::warn!("{}; summarizing the usual way instead", error_for_log(&e));
                let fallback = summary_backend_without_llm(settings, overrides)?;
                run_summary(settings, meeting, &fallback, overrides)
                    .map(|(s, used, _)| (s, used, Some(b.display_name())))
            }
        };
    }
    match backend {
        SummaryBackend::Legacy(kind) => {
            summarize_legacy(settings, meeting, kind).map(|s| (s, backend.clone(), None))
        }
        SummaryBackend::Assistant(b) => match summarize_with(settings, meeting, b) {
            Ok(s) => Ok((s, backend.clone(), None)),
            Err(e @ (AssistantError::Agent { .. } | AssistantError::Timeout { .. }))
                if matches!(b, Backend::Agent(_)) && settings.summary_backend == "local" =>
            {
                let local = SummaryBackend::Legacy("local".to_string());
                tracing::warn!(
                    "{e}; summarizing with {} instead",
                    local.display_name(settings)
                );
                match summarize_legacy(settings, meeting, "local") {
                    Ok(s) => Ok((s, local, Some(b.display_name()))),
                    Err(local_err) => {
                        tracing::debug!("The local model's error in full: {local_err}");
                        tracing::warn!("The local model failed too: {}", error_for_log(&local_err));
                        Err(e)
                    }
                }
            }
            Err(e) => Err(e),
        },
    }
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

    let backend = choose_summary_backend(settings, overrides)?;
    let (mut summary, used, fell_back_from) = run_summary(settings, &meeting, &backend, overrides)?;
    crate::meeting::actions::resolve_dues(&mut summary, &meeting.metadata);
    let agent_name = used.display_name(settings);

    let path = save(&storage, &meeting, &meeting_id, &summary, &agent_name)?;
    Ok(SummaryOutcome {
        meeting_id: meeting_id.to_string(),
        agent: used.id(),
        agent_name,
        path,
        summary,
        fell_back_from,
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
        /// The backend that failed, as the UI names it ("Claude",
        /// "Local (llama3:latest)"), when one was chosen.
        backend: Option<String>,
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
        let backend = choose_summary_backend(&settings, &Overrides::default())
            .ok()
            .map(|b| b.display_name(&settings));
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
                tracing::warn!("Meeting {} summary failed: {}", id, error_for_log(&e));
                tracing::debug!("Meeting {} summary error in full: {}", id, e);
                emit(SummaryEvent::Failed {
                    meeting_id: id,
                    code: e.code(),
                    message: format!("Summary failed: {e}"),
                    backend,
                });
            }
            Err(e) => {
                tracing::error!("Meeting {} summary task failed: {}", id, e);
                emit(SummaryEvent::Failed {
                    meeting_id: id,
                    code: "internal",
                    message: format!("Summary failed: {e}"),
                    backend,
                });
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{fake_ollama, Sandbox};
    use super::*;

    const REPLY: &str = r#"{"overview": "Steve and Pete agreed to ship Friday.",
        "key_points": ["Budget is 40k"],
        "decisions": ["Ship on Friday"],
        "action_items": [{"description": "Tag the release", "owner": "Steve", "due": "Friday"}],
        "chapters": [{"start": "00:41", "title": "Release date"}, {"start": "01:00", "title": "Budget"}]}"#;

    /// A summary that cannot be parsed is quoted in its error, by either
    /// backend and through the "no backend" wrapper. The log line at the
    /// default level drops the quote and keeps every other failure as it is.
    #[test]
    fn the_log_line_for_an_unreadable_summary_leaves_the_answer_out() {
        let answer = "They agreed to move the launch to March";
        let parse = parse_summary_response(answer, None).unwrap_err();
        assert!(parse.to_string().contains(answer), "{parse}");

        let quoting = [
            AssistantError::Parse(parse.to_string()),
            AssistantError::Local(parse.to_string()),
            AssistantError::NoBackend {
                message: parse.to_string(),
                model: "llama3".into(),
            },
        ];
        for error in quoting {
            assert!(error.to_string().contains(answer), "{error}");
            let line = error_for_log(&error);
            assert!(!line.contains("launch"), "{line}");
            assert!(line.contains("could not be read"), "{line}");
        }

        let other = AssistantError::Local("Ollama not available at localhost:11434".into());
        assert_eq!(error_for_log(&other), other.to_string());
    }

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

    /// A summary as Ollama's /api/generate returns it, whole or streamed.
    fn ollama_reply() -> String {
        serde_json::json!({"response": REPLY, "done": true}).to_string()
    }

    fn summarize(
        settings: &AssistantSettings,
        sb: &Sandbox,
    ) -> Result<SummaryOutcome, AssistantError> {
        summarize_meeting(
            settings,
            &sb.storage_config(),
            "latest",
            &Overrides::default(),
        )
    }

    /// Pete's 1.3.0 config: `[meeting.summary] backend = "local"` from
    /// before the assistant, and Claude allowed since. Claude summarizes,
    /// as `assistant status` says; Ollama is never called (its URL is a
    /// closed port, so calling it would fail the summary).
    #[test]
    fn legacy_local_with_an_allowed_agent_uses_the_agent() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&[], REPLY);
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);
        let mut settings = sb.settings();
        settings.summary_backend = "local".into();

        let outcome = summarize(&settings, &sb).unwrap();
        assert_eq!(
            (outcome.agent.as_str(), outcome.agent_name.as_str()),
            ("claude", "Claude")
        );
        assert_eq!(sb.claude_calls(), 1);

        let status = super::super::status_json(&settings);
        assert_eq!(status["backend_name"], "Claude");
        assert_eq!(status["summary_backend"], "agent");
        assert_eq!(status["summary_backend_name"], "Claude");
        assert!(status["summary_note"].is_null());
        assert!(matches!(
            super::super::choose_backend(&settings, &Overrides::default()),
            Ok(Backend::Agent(_))
        ));
    }

    #[test]
    fn local_only_uses_the_local_model_even_with_an_allowed_agent() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);
        let (url, h) = fake_ollama("200 OK", ollama_reply());
        let mut settings = sb.settings();
        settings.summary_backend = "local".into();
        settings.local_only = true;
        settings.summary.ollama_url = url;

        let outcome = summarize(&settings, &sb).unwrap();
        assert_eq!(outcome.agent, "local:llama3.2");
        assert_eq!(outcome.agent_name, "Local (llama3.2)");
        assert_eq!(sb.claude_calls(), 0);
        assert!(h.join().unwrap().contains("/api/generate"));

        let status = super::super::status_json(&settings);
        assert_eq!(status["summary_backend"], "local");
        assert_eq!(status["summary_backend_name"], "Local (llama3.2)");
        assert_eq!(status["backend_name"], "Local (llama3.2)");
    }

    /// Someone who set up Ollama summaries and never allowed an agent keeps
    /// them, for summaries and now for questions too, instead of being asked
    /// to send transcripts to an agent they did not pick.
    #[test]
    fn legacy_local_without_consent_stays_local() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);
        let (url, h) = fake_ollama("200 OK", ollama_reply());
        let mut settings = sb.settings();
        settings.summary_backend = "local".into();
        settings.summary.ollama_url = url;

        let outcome = summarize(&settings, &sb).unwrap();
        assert_eq!(outcome.agent, "ollama");
        assert_eq!(outcome.agent_name, "Local (llama3.2)");
        assert_eq!(
            outcome.summary.summary,
            "Steve and Pete agreed to ship Friday."
        );
        assert_eq!(sb.claude_calls(), 0);
        h.join().unwrap();
        assert!(matches!(
            super::super::choose_backend(&settings, &Overrides::default()),
            Ok(Backend::Local {
                fallback: false,
                ..
            })
        ));
        let status = super::super::status_json(&settings);
        assert_eq!(status["summary_backend_name"], "Local (llama3.2)");
        assert_eq!(status["backend_name"], "Local (llama3.2)");
    }

    /// The failure Pete hit, now in plain words with Ollama's own reason.
    #[test]
    fn an_ollama_failure_says_which_backend_and_why() {
        let sb = Sandbox::new();
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);
        let (url, _h) = fake_ollama(
            "500 Internal Server Error",
            r#"{"error":"mkdir /opt/models/manifests-v2: permission denied"}"#.to_string(),
        );
        let host = url.trim_start_matches("http://").to_string();
        let mut settings = sb.settings();
        settings.summary_backend = "local".into();
        settings.summary.ollama_url = url;
        settings.summary.ollama_model = "llama3:latest".into();

        let err = summarize(&settings, &sb).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "Ollama (llama3:latest at {host}) returned an error: mkdir /opt/models/manifests-v2: permission denied"
            )
        );
        assert_eq!(err.code(), "local_failed");
    }

    /// The local model failing never sends the transcript to an agent.
    #[test]
    fn a_local_failure_never_falls_back_to_an_agent() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&[], REPLY);
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);
        let mut settings = sb.settings();
        settings.local_only = true;
        let err = summarize(&settings, &sb).unwrap_err();
        assert_eq!(err.code(), "local_failed");
        assert_eq!(sb.claude_calls(), 0);
    }

    #[test]
    fn a_failed_agent_hands_over_to_a_configured_local_model_that_works() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        std::fs::write(sb.out().join("claude.fail"), "").unwrap();
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);
        let (url, h) = fake_ollama("200 OK", ollama_reply());
        let mut settings = sb.settings();
        settings.summary_backend = "local".into();
        settings.summary.ollama_url = url;

        let outcome = summarize(&settings, &sb).unwrap();
        assert_eq!(outcome.agent_name, "Local (llama3.2)");
        assert_eq!(outcome.fell_back_from.as_deref(), Some("Claude"));
        h.join().unwrap();
    }

    #[test]
    fn a_failed_agent_is_reported_when_local_is_not_configured_or_fails() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        std::fs::write(sb.out().join("claude.fail"), "").unwrap();
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);

        // No local model set up: the default summary backend.
        let err = summarize(&sb.settings(), &sb).unwrap_err();
        assert_eq!(err.code(), "agent_failed");
        assert!(err.to_string().starts_with("Claude failed: "), "{err}");

        // Set up, but not answering: still Claude's error.
        let mut settings = sb.settings();
        settings.summary_backend = "local".into();
        let err = summarize(&settings, &sb).unwrap_err();
        assert_eq!(err.code(), "agent_failed");
        assert!(err.to_string().contains("Not logged in"), "{err}");
    }

    /// The only way questions and summaries still part: an older remote
    /// summarizer with no agent allowed. Status says so.
    #[test]
    fn status_says_when_summaries_and_questions_differ() {
        let sb = Sandbox::new();
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);
        let mut settings = sb.settings();
        settings.summary_backend = "remote".into();
        settings.summary.remote_endpoint = Some("https://summaries.example.com/v1/sum".into());
        let status = super::super::status_json(&settings);
        assert_eq!(status["summary_backend"], "remote");
        assert_eq!(
            status["summary_backend_name"],
            "Remote (summaries.example.com)"
        );
        assert_eq!(status["backend_name"], "Local (llama3.2)");
        assert_eq!(
            status["summary_note"],
            "Summaries use Remote (summaries.example.com) ([meeting.summary] backend = \"remote\"); questions use Local (llama3.2)."
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

    /// Omarchy's model writes the summary when it is chosen and running,
    /// ahead of an allowed agent.
    #[test]
    fn omarchy_summarizes_when_running() {
        use crate::llm::fake_server::{FakeServer, Reply};
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);
        let server = FakeServer::start(vec![Reply::stream(&[REPLY])]);
        let mut settings = sb.settings();
        sb.use_omarchy(&mut settings, "active", &server.base_url());

        let outcome = summarize(&settings, &sb).unwrap();
        assert_eq!(outcome.agent, "omarchy:qwen-test");
        assert_eq!(outcome.agent_name, "Omarchy (qwen-test)");
        assert_eq!(
            outcome.summary.summary,
            "Steve and Pete agreed to ship Friday."
        );
        assert!(outcome.fell_back_from.is_none());
        assert_eq!(sb.claude_calls(), 0);
    }

    /// The model failing mid-request hands the summary to what would have
    /// written it without Omarchy: here the allowed agent.
    #[test]
    fn a_failing_omarchy_model_falls_back_to_the_agent() {
        use crate::llm::fake_server::{FakeServer, Reply};
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&[], REPLY);
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);
        let server = FakeServer::start(vec![Reply::status(
            500,
            r#"{"error":{"message":"out of memory"}}"#,
        )]);
        let mut settings = sb.settings();
        sb.use_omarchy(&mut settings, "active", &server.base_url());

        let outcome = summarize(&settings, &sb).unwrap();
        assert_eq!(outcome.agent, "claude");
        assert_eq!(
            outcome.fell_back_from.as_deref(),
            Some("Omarchy (qwen-test)")
        );
        assert_eq!(sb.claude_calls(), 1);
    }

    /// With no Omarchy model running, the summary goes the usual way.
    #[test]
    fn no_omarchy_model_summarizes_the_usual_way() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&[], REPLY);
        sb.meeting("Sync", &[(41, "Remote", "We ship on Friday.")]);
        let mut settings = sb.settings();
        sb.use_omarchy(&mut settings, "inactive", "http://127.0.0.1:9/v1");
        let outcome = summarize(&settings, &sb).unwrap();
        assert_eq!(outcome.agent, "claude");
        assert!(outcome.fell_back_from.is_none());
    }
}
