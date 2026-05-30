//! `voxtype record start|stop|toggle|cancel` — write override files for the
//! daemon and send the appropriate signal. The override files (model,
//! output_mode, profile, smart_auto_submit, auto_submit, shift_enter_newlines)
//! are intentionally separate sentinels under `runtime_dir/`; merging them
//! would invent write-race surface that doesn't exist today (see
//! `docs/REFACTORING.md`).

use voxtype::{config, daemon_status, RecordAction};

/// Send a record command to the running daemon via Unix signals or file triggers
pub(crate) fn send_record_command(
    config: &config::Config,
    action: RecordAction,
    top_level_model: Option<&str>,
) -> anyhow::Result<()> {
    use voxtype::OutputModeOverride;

    // Verify the daemon is alive before writing any override files; the
    // process-existence check (and stale-lockfile cleanup) lives in
    // `daemon_status::check_daemon_running`, so this stays in sync with the
    // checks in `voxtype meeting` and `voxtype status`.
    let pid = daemon_status::check_daemon_running()?;

    // Handle cancel separately (uses file trigger instead of signal)
    if matches!(action, RecordAction::Cancel) {
        let cancel_file = config::Config::runtime_dir().join("cancel");
        std::fs::write(&cancel_file, "cancel")
            .map_err(|e| anyhow::anyhow!("Failed to write cancel file: {}", e))?;
        return Ok(());
    }

    // Write output mode override file if specified
    // For file mode, format is "file" or "file:/path/to/file"
    if let Some(mode_override) = action.output_mode_override() {
        let override_file = config::Config::runtime_dir().join("output_mode_override");
        let mode_str = match mode_override {
            OutputModeOverride::Type => "type".to_string(),
            OutputModeOverride::Clipboard => "clipboard".to_string(),
            OutputModeOverride::Paste => "paste".to_string(),
            OutputModeOverride::File => {
                // Check if explicit path was provided with --file=path
                match action.file_path() {
                    Some(path) if !path.is_empty() => format!("file:{}", path),
                    _ => "file".to_string(),
                }
            }
        };
        std::fs::write(&override_file, mode_str)
            .map_err(|e| anyhow::anyhow!("Failed to write output mode override: {}", e))?;
    }

    // Write model override file if specified (subcommand --model takes priority over top-level --model)
    let model_override = action.model_override().or(top_level_model);
    if let Some(model) = model_override {
        let override_file = config::Config::runtime_dir().join("model_override");
        std::fs::write(&override_file, model)
            .map_err(|e| anyhow::anyhow!("Failed to write model override: {}", e))?;
    }

    // Write smart auto-submit override file if specified
    if let Some(enabled) = action.smart_auto_submit_override() {
        let override_file = config::Config::runtime_dir().join("smart_auto_submit_override");
        std::fs::write(&override_file, if enabled { "true" } else { "false" })
            .map_err(|e| anyhow::anyhow!("Failed to write smart auto-submit override: {}", e))?;
    }

    // Write profile override file if specified
    if let Some(profile_name) = action.profile() {
        // Validate that the profile exists in config
        if config.get_profile(profile_name).is_none() {
            let available = config.profile_names();
            if available.is_empty() {
                eprintln!("Error: Profile '{}' not found.", profile_name);
                eprintln!();
                eprintln!("No profiles are configured. Add profiles to your config.toml:");
                eprintln!();
                eprintln!("  [profiles.{}]", profile_name);
                eprintln!("  post_process_command = \"your-command-here\"");
            } else {
                eprintln!("Error: Profile '{}' not found.", profile_name);
                eprintln!();
                eprintln!(
                    "Available profiles: {}",
                    available
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            std::process::exit(1);
        }

        let profile_file = config::Config::runtime_dir().join("profile_override");
        std::fs::write(&profile_file, profile_name)
            .map_err(|e| anyhow::anyhow!("Failed to write profile override: {}", e))?;
    }

    // Write auto_submit override file if specified
    if let Some(value) = action.auto_submit_override() {
        let override_file = config::Config::runtime_dir().join("auto_submit_override");
        std::fs::write(&override_file, if value { "true" } else { "false" })
            .map_err(|e| anyhow::anyhow!("Failed to write auto_submit override: {}", e))?;
    }

    // Write shift_enter_newlines override file if specified
    if let Some(value) = action.shift_enter_newlines_override() {
        let override_file = config::Config::runtime_dir().join("shift_enter_override");
        std::fs::write(&override_file, if value { "true" } else { "false" })
            .map_err(|e| anyhow::anyhow!("Failed to write shift_enter override: {}", e))?;
    }

    // For toggle, we need to read current state to decide which signal to send
    let signal: libc::c_int = match &action {
        RecordAction::Start { .. } => libc::SIGUSR1,
        RecordAction::Stop { .. } => libc::SIGUSR2,
        RecordAction::Toggle { .. } => {
            // Read current state to determine action
            let state_file = match config.resolve_state_file() {
                Some(path) => path,
                None => {
                    eprintln!("Error: Cannot toggle recording without state_file configured.");
                    eprintln!();
                    eprintln!("Add to your config.toml:");
                    eprintln!("  state_file = \"auto\"");
                    eprintln!();
                    eprintln!("Or use explicit start/stop commands:");
                    eprintln!("  voxtype record start");
                    eprintln!("  voxtype record stop");
                    std::process::exit(1);
                }
            };

            let current_state =
                std::fs::read_to_string(&state_file).unwrap_or_else(|_| "idle".to_string());

            // "recording" covers the batch and eager paths. "streaming"
            // covers the Parakeet streaming path. Both are active
            // capture states whose toggle should send a stop signal,
            // not start a second session. Without this, toggling
            // during streaming silently starts a new session while
            // the original keeps running until the 60s safety
            // timeout fires — leaking audio into whatever window
            // has focus.
            let active = matches!(current_state.trim(), "recording" | "streaming");
            if active {
                libc::SIGUSR2 // Stop
            } else {
                libc::SIGUSR1 // Start
            }
        }
        RecordAction::Cancel => unreachable!(), // Handled above
    };

    let result = unsafe { libc::kill(pid, signal) };
    if result != 0 {
        return Err(anyhow::anyhow!(
            "Failed to send signal to daemon: {}",
            std::io::Error::last_os_error()
        ));
    }

    Ok(())
}
