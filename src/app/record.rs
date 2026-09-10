//! `voxtype record start|stop|toggle|cancel` — write override files for the
//! daemon and send the appropriate signal. The override files (model,
//! output_mode, profile, smart_auto_submit, auto_submit, shift_enter_newlines)
//! are intentionally separate sentinels under `runtime_dir/`; merging them
//! would invent write-race surface that doesn't exist today (see
//! `docs/REFACTORING.md`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use voxtype::daemon::result_sidecar_path;
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

    // Parakeet uses the model loaded by the daemon, so a model supplied on one
    // record command cannot take effect without restarting it. Reject before
    // writing override files that could leak into the next recording.
    let model_override = action.model_override().or(top_level_model);
    if model_override.is_some() && config.engine == config::TranscriptionEngine::Parakeet {
        anyhow::bail!(
            "Per-record model overrides are not supported for Parakeet. \
             Set [parakeet] model in config.toml and restart the daemon instead."
        );
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

    // Write the OSD suppression sentinel if --no-osd was passed. Only written,
    // never cleared here: like the other overrides the daemon consumes and
    // removes it, so a stale file cannot silence a later recording.
    if action.suppress_osd() {
        let override_file = config::Config::runtime_dir().join("no_osd_override");
        std::fs::write(&override_file, "true")
            .map_err(|e| anyhow::anyhow!("Failed to write no_osd override: {}", e))?;
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

    // With --wait, clear a previous run's completion sidecar before signalling,
    // so the wait below cannot mistake it for this recording's outcome.
    let wait_target = match &action {
        RecordAction::Stop {
            wait: true,
            wait_file,
            ..
        } => {
            let path = resolve_wait_target(config, wait_file.as_deref()).ok_or_else(|| {
                anyhow::anyhow!(
                    "--wait needs a transcript file: start the recording with --file, \
                     set output.mode = \"file\", or pass --wait-file <PATH>"
                )
            })?;
            let _ = std::fs::remove_file(result_sidecar_path(&path));
            Some(path)
        }
        _ => None,
    };

    let result = unsafe { libc::kill(pid, signal) };
    if result != 0 {
        return Err(anyhow::anyhow!(
            "Failed to send signal to daemon: {}",
            std::io::Error::last_os_error()
        ));
    }

    if let Some(transcript) = wait_target {
        let (as_json, timeout) = match &action {
            RecordAction::Stop { json, timeout, .. } => (*json, *timeout),
            _ => (false, 120),
        };
        let outcome = await_transcription(&transcript, Duration::from_secs(timeout));
        report_outcome(&outcome, as_json);
        std::process::exit(outcome.exit_code());
    }

    Ok(())
}

/// Which transcript `--wait` should block on.
///
/// An explicit `--wait-file` wins. Otherwise the pending `--file` override this
/// recording was started with is used, so the common case needs no repetition;
/// failing that, a configured file-output path.
fn resolve_wait_target(config: &config::Config, explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    let pending =
        std::fs::read_to_string(config::Config::runtime_dir().join("output_mode_override")).ok();
    if let Some(value) = pending.as_deref().map(str::trim) {
        if let Some(path) = value.strip_prefix("file:") {
            let path = path.trim();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }

    config.output.file_path.clone()
}

/// Outcome of a `--wait` stop, as reported to the caller.
struct WaitOutcome {
    status: String,
    text: String,
    message: Option<String>,
}

impl WaitOutcome {
    fn exit_code(&self) -> i32 {
        match self.status.as_str() {
            "ok" => 0,
            "empty" => 3,
            "timeout" => 4,
            _ => 1,
        }
    }
}

/// Block until the daemon publishes this recording's outcome.
///
/// The daemon writes the completion sidecar after the transcript itself, so
/// seeing the sidecar means the transcript is complete. A state file that
/// returns to idle without one is the backstop: that means the recording ended
/// down a path that produced no transcript.
fn await_transcription(transcript: &Path, timeout: Duration) -> WaitOutcome {
    const POLL: Duration = Duration::from_millis(50);
    // How long to keep looking for a sidecar after the daemon reports idle.
    const SETTLE: Duration = Duration::from_millis(750);

    let sidecar = result_sidecar_path(transcript);
    let state_file = config::Config::runtime_dir().join("state");
    let deadline = Instant::now() + timeout;
    let mut idle_since: Option<Instant> = None;

    loop {
        if let Ok(body) = std::fs::read_to_string(&sidecar) {
            let _ = std::fs::remove_file(&sidecar);
            return finish(transcript, &body);
        }

        let state = std::fs::read_to_string(&state_file)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if state == "idle" {
            match idle_since {
                Some(since) if since.elapsed() >= SETTLE => {
                    return WaitOutcome {
                        status: "empty".to_string(),
                        text: String::new(),
                        message: Some(
                            "the daemon returned to idle without producing a transcript"
                                .to_string(),
                        ),
                    };
                }
                Some(_) => {}
                None => idle_since = Some(Instant::now()),
            }
        } else {
            idle_since = None;
        }

        if Instant::now() >= deadline {
            return WaitOutcome {
                status: "timeout".to_string(),
                text: String::new(),
                message: Some(format!(
                    "no outcome within {}s (daemon state: {})",
                    timeout.as_secs(),
                    if state.is_empty() { "unknown" } else { &state }
                )),
            };
        }

        std::thread::sleep(POLL);
    }
}

/// Turn a sidecar body into an outcome, reading the transcript when there is one.
fn finish(transcript: &Path, sidecar_body: &str) -> WaitOutcome {
    let parsed: serde_json::Value = match serde_json::from_str(sidecar_body.trim()) {
        Ok(value) => value,
        Err(e) => {
            return WaitOutcome {
                status: "error".to_string(),
                text: String::new(),
                message: Some(format!("unreadable completion record: {}", e)),
            }
        }
    };

    let status = parsed
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("error")
        .to_string();
    let message = parsed
        .get("message")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let text = if status == "ok" {
        std::fs::read_to_string(transcript)
            .map(|t| t.trim_end_matches('\n').to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    WaitOutcome {
        status,
        text,
        message,
    }
}

/// Print the outcome: one JSON object, or the transcript on stdout.
fn report_outcome(outcome: &WaitOutcome, as_json: bool) {
    if as_json {
        let body = serde_json::json!({
            "status": outcome.status,
            "text": outcome.text,
            "chars": outcome.text.chars().count(),
            "message": outcome.message,
        });
        println!("{}", body);
        return;
    }

    if outcome.status == "ok" {
        println!("{}", outcome.text);
    } else if let Some(message) = &outcome.message {
        eprintln!("{}: {}", outcome.status, message);
    } else {
        eprintln!("{}", outcome.status);
    }
}
