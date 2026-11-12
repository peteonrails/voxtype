//! `voxtype meeting <action>` — start/stop/pause/resume/status/list/export/show/delete/label/summarize.

use std::path::PathBuf;
use voxtype::{config, daemon_status::check_daemon_running, meeting, setup, MeetingAction};

/// Run a meeting command
pub(crate) async fn run_meeting_command(
    config: &config::Config,
    action: MeetingAction,
) -> anyhow::Result<()> {
    use meeting::{export_meeting, ExportFormat, ExportOptions, MeetingConfig, StorageConfig};

    // Convert config to meeting config
    let storage_path = if config.meeting.storage_path == "auto" {
        StorageConfig::default_storage_path()
    } else {
        PathBuf::from(&config.meeting.storage_path)
    };

    let meeting_config = MeetingConfig {
        enabled: config.meeting.enabled,
        chunk_duration_secs: config.meeting.chunk_duration_secs,
        storage: StorageConfig {
            storage_path,
            retain_audio: config.meeting.retain_audio,
            max_meetings: 0,
            my_name: config.meeting.my_name.clone(),
        },
        retain_audio: config.meeting.retain_audio,
        max_duration_mins: config.meeting.max_duration_mins,
        vad_threshold: config.meeting.audio.vad_threshold,
        diarization: None,
        echo_rules: (&config.meeting.audio).into(),
    };

    match action {
        MeetingAction::Start {
            title,
            diarization,
            engine,
            model,
            language,
            url,
        } => {
            // Check if meeting mode is enabled
            if !config.meeting.enabled {
                eprintln!("Error: Meeting mode is disabled in config.");
                eprintln!();
                eprintln!("Enable it by adding to config.toml:");
                eprintln!("  [meeting]");
                eprintln!("  enabled = true");
                std::process::exit(1);
            }

            // Refuse a meeting that could not transcribe before it starts,
            // with the fix, rather than leave the daemon to log it.
            let options = config::MeetingStartOptions {
                engine,
                model,
                language,
                url: url.filter(|u| !u.trim().is_empty()),
            };
            if let Err(e) = meeting::start::meeting_transcription_config(config, &options) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }

            // Check if daemon is running
            check_daemon_running()?;

            // Check if meeting already in progress
            let runtime_dir = config::Config::runtime_dir();
            if meeting_phase(&runtime_dir).0 != MeetingPhase::Idle {
                refuse_start();
            }

            // --diarization ml and hybrid require the ml-diarization feature at build
            // time. Without it the daemon's diarizer factory silently falls
            // back, leaving the CLI's "(diarization backend: ml)" exit
            // message a lie. Reject the request up front with a pointer to
            // the binaries that DO carry ml-diarization.
            let needs_ml = diarization
                .as_deref()
                .filter(|b| matches!(*b, "ml" | "hybrid"));
            if let (Some(backend), false) = (needs_ml, cfg!(feature = "ml-diarization")) {
                eprintln!(
                    "Error: --diarization {backend} requested but this binary was not built with"
                );
                eprintln!(
                    "  the `ml-diarization` feature. ECAPA-TDNN diarization is shipped in the"
                );
                eprintln!(
                    "  ONNX binaries (voxtype-onnx-avx2, voxtype-onnx-avx512, voxtype-onnx-cuda-*,"
                );
                eprintln!(
                    "  voxtype-onnx-migraphx). Install one of those, or omit --diarization to"
                );
                eprintln!("  use the source-based `simple` backend.");
                std::process::exit(1);
            }

            // Ensure GTCRN speech enhancement model is available
            setup::model::ensure_gtcrn_model();

            // A --diarization override only changes the *backend*; it cannot
            // turn diarization on when config has disabled it. Warn loudly so
            // users don't think they're getting speaker labels they aren't.
            let diarization_active = config.meeting.diarization.enabled;
            if diarization.is_some() && !diarization_active {
                eprintln!(
                    "Warning: --diarization is a backend override and only takes effect when"
                );
                eprintln!(
                    "  [meeting.diarization] enabled = true in config; diarization is disabled,"
                );
                eprintln!("  so the override will be ignored for this meeting.");
            }

            // Write the diarization override first so it's visible by the time
            // the daemon picks up the start trigger.
            let diarization_file = runtime_dir.join("meeting_start_diarization");
            if let Some(ref backend) = diarization {
                std::fs::write(&diarization_file, backend)?;
            } else {
                // Clear any stale override left from a prior run.
                let _ = std::fs::remove_file(&diarization_file);
            }

            let options_file = runtime_dir.join("meeting_start_options");
            if options == config::MeetingStartOptions::default() {
                let _ = std::fs::remove_file(&options_file);
            } else {
                std::fs::write(&options_file, serde_json::to_string(&options)?)?;
            }

            // Write start trigger file (with optional title). The claim is
            // atomic, so of two starts issued together only one gets in.
            let start_file = runtime_dir.join("meeting_start");
            if !claim_start_trigger(&start_file, &title.unwrap_or_default())? {
                refuse_start();
            }
            // A meeting that came up between the check above and the claim
            // leaves this trigger to be refused by the daemon; say so here.
            if matches!(
                base_meeting_state(&runtime_dir).0,
                MeetingPhase::Recording | MeetingPhase::Paused
            ) {
                let _ = std::fs::remove_file(&start_file);
                refuse_start();
            }

            let suffix = diarization
                .as_deref()
                .filter(|_| diarization_active)
                .map(|b| format!(" (diarization backend: {})", b))
                .unwrap_or_default();
            println!(
                "Meeting start requested{}. Check status with 'voxtype meeting status'.",
                suffix
            );
        }

        MeetingAction::Stop => {
            check_daemon_running()?;

            let runtime_dir = config::Config::runtime_dir();
            match meeting_phase(&runtime_dir).0 {
                MeetingPhase::Idle => {
                    eprintln!("Error: No meeting in progress.");
                    std::process::exit(1);
                }
                MeetingPhase::Stopping => {
                    println!("Meeting stop already requested.");
                    return Ok(());
                }
                _ => {}
            }

            // Write stop trigger file
            std::fs::write(runtime_dir.join("meeting_stop"), "")?;

            println!("Meeting stop requested.");
        }

        MeetingAction::Pause => {
            check_daemon_running()?;

            let runtime_dir = config::Config::runtime_dir();
            match meeting_phase(&runtime_dir).0 {
                MeetingPhase::Recording => {}
                MeetingPhase::Pausing => {
                    println!("Meeting pause already requested.");
                    return Ok(());
                }
                // Withdraw a resume the daemon has not acted on yet; if it
                // already has, fall through and pause the resumed meeting.
                MeetingPhase::Resuming
                    if std::fs::remove_file(runtime_dir.join("meeting_resume")).is_ok() =>
                {
                    println!("Meeting pause requested.");
                    return Ok(());
                }
                MeetingPhase::Resuming => {}
                MeetingPhase::Idle => {
                    eprintln!("Error: No meeting in progress.");
                    std::process::exit(1);
                }
                _ => {
                    eprintln!("Error: No active meeting to pause.");
                    std::process::exit(1);
                }
            }

            // Write pause trigger file
            std::fs::write(runtime_dir.join("meeting_pause"), "")?;

            println!("Meeting pause requested.");
        }

        MeetingAction::Resume => {
            check_daemon_running()?;

            let runtime_dir = config::Config::runtime_dir();
            match meeting_phase(&runtime_dir).0 {
                MeetingPhase::Paused => {}
                MeetingPhase::Resuming => {
                    println!("Meeting resume already requested.");
                    return Ok(());
                }
                // Withdraw a pause the daemon has not acted on yet; if it
                // already has, fall through and resume.
                MeetingPhase::Pausing
                    if std::fs::remove_file(runtime_dir.join("meeting_pause")).is_ok() =>
                {
                    println!("Meeting resume requested.");
                    return Ok(());
                }
                MeetingPhase::Pausing => {}
                _ => {
                    eprintln!("Error: No paused meeting to resume.");
                    std::process::exit(1);
                }
            }

            // Write resume trigger file
            std::fs::write(runtime_dir.join("meeting_resume"), "")?;

            println!("Meeting resume requested.");
        }

        MeetingAction::Status { json } => {
            let (phase, meeting_id) = meeting_phase(&config::Config::runtime_dir());
            let active = phase != MeetingPhase::Idle;

            // The echo handling the running meeting uses, as the daemon
            // recorded it at start.
            let echo_cancel = meeting_id.as_deref().and_then(|id| {
                let storage = meeting::MeetingStorage::open(meeting_config.storage.clone()).ok()?;
                let id = meeting::MeetingId::parse(id).ok()?;
                storage.get_context(&id).ok()?.echo_cancel
            });
            if json {
                let payload = serde_json::json!({
                    "active": active,
                    "status": phase.as_str(),
                    "meeting_id": meeting_id,
                    "echo_cancel": echo_cancel,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else if !active {
                println!("No meeting currently in progress.");
                println!();
                println!("Use 'voxtype meeting list' to see past meetings.");
            } else {
                println!("Meeting Status: {}", phase.as_str());
                if let Some(id) = meeting_id {
                    println!("Meeting ID: {}", id);
                }
                if let Some(path) = echo_cancel {
                    println!("Echo handling: {}", path);
                }
            }
        }

        MeetingAction::List { limit, json } => {
            if json {
                let entries = meeting::list_meeting_entries(&meeting_config, Some(limit))
                    .map_err(|e| anyhow::anyhow!("Error listing meetings: {}", e))?;
                println!("{}", serde_json::to_string_pretty(&entries)?);
                return Ok(());
            }
            match meeting::list_meetings(&meeting_config, Some(limit)) {
                Ok(meetings) => {
                    if meetings.is_empty() {
                        println!("No meetings found.");
                        return Ok(());
                    }

                    println!("Recent Meetings");
                    println!("===============\n");

                    for m in meetings {
                        let duration = m
                            .duration_secs
                            .map(|d| {
                                let mins = d / 60;
                                let secs = d % 60;
                                format!("{}m {}s", mins, secs)
                            })
                            .unwrap_or_else(|| "in progress".to_string());

                        println!("{}", m.display_title());
                        println!("  ID: {}", m.id);
                        println!("  Date: {}", m.started_at.format("%Y-%m-%d %H:%M"));
                        println!("  Duration: {}", duration);
                        println!("  Status: {:?}", m.status);
                        println!();
                    }
                }
                Err(e) => {
                    eprintln!("Error listing meetings: {}", e);
                    std::process::exit(1);
                }
            }
        }

        MeetingAction::Export {
            meeting_id,
            format,
            output,
            timestamps,
            speakers,
            metadata,
        } => {
            let export_format = ExportFormat::parse(&format).ok_or_else(|| {
                anyhow::anyhow!(
                    "Unknown export format '{}'. Valid formats: {}",
                    format,
                    ExportFormat::all_names().join(", ")
                )
            })?;

            let options = ExportOptions {
                include_timestamps: timestamps,
                include_speakers: speakers,
                include_metadata: metadata,
                line_width: 0,
            };

            let meeting_data = match meeting::get_meeting(&meeting_config, &meeting_id) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Error loading meeting: {}", e);
                    std::process::exit(1);
                }
            };

            let content = match export_meeting(&meeting_data, export_format, &options) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error exporting meeting: {}", e);
                    std::process::exit(1);
                }
            };

            if let Some(path) = output {
                let file_path = if path.is_dir() {
                    let title = meeting_data.metadata.display_title();
                    let safe_title =
                        title.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-");
                    let basename = if safe_title.trim().is_empty() {
                        format!("meeting-{}", meeting_data.metadata.id)
                    } else {
                        safe_title
                    };
                    path.join(format!("{}.{}", basename, export_format.extension()))
                } else {
                    path
                };
                std::fs::write(&file_path, &content)?;
                println!("Exported to {}", file_path.display());
            } else {
                println!("{}", content);
            }
        }

        MeetingAction::Actions {
            meeting_id,
            send,
            all: _,
            pick,
            mine,
            resend,
            remind_in,
            project,
            json,
        } => {
            let request = ActionsRequest {
                meeting_id,
                send,
                pick,
                mine,
                resend,
                remind_in: remind_in.unwrap_or(config.meeting.actions.remind_in_mins),
                project: project.or_else(|| config.meeting.actions.todoist_project_id.clone()),
                json,
            };
            if let Err(e) = run_actions(config, &meeting_config.storage, request) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }

        MeetingAction::Share {
            meeting_id,
            transcript,
            format,
            to,
            output,
            command,
            preview,
            json,
        } => {
            use meeting::share::{self, ShareFormat, ShareRequest, ShareTarget, Tools};
            let format = ShareFormat::parse(&format)
                .ok_or_else(|| anyhow::anyhow!("Unknown share format '{}'", format))?;
            let target = ShareTarget::parse(&to)
                .ok_or_else(|| anyhow::anyhow!("Unknown share destination '{}'", to))?;
            let data = match meeting::get_meeting(&meeting_config, &meeting_id) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Error loading meeting: {}", e);
                    std::process::exit(1);
                }
            };
            let shared = share::compose(&data, transcript);
            let command = command.or_else(|| config.meeting.share.command.clone());
            let tools = Tools::default();

            if preview {
                // Email sends text, whatever --format says.
                let shown = if target == ShareTarget::Email {
                    ShareFormat::Text
                } else {
                    format
                };
                let body = shared.render(shown);
                if json {
                    let out = serde_json::json!({
                        "meeting_id": shared.meeting_id,
                        "title": shared.title,
                        "subject": shared.subject,
                        "format": shown.name(),
                        "transcript": transcript,
                        "sections": shared.sections,
                        "transcript_lines": shared.transcript_lines,
                        "chars": body.chars().count(),
                        "body": body,
                        "available": tools.available(command.as_deref()),
                        "command": command,
                        "folder": share::share_folder(&config.meeting.share.folder),
                    });
                    println!("{}", serde_json::to_string_pretty(&out)?);
                } else {
                    print!("{body}");
                }
                return Ok(());
            }

            let request = ShareRequest {
                target,
                format,
                with_transcript: transcript,
                output,
                folder: config.meeting.share.folder.clone(),
                command,
                timeout: std::time::Duration::from_secs(config.meeting.share.timeout_secs),
            };
            match share::send(&data, &shared, &request, &tools).await {
                Ok(outcome) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&outcome)?);
                    } else {
                        let what = if transcript {
                            "Notes and transcript"
                        } else {
                            "Notes"
                        };
                        match target {
                            ShareTarget::Clipboard => {
                                println!("{what} for \"{}\" copied to the clipboard.", shared.title)
                            }
                            ShareTarget::File => println!(
                                "{what} saved to {}",
                                outcome
                                    .path
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default()
                            ),
                            ShareTarget::Email => {
                                println!("Opened a mail draft for \"{}\".", shared.title);
                                if let Some(path) = &outcome.path {
                                    println!("Attach {} before sending.", path.display());
                                }
                            }
                            ShareTarget::Command => {
                                println!("{what} sent to the share command.");
                                if let Some(out) = outcome.command_status.filter(|o| !o.is_empty())
                                {
                                    println!("{out}");
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error sharing meeting: {}", e);
                    std::process::exit(1);
                }
            }
        }

        MeetingAction::Show { meeting_id, json } => {
            match meeting::get_meeting(&meeting_config, &meeting_id) {
                Ok(meeting) if json => {
                    let v = super::meeting_assistant::show_json(&meeting)?;
                    println!("{}", serde_json::to_string_pretty(&v)?);
                }
                Ok(meeting) => {
                    println!("{}", meeting.metadata.display_title());
                    println!("{}", "=".repeat(meeting.metadata.display_title().len()));
                    println!();
                    println!("ID:       {}", meeting.metadata.id);
                    println!(
                        "Started:  {}",
                        meeting.metadata.started_at.format("%Y-%m-%d %H:%M UTC")
                    );
                    if let Some(ended) = meeting.metadata.ended_at {
                        println!("Ended:    {}", ended.format("%Y-%m-%d %H:%M UTC"));
                    }
                    if let Some(duration) = meeting.metadata.duration_secs {
                        let hours = duration / 3600;
                        let mins = (duration % 3600) / 60;
                        let secs = duration % 60;
                        if hours > 0 {
                            println!("Duration: {}h {}m {}s", hours, mins, secs);
                        } else {
                            println!("Duration: {}m {}s", mins, secs);
                        }
                    }
                    println!("Status:   {:?}", meeting.metadata.status);
                    println!("Chunks:   {}", meeting.metadata.chunk_count);
                    if let Some(ref url) = meeting.metadata.url {
                        println!("Link:     {}", url);
                    }
                    println!();
                    println!("Transcript:");
                    println!("-----------");
                    println!("Segments: {}", meeting.transcript.segments.len());
                    println!("Words:    {}", meeting.transcript.word_count());
                    println!("Speakers: {}", meeting.transcript.speakers().join(", "));
                    println!();
                    println!(
                        "Use 'voxtype meeting export {}' to export the transcript.",
                        meeting_id
                    );
                    if meeting.transcript.speakers().iter().any(|s| s == "You") {
                        offer_my_name(config);
                    }
                }
                Err(e) => {
                    eprintln!("Error loading meeting: {}", e);
                    std::process::exit(1);
                }
            }
        }

        MeetingAction::Delete { meeting_id, force } => {
            if !force {
                eprintln!("This will permanently delete the meeting and all associated files.");
                eprintln!("Use --force to confirm deletion.");
                std::process::exit(1);
            }

            let storage = meeting::MeetingStorage::open(meeting_config.storage.clone())
                .map_err(|e| anyhow::anyhow!("Failed to open storage: {}", e))?;

            let id = storage
                .resolve_meeting_id(&meeting_id)
                .map_err(|e| anyhow::anyhow!("Meeting not found: {}", e))?;

            storage
                .delete_meeting(&id)
                .map_err(|e| anyhow::anyhow!("Failed to delete meeting: {}", e))?;

            println!("Meeting {} deleted.", meeting_id);
        }

        MeetingAction::Recover {
            finish,
            discard,
            json,
        } => {
            super::meeting_recover::run_recover(
                config,
                &meeting_config,
                finish.as_deref(),
                discard.as_deref(),
                json,
            )
            .await?;
        }

        MeetingAction::Label {
            meeting_id,
            speaker_id,
            label,
        } => {
            let storage = meeting::MeetingStorage::open(meeting_config.storage.clone())
                .map_err(|e| anyhow::anyhow!("Failed to open storage: {}", e))?;

            let id = storage
                .resolve_meeting_id(&meeting_id)
                .map_err(|e| anyhow::anyhow!("Meeting not found: {}", e))?;

            let keys = storage
                .set_speaker_label(&id, &speaker_id, &label)
                .map_err(|e| anyhow::anyhow!("Failed to label speaker: {}", e))?;

            // A running meeting: have the daemon name the lines it already
            // sent, so every view (and one reopened later) shows the name,
            // not only lines transcribed from now on.
            if meeting_is_live(&id) {
                poke_daemon("meeting_edit", "label")?;
            }

            println!(
                "Labeled {} as '{}' in meeting {}",
                keys.join(", "),
                label.trim(),
                meeting_id
            );
        }

        MeetingAction::View {
            meeting_id,
            ask_id,
            tab,
            ask,
            no_open,
            minimize,
            expand,
        } => {
            use voxtype::meeting::view::frontend::{self, ViewFrontend};
            let resolution = frontend::resolve(
                frontend::effective_configured(&config.meeting.view),
                &voxtype::osd::frontend::SystemProbe,
            );
            tracing::debug!("Meeting view: {}", resolution.reason);
            if resolution.active != ViewFrontend::Quickshell {
                let args = super::meeting_watch::ViewArgs {
                    meeting_id,
                    ask_id,
                    tab,
                    ask,
                    no_open,
                };
                return super::meeting_watch::open_view(&meeting_config, &resolution, args).await;
            }
            let path = config::Config::runtime_dir().join("meeting-view.request");
            let request = ViewRequest {
                meeting_id: meeting_id.as_deref(),
                ask_id: ask_id.as_deref(),
                // An answer is shown on the Ask tab.
                tab: tab.as_deref().or(ask_id.as_ref().map(|_| "ask")),
                ask: ask.as_deref(),
                open: !no_open,
                window: if minimize {
                    Some("minimize")
                } else if expand {
                    Some("expand")
                } else {
                    None
                },
            };
            write_view_request(&path, &request)?;
            if no_open && ask.is_none() {
                println!("Nothing to do: --no-open without --ask.");
            } else {
                println!(
                    "Asked the meeting view to {}. It opens in the Omarchy shell's Voxtype plugin or voxtype's Quickshell shell, whichever is running.",
                    if no_open { "ask" } else { "open" }
                );
            }
        }

        MeetingAction::Suggest { dry_run, watch } => {
            run_meeting_suggest(config, None, dry_run, watch).await?;
        }
        MeetingAction::Reassign {
            meeting_id,
            segment_id,
            speaker,
            reset,
            json,
        } => {
            let storage = meeting::MeetingStorage::open(meeting_config.storage.clone())
                .map_err(|e| anyhow::anyhow!("Failed to open storage: {}", e))?;
            let id = storage
                .resolve_meeting_id(&meeting_id)
                .map_err(|e| anyhow::anyhow!("Meeting not found: {}", e))?;

            // While the meeting records, the daemon owns transcript.json and
            // rewrites it after every chunk: store the move and let the
            // daemon apply it, rather than racing it for the file.
            let live = meeting_is_live(&id);

            let segment = if reset {
                storage.reset_segment_speaker(&id, segment_id, !live)
            } else {
                let speaker = speaker.as_deref().unwrap_or_default();
                storage.reassign_segment(&id, segment_id, speaker, !live)
            }
            .map_err(|e| anyhow::anyhow!("Failed to reassign segment: {}", e))?;

            if live {
                poke_daemon("meeting_edit", &segment_id.to_string())?;
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&segment)?);
            } else if reset {
                println!(
                    "Segment {} is back with {} in meeting {}",
                    segment.id,
                    segment.speaker_display(),
                    meeting_id
                );
            } else {
                println!(
                    "Moved segment {} to {} in meeting {}",
                    segment.id,
                    segment.speaker_display(),
                    meeting_id
                );
            }
        }

        MeetingAction::Edit {
            meeting_id,
            segment_id,
            text,
            reset,
            json,
        } => {
            let storage = meeting::MeetingStorage::open(meeting_config.storage.clone())
                .map_err(|e| anyhow::anyhow!("Failed to open storage: {}", e))?;
            let id = storage
                .resolve_meeting_id(&meeting_id)
                .map_err(|e| anyhow::anyhow!("Meeting not found: {}", e))?;
            let live = meeting_is_live(&id);

            let segment = if reset {
                storage.reset_segment_text(&id, segment_id, !live)
            } else {
                storage.edit_segment_text(
                    &id,
                    segment_id,
                    text.as_deref().unwrap_or_default(),
                    !live,
                )
            }
            .map_err(|e| anyhow::anyhow!("Failed to edit segment: {}", e))?;
            if live {
                poke_daemon("meeting_edit", &segment_id.to_string())?;
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&segment)?);
            } else if reset {
                println!(
                    "Segment {} is back to its transcribed text in meeting {}",
                    segment.id, meeting_id
                );
            } else {
                println!("Edited segment {} in meeting {}", segment.id, meeting_id);
            }
        }

        MeetingAction::DeleteLine {
            meeting_id,
            segment_id,
            restore,
            json,
        } => {
            let storage = meeting::MeetingStorage::open(meeting_config.storage.clone())
                .map_err(|e| anyhow::anyhow!("Failed to open storage: {}", e))?;
            let id = storage
                .resolve_meeting_id(&meeting_id)
                .map_err(|e| anyhow::anyhow!("Meeting not found: {}", e))?;
            let live = meeting_is_live(&id);

            let segment = if restore {
                storage.restore_segment(&id, segment_id, !live)
            } else {
                storage.delete_segment(&id, segment_id, !live)
            }
            .map_err(|e| anyhow::anyhow!("Failed to change segment: {}", e))?;
            if live {
                poke_daemon("meeting_edit", &segment_id.to_string())?;
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&segment)?);
            } else if restore {
                println!("Restored segment {} in meeting {}", segment.id, meeting_id);
            } else {
                println!(
                    "Deleted segment {} from meeting {} (restore with --restore)",
                    segment.id, meeting_id
                );
            }
        }

        MeetingAction::Search {
            query,
            limit,
            speaker,
            since,
            json,
        } => {
            run_meeting_search(config, &meeting_config, query, limit, speaker, since, json)?;
        }

        MeetingAction::MyName { json } => {
            let configured = config
                .meeting
                .my_name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty());
            let suggestion = match configured {
                Some(_) => None,
                None => meeting::my_name::account_full_name(),
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "my_name": configured,
                        "suggestion": suggestion,
                    }))?
                );
            } else if let Some(name) = configured {
                println!("Meetings show your side as \"{}\".", name);
            } else {
                println!("Meetings show your side as \"You\": [meeting] my_name is not set.");
                if let Some(name) = suggestion {
                    println!("Your account's full name is {}. To use it:", name);
                    println!(
                        "  voxtype config set meeting.my_name \"{}\"",
                        name.replace('"', "\\\"")
                    );
                }
            }
        }

        MeetingAction::Notes {
            meeting_id,
            append,
            set_file,
            json,
        } => {
            let storage = meeting::MeetingStorage::open(meeting_config.storage.clone())
                .map_err(|e| anyhow::anyhow!("Failed to open storage: {}", e))?;
            let id = storage
                .resolve_meeting_id(&meeting_id)
                .map_err(|e| anyhow::anyhow!("Meeting not found: {}", e))?;

            let written = if let Some(text) = append {
                Some(storage.append_notes(&id, &text))
            } else if let Some(path) = set_file {
                let text = if path.as_os_str() == "-" {
                    let mut text = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
                    text
                } else {
                    std::fs::read_to_string(&path)
                        .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", path.display(), e))?
                };
                Some(storage.set_notes(&id, &text))
            } else {
                None
            };

            let wrote = written.is_some();
            let notes = match written {
                Some(result) => {
                    let notes =
                        result.map_err(|e| anyhow::anyhow!("Failed to save notes: {}", e))?;
                    if meeting_is_live(&id) {
                        poke_daemon("meeting_notes", "")?;
                    }
                    notes
                }
                None => storage
                    .load_notes(&id)
                    .map_err(|e| anyhow::anyhow!("Failed to read notes: {}", e))?,
            };

            if json {
                println!("{}", serde_json::to_string_pretty(&notes)?);
            } else if wrote {
                println!("Notes saved for meeting {}", meeting_id);
            } else {
                print!("{}", notes.notes);
                if !notes.notes.is_empty() && !notes.notes.ends_with('\n') {
                    println!();
                }
            }
        }

        MeetingAction::Annotate {
            meeting_id,
            segment_id,
            text,
            highlight,
            remove,
            json,
        } => {
            let storage = meeting::MeetingStorage::open(meeting_config.storage.clone())
                .map_err(|e| anyhow::anyhow!("Failed to open storage: {}", e))?;
            let id = storage
                .resolve_meeting_id(&meeting_id)
                .map_err(|e| anyhow::anyhow!("Meeting not found: {}", e))?;
            let live = meeting_is_live(&id);

            let segment = if remove {
                storage.remove_annotation(&id, segment_id, !live)
            } else {
                storage.annotate_segment(
                    &id,
                    segment_id,
                    text.as_deref().unwrap_or_default(),
                    highlight,
                    !live,
                )
            }
            .map_err(|e| anyhow::anyhow!("Failed to annotate segment: {}", e))?;

            if live {
                poke_daemon("meeting_edit", &segment_id.to_string())?;
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&segment)?);
            } else if remove {
                println!(
                    "Removed the annotation on segment {} in meeting {}",
                    segment.id, meeting_id
                );
            } else {
                println!("Annotated segment {} in meeting {}", segment.id, meeting_id);
            }
        }

        MeetingAction::Import {
            file,
            title,
            engine,
            model,
            language,
            date,
            channels,
            diarization,
            json,
        } => {
            use meeting::import;
            let date = date
                .map(|d| {
                    chrono::DateTime::parse_from_rfc3339(&d)
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .map_err(|_| {
                            anyhow::anyhow!(
                                "Invalid --date '{d}': use RFC 3339, e.g. 2026-11-02T15:00:00Z"
                            )
                        })
                })
                .transpose()?;
            let request = import::ImportRequest {
                file: file.clone(),
                title,
                options: config::MeetingStartOptions {
                    engine,
                    model,
                    language,
                    url: None,
                },
                diarization,
                date,
                channels,
            };
            let name = file.display().to_string();
            let mut report = progress_reporter(json, format!("Importing {name}"));
            let outcome = import::import(config, &request, None, &mut report).await?;
            if json {
                let mut line = serde_json::to_value(&outcome)?;
                line["event"] = "imported".into();
                println!("{line}");
            } else {
                println!(
                    "Imported {} as meeting {} ({} lines, {}, {} {}).",
                    name,
                    outcome.meeting_id,
                    outcome.segments,
                    format_ms(outcome.duration_ms),
                    outcome.engine,
                    outcome.model
                );
                println!("Folder: {}", outcome.folder.display());
            }
        }

        MeetingAction::Retranscribe {
            meeting_id,
            engine,
            model,
            language,
            diarization,
            keep_edits: _,
            discard_edits,
            json,
        } => {
            use meeting::retranscribe;
            let storage = meeting::MeetingStorage::open(meeting_config.storage.clone())
                .map_err(|e| anyhow::anyhow!("Failed to open storage: {}", e))?;
            let id = storage
                .resolve_meeting_id(&meeting_id)
                .map_err(|e| anyhow::anyhow!("Meeting not found: {}", e))?;
            if live_meeting_id(&config::Config::runtime_dir()).as_deref() == Some(&id.to_string()) {
                anyhow::bail!(
                    "Meeting {id} is still recording. Stop it first: voxtype meeting stop"
                );
            }
            let request = retranscribe::RetranscribeRequest {
                meeting: id.to_string(),
                options: config::MeetingStartOptions {
                    engine,
                    model,
                    language,
                    url: None,
                },
                diarization,
                keep_edits: !discard_edits,
            };
            let mut report = progress_reporter(json, format!("Transcribing {id} again"));
            let outcome = retranscribe::retranscribe(config, &request, None, &mut report).await?;
            if json {
                let mut line = serde_json::to_value(&outcome)?;
                line["event"] = "retranscribed".into();
                println!("{line}");
            } else {
                let e = &outcome.edits;
                println!(
                    "Transcribed meeting {} again with {} {}: {} lines.",
                    outcome.meeting_id, outcome.engine, outcome.model, outcome.segments
                );
                if let Some(ref previous) = outcome.previous_transcript {
                    println!("The previous transcript is kept as {previous}.");
                }
                if discard_edits {
                    println!(
                        "Edits: {} dropped as asked; annotations: {} carried over, {} had no matching line.",
                        e.edits_discarded, e.annotations_carried, e.annotations_dropped
                    );
                } else {
                    println!(
                        "Edits: {} carried over, {} had no matching line; annotations: {} carried over, {} had no matching line.",
                        e.edits_carried,
                        e.edits_dropped,
                        e.annotations_carried,
                        e.annotations_dropped
                    );
                }
            }
        }

        MeetingAction::Play {
            meeting_id,
            segment,
            at,
            track,
            stop,
            status,
            json,
        } => {
            use meeting::playback;
            let runtime_dir = config::Config::runtime_dir();

            if stop || status {
                let mut stop_message = "Nothing was playing";
                let report = if stop {
                    let before = playback::stop(&runtime_dir);
                    if before.state == "playing" {
                        stop_message = "Playback stopped";
                    }
                    playback::PlaybackStatus {
                        state: "stopped".to_string(),
                        position_ms: None,
                        ..before
                    }
                } else {
                    playback::status(&runtime_dir)
                };
                if json {
                    println!("{}", serde_json::to_string(&report)?);
                } else if stop {
                    println!("{}", stop_message);
                } else if report.state == "playing" {
                    println!(
                        "Playing the {} track of meeting {} from {}",
                        report.track.map(|t| t.to_string()).unwrap_or_default(),
                        report.meeting_id.as_deref().unwrap_or("?"),
                        format_ms(report.position_ms.or(report.from_ms).unwrap_or(0)),
                    );
                } else {
                    println!("Nothing is playing");
                }
                return Ok(());
            }

            let meeting_id = meeting_id.expect("clap requires a meeting without --stop/--status");
            let track: playback::Track = track.parse().map_err(|e: String| anyhow::anyhow!(e))?;
            let storage = meeting::MeetingStorage::open(meeting_config.storage.clone())
                .map_err(|e| anyhow::anyhow!("Failed to open storage: {}", e))?;
            let id = storage
                .resolve_meeting_id(&meeting_id)
                .map_err(|e| anyhow::anyhow!("Meeting not found: {}", e))?;
            let start_at = match (segment, at) {
                (Some(segment), _) => Some(playback::StartAt::Segment(segment)),
                (None, Some(at)) => Some(playback::StartAt::Time(playback::parse_time(&at)?)),
                (None, None) => None,
            };
            let from_ms = playback::resolve_start(&storage, &id, start_at)?;
            let file = playback::track_file(&storage, &id, track)?;
            let tools = playback::Tools::detect()?;
            let started = playback::start(&runtime_dir, &tools, &id, track, &file, from_ms)?;

            if json {
                println!("{}", serde_json::to_string(&started)?);
            } else {
                println!(
                    "Playing the {} track of meeting {} from {}. Stop it with: voxtype meeting play --stop",
                    track,
                    id,
                    format_ms(from_ms)
                );
            }
        }

        MeetingAction::Summarize {
            meeting_id,
            format,
            output,
            regenerate,
            agent,
            local,
        } => {
            use super::meeting_assistant::{run_summarize, SummarizeArgs};
            let args = SummarizeArgs {
                meeting_id,
                format,
                output,
                regenerate,
                agent,
                local,
            };
            run_summarize(config, &meeting_config, args).await?;
        }

        MeetingAction::Ask {
            meeting_id,
            question,
            last,
            json,
            notify,
            agent,
            local,
        } => {
            use super::meeting_assistant::{run_ask, AskArgs};
            let args = AskArgs {
                meeting_id,
                question,
                last,
                json,
                notify,
                agent,
                local,
            };
            run_ask(config, &meeting_config, args).await?;
        }

        MeetingAction::Watch { json, levels } => {
            super::meeting_watch::run_watch(json, levels, &meeting_config).await?;
        }

        MeetingAction::Soundcheck {
            seconds,
            json,
            stop,
        } => {
            super::meeting_soundcheck::run_soundcheck(config, seconds, json, stop).await?;
        }

        MeetingAction::Assistant { action } => {
            super::meeting_assistant::run_assistant(config, action)?;
        }
    }

    Ok(())
}

