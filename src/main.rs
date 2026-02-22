//! Voxtype - Push-to-talk voice-to-text for Linux
//!
//! Run with `voxtype` or `voxtype daemon` to start the daemon.
//! Use `voxtype setup` to check dependencies and download models.
//! Use `voxtype transcribe <file>` to transcribe an audio file.

use clap::Parser;
use std::path::PathBuf;
use std::process::Command;
use tracing_subscriber::EnvFilter;
use voxtype::{
    config, cpu, daemon, meeting, setup, transcribe, vad, Cli, Commands, MeetingAction,
    RecordAction, SetupAction,
};

/// Parse a comma-separated list of driver names into OutputDriver vec
fn parse_driver_order(s: &str) -> Result<Vec<config::OutputDriver>, String> {
    s.split(',')
        .map(|d| d.trim().parse::<config::OutputDriver>())
        .collect()
}

/// Check if running as root and warn for commands that don't need elevated privileges.
/// Returns true if running as root.
fn warn_if_root(command_name: &str) -> bool {
    // SAFETY: getuid() is always safe to call
    let is_root = unsafe { libc::getuid() } == 0;
    if is_root {
        eprintln!(
            "Warning: Running 'voxtype setup {}' as root is not recommended.",
            command_name
        );
        eprintln!("  - Models will download to /root/.local/share/voxtype/ instead of your user directory");
        eprintln!(
            "  - Config changes will apply to /root/.config/voxtype/ instead of your user config"
        );
        eprintln!("  - Cannot restart your user's voxtype daemon from root");
        eprintln!();
        eprintln!("Run without sudo: voxtype setup {}", command_name);
        eprintln!();
    }
    is_root
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Install SIGILL handler early to catch illegal instruction crashes
    // and provide a helpful error message instead of core dumping
    cpu::install_sigill_handler();

    // Reset SIGPIPE to default behavior (terminate silently) to avoid panics
    // when output is piped through commands like `head` that close the pipe early
    reset_sigpipe();

    let cli = Cli::parse();

    // Check if this is the worker command (needs stderr-only logging)
    let is_worker = matches!(cli.command, Some(Commands::TranscribeWorker { .. }));

    // Initialize logging
    let log_level = if cli.quiet {
        "error"
    } else {
        match cli.verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };

    if is_worker {
        // Worker uses stderr for logging (stdout is reserved for IPC protocol)
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new(format!("voxtype={},warn", log_level))),
            )
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new(format!("voxtype={},warn", log_level))),
            )
            .with_target(false)
            .init();
    }

    // Load configuration
    let config_path = cli.config.clone().or_else(config::Config::default_path);
    let mut config = config::load_config(cli.config.as_deref())?;

    // Apply CLI overrides
    if cli.clipboard {
        config.output.mode = config::OutputMode::Clipboard;
    }
    if cli.paste {
        config.output.mode = config::OutputMode::Paste;
    }
    if cli.restore_clipboard {
        config.output.restore_clipboard = true;
    }
    if let Some(delay) = cli.restore_clipboard_delay_ms {
        config.output.restore_clipboard_delay_ms = delay;
    }
    let top_level_model = cli.model.clone();
    if let Some(model) = cli.model {
        if setup::model::is_valid_model(&model) {
            config.whisper.model = model;
        } else {
            let default_model = &config.whisper.model;
            tracing::warn!(
                "Unknown model '{}', using default model '{}'",
                model,
                default_model
            );
            let _ = Command::new("notify-send")
                .args([
                    "--app-name=Voxtype",
                    "--expire-time=5000",
                    "Voxtype: Invalid Model",
                    &format!("Unknown model '{}', using '{}'", model, default_model),
                ])
                .spawn();
        }
    }
    if let Some(engine) = cli.engine {
        match engine.to_lowercase().as_str() {
            "whisper" => config.engine = config::TranscriptionEngine::Whisper,
            "parakeet" => config.engine = config::TranscriptionEngine::Parakeet,
            "moonshine" => config.engine = config::TranscriptionEngine::Moonshine,
            "sensevoice" => config.engine = config::TranscriptionEngine::SenseVoice,
            "paraformer" => config.engine = config::TranscriptionEngine::Paraformer,
            "dolphin" => config.engine = config::TranscriptionEngine::Dolphin,
            "omnilingual" => config.engine = config::TranscriptionEngine::Omnilingual,
            _ => {
                eprintln!(
                    "Error: Invalid engine '{}'. Valid options: whisper, parakeet, moonshine, sensevoice, paraformer, dolphin, omnilingual",
                    engine
                );
                std::process::exit(1);
            }
        }
    }

    // Hotkey overrides
    if let Some(hotkey) = cli.hotkey {
        config.hotkey.key = hotkey;
    }
    if cli.toggle {
        config.hotkey.mode = config::ActivationMode::Toggle;
    }
    if cli.no_hotkey {
        config.hotkey.enabled = false;
    }
    if let Some(cancel_key) = cli.cancel_key {
        config.hotkey.cancel_key = Some(cancel_key);
    }
    if let Some(model_modifier) = cli.model_modifier {
        config.hotkey.model_modifier = Some(model_modifier);
    }

    // Whisper overrides
    if let Some(delay) = cli.pre_type_delay {
        config.output.pre_type_delay_ms = delay;
    }
    if let Some(delay) = cli.wtype_delay {
        tracing::warn!("--wtype-delay is deprecated, use --pre-type-delay instead");
        config.output.pre_type_delay_ms = delay;
    }
    if cli.no_whisper_context_optimization {
        config.whisper.context_window_optimization = false;
    }
    if let Some(prompt) = cli.initial_prompt {
        config.whisper.initial_prompt = Some(prompt);
    }
    if let Some(lang) = cli.language {
        config.whisper.language = config::LanguageConfig::from_comma_separated(&lang);
    }
    if cli.translate {
        config.whisper.translate = true;
    }
    if let Some(threads) = cli.threads {
        config.whisper.threads = Some(threads);
    }
    if cli.gpu_isolation {
        config.whisper.gpu_isolation = true;
    }
    if cli.on_demand_loading {
        config.whisper.on_demand_loading = true;
    }
    if let Some(ref mode) = cli.whisper_mode {
        match mode.to_lowercase().as_str() {
            "local" => config.whisper.mode = Some(config::WhisperMode::Local),
            "remote" => config.whisper.mode = Some(config::WhisperMode::Remote),
            "cli" => config.whisper.mode = Some(config::WhisperMode::Cli),
            _ => {
                eprintln!(
                    "Error: Invalid whisper mode '{}'. Valid options: local, remote, cli",
                    mode
                );
                std::process::exit(1);
            }
        }
    }
    if let Some(model) = cli.secondary_model {
        config.whisper.secondary_model = Some(model);
    }
    if cli.eager_processing {
        config.whisper.eager_processing = true;
    }
    if let Some(endpoint) = cli.remote_endpoint {
        config.whisper.remote_endpoint = Some(endpoint);
    }
    if let Some(model) = cli.remote_model {
        config.whisper.remote_model = Some(model);
    }
    if let Some(key) = cli.remote_api_key {
        config.whisper.remote_api_key = Some(key);
    }

    // Audio overrides
    if let Some(device) = cli.audio_device {
        config.audio.device = device;
    }
    if let Some(max_dur) = cli.max_duration {
        config.audio.max_duration_secs = max_dur;
    }
    if cli.audio_feedback {
        config.audio.feedback.enabled = true;
    }
    if cli.no_audio_feedback {
        config.audio.feedback.enabled = false;
    }

    // Output overrides
    if let Some(append_text) = cli.append_text {
        config.output.append_text = Some(append_text);
    }
    if let Some(ref driver_str) = cli.driver {
        match parse_driver_order(driver_str) {
            Ok(drivers) => {
                config.output.driver_order = Some(drivers);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
    if cli.auto_submit {
        config.output.auto_submit = true;
    }
    if cli.no_auto_submit {
        config.output.auto_submit = false;
    }
    if cli.shift_enter_newlines {
        config.output.shift_enter_newlines = true;
    }
    if cli.no_shift_enter_newlines {
        config.output.shift_enter_newlines = false;
    }
    if let Some(delay) = cli.type_delay {
        config.output.type_delay_ms = delay;
    }
    if cli.fallback_to_clipboard {
        config.output.fallback_to_clipboard = true;
    }
    if cli.no_fallback_to_clipboard {
        config.output.fallback_to_clipboard = false;
    }
    if cli.spoken_punctuation {
        config.text.spoken_punctuation = true;
    }
    if let Some(keys) = cli.paste_keys {
        config.output.paste_keys = Some(keys);
    }
    if let Some(layout) = cli.dotool_xkb_layout {
        config.output.dotool_xkb_layout = Some(layout);
    }
    if let Some(variant) = cli.dotool_xkb_variant {
        config.output.dotool_xkb_variant = Some(variant);
    }
    if let Some(path) = cli.file_path {
        config.output.file_path = Some(path);
    }
    if let Some(ref mode) = cli.file_mode {
        match mode.to_lowercase().as_str() {
            "overwrite" => config.output.file_mode = config::FileMode::Overwrite,
            "append" => config.output.file_mode = config::FileMode::Append,
            _ => {
                eprintln!(
                    "Error: Invalid file mode '{}'. Valid options: overwrite, append",
                    mode
                );
                std::process::exit(1);
            }
        }
    }
    if let Some(cmd) = cli.pre_output_command {
        config.output.pre_output_command = Some(cmd);
    }
    if let Some(cmd) = cli.post_output_command {
        config.output.post_output_command = Some(cmd);
    }
    if let Some(cmd) = cli.pre_recording_command {
        config.output.pre_recording_command = Some(cmd);
    }

    // VAD overrides
    if cli.vad {
        config.vad.enabled = true;
    }
    if let Some(threshold) = cli.vad_threshold {
        config.vad.threshold = threshold.clamp(0.0, 1.0);
    }
    if let Some(ref backend) = cli.vad_backend {
        config.vad.backend = match backend.to_lowercase().as_str() {
            "auto" => config::VadBackend::Auto,
            "energy" => config::VadBackend::Energy,
            "whisper" => config::VadBackend::Whisper,
            _ => {
                eprintln!(
                    "Unknown VAD backend '{}'. Valid options: auto, energy, whisper",
                    backend
                );
                std::process::exit(1);
            }
        };
    }
    if let Some(min_speech) = cli.vad_min_speech_ms {
        config.vad.min_speech_duration_ms = min_speech;
    }

    // Run the appropriate command
    match cli.command.unwrap_or(Commands::Daemon) {
        Commands::Daemon => {
            let mut daemon = daemon::Daemon::new(config, config_path);
            daemon.run().await?;
        }

        Commands::Transcribe { file, engine } => {
            if let Some(engine_name) = engine {
                match engine_name.to_lowercase().as_str() {
                    "whisper" => config.engine = config::TranscriptionEngine::Whisper,
                    "parakeet" => config.engine = config::TranscriptionEngine::Parakeet,
                    "moonshine" => config.engine = config::TranscriptionEngine::Moonshine,
                    "sensevoice" => config.engine = config::TranscriptionEngine::SenseVoice,
                    "paraformer" => config.engine = config::TranscriptionEngine::Paraformer,
                    "dolphin" => config.engine = config::TranscriptionEngine::Dolphin,
                    "omnilingual" => config.engine = config::TranscriptionEngine::Omnilingual,
                    _ => {
                        eprintln!("Error: Invalid engine '{}'. Valid options: whisper, parakeet, moonshine, sensevoice, paraformer, dolphin, omnilingual", engine_name);
                        std::process::exit(1);
                    }
                }
            }
            transcribe_file(&config, &file)?;
        }

        Commands::TranscribeWorker {
            model,
            language,
            translate,
            threads,
        } => {
            // Internal command: run transcription worker process
            // This is spawned by the daemon when gpu_isolation is enabled
            // Use command-line overrides if provided, otherwise use config
            let mut whisper_config = config.whisper.clone();
            if let Some(m) = model {
                whisper_config.model = m;
            }
            if let Some(l) = language {
                // Parse comma-separated language string back to LanguageConfig
                whisper_config.language = config::LanguageConfig::from_comma_separated(&l);
            }
            if translate {
                whisper_config.translate = true;
            }
            if let Some(t) = threads {
                whisper_config.threads = Some(t);
            }
            transcribe::worker::run_worker(&whisper_config)?;
        }

        Commands::Setup {
            action,
            download,
            model,
            quiet,
            no_post_install,
        } => {
            match action {
                Some(SetupAction::Check) => {
                    warn_if_root("check");
                    setup::run_checks(&config).await?;
                }
                Some(SetupAction::Systemd { uninstall, status }) => {
                    warn_if_root("systemd");
                    if status {
                        setup::systemd::status().await?;
                    } else if uninstall {
                        setup::systemd::uninstall().await?;
                    } else {
                        setup::systemd::install().await?;
                    }
                }
                Some(SetupAction::Waybar {
                    json,
                    css,
                    install,
                    uninstall,
                }) => {
                    warn_if_root("waybar");
                    if install {
                        setup::waybar::install()?;
                    } else if uninstall {
                        setup::waybar::uninstall()?;
                    } else if json {
                        println!("{}", setup::waybar::get_json_config());
                    } else if css {
                        println!("{}", setup::waybar::get_css_config());
                    } else {
                        setup::waybar::print_config();
                    }
                }
                Some(SetupAction::Dms {
                    install,
                    uninstall,
                    qml,
                }) => {
                    warn_if_root("dms");
                    if install {
                        setup::dms::install()?;
                    } else if uninstall {
                        setup::dms::uninstall()?;
                    } else if qml {
                        println!("{}", setup::dms::get_qml_config());
                    } else {
                        setup::dms::print_config();
                    }
                }
                Some(SetupAction::Model { list, set, restart }) => {
                    warn_if_root("model");
                    if list {
                        setup::model::list_installed();
                    } else if let Some(model_name) = set {
                        setup::model::set_model(&model_name, restart).await?;
                    } else {
                        setup::model::interactive_select().await?;
                    }
                }
                Some(SetupAction::Gpu {
                    enable,
                    disable,
                    status,
                }) => {
                    if status {
                        setup::gpu::show_status();
                    } else if enable {
                        setup::gpu::enable()?;
                    } else if disable {
                        setup::gpu::disable()?;
                    } else {
                        // Default: show status
                        setup::gpu::show_status();
                    }
                }
                Some(SetupAction::Onnx {
                    enable,
                    disable,
                    status,
                })
                | Some(SetupAction::Parakeet {
                    enable,
                    disable,
                    status,
                }) => {
                    warn_if_root("onnx");
                    if status {
                        setup::parakeet::show_status();
                    } else if enable {
                        setup::parakeet::enable()?;
                    } else if disable {
                        setup::parakeet::disable()?;
                    } else {
                        // Default: show status
                        setup::parakeet::show_status();
                    }
                }
                Some(SetupAction::Compositor { compositor_type }) => {
                    warn_if_root("compositor");
                    setup::compositor::run(&compositor_type).await?;
                }
                Some(SetupAction::Vad { status }) => {
                    warn_if_root("vad");
                    if status {
                        setup::vad::show_status();
                    } else {
                        setup::vad::download_model()?;
                    }
                }
                None => {
                    // Default: run setup (non-blocking)
                    warn_if_root("");
                    setup::run_setup(&config, download, model.as_deref(), quiet, no_post_install)
                        .await?;
                }
            }
        }

        Commands::Config => {
            show_config(&config).await?;
        }

        Commands::Status {
            follow,
            format,
            extended,
            icon_theme,
        } => {
            run_status(&config, follow, &format, extended, icon_theme).await?;
        }

        Commands::Record { action } => {
            send_record_command(&config, action, top_level_model.as_deref())?;
        }

        Commands::Meeting { action } => {
            run_meeting_command(&config, action).await?;
        }
    }

    Ok(())
}

/// Check if the daemon is running, exit with error if not
fn check_daemon_running() -> anyhow::Result<()> {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let pid_file = config::Config::runtime_dir().join("pid");

    if !pid_file.exists() {
        eprintln!("Error: Voxtype daemon is not running.");
        eprintln!("Start it with: voxtype daemon");
        std::process::exit(1);
    }

    let pid_str = std::fs::read_to_string(&pid_file)
        .map_err(|e| anyhow::anyhow!("Failed to read PID file: {}", e))?;

    let pid: i32 = pid_str
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid PID in file: {}", e))?;

    // Check if the process is actually running
    if kill(Pid::from_raw(pid), None).is_err() {
        // Process doesn't exist, clean up stale PID file
        let _ = std::fs::remove_file(&pid_file);
        eprintln!("Error: Voxtype daemon is not running (stale PID file removed).");
        eprintln!("Start it with: voxtype daemon");
        std::process::exit(1);
    }

    Ok(())
}

/// Send a record command to the running daemon via Unix signals or file triggers
fn send_record_command(
    config: &config::Config,
    action: RecordAction,
    top_level_model: Option<&str>,
) -> anyhow::Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    use voxtype::OutputModeOverride;

    // Read PID from the pid file
    let pid_file = config::Config::runtime_dir().join("pid");

    if !pid_file.exists() {
        eprintln!("Error: Voxtype daemon is not running.");
        eprintln!("Start it with: voxtype daemon");
        std::process::exit(1);
    }

    let pid_str = std::fs::read_to_string(&pid_file)
        .map_err(|e| anyhow::anyhow!("Failed to read PID file: {}", e))?;

    let pid: i32 = pid_str
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid PID in file: {}", e))?;

    // Check if the process is actually running
    if kill(Pid::from_raw(pid), None).is_err() {
        // Process doesn't exist, clean up stale PID file
        let _ = std::fs::remove_file(&pid_file);
        eprintln!("Error: Voxtype daemon is not running (stale PID file removed).");
        eprintln!("Start it with: voxtype daemon");
        std::process::exit(1);
    }

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
    let signal = match &action {
        RecordAction::Start { .. } => Signal::SIGUSR1,
        RecordAction::Stop { .. } => Signal::SIGUSR2,
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

            if current_state.trim() == "recording" {
                Signal::SIGUSR2 // Stop
            } else {
                Signal::SIGUSR1 // Start
            }
        }
        RecordAction::Cancel => unreachable!(), // Handled above
    };

    kill(Pid::from_raw(pid), signal)
        .map_err(|e| anyhow::anyhow!("Failed to send signal to daemon: {}", e))?;

    Ok(())
}

