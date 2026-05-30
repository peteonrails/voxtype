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
        MeetingAction::Start { title, diarization } => {
            // Check if meeting mode is enabled
            if !config.meeting.enabled {
                eprintln!("Error: Meeting mode is disabled in config.");
                eprintln!();
                eprintln!("Enable it by adding to config.toml:");
                eprintln!("  [meeting]");
                eprintln!("  enabled = true");
                std::process::exit(1);
            }

            // Check if daemon is running
            check_daemon_running()?;

            // Check if meeting already in progress
            let meeting_state_file = config::Config::runtime_dir().join("meeting_state");
            if meeting_state_file.exists() {
                let state = std::fs::read_to_string(&meeting_state_file).unwrap_or_default();
                if state.starts_with("recording") || state.starts_with("paused") {
                    eprintln!("Error: A meeting is already in progress.");
                    eprintln!("Use 'voxtype meeting stop' to end it first.");
                    std::process::exit(1);
                }
            }

            // --diarization ml requires the ml-diarization feature at build
            // time. Without it the daemon's diarizer factory silently falls
            // back, leaving the CLI's "(diarization backend: ml)" exit
            // message a lie. Reject the request up front with a pointer to
            // the binaries that DO carry ml-diarization.
            if diarization.as_deref() == Some("ml") && !cfg!(feature = "ml-diarization") {
                eprintln!("Error: --diarization ml requested but this binary was not built with");
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
            let runtime_dir = config::Config::runtime_dir();
            let diarization_file = runtime_dir.join("meeting_start_diarization");
            if let Some(ref backend) = diarization {
                std::fs::write(&diarization_file, backend)?;
            } else {
                // Clear any stale override left from a prior run.
                let _ = std::fs::remove_file(&diarization_file);
            }

            // Write start trigger file (with optional title)
            let start_file = runtime_dir.join("meeting_start");
            let content = title.unwrap_or_default();
            std::fs::write(&start_file, content)?;

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

            // Check if meeting is in progress
            let meeting_state_file = config::Config::runtime_dir().join("meeting_state");
            if !meeting_state_file.exists() {
                eprintln!("Error: No meeting in progress.");
                std::process::exit(1);
            }

            let state = std::fs::read_to_string(&meeting_state_file).unwrap_or_default();
            if state.starts_with("idle") || state.is_empty() {
                eprintln!("Error: No meeting in progress.");
                std::process::exit(1);
            }

            // Write stop trigger file
            let stop_file = config::Config::runtime_dir().join("meeting_stop");
            std::fs::write(&stop_file, "")?;

            println!("Meeting stop requested.");
        }

        MeetingAction::Pause => {
            check_daemon_running()?;

            // Check if meeting is active (not paused)
            let meeting_state_file = config::Config::runtime_dir().join("meeting_state");
            if !meeting_state_file.exists() {
                eprintln!("Error: No meeting in progress.");
                std::process::exit(1);
            }

            let state = std::fs::read_to_string(&meeting_state_file).unwrap_or_default();
            if !state.starts_with("recording") {
                eprintln!("Error: No active meeting to pause.");
                std::process::exit(1);
            }

            // Write pause trigger file
            let pause_file = config::Config::runtime_dir().join("meeting_pause");
            std::fs::write(&pause_file, "")?;

            println!("Meeting pause requested.");
        }

        MeetingAction::Resume => {
            check_daemon_running()?;

            // Check if meeting is paused
            let meeting_state_file = config::Config::runtime_dir().join("meeting_state");
            if !meeting_state_file.exists() {
                eprintln!("Error: No paused meeting to resume.");
                std::process::exit(1);
            }

            let state = std::fs::read_to_string(&meeting_state_file).unwrap_or_default();
            if !state.starts_with("paused") {
                eprintln!("Error: No paused meeting to resume.");
                std::process::exit(1);
            }

            // Write resume trigger file
            let resume_file = config::Config::runtime_dir().join("meeting_resume");
            std::fs::write(&resume_file, "")?;

            println!("Meeting resume requested.");
        }

        MeetingAction::Status => {
            // Read meeting state file
            let meeting_state_file = config::Config::runtime_dir().join("meeting_state");
            if !meeting_state_file.exists() {
                println!("No meeting currently in progress.");
                println!();
                println!("Use 'voxtype meeting list' to see past meetings.");
                return Ok(());
            }

            let state = std::fs::read_to_string(&meeting_state_file).unwrap_or_default();
            let lines: Vec<&str> = state.lines().collect();

            if lines.is_empty() || lines[0] == "idle" {
                println!("No meeting currently in progress.");
                println!();
                println!("Use 'voxtype meeting list' to see past meetings.");
            } else {
                let status = lines[0];
                let meeting_id = lines.get(1).unwrap_or(&"");

                println!("Meeting Status: {}", status);
                if !meeting_id.is_empty() {
                    println!("Meeting ID: {}", meeting_id);
                }
            }
        }

        MeetingAction::List { limit } => {
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

        MeetingAction::Show { meeting_id } => {
            match meeting::get_meeting(&meeting_config, &meeting_id) {
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

            // Parse speaker_id - accept "SPEAKER_00", "0", "00", etc.
            let speaker_num: u32 = if speaker_id.starts_with("SPEAKER_") {
                speaker_id
                    .trim_start_matches("SPEAKER_")
                    .parse()
                    .map_err(|_| anyhow::anyhow!("Invalid speaker ID format: {}", speaker_id))?
            } else {
                speaker_id.parse().map_err(|_| {
                    anyhow::anyhow!(
                        "Invalid speaker ID: {}. Use SPEAKER_XX or a number.",
                        speaker_id
                    )
                })?
            };

            storage
                .set_speaker_label(&id, speaker_num, &label)
                .map_err(|e| anyhow::anyhow!("Failed to set speaker label: {}", e))?;

            println!(
                "Labeled SPEAKER_{:02} as '{}' in meeting {}",
                speaker_num, label, meeting_id
            );
        }

        MeetingAction::Summarize {
            meeting_id,
            format,
            output,
        } => {
            // Load meeting
            let meeting = meeting::get_meeting(&meeting_config, &meeting_id)
                .map_err(|e| anyhow::anyhow!("Failed to load meeting: {}", e))?;

            // Create summary config from meeting config
            let summary_config = meeting::summary::SummaryConfig {
                backend: config.meeting.summary.backend.clone(),
                ollama_url: config.meeting.summary.ollama_url.clone(),
                ollama_model: config.meeting.summary.ollama_model.clone(),
                remote_endpoint: config.meeting.summary.remote_endpoint.clone(),
                remote_api_key: config.meeting.summary.remote_api_key.clone(),
                timeout_secs: config.meeting.summary.timeout_secs,
            };

            // Create summarizer
            let summarizer = meeting::summary::create_summarizer(&summary_config)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Summarization not configured. Set [meeting.summary] backend in config.toml:\n\n\
                        [meeting.summary]\n\
                        backend = \"local\"  # or \"remote\"\n\
                        ollama_url = \"http://localhost:11434\"\n\
                        ollama_model = \"llama3.2\""
                    )
                })?;

            // Check availability
            if !summarizer.is_available() {
                return Err(anyhow::anyhow!(
                    "Summarizer '{}' is not available. Check that Ollama is running.",
                    summarizer.name()
                ));
            }

            eprintln!("Generating summary using {}...", summarizer.name());

            // Generate summary
            let summary = summarizer
                .summarize(&meeting)
                .map_err(|e| anyhow::anyhow!("Summarization failed: {}", e))?;

            // Format output
            let content = match format.as_str() {
                "json" => serde_json::to_string_pretty(&summary)
                    .map_err(|e| anyhow::anyhow!("Failed to serialize summary: {}", e))?,
                "text" => {
                    let mut text = String::new();
                    text.push_str(&format!("Summary: {}\n\n", summary.summary));

                    if !summary.key_points.is_empty() {
                        text.push_str("Key Points:\n");
                        for point in &summary.key_points {
                            text.push_str(&format!("  - {}\n", point));
                        }
                        text.push('\n');
                    }

                    if !summary.action_items.is_empty() {
                        text.push_str("Action Items:\n");
                        for item in &summary.action_items {
                            let assignee = item
                                .assignee
                                .as_ref()
                                .map(|a| format!(" ({})", a))
                                .unwrap_or_default();
                            text.push_str(&format!("  - {}{}\n", item.description, assignee));
                        }
                        text.push('\n');
                    }

                    if !summary.decisions.is_empty() {
                        text.push_str("Decisions:\n");
                        for decision in &summary.decisions {
                            text.push_str(&format!("  - {}\n", decision));
                        }
                    }

                    text
                }
                _ => meeting::summary::summary_to_markdown(&summary),
            };

            // Output
            if let Some(path) = output {
                std::fs::write(&path, &content)?;
                eprintln!("Summary saved to {:?}", path);
            } else {
                println!("{}", content);
            }
        }
    }

    Ok(())
}
