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
