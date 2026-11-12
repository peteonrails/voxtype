//! The "A meeting didn't finish" notification, posted once per interrupted
//! meeting when the daemon starts.
//!
//! zbus rather than `notify-send`, because the click has to come back: an
//! action arrives as `ActionInvoked(id, key)`. Which meetings were already
//! announced is kept in the state directory, so a later restart never
//! announces the same meeting again.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use zbus::{zvariant::Value, Connection, Proxy};

use super::RecoverableMeeting;

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const STATE_FILE: &str = "meeting-recover-notified.json";

/// Action keys. `default` is what most servers invoke on a click on the
/// notification itself.
pub const ACTION_DEFAULT: &str = "default";
pub const ACTION_OPEN: &str = "open";
pub const ACTION_FINISH: &str = "finish";

/// What a click asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Open the meeting view, whose card offers to finish or discard.
    Open,
    /// Finish this meeting with `voxtype meeting recover --finish`.
    Finish(String),
}

impl Choice {
    /// The voxtype arguments that carry it out.
    pub fn args(&self) -> Vec<String> {
        match self {
            Choice::Open => vec!["meeting".into(), "view".into()],
            Choice::Finish(id) => vec![
                "meeting".into(),
                "recover".into(),
                "--finish".into(),
                id.clone(),
            ],
        }
    }
}

/// Where the announced meetings are kept.
pub fn state_path() -> PathBuf {
    crate::config::Config::state_dir().join(STATE_FILE)
}

/// The meetings of `current` not announced before, and records all of
/// `current` as announced. IDs no longer in `current` (finished or
/// discarded) are forgotten, so the file stays small.
pub fn unannounced(path: &Path, current: &[RecoverableMeeting]) -> Vec<RecoverableMeeting> {
    let seen: BTreeSet<String> = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let new: Vec<RecoverableMeeting> = current
        .iter()
        .filter(|m| !seen.contains(&m.id))
        .cloned()
        .collect();
    let now: BTreeSet<&str> = current.iter().map(|m| m.id.as_str()).collect();
    if now.iter().copied().ne(seen.iter().map(String::as_str)) {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_string(&now)?)?;
            std::fs::rename(tmp, path)
        };
        if let Err(e) = write() {
            tracing::warn!("Could not record the announced meetings: {}", e);
        }
    }
    new
}

/// Summary, body and `(key, label)` actions for `meetings` (not empty).
/// One meeting offers Finish and Open; several offer Open, where the
/// meeting view lists them.
pub fn wording(meetings: &[RecoverableMeeting]) -> (String, String, Vec<(String, String)>) {
    let title = |m: &RecoverableMeeting| {
        crate::meeting::status_notice::display_title(m.title.as_deref(), Some(m.started_at))
    };
    let open = (ACTION_OPEN.to_string(), "Open".to_string());
    match meetings {
        [one] => (
            "A meeting didn't finish".to_string(),
            format!(
                "{}\nVoxtype stopped while it recorded. Finish it from what was saved, or open the meeting view.",
                title(one)
            ),
            vec![(ACTION_FINISH.to_string(), "Finish".to_string()), open],
        ),
        many => (
            format!("{} meetings didn't finish", many.len()),
            format!(
                "{}\nVoxtype stopped while they recorded. Open the meeting view to finish or discard them.",
                many.iter().map(title).collect::<Vec<_>>().join(", ")
            ),
            vec![open],
        ),
    }
}