/// `voxtype meeting suggest`; `config_path` is passed on to the
/// `meeting start` a live prompt runs.
pub(crate) async fn run_meeting_suggest(
    config: &config::Config,
    config_path: Option<PathBuf>,
    dry_run: bool,
    watch: bool,
) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    return meeting::detect::runner::suggest(config, config_path, dry_run, watch).await;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (config, config_path, dry_run, watch);
        anyhow::bail!("Meeting detection is only available on Linux.")
    }
}

/// `voxtype meeting search`.
fn run_meeting_search(
    config: &config::Config,
    meeting_config: &meeting::MeetingConfig,
    query: Vec<String>,
    limit: u32,
    speaker: Option<String>,
    since: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    use meeting::search::{parse_since, SearchHit, SearchQuery};
    let since = match since {
        Some(s) => Some(parse_since(&s).ok_or_else(|| {
            anyhow::anyhow!("--since takes a date like 2026-11-01 or a time like 2026-11-01T09:00:00Z, not '{}'", s)
        })?),
        None => None,
    };
    let text = query.join(" ");
    let storage = meeting::MeetingStorage::open(meeting_config.storage.clone())?;
    let hits = storage.search(&SearchQuery {
        text: text.clone(),
        limit,
        speaker: speaker.clone(),
        since,
    })?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "query": text,
                "speaker": speaker,
                "since": since.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                "results": hits,
            }))?
        );
        return Ok(());
    }

    if hits.is_empty() {
        println!("No meetings mention \"{}\".", text);
    }
    let mut last_meeting: Option<&str> = None;
    for hit in &hits {
        let hit: &SearchHit = hit;
        if last_meeting != Some(hit.meeting_id.as_str()) {
            let date = chrono::DateTime::parse_from_rfc3339(&hit.date)
                .map(|d| {
                    d.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_else(|_| hit.date.clone());
            if last_meeting.is_some() {
                println!();
            }
            println!(
                "{}  {}  ({})",
                date,
                hit.title,
                &hit.meeting_id[..8.min(hit.meeting_id.len())]
            );
            last_meeting = Some(&hit.meeting_id);
        }
        let place = match (hit.start_ms, hit.speaker.as_deref()) {
            (Some(ms), Some(who)) => format!("{}  {}:", format_ms(ms), who),
            (Some(ms), None) => format_ms(ms),
            _ => format!(
                "{}:",
                serde_json::to_value(hit.kind)?.as_str().unwrap_or("")
            ),
        };
        println!("  {}  {}", place, hit.snippet.replace('\n', " "));
    }

    if hits.iter().any(|h| h.speaker.as_deref() == Some("You")) {
        offer_my_name(config);
    }
    Ok(())
}

/// Once, in a terminal: suggest setting `[meeting] my_name` from the
/// account's full name. Never sets it.
fn offer_my_name(config: &config::Config) {
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() {
        return;
    }
    if let Some(name) = meeting::my_name::suggest_once(
        config.meeting.my_name.as_deref(),
        meeting::my_name::account_full_name(),
        &meeting::my_name::marker_path(),
    ) {
        eprintln!();
        eprintln!("{}", meeting::my_name::suggestion_text(&name));
    }
}

/// Where a meeting is, as the CLI sees it: the daemon's `meeting_state` file
/// plus any trigger the daemon has not acted on yet.
///
/// The daemon handles triggers between meeting chunks, so while it is
/// transcribing one a request can wait on disk for seconds. Counting pending
/// triggers keeps `status` and the next command in step with what was asked,
/// instead of reporting the state from before the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeetingPhase {
    Idle,
    Starting,
    Recording,
    Pausing,
    Paused,
    Resuming,
    Stopping,
    Finalizing,
}

