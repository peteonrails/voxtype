//! `voxtype meeting ask`, `meeting assistant` and `meeting summarize`.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use voxtype::meeting::assistant::{
    self, agent, asks, summarize, AssistantError, AssistantSettings, Backend, Overrides,
};
use voxtype::meeting::{self, MeetingConfig, MeetingData, MeetingSummary};
use voxtype::{config, AssistantAction};

/// How long `ask --notify` waits for the notification to be answered.
const NOTIFY_WAIT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// `meeting show --json`: the meeting plus its saved questions and answers.
pub(crate) fn show_json(meeting: &MeetingData) -> anyhow::Result<serde_json::Value> {
    let mut v = serde_json::to_value(meeting)?;
    let asks = assistant::meeting_folder(meeting)
        .map(asks::load)
        .unwrap_or_default();
    v["asks"] = serde_json::to_value(asks)?;
    Ok(v)
}

/// What the user chose at the consent prompt.
enum Decision {
    Agent(String),
    Local,
    NotNow,
}

/// Ask on the terminal whether transcripts may go to an agent: one line per
/// agent that could answer, then the local model, then "not now".
fn prompt_consent(settings: &AssistantSettings, wanted: &str) -> Decision {
    let res = settings.resolve(None);
    let mut options: Vec<(String, String)> = res
        .candidates
        .iter()
        .filter(|c| c.supported && c.found)
        .map(|c| {
            let how = if c.tool_less {
                String::new()
            } else {
                " (in a bwrap jail)".to_string()
            };
            (c.id.clone(), format!("Send to {}{}", c.name, how))
        })
        .collect();
    // The agent that was asked for goes first.
    options.sort_by_key(|(id, _)| id != wanted);
    if !options.iter().any(|(id, _)| id == wanted) {
        if let Some(s) = agent::spec(wanted) {
            options.insert(0, (s.id.to_string(), format!("Send to {}", s.name)));
        }
    }

    eprintln!("Meeting transcripts go to the agent's account, usually a cloud service.");
    eprintln!("This is asked once per agent; change it later with `voxtype meeting assistant`.");
    for (i, (_, label)) in options.iter().enumerate() {
        eprintln!("  [{}] {}", i + 1, label);
    }
    eprintln!(
        "  [l] Use a local model ({})",
        settings.summary.ollama_model
    );
    eprintln!("  [n] Not now");
    eprint!("Choice [1]: ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return Decision::NotNow;
    }
    let choice = line.trim().to_ascii_lowercase();
    match choice.as_str() {
        "" if !options.is_empty() => Decision::Agent(options[0].0.clone()),
        "l" | "local" => Decision::Local,
        "n" | "no" | "not now" => Decision::NotNow,
        n => match n.parse::<usize>() {
            Ok(i) if i >= 1 && i <= options.len() => Decision::Agent(options[i - 1].0.clone()),
            _ => Decision::NotNow,
        },
    }
}

fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Choose the backend, asking for consent on a terminal when it is missing.
fn backend_for(
    settings: &AssistantSettings,
    overrides: &Overrides,
    allow_prompt: bool,
) -> Result<Backend, AssistantError> {
    match assistant::choose_backend(settings, overrides) {
        Err(AssistantError::ConsentRequired { agent, name }) if allow_prompt => {
            match prompt_consent(settings, &agent) {
                Decision::Agent(id) => {
                    let mut consent = settings.consent();
                    let path = agent::spec(&id)
                        .and_then(|s| agent::find_binary(&settings.env, s.binary, &consent, s.id));
                    consent.allow(&id, path).map_err(|e| {
                        AssistantError::Storage(format!("Could not save consent: {e}"))
                    })?;
                    eprintln!("Allowed. Revoke with: voxtype meeting assistant revoke {id}");
                    let overrides = Overrides {
                        agent: Some(id),
                        ..overrides.clone()
                    };
                    assistant::choose_backend(settings, &overrides)
                }
                Decision::Local => assistant::choose_backend(
                    settings,
                    &Overrides {
                        local: true,
                        ..overrides.clone()
                    },
                ),
                Decision::NotNow => Err(AssistantError::ConsentRequired { agent, name }),
            }
        }
        other => other,
    }
}

