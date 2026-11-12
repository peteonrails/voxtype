//! `voxtype meeting soundcheck`: the meeting's microphone and loopback
//! levels, without recording.
//!
//! With the daemon running, the daemon opens the audio (it owns capture) on
//! a trigger file and streams the events on meeting.sock; this command reads
//! them back. Without it, this command opens the audio itself the same way.

use std::io::{IsTerminal, Write};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use voxtype::config::Config;
use voxtype::meeting::levels::{self, Levels};
use voxtype::meeting::soundcheck::{self, Request, SoundcheckEvent};
use voxtype::meeting::view::client::{spawn_feed, Feed};

pub(crate) async fn run_soundcheck(
    config: &Config,
    seconds: u64,
    json: bool,
    stop: bool,
) -> anyhow::Result<()> {
    let daemon = voxtype::daemon_status::read_pid_if_alive().is_some();
    if stop {
        if daemon {
            write_trigger(Request::Stop)?;
            println!("Asked the daemon to stop the sound check.");
        } else {
            println!("No sound check is running (the daemon is not running).");
        }
        return Ok(());
    }
    if let Some(state) = meeting_state() {
        anyhow::bail!(
            "A meeting is {state}. Its levels are already live: voxtype meeting watch --levels"
        );
    }
    let mut out = Output::new(json);
    if daemon {
        through_daemon(seconds, &mut out).await
    } else {
        standalone(config, seconds, &mut out).await
    }
}

/// The running meeting's state, if one is recording, paused or finishing.
fn meeting_state() -> Option<String> {
    let text = std::fs::read_to_string(Config::runtime_dir().join("meeting_state")).ok()?;
    let state = text.lines().next()?.trim().to_string();
    matches!(state.as_str(), "recording" | "paused" | "finalizing").then_some(state)
}

fn write_trigger(request: Request) -> std::io::Result<()> {
    let dir = Config::runtime_dir();
    std::fs::create_dir_all(&dir)?;
    let staged = dir.join(format!(
        "{}.{}.tmp",
        soundcheck::TRIGGER,
        std::process::id()
    ));
    std::fs::write(&staged, request.to_json())?;
    std::fs::rename(staged, dir.join(soundcheck::TRIGGER))
}

/// Ask the daemon for a sound check and relay its events until it stops.
async fn through_daemon(seconds: u64, out: &mut Output) -> anyhow::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _feed = spawn_feed(voxtype::meeting::events::default_socket_path(), tx);
    // Be connected before asking, or the `running` event can be missed.
    let connected = Instant::now() + Duration::from_secs(5);
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Feed::Connected) => break,
            Ok(_) => {}
            Err(_) if Instant::now() < connected => {}
            Err(_) => anyhow::bail!("Could not connect to the daemon's meeting.sock"),
        }
    }
    write_trigger(Request::Start { seconds })?;

    let (lines_tx, mut lines) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for msg in rx {
            if let Feed::Line(line) = msg {
                if lines_tx.send(line).is_err() {
                    break;
                }
            }
        }
    });

    // The daemon ends the check itself; this is only in case it never says.
    let give_up = tokio::time::sleep(Duration::from_secs(seconds + 15));
    tokio::pin!(give_up);
    let mut started = false;
    let mut stopping = false;
    loop {
        tokio::select! {
            line = lines.recv() => {
                let Some(line) = line else { anyhow::bail!("Lost the connection to the daemon") };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                match v["event"].as_str() {
                    Some("soundcheck") => {
                        let state = v["state"].as_str().unwrap_or("");
                        // A `stopped` from before this check started is not ours.
                        if state == "running" {
                            started = true;
                        } else if !started
                            && !stopping
                            && (v["reason"] == "stopped" || v["reason"] == "done")
                        {
                            continue;
                        }
                        out.event(&line, &v);
                        if state == "stopped" {
                            return finish(&v);
                        }
                    }
                    Some("levels") if v["soundcheck"] == true && started => out.levels_line(&line, &v),
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c(), if !stopping => {
                stopping = true;
                write_trigger(Request::Stop)?;
                give_up.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(3));
            }
            _ = &mut give_up => {
                if stopping {
                    out.finish();
                    return Ok(());
                }
                anyhow::bail!("The daemon did not report the sound check ending");
            }
        }
    }
}

