//! The offer notification over `org.freedesktop.Notifications`.
//!
//! zbus rather than `notify-send`, because the answer has to come back: a
//! button arrives as `ActionInvoked(id, key)`, a dismissal as
//! `NotificationClosed(id, reason)`.

use std::collections::HashMap;

use futures_util::StreamExt;
use zbus::{zvariant::Value, Connection, Proxy};

use super::{actions, wording, Answer, OfferKind};

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// Post the offer and wait for the user's answer. A server without action
/// buttons gets a body that points at the tray entry instead, and its
/// dismissal counts as Not now.
pub async fn ask(conn: &Connection, kind: OfferKind) -> anyhow::Result<Answer> {
    let proxy = Proxy::new(conn, DEST, PATH, DEST).await?;
    // Subscribe before posting, so an instant answer is not missed.
    let mut invoked = proxy.receive_signal("ActionInvoked").await?;
    let mut closed = proxy.receive_signal("NotificationClosed").await?;

    let with_actions = proxy
        .call::<_, _, Vec<String>>("GetCapabilities", &())
        .await
        .map(|caps| caps.iter().any(|c| c == "actions"))
        .unwrap_or(false);
    let (summary, body) = wording(kind);
    let mut body = body.to_string();
    let mut keys: Vec<&str> = Vec::new();
    if with_actions {
        for (key, label) in actions() {
            keys.push(key);
            keys.push(label);
        }
    } else {
        body.push_str("\nTo turn it on, choose \"Turn on meetings...\" in the Voxtype tray menu.");
    }
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("desktop-entry", Value::from("voxtype"));
    // Stays until answered: an offer that times out unseen asks nothing.
    let id: u32 = proxy
        .call(
            "Notify",
            &(
                "Voxtype",
                0u32,
                "audio-input-microphone",
                summary,
                body.as_str(),
                keys,
                hints,
                0i32,
            ),
        )
        .await?;

    loop {
        tokio::select! {
            Some(msg) = invoked.next() => {
                if let Ok((got, key)) = msg.body().deserialize::<(u32, String)>() {
                    if got == id {
                        let _ = proxy.call::<_, _, ()>("CloseNotification", &(id,)).await;
                        return Ok(Answer::from_action(&key));
                    }
                }
            }
            Some(msg) = closed.next() => {
                if let Ok((got, _reason)) = msg.body().deserialize::<(u32, u32)>() {
                    if got == id {
                        return Ok(Answer::NotNow);
                    }
                }
            }
            else => return Ok(Answer::NotNow),
        }
    }
}