/// Transcribe an audio file
fn transcribe_file(config: &config::Config, path: &PathBuf) -> anyhow::Result<()> {
    use hound::WavReader;

    println!("Loading audio file: {:?}", path);

    let reader = WavReader::open(path)?;
    let spec = reader.spec();

    println!(
        "Audio format: {} Hz, {} channel(s), {:?}",
        spec.sample_rate, spec.channels, spec.sample_format
    );

    // Convert samples to f32 mono at 16kHz
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_val = (1 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / max_val)
                .collect()
        }
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .filter_map(|s| s.ok())
            .collect(),
    };

    // Mix to mono if stereo
    let mono_samples: Vec<f32> = if spec.channels > 1 {
        samples
            .chunks(spec.channels as usize)
            .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
            .collect()
    } else {
        samples
    };

    // Resample to 16kHz if needed
    let final_samples = if spec.sample_rate != 16000 {
        println!("Resampling from {} Hz to 16000 Hz...", spec.sample_rate);
        resample(&mono_samples, spec.sample_rate, 16000)
    } else {
        mono_samples
    };

    println!(
        "Processing {} samples ({:.2}s)...",
        final_samples.len(),
        final_samples.len() as f32 / 16000.0
    );

    // Run VAD if enabled
    if let Ok(Some(vad)) = vad::create_vad(config) {
        match vad.detect(&final_samples) {
            Ok(result) => {
                println!(
                    "VAD: {:.2}s speech ({:.1}% of audio)",
                    result.speech_duration_secs,
                    result.speech_ratio * 100.0
                );
                if !result.has_speech {
                    println!("No speech detected, skipping transcription.");
                    return Ok(());
                }
            }
            Err(e) => {
                eprintln!("VAD warning: {}", e);
                // Continue with transcription if VAD fails
            }
        }
    }

    // Create transcriber and transcribe
    let transcriber = transcribe::create_transcriber(config)?;
    let text = transcriber.transcribe(&final_samples)?;

    println!("\n{}", text);
    Ok(())
}

