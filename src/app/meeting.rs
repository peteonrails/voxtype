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
        },
        retain_audio: config.meeting.retain_audio,
        max_duration_mins: config.meeting.max_duration_mins,
        vad_threshold: config.meeting.audio.vad_threshold,
        diarization: None,
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

            if json {
                let payload = serde_json::json!({
                    "active": active,
                    "status": phase.as_str(),
                    "meeting_id": meeting_id,
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
                    "Unknown export format '{}'. Valid formats: text, markdown, json",
                    format
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
