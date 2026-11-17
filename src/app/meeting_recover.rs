//! `voxtype meeting recover`: meetings the daemon never finished.

use std::sync::{Arc, Mutex};

use voxtype::config::Config;
use voxtype::meeting::assistant::summarize::{AutoSummary, SummaryEvent};
use voxtype::meeting::assistant::AssistantSettings;
use voxtype::meeting::data::MeetingId;
use voxtype::meeting::hook::{HookOutcome, PostMeeting, PostMeetingSettings};
use voxtype::meeting::recover::{self, RecoverableMeeting};
use voxtype::meeting::{MeetingConfig, MeetingStorage};

pub(crate) async fn run_recover(
    config: &Config,
    meeting_config: &MeetingConfig,
    finish: Option<&str>,
    discard: Option<&str>,
    json: bool,
) -> anyhow::Result<()> {
    let storage = MeetingStorage::open(meeting_config.storage.clone())
        .map_err(|e| anyhow::anyhow!("Failed to open storage: {}", e))?;
    let daemon = voxtype::daemon_status::read_pid_if_alive().is_some();
    let running = if daemon { running_meeting() } else { None };

    if let Some(id) = discard {
        let id = resolve(&storage, id)?;
        recover::discard(&storage, &id, running.as_ref())?;
        notify_daemon(daemon);
        if json {
            println!("{}", serde_json::json!({ "discarded": id.to_string() }));
        } else {
            println!("Discarded meeting {}.", id);
        }
        return Ok(());
    }

    if let Some(id) = finish {
        let id = resolve(&storage, id)?;
        // What the daemon kept but never transcribed comes first, so the
        // summary sees the whole meeting.
        let transcribed = match recover::transcribe_tail(
            config,
            &storage,
            &id,
            running.as_ref(),
            None,
        )
        .await
        {
            Ok(chunks) => {
                if chunks > 0 && !json {
                    println!("Transcribed {chunks} chunk(s) the daemon never got to.");
                }
                chunks
            }
            Err(e @ recover::RecoverError::NotFound(_))
            | Err(e @ recover::RecoverError::NotRecoverable { .. }) => return Err(e.into()),
            Err(e) => {
                eprintln!("Could not transcribe the audio past the transcript ({e}); finishing with the transcript as it is.");
                0
            }
        };
        let metadata = recover::finish(&storage, &id, running.as_ref())?;
        notify_daemon(daemon);
        if !json {
            println!(
                "Finished meeting {} ({}).",
                metadata.display_title(),
                duration(metadata.duration_secs.unwrap_or(0))
            );
        }
        let summary = summarize(config, meeting_config, id, json).await;
        let post = PostMeeting {
            storage: meeting_config.storage.clone(),
            meeting_id: id,
            settings: PostMeetingSettings::from_config(config),
        }
        .run()
        .await;
        notify_daemon(daemon);
        if json {
            // The meeting's metadata as `meeting show --json` has it (with
            // the summary once written), and what recovering it did.
            let metadata = storage.get_meeting(&id)?.unwrap_or(metadata);
            let mut out = serde_json::to_value(&metadata)?;
            out["recovered"] = serde_json::json!({
                "transcribed_chunks": transcribed,
                "summary": summary,
                "post_meeting_command": post.map(hook_result),
            });
            println!("{out}");
        } else if let Some(outcome) = post {
            println!("post_meeting_command: {}", hook_result(outcome));
        }
        return Ok(());
    }

    let meetings = recover::recoverable(
        &storage,
        running.as_ref(),
        config.meeting.chunk_duration_secs,
    )?;
    if json {
        println!("{}", serde_json::json!({ "meetings": meetings }));
    } else {
        print_list(&meetings);
    }
    Ok(())
}

fn resolve(storage: &MeetingStorage, id: &str) -> anyhow::Result<MeetingId> {
    storage
        .resolve_meeting_id(id)
        .map_err(|e| anyhow::anyhow!("Meeting not found: {}", e))
}