impl MeetingPhase {
    fn as_str(self) -> &'static str {
        match self {
            MeetingPhase::Idle => "idle",
            MeetingPhase::Starting => "starting",
            MeetingPhase::Recording => "recording",
            MeetingPhase::Pausing => "pausing",
            MeetingPhase::Paused => "paused",
            MeetingPhase::Resuming => "resuming",
            MeetingPhase::Stopping => "stopping",
            MeetingPhase::Finalizing => "finalizing",
        }
    }
}

/// The daemon's own view from `meeting_state`.
fn base_meeting_state(runtime_dir: &std::path::Path) -> (MeetingPhase, Option<String>) {
    let state = std::fs::read_to_string(runtime_dir.join("meeting_state")).unwrap_or_default();
    let mut lines = state.lines();
    let phase = match lines.next().map(str::trim) {
        Some("recording") => MeetingPhase::Recording,
        Some("paused") => MeetingPhase::Paused,
        Some("finalizing") => MeetingPhase::Finalizing,
        _ => return (MeetingPhase::Idle, None),
    };
    let id = lines
        .next()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    (phase, id)
}

/// Whether the daemon is recording, paused on or finishing this meeting.
fn meeting_is_live(id: &meeting::MeetingId) -> bool {
    live_meeting_id(&config::Config::runtime_dir()).as_deref() == Some(&id.to_string())
}

