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

/// `voxtype meeting view` for the views other than Quickshell.
pub(crate) struct ViewArgs {
    pub meeting_id: Option<String>,
    pub ask_id: Option<String>,
    pub tab: Option<String>,
    pub ask: Option<String>,
    pub no_open: bool,
}

/// The question and `--last` window behind each `meeting view --ask` choice.
fn quick_question(name: &str) -> (&'static str, Option<&'static str>) {
    match name {
        "last-5m" => ("Summarize the last five minutes.", Some("5m")),
        "last-speaker" => ("What did the last remote speaker just say?", Some("2m")),
        _ => ("What are the action items so far?", None),
    }
}

pub(crate) async fn open_view(
    meetings: &MeetingConfig,
    resolution: &voxtype::meeting::view::frontend::ViewResolution,
    args: ViewArgs,
) -> anyhow::Result<()> {
    use voxtype::meeting::view::frontend::ViewFrontend;
    let exe = std::env::current_exe()?;
    let global: Vec<String> = config::Config::path_override()
        .map(|p| vec!["--config".to_string(), p.display().to_string()])
        .unwrap_or_default();

    // A quick question without opening anything: answer as a notification.
    if args.no_open {
        let Some(ask) = args.ask.as_deref() else {
            println!("Nothing to do: --no-open without --ask.");
            return Ok(());
        };
        let (question, last) = quick_question(ask);
        let id = args.meeting_id.clone().unwrap_or_else(|| "latest".into());
        let mut cmd = std::process::Command::new(&exe);
        cmd.args(&global)
            .args(["meeting", "ask", &id, question, "--notify"]);
        if let Some(last) = last {
            cmd.args(["--last", last]);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        println!("Asking; the answer arrives as a notification.");
        return Ok(());
    }

    match resolution.active {
        ViewFrontend::Gtk4 => {
            use std::os::unix::process::CommandExt;

            let path = resolution.path.clone().expect("gtk4 resolves with a path");
            let mut cmd = std::process::Command::new(path);
            if let Some(id) = &args.meeting_id {
                cmd.args(["--meeting", id]);
            }
            if let Some(tab) = &args.tab {
                cmd.args(["--tab", tab]);
            }
            if let Some(ask_id) = &args.ask_id {
                cmd.args(["--ask-id", ask_id]);
            }
            if let Some(ask) = &args.ask {
                cmd.args(["--ask", quick_question(ask).0]);
            }
            if let Some(p) = config::Config::path_override() {
                cmd.arg("--config").arg(p);
            }
            // The history reader may own the controlling terminal's session.
            // Its exit must not send SIGHUP to the newly opened window.
            cmd.process_group(0);
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            println!("Opened the meeting window.");
        }
        ViewFrontend::Terminal => {
            if std::io::stdout().is_terminal() {
                return run_watch(false, false, meetings).await;
            }
            let Ok(term) = which::which("xdg-terminal-exec") else {
                anyhow::bail!(
                    "[meeting] view = \"terminal\" needs a terminal: run `voxtype meeting watch` in one, \
                     or install xdg-terminal-exec so `meeting view` can open one."
                );
            };
            std::process::Command::new(term)
                .arg(&exe)
                .args(&global)
                .args(["meeting", "watch"])
                .stdin(std::process::Stdio::null())
                .spawn()?;
            println!("Opened `voxtype meeting watch` in a terminal.");
        }
        _ => {
            anyhow::bail!(
                "No meeting view to open ({}).\n  \
                 Set [meeting] view, install the GTK4 window (voxtype-meetings-gtk4), \
                 or watch in a terminal with: voxtype meeting watch",
                resolution.reason
            );
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
