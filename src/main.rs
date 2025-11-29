//! Voxtype - Push-to-talk voice-to-text for Wayland
//!
//! Run with `voxtype` or `voxtype daemon` to start the daemon.
//! Use `voxtype setup` to check dependencies and download models.
//! Use `voxtype transcribe <file>` to transcribe an audio file.

mod audio;
mod config;
mod daemon;
mod error;
mod hotkey;
mod output;
mod state;
mod transcribe;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "voxtype")]
#[command(author, version, about = "Push-to-talk voice-to-text for Wayland")]
#[command(long_about = "
Voxtype is a push-to-talk voice-to-text tool for Wayland Linux systems.
Hold a hotkey to record, release to transcribe and output the text.

SETUP:
  1. Add yourself to the input group: sudo usermod -aG input $USER
  2. Log out and back in
  3. Start ydotool daemon: systemctl --user enable --now ydotool
  4. Run: voxtype setup (to download whisper model)
  5. Run: voxtype (to start the daemon)

USAGE:
  Hold ScrollLock (default) while speaking, release to transcribe.
  Text will be typed at cursor position, or copied to clipboard as fallback.
")]
struct Cli {
    /// Path to config file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Increase verbosity (-v = debug, -vv = trace)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Quiet mode (errors only)
    #[arg(short, long)]
    quiet: bool,

    /// Force clipboard mode (don't try to type)
    #[arg(long)]
    clipboard: bool,

    /// Override whisper model (tiny, base, small, medium, large-v3)
    #[arg(long, value_name = "MODEL")]
    model: Option<String>,

    /// Override hotkey (e.g., SCROLLLOCK, PAUSE, F13)
    #[arg(long, value_name = "KEY")]
    hotkey: Option<String>,

    /// Use toggle mode (press to start/stop) instead of push-to-talk (hold to record)
    #[arg(long)]
    toggle: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run as daemon (default if no command specified)
    Daemon,

    /// Transcribe an audio file (WAV, 16kHz, mono)
    Transcribe {
        /// Path to audio file
        file: PathBuf,
    },

    /// Check setup and optionally download models
    Setup {
        /// Download model if missing
        #[arg(long)]
        download: bool,
    },

    /// Show current configuration
    Config,

    /// Show daemon status (for Waybar/polybar integration)
    Status {
        /// Continuously output status changes as JSON (for Waybar exec)
        #[arg(long)]
        follow: bool,

        /// Output format: "text" (default) or "json" (for Waybar)
        #[arg(long, default_value = "text")]
        format: String,
    },
}

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

        Commands::Setup { download } => {
            run_setup(&config, download).await?;
        }

        Commands::Config => {
            show_config(&config)?;
        }

        Commands::Status { follow, format } => {
            run_status(&config, follow, &format).await?;
        }
    }

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

/// Default configuration file content
const DEFAULT_CONFIG: &str = r#"# Voxtype Configuration
#
# Location: ~/.config/voxtype/config.toml
# All settings can be overridden via CLI flags

[hotkey]
# Key to hold for push-to-talk
# Common choices: SCROLLLOCK, PAUSE, RIGHTALT, F13-F24
# Use `evtest` to find key names for your keyboard
key = "SCROLLLOCK"

# Optional modifier keys that must also be held
# Example: modifiers = ["LEFTCTRL", "LEFTALT"]
modifiers = []

# Activation mode: "push_to_talk" or "toggle"
# - push_to_talk: Hold hotkey to record, release to transcribe (default)
# - toggle: Press hotkey once to start recording, press again to stop
# mode = "push_to_talk"

[audio]
# Audio input device ("default" uses system default)
# List devices with: pactl list sources short
device = "default"

# Sample rate in Hz (whisper expects 16000)
sample_rate = 16000

# Maximum recording duration in seconds (safety limit)
max_duration_secs = 60

# [audio.feedback]
# Enable audio feedback sounds (beeps when recording starts/stops)
# enabled = true
#
# Sound theme: "default", "subtle", "mechanical", or path to custom theme directory
# theme = "default"
#
# Volume level (0.0 to 1.0)
# volume = 0.7

