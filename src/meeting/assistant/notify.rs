//! `meeting ask --notify`: the answer as a desktop notification, with an
//! "Open" action that runs `[meeting.assistant] open_command`.
//!
//! Talks to org.freedesktop.Notifications directly, since the action has to
//! come back. The command runs through `sh -c` with VOXTYPE_MEETING_ID and
//! VOXTYPE_ASK_ID set, so the meeting view can raise itself on that answer.

use futures_util::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
use zbus::zvariant::Value;

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const OPEN_ACTION: &str = "open";

/// The command an Open click runs, and what it is about.
#[derive(Debug, Clone)]
pub struct OpenAction {
    pub command: String,
    pub meeting_id: String,
    pub ask_id: String,
}

/// How the notification ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Shown with no action to wait for.
    Shown,
    /// Open (or the body) was clicked and the command started.
    Opened,
    Dismissed,
    /// Nobody answered within the wait.
    TimedOut,
}

/// `text` cut to `max` characters at a word boundary, with an ellipsis,
/// and with markup characters escaped (servers may parse body markup).
pub fn notification_body(text: &str, max: usize) -> String {
    let text = text.trim();
    let cut: String = if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max).collect();
        if let Some(space) = s.rfind(char::is_whitespace) {
            if space > max / 2 {
                s.truncate(space);
            }
        }
        format!("{}…", s.trim_end())
    };
    cut.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Show the notification and, when there is an Open action, wait up to
/// `wait` for the user to answer it.
pub async fn notify_answer(
    conn: &zbus::Connection,
    title: &str,
    body: &str,
    open: Option<&OpenAction>,
    wait: Duration,
) -> zbus::Result<Outcome> {
    let proxy = zbus::Proxy::new(conn, DEST, PATH, DEST).await?;
    // Subscribe before Notify so a fast click cannot be missed.
    let mut invoked = proxy.receive_signal("ActionInvoked").await?;
    let mut closed = proxy.receive_signal("NotificationClosed").await?;

    let actions: Vec<&str> = match open {
        Some(_) => vec!["default", "Open", OPEN_ACTION, "Open"],
        None => Vec::new(),
    };
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("desktop-entry", Value::from("voxtype"));
    hints.insert("category", Value::from("im.received"));
    let id: u32 = proxy
        .call(
            "Notify",
            &(
                "Voxtype", 0u32, "voxtype", title, body, actions, hints, -1i32,
            ),
        )
        .await?;

    let Some(open) = open else {
        return Ok(Outcome::Shown);
    };

    let outcome = tokio::time::timeout(wait, async {
        loop {
            tokio::select! {
                Some(msg) = invoked.next() => {
                    if let Ok((nid, key)) = msg.body().deserialize::<(u32, String)>() {
                        if nid == id && (key == OPEN_ACTION || key == "default") {
                            return Outcome::Opened;
                        }
                    }
                }
                Some(msg) = closed.next() => {
                    if let Ok((nid, _reason)) = msg.body().deserialize::<(u32, u32)>() {
                        if nid == id {
                            return Outcome::Dismissed;
                        }
                    }
                }
                else => return Outcome::Dismissed,
            }
        }
    })
    .await
    .unwrap_or(Outcome::TimedOut);

    if outcome == Outcome::Opened {
        run_open(open);
        let _ = proxy.call::<_, _, ()>("CloseNotification", &(id,)).await;
    }
    Ok(outcome)
}

fn run_open(open: &OpenAction) {
    match std::process::Command::new("sh")
        .arg("-c")
        .arg(&open.command)
        .env("VOXTYPE_MEETING_ID", &open.meeting_id)
        .env("VOXTYPE_ASK_ID", &open.ask_id)
        .stdin(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            // Reap it without blocking the caller.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!("Could not run the Open command: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_are_trimmed_and_escaped() {
        assert_eq!(
            notification_body("  a <b> & c ", 100),
            "a &lt;b&gt; &amp; c"
        );
        let long = "word ".repeat(100);
        let body = notification_body(&long, 50);
        assert!(body.ends_with('…'));
        assert!(body.chars().count() <= 51);
        assert!(!body.contains("wor…"));
    }
}