/// Tell the daemon, through a trigger file, that a running meeting changed
/// in storage.
fn poke_daemon(trigger: &str, content: &str) -> std::io::Result<()> {
    let runtime_dir = config::Config::runtime_dir();
    std::fs::create_dir_all(&runtime_dir)?;
    std::fs::write(runtime_dir.join(trigger), content)
}

/// The ID of the meeting the daemon is recording, paused or finishing.
fn live_meeting_id(runtime_dir: &std::path::Path) -> Option<String> {
    match base_meeting_state(runtime_dir) {
        (MeetingPhase::Idle, _) => None,
        (_, id) => id,
    }
}

fn meeting_phase(runtime_dir: &std::path::Path) -> (MeetingPhase, Option<String>) {
    let pending = |name: &str| runtime_dir.join(name).exists();
    let (base, id) = base_meeting_state(runtime_dir);
    let phase = match base {
        MeetingPhase::Idle if pending("meeting_start") => MeetingPhase::Starting,
        MeetingPhase::Recording | MeetingPhase::Paused if pending("meeting_stop") => {
            MeetingPhase::Stopping
        }
        MeetingPhase::Recording if pending("meeting_pause") => MeetingPhase::Pausing,
        MeetingPhase::Paused if pending("meeting_resume") => MeetingPhase::Resuming,
        other => other,
    };
    (phase, id)
}

