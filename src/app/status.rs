//! `voxtype status` — read the daemon's state file, optionally render as
//! Waybar-flavoured JSON, optionally follow with inotify. The JSON shape
//! itself lives in `voxtype::status_json` (a library module) so external
//! callers can emit the same contract.

use voxtype::{
    config,
    daemon_status::is_daemon_running,
    status_json::{format_state_json, ExtendedStatusInfo},
};

/// Run the status command - show current daemon state
pub(crate) async fn run_status(
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