fn load_meeting(meeting_config: &MeetingConfig, id: &str) -> anyhow::Result<MeetingData> {
    meeting::get_meeting(meeting_config, id)
        .map_err(|e| anyhow::anyhow!("Failed to load meeting: {e}"))
}

fn json_line(v: serde_json::Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

pub(crate) struct AskArgs {
    pub meeting_id: String,
    pub question: String,
    pub last: Option<String>,
    pub json: bool,
    pub notify: bool,
    pub agent: Option<String>,
    pub local: bool,
}

pub(crate) async fn run_ask(
    config: &config::Config,
    meeting_config: &MeetingConfig,
    args: AskArgs,
) -> anyhow::Result<()> {
    let settings = AssistantSettings::from_config(config);
    let fail = |e: &AssistantError, backend: Option<&Backend>, json: bool| -> ! {
        if json {
            json_line(serde_json::json!({
                "event": "error",
                "code": e.code(),
                "message": e.to_string(),
                "backend": backend.map(Backend::display_name),
            }));
        } else {
            eprintln!("Error: {e}");
        }
        std::process::exit(1);
    };

    if args.question.trim().is_empty() {
        anyhow::bail!("The question is empty");
    }
    let last = args
        .last
        .as_deref()
        .map(assistant::parse_duration)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--last: {e}"))?;
    let meeting = load_meeting(meeting_config, &args.meeting_id)?;
    let overrides = Overrides {
        local: args.local,
        agent: args.agent.clone(),
    };
    let backend = match backend_for(&settings, &overrides, !args.json && interactive()) {
        Ok(b) => b,
        Err(e) => fail(&e, None, args.json),
    };

    let meeting_id = meeting.metadata.id.to_string();
    if args.json {
        json_line(serde_json::json!({
            "event": "start",
            "meeting_id": meeting_id,
            "agent": backend.id(),
            "agent_name": backend.display_name(),
            "question": args.question.trim(),
            "last_secs": last.map(|d| d.as_secs()),
        }));
    }

    let request = assistant::AskRequest {
        question: args.question.clone(),
        last,
    };
    let json = args.json;
    let task_settings = settings.clone();
    let task_backend = backend.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut stdout = std::io::stdout();
        let mut on_text = |text: &str| {
            if json {
                if !text.is_empty() {
                    json_line(serde_json::json!({"event": "delta", "text": text}));
                }
            } else {
                let _ = stdout.write_all(text.as_bytes());
                let _ = stdout.flush();
            }
        };
        assistant::ask(
            &task_settings,
            &meeting,
            &request,
            &task_backend,
            &mut on_text,
        )
    })
    .await?;

    let record = match result {
        Ok(r) => r,
        Err(e) => {
            if !json {
                println!();
            }
            fail(&e, Some(&backend), json)
        }
    };
    if json {
        let mut v = serde_json::to_value(&record)?;
        v["event"] = "answer".into();
        v["meeting_id"] = meeting_id.clone().into();
        json_line(v);
    } else {
        println!();
        eprintln!("Answered by {}", record.agent_name);
    }

    if args.notify {
        notify(&settings, &meeting_id, &record).await;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn notify(settings: &AssistantSettings, meeting_id: &str, record: &asks::AskRecord) {
    use assistant::notify::{notification_body, notify_answer, OpenAction};
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: no notification shown (no session bus: {e})");
            return;
        }
    };
    let exe = std::env::current_exe().ok();
    let command = assistant::open_action_command(
        settings.open_command.as_deref(),
        exe.as_deref(),
        assistant::binary_has_meeting_view,
    );
    let open = command.map(|command| OpenAction {
        command,
        meeting_id: meeting_id.to_string(),
        ask_id: record.id.clone(),
    });
    let title = format!("{} answered: {}", record.agent_name, record.question);
    let title: String = notification_body(&title, 80);
    let body = notification_body(&record.answer, 300);
    if let Err(e) = notify_answer(&conn, &title, &body, open.as_ref(), NOTIFY_WAIT).await {
        eprintln!("Warning: could not show the notification: {e}");
    }
}

#[cfg(not(target_os = "linux"))]
async fn notify(_settings: &AssistantSettings, _meeting_id: &str, _record: &asks::AskRecord) {
    eprintln!("Warning: --notify is only supported on Linux");
}