[whisper]
# Model to use for transcription
# Options: tiny, tiny.en, base, base.en, small, small.en, medium, medium.en, large-v3
# .en models are English-only but faster and more accurate for English
# Or provide absolute path to a custom .bin model file
model = "base.en"

# Language for transcription
# Use "en" for English, "auto" for auto-detection
# See: https://github.com/openai/whisper#available-models-and-languages
language = "en"

# Translate non-English speech to English
translate = false

# Number of CPU threads for inference (omit for auto-detection)
# threads = 4

[output]
# Primary output mode: "type" or "clipboard"
# - type: Simulates keyboard input at cursor position (requires ydotool)
# - clipboard: Copies text to clipboard (requires wl-copy)
mode = "type"

# Fall back to clipboard if typing fails
fallback_to_clipboard = true

# Delay between typed characters in milliseconds
# 0 = fastest possible, increase if characters are dropped
type_delay_ms = 0

[output.notification]
# Show notification when recording starts (hotkey pressed)
on_recording_start = false

# Show notification when recording stops (transcription beginning)
on_recording_stop = false

# Show notification with transcribed text after transcription completes
on_transcription = true

# State file for external integrations (Waybar, polybar, etc.)
# Uncomment to enable. Use "auto" for default location ($XDG_RUNTIME_DIR/voxtype/state)
# or provide a custom path. The daemon writes state ("idle", "recording", "transcribing")
# to this file whenever it changes.
# state_file = "auto"
"#;

/// Run the setup command
async fn run_setup(config: &config::Config, download: bool) -> anyhow::Result<()> {
    println!("Voxtype Setup\n");
    println!("=============\n");

    // Ensure directories exist first
    println!("Creating directories...");
    config::Config::ensure_directories()?;
    println!("  ✓ Config directory: {:?}", config::Config::config_dir().unwrap_or_default());
    println!("  ✓ Models directory: {:?}", config::Config::models_dir());

    // Create default config file if it doesn't exist
    if let Some(config_path) = config::Config::default_path() {
        if !config_path.exists() {
            println!("\nCreating default config file...");
            std::fs::write(&config_path, DEFAULT_CONFIG)?;
            println!("  ✓ Created: {:?}", config_path);
        } else {
            println!("\n  Config file exists: {:?}", config_path);
        }
    }

    let mut all_ok = true;

    // Check input group
    println!("Checking input group membership...");
    let groups_output = std::process::Command::new("groups").output()?;
    let groups_str = String::from_utf8_lossy(&groups_output.stdout);
    if groups_str.contains("input") {
        println!("  ✓ User is in 'input' group");
    } else {
        println!("  ✗ User is NOT in 'input' group");
        println!("    Run: sudo usermod -aG input $USER");
        println!("    Then log out and back in");
        all_ok = false;
    }

    // Check ydotool
    println!("\nChecking ydotool...");
    let ydotool_check = tokio::process::Command::new("which")
        .arg("ydotool")
        .output()
        .await?;
    if ydotool_check.status.success() {
        println!("  ✓ ydotool found");

        // Check daemon
        let daemon_check = tokio::process::Command::new("systemctl")
            .args(["--user", "is-active", "ydotool"])
            .output()
            .await?;
        if daemon_check.status.success() {
            println!("  ✓ ydotool daemon running");
        } else {
            println!("  ✗ ydotool daemon not running");
            println!("    Run: systemctl --user enable --now ydotool");
            all_ok = false;
        }
    } else {
        println!("  ✗ ydotool not found (typing won't work, will use clipboard)");
        println!("    Install via your package manager");
    }

    // Check wl-copy
    println!("\nChecking wl-clipboard...");
    let wlcopy_check = tokio::process::Command::new("which")
        .arg("wl-copy")
        .output()
        .await?;
    if wlcopy_check.status.success() {
        println!("  ✓ wl-copy found");
    } else {
        println!("  ✗ wl-copy not found");
        println!("    Install wl-clipboard via your package manager");
        all_ok = false;
    }

    // Check whisper model
    println!("\nChecking whisper model...");
    let models_dir = config::Config::models_dir();
    let model_name = &config.whisper.model;

    let model_filename = match model_name.as_str() {
        "tiny" => "ggml-tiny.bin",
        "tiny.en" => "ggml-tiny.en.bin",
        "base" => "ggml-base.bin",
        "base.en" => "ggml-base.en.bin",
        "small" => "ggml-small.bin",
        "small.en" => "ggml-small.en.bin",
        "medium" => "ggml-medium.bin",
        "medium.en" => "ggml-medium.en.bin",
        "large-v3" => "ggml-large-v3.bin",
        other => other,
    };

    let model_path = models_dir.join(model_filename);

    if model_path.exists() {
        let size = std::fs::metadata(&model_path)
            .map(|m| m.len() as f64 / 1024.0 / 1024.0)
            .unwrap_or(0.0);
        println!("  ✓ Model found: {:?} ({:.0} MB)", model_path, size);
    } else {
        println!("  ✗ Model not found: {:?}", model_path);
        all_ok = false;

        if download {
            println!("\n  Downloading model...");
            std::fs::create_dir_all(&models_dir)?;

            let url = transcribe::whisper::get_model_url(model_name);
            println!("  URL: {}", url);

            let response = reqwest::get(&url).await?;
            let total_size = response.content_length().unwrap_or(0);
            println!("  Size: {:.0} MB", total_size as f64 / 1024.0 / 1024.0);

            let bytes = response.bytes().await?;
            std::fs::write(&model_path, &bytes)?;
            println!("  ✓ Downloaded to {:?}", model_path);
        } else {
            let url = transcribe::whisper::get_model_url(model_name);
            println!("\n  To download automatically, run: voxtype setup --download");
            println!("  Or manually download from:");
            println!("    {}", url);
            println!("  And place in: {:?}", models_dir);
        }
    }

    // Summary
    println!("\n---");
    if all_ok {
        println!("✓ All checks passed! Run 'voxtype' to start.");
    } else {
        println!("✗ Some checks failed. Please fix the issues above.");
    }

    Ok(())
}