/// Simple linear resampling
fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = (samples.len() as f64 * ratio).ceil() as usize;
    let mut output = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_idx = i as f64 / ratio;
        let idx = src_idx.floor() as usize;
        let frac = (src_idx - idx as f64) as f32;

        let sample = if idx + 1 < samples.len() {
            samples[idx] * (1.0 - frac) + samples[idx + 1] * frac
        } else {
            samples.get(idx).copied().unwrap_or(0.0)
        };

        output.push(sample);
    }

    output
}

/// Extended status info for JSON output
struct ExtendedStatusInfo {
    model: String,
    device: String,
    backend: String,
}

impl ExtendedStatusInfo {
    fn from_config(config: &config::Config) -> Self {
        let backend = setup::gpu::detect_current_backend()
            .map(|b| match b {
                setup::gpu::Backend::Cpu => "CPU (legacy)",
                setup::gpu::Backend::Native => "CPU (native)",
                setup::gpu::Backend::Avx2 => "CPU (AVX2)",
                setup::gpu::Backend::Avx512 => "CPU (AVX-512)",
                setup::gpu::Backend::Vulkan => "GPU (Vulkan)",
            })
            .unwrap_or("unknown")
            .to_string();

        Self {
            model: config.whisper.model.clone(),
            device: config.audio.device.clone(),
            backend,
        }
    }
}