pub(crate) fn run_assistant(
    config: &config::Config,
    action: AssistantAction,
) -> anyhow::Result<()> {
    let settings = AssistantSettings::from_config(config);
    match action {
        AssistantAction::Status { json: true } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&assistant::status_json(&settings))?
            );
        }
        AssistantAction::Status { json: false } => print_status(&settings),
        AssistantAction::Allow { agent: id } => {
            let spec = agent::spec(&id).ok_or_else(|| {
                anyhow::anyhow!(
                    "Unknown agent '{id}'. Supported: {}",
                    assistant::SUPPORTED_AGENTS.join(", ")
                )
            })?;
            if let agent::Support::Unsupported(why) = spec.support {
                anyhow::bail!("{} is not supported: {why}", spec.name);
            }
            let mut consent = settings.consent();
            let path = agent::find_binary(&settings.env, spec.binary, &consent, spec.id);
            if path.is_none() {
                eprintln!(
                    "Warning: {} was not found on PATH; consent is saved for when it is.",
                    spec.binary
                );
            }
            consent.allow(spec.id, path)?;
            println!(
                "Meeting transcripts may now go to {}. Revoke with: voxtype meeting assistant revoke {}",
                spec.name, spec.id
            );
            if spec.support == agent::Support::Jailed {
                println!(
                    "Note: {}'s command tool cannot be turned off, so it runs in a bwrap jail \
                     that sees no files from your home except its auth.json.",
                    spec.name
                );
                if let Some(Err(why)) = consent
                    .get(spec.id)
                    .and_then(|e| e.path.clone())
                    .map(|p| agent::codex_jail(&settings.env, &p))
                {
                    eprintln!("Warning: {} cannot be used yet: {why}", spec.name);
                }
            }
        }
        AssistantAction::Revoke { agent: id } => {
            let key = agent::spec(&id).map(|s| s.id).unwrap_or(id.as_str());
            if settings.consent().revoke(key)? {
                println!("Meeting transcripts will no longer go to {key}.");
            } else {
                println!("{key} was not allowed.");
            }
        }
    }
    Ok(())
}

fn print_status(settings: &AssistantSettings) {
    let consent = settings.consent();
    let res = settings.resolve(None);
    let source = |s: &str| match s {
        "config" => "set in [meeting.assistant] agent",
        "omarchy" => "the Omarchy default agent",
        "path" => "found on PATH",
        _ => "",
    };
    if settings.local_only {
        println!(
            "Assistant: local model only ({} at {})",
            settings.summary.ollama_model, settings.summary.ollama_url
        );
    } else {
        match &res.agent {
            Some(a) => {
                println!(
                    "Assistant: {} ({}) at {}",
                    a.name,
                    source(a.source),
                    a.path.display()
                );
                match consent.get(&a.id) {
                    Some(e) => println!(
                        "Consent:   allowed {}",
                        e.allowed_at
                            .with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M")
                    ),
                    None => println!(
                        "Consent:   not yet. Allow with: voxtype meeting assistant allow {}",
                        a.id
                    ),
                }
            }
            None => {
                if let Some(p) = &res.problem {
                    println!("Assistant: none: {p}");
                } else {
                    println!(
                        "Assistant: no agent found; using the local model ({} at {})",
                        settings.summary.ollama_model, settings.summary.ollama_url
                    );
                }
            }
        }
    }
    if res.choice_required {
        println!("           Several agents were found; allow the one you want to use.");
    }
    if let Some(d) = &res.omarchy_default {
        println!("Omarchy default agent: {d}");
    }
    println!(
        "Summary at stop: {}",
        match (settings.summary_auto, settings.auto_summary()) {
            (Some(_), true) => "on",
            (Some(_), false) => "off",
            (None, true) => "on (an agent is allowed)",
            (None, false) => "off until an agent is allowed",
        }
    );
    if !res.candidates.is_empty() {
        println!("Agents:");
        for c in &res.candidates {
            let mode = if !c.supported {
                "not supported"
            } else if c.tool_less {
                "no tools"
            } else {
                "bwrap jail"
            };
            let allowed = if c.consent { "allowed" } else { "not allowed" };
            println!(
                "  {:<8} {:<18} {:<12} {}",
                c.id,
                mode,
                allowed,
                c.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .or_else(|| c.note.clone())
                    .unwrap_or_default()
            );
        }
    }
}