/// Post the notification and wait for a click. `None` when it was
/// dismissed, or when the server cannot show actions, in which case the
/// body names the command instead.
pub async fn announce(
    conn: &Connection,
    meetings: &[RecoverableMeeting],
) -> anyhow::Result<Option<Choice>> {
    let proxy = Proxy::new(conn, DEST, PATH, DEST).await?;
    // Subscribe before posting, so an instant click is not missed.
    let mut invoked = proxy.receive_signal("ActionInvoked").await?;
    let mut closed = proxy.receive_signal("NotificationClosed").await?;
    let with_actions = proxy
        .call::<_, _, Vec<String>>("GetCapabilities", &())
        .await
        .map(|caps| caps.iter().any(|c| c == "actions"))
        .unwrap_or(false);

    let (summary, mut body, actions) = wording(meetings);
    let mut keys: Vec<&str> = Vec::new();
    if with_actions {
        for (key, label) in &actions {
            keys.push(key);
            keys.push(label);
        }
    } else {
        body.push_str("\nRun: voxtype meeting recover");
    }
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("desktop-entry", Value::from("voxtype"));
    let id: u32 = proxy
        .call(
            "Notify",
            &(
                "Voxtype",
                0u32,
                "audio-input-microphone",
                summary.as_str(),
                body.as_str(),
                keys,
                hints,
                0i32,
            ),
        )
        .await?;
    if !with_actions {
        return Ok(None);
    }

    loop {
        tokio::select! {
            Some(msg) = invoked.next() => {
                if let Ok((got, key)) = msg.body().deserialize::<(u32, String)>() {
                    if got == id {
                        let _ = proxy.call::<_, _, ()>("CloseNotification", &(id,)).await;
                        return Ok(match key.as_str() {
                            ACTION_FINISH if meetings.len() == 1 => {
                                Some(Choice::Finish(meetings[0].id.clone()))
                            }
                            ACTION_OPEN | ACTION_DEFAULT => Some(Choice::Open),
                            _ => None,
                        });
                    }
                }
            }
            Some(msg) = closed.next() => {
                if let Ok((got, _reason)) = msg.body().deserialize::<(u32, u32)>() {
                    if got == id {
                        return Ok(None);
                    }
                }
            }
            else => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(id: &str, title: Option<&str>) -> RecoverableMeeting {
        RecoverableMeeting {
            id: id.into(),
            title: title.map(String::from),
            started_at: chrono::Utc::now(),
            last_chunk_ms: 0,
            has_audio: false,
            untranscribed_chunks: 0,
            lines: 0,
            stopped_at: chrono::Utc::now(),
            duration_secs: 0,
        }
    }

    #[test]
    fn each_meeting_is_announced_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join(STATE_FILE);
        let a = meeting("a", Some("Standup"));
        let b = meeting("b", Some("Retro"));
        assert_eq!(
            unannounced(&path, std::slice::from_ref(&a)),
            vec![a.clone()]
        );
        // The next start: nothing new.
        assert!(unannounced(&path, std::slice::from_ref(&a)).is_empty());
        // Another crash adds b; only b is new.
        assert_eq!(unannounced(&path, &[a.clone(), b.clone()]), vec![b.clone()]);
        // a was finished; b stays announced and a is forgotten.
        assert!(unannounced(&path, std::slice::from_ref(&b)).is_empty());
        let kept: BTreeSet<String> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(kept, BTreeSet::from(["b".to_string()]));
    }

    #[test]
    fn one_meeting_offers_finish_and_several_offer_the_view() {
        let (summary, body, actions) = wording(&[meeting("a", Some("Standup"))]);
        assert_eq!(summary, "A meeting didn't finish");
        assert!(body.starts_with("Standup\n"), "{body}");
        assert_eq!(
            actions,
            vec![
                ("finish".to_string(), "Finish".to_string()),
                ("open".to_string(), "Open".to_string())
            ]
        );
        let (summary, body, actions) =
            wording(&[meeting("a", Some("Standup")), meeting("b", Some("Retro"))]);
        assert_eq!(summary, "2 meetings didn't finish");
        assert!(body.starts_with("Standup, Retro\n"), "{body}");
        assert_eq!(actions, vec![("open".to_string(), "Open".to_string())]);
        assert_eq!(
            Choice::Finish("a".into()).args(),
            ["meeting", "recover", "--finish", "a"]
        );
        assert_eq!(Choice::Open.args(), ["meeting", "view"]);
    }
}
