//! Voxtype - Push-to-talk voice-to-text for Linux
//!
//! Run with `voxtype` or `voxtype daemon` to start the daemon.
//! Use `voxtype setup` to check dependencies and download models.
//! Use `voxtype transcribe <file>` to transcribe an audio file.

mod audio;
mod cli;
mod config;
mod daemon;
mod error;
mod hotkey;
mod output;
mod setup;
mod state;
mod text;
mod transcribe;

use clap::Parser;
use cli::{Cli, Commands, RecordAction, SetupAction};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

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

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(format!("voxtype={},warn", log_level))),
        )
        .with_target(false)
        .init();

    // Load configuration
    let mut config = config::load_config(cli.config.as_deref())?;

    // Apply CLI overrides
    if cli.clipboard {
        config.output.mode = config::OutputMode::Clipboard;
    }
    if cli.paste {
        config.output.mode = config::OutputMode::Paste;
    }
    if let Some(model) = cli.model {
        config.whisper.model = model;
    }
    if let Some(hotkey) = cli.hotkey {
        config.hotkey.key = hotkey;
    }
    if cli.toggle {
        config.hotkey.mode = config::ActivationMode::Toggle;
    }

    // Run the appropriate command
    match cli.command.unwrap_or(Commands::Daemon) {
        Commands::Daemon => {
            let mut daemon = daemon::Daemon::new(config);
            daemon.run().await?;
        }

        Commands::Transcribe { file } => {
            transcribe_file(&config, &file)?;
        }

        Commands::Setup { action, download } => {
            match action {
                Some(SetupAction::Systemd { uninstall, status }) => {
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
                Some(SetupAction::Model { list }) => {
                    if list {
                        setup::model::list_installed();
                    } else {
                        setup::model::interactive_select().await?;
                    }
                }
                Some(SetupAction::Gpu { enable, disable, status }) => {
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
                None => {
                    // Default: run basic setup (backwards compatible)
                    setup::run_basic_setup(&config, download).await?;
                }
            }
        }

        Commands::Config => {
            show_config(&config).await?;
        }

        Commands::Status { follow, format, extended, icon_theme } => {
            run_status(&config, follow, &format, extended, icon_theme).await?;
        }

        Commands::Record { action } => {
            send_record_command(&config, action)?;
        }
    }

    Ok(())
}

/// Send a record command to the running daemon via Unix signals
fn send_record_command(config: &config::Config, action: RecordAction) -> anyhow::Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

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

    // For toggle, we need to read current state to decide which signal to send
    let signal = match action {
        RecordAction::Start => Signal::SIGUSR1,
        RecordAction::Stop => Signal::SIGUSR2,
        RecordAction::Toggle => {
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

            let current_state = std::fs::read_to_string(&state_file)
                .unwrap_or_else(|_| "idle".to_string());

            if current_state.trim() == "recording" {
                Signal::SIGUSR2 // Stop
            } else {
                Signal::SIGUSR1 // Start
            }
        }
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
        println!(
            "Resampling from {} Hz to 16000 Hz...",
            spec.sample_rate
        );
        resample(&mono_samples, spec.sample_rate, 16000)
    } else {
        mono_samples
    };

    println!(
        "Processing {} samples ({:.2}s)...",
        final_samples.len(),
        final_samples.len() as f32 / 16000.0
    );

    // Create transcriber and transcribe
    let transcriber = transcribe::create_transcriber(&config.whisper)?;
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
                setup::gpu::Backend::Cpu => "CPU (native)",
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
                            println!("{}", format_state_json(&new_state, &icons, ext_info.as_ref()));
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
                        println!("{}", format_state_json("stopped", &icons, ext_info.as_ref()));
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

    println!("\n[whisper]");
    println!("  model = {:?}", config.whisper.model);
    println!("  language = {:?}", config.whisper.language);
    println!("  translate = {}", config.whisper.translate);
    if let Some(threads) = config.whisper.threads {
        println!("  threads = {}", threads);
    }

    println!("\n[output]");
    println!("  mode = {:?}", config.output.mode);
    println!(
        "  fallback_to_clipboard = {}",
        config.output.fallback_to_clipboard
    );
    println!("  type_delay_ms = {}", config.output.type_delay_ms);

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
    println!("  (resolved icons: idle={:?} recording={:?} transcribing={:?} stopped={:?})",
        icons.idle, icons.recording, icons.transcribing, icons.stopped);

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