pub(crate) struct SummarizeArgs {
    pub meeting_id: String,
    pub format: String,
    pub output: Option<PathBuf>,
    pub regenerate: bool,
    pub agent: Option<String>,
    pub local: bool,
}

pub(crate) async fn run_summarize(
    config: &config::Config,
    meeting_config: &MeetingConfig,
    args: SummarizeArgs,
) -> anyhow::Result<()> {
    let settings = AssistantSettings::from_config(config);
    let meeting = load_meeting(meeting_config, &args.meeting_id)?;

    let summary = match meeting.metadata.summary.clone() {
        Some(saved) if !args.regenerate && args.agent.is_none() && !args.local => {
            eprintln!(
                "Saved summary{}. Use --regenerate for a new one.",
                saved
                    .model
                    .as_deref()
                    .map(|m| format!(" by {m}"))
                    .unwrap_or_default()
            );
            saved
        }
        _ => {
            let mut overrides = Overrides {
                local: args.local,
                agent: args.agent.clone(),
            };
            // Ask for consent up front on a terminal, like `ask` does. The
            // older "local"/"remote" summarizers need none, nor does a
            // chosen "omarchy"/"openai" model.
            let legacy = matches!(
                assistant::choose_summary_backend(&settings, &overrides),
                Ok(assistant::SummaryBackend::Legacy(_)
                    | assistant::SummaryBackend::Assistant(Backend::Llm(_)))
            );
            if interactive() && !legacy {
                match backend_for(&settings, &overrides, true) {
                    Ok(Backend::Agent(a)) => overrides.agent = Some(a.id),
                    Ok(Backend::Local {
                        fallback: false, ..
                    }) => overrides.local = true,
                    Ok(_) => {}
                    Err(e @ AssistantError::ConsentRequired { .. }) => anyhow::bail!("{e}"),
                    Err(_) => {}
                }
            }
            let id = meeting.metadata.id.to_string();
            let storage = meeting_config.storage.clone();
            eprintln!("Summarizing...");
            let outcome = tokio::task::spawn_blocking(move || {
                summarize::summarize_meeting(&settings, &storage, &id, &overrides)
            })
            .await?
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            if let Some(failed) = &outcome.fell_back_from {
                eprintln!(
                    "{failed} failed; summarized with {} instead.",
                    outcome.agent_name
                );
            }
            eprintln!(
                "Summarized by {}{}",
                outcome.agent_name,
                outcome
                    .path
                    .as_ref()
                    .map(|p| format!(", saved to {}", p.display()))
                    .unwrap_or_default()
            );
            outcome.summary
        }
    };

    let content = format_summary(&summary, &args.format)?;
    if let Some(path) = args.output {
        std::fs::write(&path, &content)?;
        eprintln!("Summary saved to {:?}", path);
    } else {
        println!("{}", content);
    }
    Ok(())
}

fn format_summary(summary: &MeetingSummary, format: &str) -> anyhow::Result<String> {
    Ok(match format {
        "json" => serde_json::to_string_pretty(summary)?,
        "text" => {
            let mut text = String::new();
            text.push_str(&format!("Summary: {}\n\n", summary.summary));
            let list = |text: &mut String, title: &str, items: Vec<String>| {
                if !items.is_empty() {
                    text.push_str(&format!("{title}:\n"));
                    for item in items {
                        text.push_str(&format!("  - {item}\n"));
                    }
                    text.push('\n');
                }
            };
            list(&mut text, "Key Points", summary.key_points.clone());
            list(
                &mut text,
                "Action Items",
                summary
                    .action_items
                    .iter()
                    .map(|i| match &i.assignee {
                        Some(a) => format!("{} ({})", i.description, a),
                        None => i.description.clone(),
                    })
                    .collect(),
            );
            list(&mut text, "Decisions", summary.decisions.clone());
            list(
                &mut text,
                "Chapters",
                summary
                    .chapters
                    .iter()
                    .map(|c| format!("[{}] {}", assistant::prompt::ts(c.start_ms), c.title))
                    .collect(),
            );
            text.trim_end().to_string()
        }
        _ => meeting::summary::summary_to_markdown(summary),
    })
}
