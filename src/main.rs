//! Voxtype - Push-to-talk voice-to-text for Linux
//!
//! Run with `voxtype` or `voxtype daemon` to start the daemon.
//! Use `voxtype setup` to check dependencies and download models.
//! Use `voxtype transcribe <file>` to transcribe an audio file.
//!
//! The binary entry point is intentionally thin: install the SIGILL handler
//! before any other code, reset SIGPIPE, parse CLI, set up logging, load
//! config, then hand off to `app::run`. Every long handler (status, meeting,
//! record, …) lives under `src/app/`.

mod app;

use app::sigpipe;
use clap::Parser;
use tracing_subscriber::EnvFilter;
use voxtype::{config, cpu, Cli, Commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Install SIGILL handler early to catch illegal instruction crashes
    // and provide a helpful error message instead of core dumping
    cpu::install_sigill_handler();

    // Reset SIGPIPE to default behavior (terminate silently) to avoid panics
    // when output is piped through commands like `head` that close the pipe early
    sigpipe::reset_sigpipe();

    let cli = Cli::parse();

    // Check if this is the worker command (needs stderr-only logging)
    let is_worker = matches!(cli.command, Some(Commands::TranscribeWorker { .. }));
    // Import, retranscribe, the sound check and the waveform print their
    // own progress, meters or result, and with --json their stdout is
    // NDJSON (the waveform's is one JSON object), so logs go to stderr and
    // only warnings show unless -v asks for more. A sound check with no
    // daemon running opens the audio itself, and its info lines (the echo
    // canceller, the loopback source) landed in the NDJSON.
    let is_import = matches!(
        cli.command,
        Some(Commands::Meeting {
            action: voxtype::MeetingAction::Import { .. }
                | voxtype::MeetingAction::Retranscribe { .. }
                | voxtype::MeetingAction::Soundcheck { .. }
                | voxtype::MeetingAction::Waveform { .. }
        })
    );

    // History's JSON contract includes output actions, whose drivers log at
    // info level. Keep those diagnostics away from machine-readable stdout.
    let is_history_json = matches!(cli.command, Some(Commands::History { json: true, .. }));

    // Initialize logging
    let log_level = if cli.quiet {
        "error"
    } else {
        match cli.verbose {
            0 if is_import => "warn",
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };

    // ureq logs each request's full URL at debug, and a calendar's iCal
    // link is a secret: its debug lines stay off whatever RUST_LOG asks.
    let log_filter = |level: &str| {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(format!("voxtype={},warn", level)))
            .add_directive("ureq=info".parse().expect("a valid directive"))
    };

    if is_worker || is_import || is_history_json {
        // Worker uses stderr for logging (stdout is reserved for IPC protocol)
        tracing_subscriber::fmt()
            .with_env_filter(log_filter(log_level))
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(log_filter(log_level))
            .with_target(false)
            .init();
    }

    // Every command that writes the config goes through default_path(), so
    // point it at --config before anything runs.
    if let Some(path) = &cli.config {
        config::Config::set_path_override(path.clone());
    }

    // Load configuration. config_path tracks the file we actually loaded (or
    // would load), so subprocess transcribers can reuse the same source.
    let config_path = cli
        .config
        .clone()
        .or_else(config::Config::resolve_existing_path)
        .or_else(config::Config::default_path);
    let config = config::load_config(cli.config.as_deref())?;

    app::run(cli, config_path, config).await
}