/// Check if the daemon is actually running by verifying the PID file
fn is_daemon_running() -> bool {
    let pid_path = config::Config::runtime_dir().join("pid");

    // Read PID from file
    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => return false, // No PID file = not running
    };

    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => return false, // Invalid PID = not running
    };

    // Check if process exists by testing /proc/{pid}
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}

/// Run the status command - show current daemon state
async fn run_status(
    config: &config::Config,
    follow: bool,
    format: &str,
    extended: bool,
    icon_theme_override: Option<String>,
) -> anyhow::Result<()> {
    let state_file = config.resolve_state_file();

    if state_file.is_none() {
        eprintln!("Error: state_file is not configured.");
        eprintln!();
        eprintln!("To enable status monitoring, add to your config.toml:");
        eprintln!();
        eprintln!("  state_file = \"auto\"");
        eprintln!();
        eprintln!("This enables external integrations like Waybar to monitor voxtype state.");
        std::process::exit(1);
    }

    let state_path = state_file.unwrap();
    let ext_info = if extended {
        Some(ExtendedStatusInfo::from_config(config))
    } else {
        None
    };

    // Use CLI override if provided, otherwise use config
    let icons = if let Some(ref theme) = icon_theme_override {
        let mut status_config = config.status.clone();
        status_config.icon_theme = theme.clone();
        status_config.resolve_icons()
    } else {
        config.status.resolve_icons()
    };

    if !follow {
        // One-shot: just read and print current state
        // First check if daemon is actually running to avoid stale state
        let state = if !is_daemon_running() {
            "stopped".to_string()
        } else {
            std::fs::read_to_string(&state_path).unwrap_or_else(|_| "stopped".to_string())
        };
        let state = state.trim();

        if format == "json" {
            println!("{}", format_state_json(state, &icons, ext_info.as_ref()));
        } else {
            println!("{}", state);
        }
        return Ok(());
    }

    // Follow mode: watch for changes using inotify
    use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    // Print initial state (check if daemon is running to avoid stale state)
    let state = if !is_daemon_running() {
        "stopped".to_string()
    } else {
        std::fs::read_to_string(&state_path).unwrap_or_else(|_| "stopped".to_string())
    };
    let state = state.trim();
    if format == "json" {
        println!("{}", format_state_json(state, &icons, ext_info.as_ref()));
    } else {
        println!("{}", state);
    }

    // Set up file watcher
    let (tx, rx) = channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        NotifyConfig::default().with_poll_interval(Duration::from_millis(100)),
    )?;

    // Watch the state file's parent directory (file may not exist yet)
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent)?;
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
    }

    // Also try to watch the file directly if it exists
    if state_path.exists() {
        let _ = watcher.watch(&state_path, RecursiveMode::NonRecursive);
    }

    let mut last_state = state.to_string();

    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(_event)) => {
                // File changed, read new state
                if let Ok(new_state) = std::fs::read_to_string(&state_path) {
                    let new_state = new_state.trim().to_string();
                    if new_state != last_state {
                        if format == "json" {
                            println!(
                                "{}",
                                format_state_json(&new_state, &icons, ext_info.as_ref())
                            );
                        } else {
                            println!("{}", new_state);
                        }
                        last_state = new_state;
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("Watch error: {:?}", e);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Check if daemon stopped (file deleted or process died)
                if (!state_path.exists() || !is_daemon_running()) && last_state != "stopped" {
                    if format == "json" {
                        println!(
                            "{}",
                            format_state_json("stopped", &icons, ext_info.as_ref())
                        );
                    } else {
                        println!("stopped");
                    }
                    last_state = "stopped".to_string();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }

    Ok(())
}

/// Format state as JSON for Waybar consumption
/// The `alt` field enables Waybar's format-icons feature for custom icon mapping
fn format_state_json(
    state: &str,
    icons: &config::ResolvedIcons,
    extended: Option<&ExtendedStatusInfo>,
) -> String {
    let (text, base_tooltip) = match state {
        "recording" => (&icons.recording, "Recording..."),
        "transcribing" => (&icons.transcribing, "Transcribing..."),
        "idle" => (&icons.idle, "Voxtype ready - hold hotkey to record"),
        "stopped" => (&icons.stopped, "Voxtype not running"),
        _ => (&icons.idle, "Unknown state"),
    };

    // alt = state name (for Waybar format-icons mapping)
    // class = state name (for CSS styling)
    let alt = state;
    let class = state;

    match extended {
        Some(info) => {
            // Extended format includes model, device, backend
            let tooltip = format!(
                "{}\\nModel: {}\\nDevice: {}\\nBackend: {}",
                base_tooltip, info.model, info.device, info.backend
            );
            format!(
                r#"{{"text": "{}", "alt": "{}", "class": "{}", "tooltip": "{}", "model": "{}", "device": "{}", "backend": "{}"}}"#,
                text, alt, class, tooltip, info.model, info.device, info.backend
            )
        }
        None => {
            format!(
                r#"{{"text": "{}", "alt": "{}", "class": "{}", "tooltip": "{}"}}"#,
                text, alt, class, base_tooltip
            )
        }
    }
}

/// Show current configuration
async fn show_config(config: &config::Config) -> anyhow::Result<()> {
    println!("Current Configuration\n");
    println!("=====================\n");

    println!("[hotkey]");
    println!("  key = {:?}", config.hotkey.key);
    println!("  modifiers = {:?}", config.hotkey.modifiers);
    println!("  mode = {:?}", config.hotkey.mode);

    println!("\n[audio]");
    println!("  device = {:?}", config.audio.device);
    println!("  sample_rate = {}", config.audio.sample_rate);
    println!("  max_duration_secs = {}", config.audio.max_duration_secs);

    println!("\n[audio.feedback]");
    println!("  enabled = {}", config.audio.feedback.enabled);
    println!("  theme = {:?}", config.audio.feedback.theme);
    println!("  volume = {}", config.audio.feedback.volume);

    // Show current engine
    println!("\n[engine]");
    println!("  engine = {:?}", config.engine);

    println!("\n[whisper]");
    println!("  model = {:?}", config.whisper.model);
    println!("  language = {:?}", config.whisper.language);
    println!("  translate = {}", config.whisper.translate);
    if let Some(threads) = config.whisper.threads {
        println!("  threads = {}", threads);
    }

    // Show Parakeet status (experimental)
    println!("\n[parakeet] (EXPERIMENTAL)");
    if let Some(ref parakeet_config) = config.parakeet {
        println!("  model = {:?}", parakeet_config.model);
        if let Some(ref model_type) = parakeet_config.model_type {
            println!("  model_type = {:?}", model_type);
        }
        println!(
            "  on_demand_loading = {}",
            parakeet_config.on_demand_loading
        );
    } else {
        println!("  (not configured)");
    }

    // Check for available Parakeet models
    let models_dir = config::Config::models_dir();
    let mut parakeet_models: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("parakeet") {
                    // Check if it has the required ONNX files
                    let has_encoder = path.join("encoder-model.onnx").exists();
                    let has_decoder = path.join("decoder_joint-model.onnx").exists()
                        || path.join("model.onnx").exists();
                    if has_encoder || has_decoder {
                        parakeet_models.push(name);
                    }
                }
            }
        }
    }
    if parakeet_models.is_empty() {
        println!("  available models: (none found)");
    } else {
        println!("  available models: {}", parakeet_models.join(", "));
    }

    // Show Moonshine status (experimental)
    println!("\n[moonshine] (EXPERIMENTAL)");
    if let Some(ref moonshine_config) = config.moonshine {
        println!("  model = {:?}", moonshine_config.model);
        println!("  quantized = {}", moonshine_config.quantized);
        if let Some(threads) = moonshine_config.threads {
            println!("  threads = {}", threads);
        }
        println!(
            "  on_demand_loading = {}",
            moonshine_config.on_demand_loading
        );
    } else {
        println!("  (not configured)");
    }

    // Check for available Moonshine models
    let mut moonshine_models: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("moonshine") {
                    let has_encoder = path.join("encoder_model.onnx").exists()
                        || path.join("encoder_model_quantized.onnx").exists();
                    let has_decoder = path.join("decoder_model_merged.onnx").exists()
                        || path.join("decoder_model_merged_quantized.onnx").exists();
                    if has_encoder || has_decoder {
                        moonshine_models.push(name);
                    }
                }
            }
        }
    }
    if moonshine_models.is_empty() {
        println!("  available models: (none found)");
    } else {
        println!("  available models: {}", moonshine_models.join(", "));
    }

    // Show SenseVoice status (experimental)
    println!("\n[sensevoice] (EXPERIMENTAL)");
    if let Some(ref sensevoice_config) = config.sensevoice {
        println!("  model = {:?}", sensevoice_config.model);
        println!("  language = {:?}", sensevoice_config.language);
        println!("  use_itn = {}", sensevoice_config.use_itn);
        if let Some(threads) = sensevoice_config.threads {
            println!("  threads = {}", threads);
        }
        println!(
            "  on_demand_loading = {}",
            sensevoice_config.on_demand_loading
        );
    } else {
        println!("  (not configured)");
    }

    // Check for available SenseVoice models
    let mut sensevoice_models: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("sensevoice") {
                    let has_model = path.join("model.int8.onnx").exists()
                        || path.join("model.onnx").exists();
                    let has_tokens = path.join("tokens.txt").exists();
                    if has_model && has_tokens {
                        sensevoice_models.push(name);
                    }
                }
            }
        }
    }
    if sensevoice_models.is_empty() {
        println!("  available models: (none found)");
    } else {
        println!("  available models: {}", sensevoice_models.join(", "));
    }

    println!("\n[output]");
    println!("  mode = {:?}", config.output.mode);
    println!(
        "  fallback_to_clipboard = {}",
        config.output.fallback_to_clipboard
    );
    if let Some(ref driver_order) = config.output.driver_order {
        println!(
            "  driver_order = [{}]",
            driver_order
                .iter()
                .map(|d| format!("{:?}", d))
                .collect::<Vec<_>>()
                .join(", ")
        );
    } else {
        println!("  driver_order = (default: wtype -> dotool -> ydotool -> clipboard)");
    }
    println!("  type_delay_ms = {}", config.output.type_delay_ms);
    println!("  pre_type_delay_ms = {}", config.output.pre_type_delay_ms);
    println!("  restore_clipboard = {}", config.output.restore_clipboard);
    println!(
        "  restore_clipboard_delay_ms = {}",
        config.output.restore_clipboard_delay_ms
    );

    println!("\n[output.notification]");
    println!(
        "  on_recording_start = {}",
        config.output.notification.on_recording_start
    );
    println!(
        "  on_recording_stop = {}",
        config.output.notification.on_recording_stop
    );
    println!(
        "  on_transcription = {}",
        config.output.notification.on_transcription
    );

    println!("\n[status]");
    println!("  icon_theme = {:?}", config.status.icon_theme);
    let icons = config.status.resolve_icons();
    println!(
        "  (resolved icons: idle={:?} recording={:?} transcribing={:?} stopped={:?})",
        icons.idle, icons.recording, icons.transcribing, icons.stopped
    );

    if let Some(ref state_file) = config.state_file {
        println!("\n[integration]");
        println!("  state_file = {:?}", state_file);
        if let Some(resolved) = config.resolve_state_file() {
            println!("  (resolves to: {:?})", resolved);
        }
    }

    // Show output chain status
    let output_status = setup::detect_output_chain().await;
    setup::print_output_chain_status(&output_status);

    println!("\n---");
    println!(
        "Config file: {:?}",
        config::Config::default_path().unwrap_or_else(|| PathBuf::from("(not found)"))
    );
    println!("Models dir: {:?}", config::Config::models_dir());

    Ok(())
}

