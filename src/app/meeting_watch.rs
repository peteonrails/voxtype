//! `voxtype meeting watch`: the terminal meeting view, or `--json` relay.

use std::io::{IsTerminal, Write};
use voxtype::config;
use voxtype::meeting::view::client::{spawn_feed, Feed};
use voxtype::meeting::MeetingConfig;

pub(crate) async fn run_watch(
    json: bool,
    levels: bool,
    meetings: &MeetingConfig,
) -> anyhow::Result<()> {
    let socket = voxtype::meeting::events::default_socket_path();
    if json || levels {
        let pass = Relay {
            events: json,
            levels,
        };
        return tokio::task::spawn_blocking(move || relay(socket, pass)).await?;
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

/// Which meeting.sock lines the relay passes on.
#[derive(Debug, Clone, Copy)]
struct Relay {
    /// Every event but `levels` (`--json`)
    events: bool,
    /// `levels` events (`--levels`)
    levels: bool,
}

impl Relay {
    fn passes(self, line: &str) -> bool {
        let is_levels =
            serde_json::from_str::<serde_json::Value>(line).is_ok_and(|v| v["event"] == "levels");
        if is_levels {
            self.levels
        } else {
            self.events
        }
    }
}

/// Copy meeting.sock to stdout, line by line, reconnecting when the daemon
/// restarts. Every connection starts with the daemon's `hello`.
fn relay(socket: std::path::PathBuf, pass: Relay) -> anyhow::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _feed = spawn_feed(socket, tx);
    let stdout = std::io::stdout();
    for msg in rx {
        if let Feed::Line(line) = msg {
            if !pass.passes(&line) {
                continue;
            }
            let mut out = stdout.lock();
            writeln!(out, "{line}")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEVELS: &str = r#"{"event":"levels","meeting_id":"m","t_ms":100,"mic":-20.0,"mic_peak":-14.0,"loopback":null,"loopback_peak":null}"#;
    const SEGMENTS: &str = r#"{"event":"segments","chunk_id":1,"segments":[]}"#;

    #[test]
    fn json_leaves_levels_out_and_levels_passes_only_them() {
        let json = Relay {
            events: true,
            levels: false,
        };
        assert!(json.passes(SEGMENTS) && !json.passes(LEVELS));
        let levels = Relay {
            events: false,
            levels: true,
        };
        assert!(!levels.passes(SEGMENTS) && levels.passes(LEVELS));
        let both = Relay {
            events: true,
            levels: true,
        };
        assert!(both.passes(SEGMENTS) && both.passes(LEVELS));
    }
}
