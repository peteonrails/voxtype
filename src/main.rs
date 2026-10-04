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

use app::{exit, sigpipe};
use clap::Parser;
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

    // Import, retranscribe, the sound check and the waveform print their
    // own progress, meters or result, so only warnings show unless -v asks
    // for more.
    let is_import = matches!(
        cli.command,
        Some(Commands::Meeting {
            action: voxtype::MeetingAction::Import { .. }
                | voxtype::MeetingAction::Retranscribe { .. }
                | voxtype::MeetingAction::Soundcheck { .. }
                | voxtype::MeetingAction::Waveform { .. }
        })
    );

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
        voxtype::logging::env_filter(&format!("voxtype={},warn", level))
            .add_directive("ureq=info".parse().expect("a valid directive"))
    };

    if logs_to_stdout(cli.command.as_ref()) {
        tracing_subscriber::fmt()
            .with_env_filter(log_filter(log_level))
            .with_target(false)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(log_filter(log_level))
            .with_target(false)
            .with_writer(std::io::stderr)
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

    // One-shot commands return here. Leave the process the way the daemon does
    // (commit 525a0a6d): without running static destructors, because the ONNX
    // Runtime exit hook aborts on glibc's heap check after a successful run
    // (#772). `app::exit` flushes stdio first. Errors keep their exit code and
    // their `Error: ...` line, which is what `Termination` would have printed.
    match app::run(cli, config_path, config).await {
        Ok(()) => exit::without_destructors(0),
        Err(err) => {
            eprintln!("Error: {err:?}");
            exit::without_destructors(1)
        }
    }
}

/// Whether this command's log belongs on stdout.
///
/// Only the long-running services log there: their stdout carries nothing
/// else, and existing setups read it (`voxtype > voxtype.log`, the macOS
/// app's `stdout.log`). Every other command prints its result on stdout,
/// where a script, a pipe or a frontend parses it (`config get`, `status`,
/// `transcribe`, anything with `--json`, the worker's IPC protocol), so its
/// log goes to stderr. Logging to stdout used to be the default, with the
/// commands known to need stderr listed one by one, and each command left
/// off that list leaked log lines into its output: a warning about an
/// unreadable config section came out of `config get` ahead of the value.
fn logs_to_stdout(command: Option<&Commands>) -> bool {
    match command {
        None | Some(Commands::Daemon) => true,
        #[cfg(target_os = "macos")]
        Some(Commands::Menubar | Commands::AppLaunch) => true,
        Some(_) => false,
    }
}