/// The meeting the running daemon is recording, from its state file.
fn running_meeting() -> Option<MeetingId> {
    let text = std::fs::read_to_string(Config::runtime_dir().join("meeting_state")).ok()?;
    let mut lines = text.lines();
    let state = lines.next()?.trim();
    if !matches!(state, "recording" | "paused" | "finalizing") {
        return None;
    }
    MeetingId::parse(lines.next()?.trim()).ok()
}

/// Have the daemon announce the list again on meeting.sock.
fn notify_daemon(daemon: bool) {
    if daemon {
        let _ = std::fs::write(Config::runtime_dir().join(recover::TRIGGER), "");
    }
}

/// Summarize as a normal stop would, `[meeting.summary]` rules and all:
/// `written`, `skipped` (auto-summary off, no consent, or no transcript) or
/// `failed`.
async fn summarize(
    config: &Config,
    meeting_config: &MeetingConfig,
    id: MeetingId,
    json: bool,
) -> serde_json::Value {
    let job = AutoSummary {
        settings: AssistantSettings::from_config(config),
        storage: meeting_config.storage.clone(),
        meeting_id: id,
    };
    let last: Arc<Mutex<Option<SummaryEvent>>> = Arc::default();
    let seen = last.clone();
    let attempted = job
        .run(move |event| {
            if !json {
                if let SummaryEvent::Summarizing { .. } = event {
                    println!("Summarizing...");
                }
            }
            *seen.lock().unwrap() = Some(event);
        })
        .await;
    let last = last.lock().unwrap().take();
    match (attempted, last) {
        (true, Some(SummaryEvent::Ready(outcome))) => {
            if !json {
                match &outcome.path {
                    Some(path) => println!("Summary written to {}", path.display()),
                    None => println!("Summary written."),
                }
            }
            serde_json::json!({ "state": "written", "agent": outcome.agent, "path": outcome.path })
        }
        (
            true,
            Some(SummaryEvent::Failed {
                message, backend, ..
            }),
        ) => {
            if !json {
                eprintln!("The summary failed: {message}");
            }
            serde_json::json!({ "state": "failed", "message": message, "backend": backend })
        }
        _ => {
            if !json {
                println!(
                    "No summary: auto-summary is off for this meeting, or it has no transcript."
                );
            }
            serde_json::json!({ "state": "skipped" })
        }
    }
}

fn hook_result(outcome: HookOutcome) -> String {
    match outcome {
        HookOutcome::Succeeded => "succeeded".into(),
        HookOutcome::Failed(Some(code)) => format!("failed (exit {code})"),
        HookOutcome::Failed(None) => "failed (killed)".into(),
        HookOutcome::TimedOut => "timed out".into(),
        HookOutcome::NotStarted(e) => format!("could not start: {e}"),
    }
}

fn duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, secs % 3600 / 60)
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

fn print_list(meetings: &[RecoverableMeeting]) {
    if meetings.is_empty() {
        println!("No meetings to recover.");
        return;
    }
    println!("Meetings that never finished (the daemon stopped while they recorded):");
    for m in meetings {
        let untranscribed = match m.untranscribed_chunks {
            0 => String::new(),
            n => format!(", {n} chunk(s) not yet transcribed"),
        };
        println!(
            "  {}  {}  {}, {} line{}{}{}  {}",
            &m.id[..8.min(m.id.len())],
            m.started_at
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M"),
            duration(m.duration_secs),
            m.lines,
            if m.lines == 1 { "" } else { "s" },
            if m.has_audio { ", audio" } else { "" },
            untranscribed,
            m.title.as_deref().unwrap_or("(untitled)"),
        );
    }
    println!();
    println!("Finish one:  voxtype meeting recover --finish <id>");
    println!("Discard one: voxtype meeting recover --discard <id>");
}