/// Run the status command - show current daemon state
async fn run_status(config: &config::Config, follow: bool, format: &str) -> anyhow::Result<()> {
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

    if !follow {
        // One-shot: just read and print current state
        let state = std::fs::read_to_string(&state_path).unwrap_or_else(|_| "stopped".to_string());
        let state = state.trim();

        if format == "json" {
            println!("{}", format_state_json(state));
        } else {
            println!("{}", state);
        }
        return Ok(());
    }

    // Follow mode: watch for changes using inotify
    use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    // Print initial state
    let state = std::fs::read_to_string(&state_path).unwrap_or_else(|_| "stopped".to_string());
    let state = state.trim();
    if format == "json" {
        println!("{}", format_state_json(state));
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
                            println!("{}", format_state_json(&new_state));
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
                // Check if file was deleted (daemon stopped)
                if !state_path.exists() && last_state != "stopped" {
                    if format == "json" {
                        println!("{}", format_state_json("stopped"));
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
fn format_state_json(state: &str) -> String {
    let (text, class, tooltip) = match state {
        "recording" => ("🎤", "recording", "Recording..."),
        "transcribing" => ("⏳", "transcribing", "Transcribing..."),
        "idle" => ("🎙️", "idle", "Voxtype ready - hold hotkey to record"),
        "stopped" => ("", "stopped", "Voxtype not running"),
        _ => ("?", "unknown", "Unknown state"),
    };

    format!(
        r#"{{"text": "{}", "class": "{}", "tooltip": "{}"}}"#,
        text, class, tooltip
    )
}

/// Show current configuration
fn show_config(config: &config::Config) -> anyhow::Result<()> {
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

    if let Some(ref state_file) = config.state_file {
        println!("\n[integration]");
        println!("  state_file = {:?}", state_file);
        if let Some(resolved) = config.resolve_state_file() {
            println!("  (resolves to: {:?})", resolved);
        }
    }

    println!("\n---");
    println!(
        "Config file: {:?}",
        config::Config::default_path().unwrap_or_else(|| PathBuf::from("(not found)"))
    );
    println!("Models dir: {:?}", config::Config::models_dir());

    Ok(())
}