/// Create the meeting start trigger with `title` as its content, unless one
/// already exists. Returns `false` when another start got there first.
///
/// The content is written to a private file and hard-linked into place, so
/// the daemon can never read the trigger before its title is in it.
/// What `voxtype meeting view` asks the running meeting view to do.
struct ViewRequest<'a> {
    meeting_id: Option<&'a str>,
    ask_id: Option<&'a str>,
    tab: Option<&'a str>,
    ask: Option<&'a str>,
    open: bool,
    /// "minimize" or "expand".
    window: Option<&'a str>,
}

/// Write the request every meeting view host watches (see
/// quickshell/voxtype-meetings/Meetings.qml). Written whole and renamed into
/// place, so a host never reads half of it. `requested_at` lets a host that
/// starts later ignore a request that was not meant for it.
fn write_view_request(path: &std::path::Path, req: &ViewRequest) -> std::io::Result<()> {
    let requested_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let request = serde_json::json!({
        "meeting_id": req.meeting_id,
        "ask_id": req.ask_id,
        "tab": req.tab,
        "ask": req.ask,
        "open": req.open,
        "window": req.window,
        "requested_at": requested_at,
    });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let staged = path.with_extension(format!("request.{}.tmp", std::process::id()));
    std::fs::write(&staged, request.to_string())?;
    std::fs::rename(&staged, path)
}