/// Reset SIGPIPE to default behavior (terminate process) instead of the Rust
/// default of ignoring it. This prevents panics when stdout is piped through
/// commands like `head` that close the pipe early.
#[cfg(unix)]
fn reset_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {
    // No-op on non-Unix platforms
}

/// Run a meeting command
async fn run_meeting_command(config: &config::Config, action: MeetingAction) -> anyhow::Result<()> {
    use meeting::{ExportFormat, ExportOptions, MeetingConfig, StorageConfig};

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
    };

    match action {
        MeetingAction::Start { title } => {
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

            // Ensure GTCRN speech enhancement model is available
            setup::model::ensure_gtcrn_model();

            // Write start trigger file (with optional title)
            let start_file = config::Config::runtime_dir().join("meeting_start");
            let content = title.unwrap_or_default();
            std::fs::write(&start_file, content)?;

            println!("Meeting start requested. Check status with 'voxtype meeting status'.");
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

            match meeting::export_meeting_by_id(
                &meeting_config,
                &meeting_id,
                export_format,
                &options,
            ) {
                Ok(content) => {
                    if let Some(path) = output {
                        std::fs::write(&path, &content)?;
                        println!("Exported to {:?}", path);
                    } else {
                        println!("{}", content);
                    }
                }
                Err(e) => {
                    eprintln!("Error exporting meeting: {}", e);
                    std::process::exit(1);
                }
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
