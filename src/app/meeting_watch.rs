//! `voxtype meeting watch`: the terminal meeting view, or `--json` relay.

use std::io::{IsTerminal, Write};
use voxtype::config;
use voxtype::meeting::view::client::{spawn_feed, Feed};
use voxtype::meeting::MeetingConfig;

pub(crate) async fn run_watch(json: bool, meetings: &MeetingConfig) -> anyhow::Result<()> {
    let socket = voxtype::meeting::events::default_socket_path();
    if json {
        return tokio::task::spawn_blocking(move || relay(socket)).await?;
    }
    if !std::io::stdout().is_terminal() {
        anyhow::bail!(
            "`voxtype meeting watch` draws a screen and needs a terminal. \
             For a script, use `voxtype meeting watch --json`."
        );
    }
    let opts = voxtype::tui::meeting_watch::WatchOptions {
        exe: std::env::current_exe()?,
        global_args: config::Config::path_override()
            .map(|p| vec!["--config".to_string(), p.display().to_string()])
            .unwrap_or_default(),
        socket,
        meetings: meetings.clone(),
    };
    tokio::task::spawn_blocking(move || voxtype::tui::meeting_watch::run(opts)).await?
}

/// Copy meeting.sock to stdout, line by line, reconnecting when the daemon
/// restarts. Every connection starts with the daemon's `hello`.
fn relay(socket: std::path::PathBuf) -> anyhow::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _feed = spawn_feed(socket, tx);
    let stdout = std::io::stdout();
    for msg in rx {
        if let Feed::Line(line) = msg {
            let mut out = stdout.lock();
            writeln!(out, "{line}")?;
            out.flush()?;
        }
    }
    Ok(())
}