fn claim_start_trigger(start_file: &std::path::Path, title: &str) -> std::io::Result<bool> {
    static STAGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let staged = start_file.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        STAGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&staged, title)?;
    let linked = std::fs::hard_link(&staged, start_file);
    let _ = std::fs::remove_file(&staged);
    match linked {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        // A filesystem without hard links: claim with an exclusive create.
        Err(_) => match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(start_file)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(title.as_bytes())?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e),
        },
    }
}

fn refuse_start() -> ! {
    eprintln!("Error: A meeting is already in progress.");
    eprintln!("Use 'voxtype meeting stop' to end it first.");
    std::process::exit(1);
}

/// Report import or retranscribe progress: to the daemon, so meeting.sock
/// carries it, and to the user, as JSON lines on stdout with `--json` or
/// one line per chunk on stderr without.
fn progress_reporter(json: bool, what: String) -> impl FnMut(&voxtype::meeting::import::Progress) {
    let runtime_dir = config::Config::runtime_dir();
    move |p| {
        voxtype::meeting::import::announce(&runtime_dir, p);
        if json {
            if let Ok(mut line) = serde_json::to_value(p) {
                line["event"] = "progress".into();
                println!("{line}");
            }
        } else if p.state == "transcribing" && p.chunks_done > 0 {
            eprintln!(
                "{what}: chunk {} of about {} ({} of {})",
                p.chunks_done,
                p.chunks_total,
                format_ms(p.position_ms),
                format_ms(p.duration_ms)
            );
        }
    }
}