/// Open the meeting's audio in this process, as the daemon would.
async fn standalone(config: &Config, seconds: u64, out: &mut Output) -> anyhow::Result<()> {
    let opened = match soundcheck::open_capture(config).await {
        Ok(opened) => opened,
        Err(e) => {
            let event = SoundcheckEvent::stopped("error", Some(e.to_string()));
            let line = tagged("soundcheck", &serde_json::to_string(&event)?);
            out.event(&line, &serde_json::to_value(&event)?);
            anyhow::bail!("{e}");
        }
    };
    let running = SoundcheckEvent::running(&opened, seconds);
    let line = tagged("soundcheck", &serde_json::to_string(&running)?);
    out.event(&line, &serde_json::from_str(&line)?);

    let mut capture = opened.capture;
    let (levels_tx, mut levels_rx) = tokio::sync::mpsc::unbounded_channel::<Levels>();
    let publisher = tokio::spawn(levels::run_publisher(
        capture.meters(),
        None,
        Arc::new(AtomicBool::new(true)),
        || true,
        move |l| {
            let _ = levels_tx.send(l);
        },
    ));

    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    let mut drain = tokio::time::interval(Duration::from_millis(100));
    let reason = loop {
        tokio::select! {
            Some(l) = levels_rx.recv() => {
                let line = tagged("levels", &serde_json::to_string(&l)?);
                out.levels_line(&line, &serde_json::from_str(&line)?);
            }
            _ = drain.tick() => {
                // Nothing records this audio; keep it from piling up.
                let _ = capture.get_samples().await;
            }
            _ = tokio::signal::ctrl_c() => break "stopped",
            _ = &mut deadline => break "done",
        }
    };
    publisher.abort();
    let _ = capture.stop().await;
    drop(opened.canceller);
    let stopped = SoundcheckEvent::stopped(reason, None);
    let line = tagged("soundcheck", &serde_json::to_string(&stopped)?);
    out.event(&line, &serde_json::from_str(&line)?);
    Ok(())
}

/// An event line as meeting.sock writes it, `event` first, from a struct
/// serialized as a JSON object.
fn tagged(event: &str, fields: &str) -> String {
    match fields.strip_prefix('{') {
        Some("}") | None => format!(r#"{{"event":"{event}"}}"#),
        Some(rest) => format!(r#"{{"event":"{event}",{rest}"#),
    }
}

/// How the check stopped, as an exit status.
fn finish(v: &serde_json::Value) -> anyhow::Result<()> {
    match v["reason"].as_str() {
        Some("error") => anyhow::bail!(
            "The sound check could not open the meeting audio: {}",
            v["message"].as_str().unwrap_or("unknown error")
        ),
        Some("meeting") => anyhow::bail!("A meeting started; the sound check gave way to it."),
        _ => Ok(()),
    }
}

/// NDJSON for scripts, or meters for a person.
struct Output {
    json: bool,
    tty: bool,
}

impl Output {
    fn new(json: bool) -> Self {
        Self {
            json,
            tty: std::io::stdout().is_terminal(),
        }
    }

    fn event(&mut self, line: &str, v: &serde_json::Value) {
        if self.json {
            println!("{line}");
            return;
        }
        self.finish();
        match v["state"].as_str() {
            Some("running") => {
                println!(
                    "Sound check for {} s: mic {} (echo {}), {}. Speak, then play something; Ctrl+C stops.",
                    v["seconds"],
                    v["mic"].as_str().unwrap_or("?"),
                    v["echo_cancel"].as_str().unwrap_or("?"),
                    if v["loopback"] == true {
                        "loopback on"
                    } else {
                        "no loopback"
                    }
                );
            }
            _ => {
                if let Some(message) = v["message"].as_str() {
                    println!("Sound check stopped: {message}");
                } else {
                    println!(
                        "Sound check stopped ({}).",
                        v["reason"].as_str().unwrap_or("?")
                    );
                }
            }
        }
    }

    fn levels_line(&mut self, line: &str, v: &serde_json::Value) {
        if self.json {
            println!("{line}");
            return;
        }
        let meter = |db: &serde_json::Value| -> String {
            match db.as_f64() {
                Some(db) => {
                    let filled = (((db + 60.0) / 60.0).clamp(0.0, 1.0) * 20.0).round() as usize;
                    format!(
                        "{}{} {:>6.1} dB",
                        "#".repeat(filled),
                        ".".repeat(20 - filled),
                        db
                    )
                }
                None => format!("{}   ---   ", " ".repeat(20)),
            }
        };
        let text = format!(
            "mic [{}]  loopback [{}]",
            meter(&v["mic"]),
            meter(&v["loopback"])
        );
        if self.tty {
            print!("\r{text}");
            let _ = std::io::stdout().flush();
        } else {
            println!("{text}");
        }
    }

    /// End a meter line drawn in place.
    fn finish(&mut self) {
        if self.tty && !self.json {
            println!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_lines_put_the_tag_first() {
        assert_eq!(
            tagged("soundcheck", r#"{"state":"stopped","reason":"done"}"#),
            r#"{"event":"soundcheck","state":"stopped","reason":"done"}"#
        );
        assert_eq!(tagged("x", "{}"), r#"{"event":"x"}"#);
    }

    #[test]
    fn an_error_or_a_meeting_ending_the_check_is_a_failure() {
        assert!(finish(&serde_json::json!({"reason": "done"})).is_ok());
        assert!(finish(&serde_json::json!({"reason": "stopped"})).is_ok());
        assert!(finish(&serde_json::json!({"reason": "error", "message": "x"})).is_err());
        assert!(finish(&serde_json::json!({"reason": "meeting"})).is_err());
    }
}