/// `m:ss`, or `h:mm:ss` past an hour, as transcript timestamps read.
fn format_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `voxtype meeting actions`, after config has filled in the defaults.
struct ActionsRequest {
    meeting_id: String,
    send: Option<String>,
    pick: Option<String>,
    mine: bool,
    resend: bool,
    remind_in: u32,
    project: Option<String>,
    json: bool,
}

fn run_actions(
    config: &config::Config,
    storage_config: &meeting::StorageConfig,
    req: ActionsRequest,
) -> anyhow::Result<()> {
    use meeting::actions::{self, reminder::Reminders, todoist, Selection, Target};
    let storage = meeting::MeetingStorage::open(storage_config.clone())?;
    let id = storage.resolve_meeting_id(&req.meeting_id)?;
    let listing = actions::list(&storage, &id, config.meeting.my_name.as_deref())?;
    let reminders = Reminders::default();
    let store = todoist::TokenStore::new(todoist::TokenStore::default_path());
    let connected = matches!(store.load(), Ok(Some(_)));
    let short = &listing.meeting_id[..8];

    let selection = Selection {
        pick: match &req.pick {
            Some(p) => actions::parse_pick(p, listing.items.len()).map_err(anyhow::Error::msg)?,
            None => Vec::new(),
        },
        mine: req.mine,
        resend: req.resend,
    };

    let Some(target) = req.send.as_deref() else {
        if req.json {
            let mut out = serde_json::to_value(&listing)?;
            let items = out["items"].as_array_mut().expect("items");
            for (item, listed) in items.iter_mut().zip(&listing.items) {
                if !selection.pick.is_empty() && !selection.pick.contains(&listed.number)
                    || selection.mine && !listed.mine
                {
                    *item = serde_json::Value::Null;
                }
            }
            items.retain(|i| !i.is_null());
            out["todoist_connected"] = connected.into();
            out["reminder_available"] = reminders.available().into();
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }
        if !listing.summarized {
            println!(
                "\"{}\" has no summary yet, so no action items. Summarize it with:\n  voxtype meeting summarize {short}",
                listing.title
            );
            return Ok(());
        }
        let shown = listing.select(&selection);
        if shown.is_empty() {
            println!("No action items in \"{}\".", listing.title);
            return Ok(());
        }
        println!("Action items from \"{}\":", listing.title);
        for item in shown {
            let mut details: Vec<String> = Vec::new();
            if let Some(owner) = &item.owner {
                details.push(if item.mine {
                    format!("{owner} (you)")
                } else {
                    owner.clone()
                });
            }
            match (item.due, &item.due_said) {
                (Some(d), _) => details.push(format!(
                    "due {}{}",
                    d.format("%a %b %-d"),
                    item.due_time
                        .as_deref()
                        .map(|t| format!(" {t}"))
                        .unwrap_or_default()
                )),
                (None, Some(said)) => details.push(format!("due \"{said}\"")),
                _ => {}
            }
            if let Some(ms) = item.source_ms {
                details.push(format!("at {}", meeting::assistant::prompt::ts(ms)));
            }
            let sent: Vec<&str> = item
                .sent
                .keys()
                .map(|t| {
                    if t == "todoist" {
                        "Todoist"
                    } else {
                        "reminder"
                    }
                })
                .collect();
            println!(
                "  {:>2}. {}{}{}",
                item.number,
                item.text,
                if details.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", details.join(", "))
                },
                if sent.is_empty() {
                    String::new()
                } else {
                    format!("  [sent: {}]", sent.join(", "))
                }
            );
        }
        let mut hints = Vec::new();
        if connected {
            hints.push(format!("voxtype meeting actions {short} --send todoist"));
        }
        if reminders.available() {
            hints.push(format!("voxtype meeting actions {short} --send reminder"));
        }
        if !hints.is_empty() {
            println!("\nSend them with:\n  {}", hints.join("\n  "));
        }
        return Ok(());
    };

    let target =
        Target::parse(target).ok_or_else(|| anyhow::anyhow!("Unknown target '{target}'"))?;
    if !listing.summarized {
        anyhow::bail!(
            "\"{}\" has no summary yet, so no action items. Summarize it with:\n  voxtype meeting summarize {short}",
            listing.title
        );
    }
    let client;
    let mut senders = actions::Senders {
        todoist: None,
        reminders: None,
        project_id: req.project.clone(),
        remind_in_mins: req.remind_in,
        now: chrono::Local::now().naive_local(),
    };
    match target {
        Target::Todoist => {
            if !connected {
                anyhow::bail!("{}", todoist::TodoistError::NotConnected);
            }
            client = todoist::HttpTodoist::new(todoist::API_BASE, store);
            senders.todoist = Some(&client);
        }
        Target::Reminder => {
            if !reminders.available() {
                anyhow::bail!(
                    "omarchy-reminder is not installed, so reminders are not available here. Send to Todoist instead:\n  voxtype meeting actions {short} --send todoist"
                );
            }
            senders.reminders = Some(&reminders);
        }
    }
    let items = listing.select(&selection);
    if items.is_empty() {
        anyhow::bail!("No action items to send.");
    }
    let results = actions::send(&storage, &listing, &items, target, &senders, req.resend);
    let failed = results.iter().filter(|r| r.status == "failed").count();
    if req.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "meeting_id": listing.meeting_id,
                "target": target.name(),
                "results": results,
            }))?
        );
    } else {
        let place = if target == Target::Todoist {
            "Todoist"
        } else {
            "reminders"
        };
        for r in &results {
            match r.status {
                "sent" => match r.minutes {
                    Some(m) => println!("  {:>2}. {}: reminder in {m} min", r.number, r.text),
                    None => println!("  {:>2}. {}: added", r.number, r.text),
                },
                "already_sent" => println!(
                    "  {:>2}. {}: already sent (--resend sends it again)",
                    r.number, r.text
                ),
                _ => eprintln!(
                    "  {:>2}. {}: failed: {}",
                    r.number,
                    r.text,
                    r.error.as_deref().unwrap_or("unknown error")
                ),
            }
        }
        let sent = results.iter().filter(|r| r.status == "sent").count();
        println!("Sent {sent} of {} to {place}.", results.len());
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn touch(dir: &std::path::Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn view_request_is_the_json_the_hosts_read() {
        let dir = runtime();
        let path = dir.path().join("voxtype").join("meeting-view.request");
        let req = ViewRequest {
            meeting_id: Some("abc"),
            ask_id: Some("q1"),
            tab: Some("ask"),
            ask: Some("last-5m"),
            open: false,
            window: Some("minimize"),
        };
        write_view_request(&path, &req).unwrap();
        let req: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(req["meeting_id"], "abc");
        assert_eq!(req["tab"], "ask");
        assert_eq!(req["ask"], "last-5m");
        assert_eq!(req["ask_id"], "q1");
        assert_eq!(req["window"], "minimize");
        assert_eq!(req["open"], false);
        assert!(req["requested_at"].as_u64().unwrap() > 0);

        // Defaults: no meeting, no tab, open; and nothing left behind.
        let req = ViewRequest {
            meeting_id: None,
            ask_id: None,
            tab: None,
            ask: None,
            open: true,
            window: None,
        };
        write_view_request(&path, &req).unwrap();
        let req: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(req["meeting_id"].is_null() && req["tab"].is_null() && req["ask"].is_null());
        assert!(req["ask_id"].is_null());
        assert_eq!(req["open"], true);
        let leftovers = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1);
    }

    #[test]
    fn a_pending_start_reads_as_starting() {
        let dir = runtime();
        assert_eq!(meeting_phase(dir.path()).0, MeetingPhase::Idle);
        touch(dir.path(), "meeting_state", "idle");
        touch(dir.path(), "meeting_start", "Standup");
        assert_eq!(meeting_phase(dir.path()).0, MeetingPhase::Starting);
    }

    #[test]
    fn pending_pause_and_resume_show_before_the_daemon_acts() {
        let dir = runtime();
        touch(dir.path(), "meeting_state", "recording\nabc-123");
        touch(dir.path(), "meeting_pause", "");
        assert_eq!(
            meeting_phase(dir.path()),
            (MeetingPhase::Pausing, Some("abc-123".to_string()))
        );

        std::fs::remove_file(dir.path().join("meeting_pause")).unwrap();
        touch(dir.path(), "meeting_state", "paused\nabc-123");
        touch(dir.path(), "meeting_resume", "");
        assert_eq!(meeting_phase(dir.path()).0, MeetingPhase::Resuming);

        touch(dir.path(), "meeting_stop", "");
        assert_eq!(meeting_phase(dir.path()).0, MeetingPhase::Stopping);
    }

    #[test]
    fn triggers_that_cannot_apply_do_not_change_the_phase() {
        let dir = runtime();
        touch(dir.path(), "meeting_state", "idle");
        touch(dir.path(), "meeting_stop", "");
        touch(dir.path(), "meeting_pause", "");
        assert_eq!(meeting_phase(dir.path()).0, MeetingPhase::Idle);

        touch(dir.path(), "meeting_state", "recording\nabc");
        touch(dir.path(), "meeting_start", "");
        assert_eq!(meeting_phase(dir.path()).0, MeetingPhase::Stopping);
        std::fs::remove_file(dir.path().join("meeting_stop")).unwrap();
        std::fs::remove_file(dir.path().join("meeting_pause")).unwrap();
        assert_eq!(meeting_phase(dir.path()).0, MeetingPhase::Recording);
    }

    #[test]
    fn finalizing_is_active_and_keeps_the_meeting_id() {
        let dir = runtime();
        touch(dir.path(), "meeting_state", "finalizing\nabc-123");
        assert_eq!(
            meeting_phase(dir.path()),
            (MeetingPhase::Finalizing, Some("abc-123".to_string()))
        );
    }

    #[test]
    fn only_one_of_two_starts_claims_the_trigger() {
        let dir = runtime();
        let start = dir.path().join("meeting_start");
        assert!(claim_start_trigger(&start, "First").unwrap());
        assert!(!claim_start_trigger(&start, "Second").unwrap());
        assert_eq!(std::fs::read_to_string(&start).unwrap(), "First");
        // No staging files left behind.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_starts_claim_exactly_once() {
        let dir = runtime();
        let start = dir.path().join("meeting_start");
        let winners: usize = (0..8)
            .map(|i| {
                let start = start.clone();
                std::thread::spawn(move || claim_start_trigger(&start, &format!("m{i}")).unwrap())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap() as usize)
            .sum();
        assert_eq!(winners, 1);
    }
}
