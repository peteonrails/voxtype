//! The Record prompt over `org.freedesktop.Notifications`.
//!
//! Sent with zbus rather than `notify-send` because the prompt needs its
//! actions back: the server reports a click as `ActionInvoked(id, key)` and
//! a dismissal as `NotificationClosed(id, reason)`, which arrive here as
//! `(id, key)` pairs, with `"closed"` for a dismissal.

use std::collections::HashMap;

use async_trait::async_trait;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::{zvariant::Value, Connection, Proxy};

/// Action keys. `default` is what most servers (mako, the Omarchy shell)
/// invoke on a plain click, so a click on the prompt body means Record.
pub const ACTION_DEFAULT: &str = "default";
pub const ACTION_RECORD: &str = "record";
pub const ACTION_DISMISS: &str = "dismiss";
pub const ACTION_STOP: &str = "stop";
pub const ACTION_KEEP: &str = "keep";
/// Sent when the notification closes without an action.
pub const CLOSED: &str = "closed";

/// A notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub summary: String,
    pub body: String,
    /// `(key, label)` pairs.
    pub actions: Vec<(String, String)>,
}

/// A user's response to a notice: its id and the action key.
pub type Response = (u32, String);

/// Shows notices and closes them. Responses arrive on the channel the
/// implementation hands out when it is created.
#[async_trait]
pub trait Notifier: Send {
    async fn show(&mut self, notice: &Notice) -> anyhow::Result<u32>;
    async fn close(&mut self, id: u32);
}

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// The session bus notification server.
pub struct BusNotifier {
    proxy: Proxy<'static>,
    actions_supported: bool,
}

impl BusNotifier {
    /// Connect to the notification server on `conn` and start forwarding
    /// its action and close signals.
    pub async fn connect(
        conn: &Connection,
    ) -> anyhow::Result<(Self, mpsc::UnboundedReceiver<Response>)> {
        let proxy = Proxy::new(conn, DEST, PATH, DEST).await?;
        let (tx, rx) = mpsc::unbounded_channel();

        let mut invoked = proxy.receive_signal("ActionInvoked").await?;
        let mut closed = proxy.receive_signal("NotificationClosed").await?;
        tokio::spawn(async move {
            loop {
                let response = tokio::select! {
                    Some(msg) = invoked.next() => msg.body().deserialize::<(u32, String)>().ok(),
                    Some(msg) = closed.next() => msg
                        .body()
                        .deserialize::<(u32, u32)>()
                        .ok()
                        .map(|(id, _reason)| (id, CLOSED.to_string())),
                    else => break,
                };
                if let Some(r) = response {
                    if tx.send(r).is_err() {
                        break;
                    }
                }
            }
        });

        let actions_supported = proxy
            .call::<_, _, Vec<String>>("GetCapabilities", &())
            .await
            .map(|caps| caps.iter().any(|c| c == "actions"))
            .unwrap_or(false);
        if !actions_supported {
            tracing::warn!(
                "The notification server does not support actions; meeting prompts will \
                 name the command to run instead of offering a Record button"
            );
        }
        Ok((
            Self {
                proxy,
                actions_supported,
            },
            rx,
        ))
    }
}

#[async_trait]
impl Notifier for BusNotifier {
    async fn show(&mut self, notice: &Notice) -> anyhow::Result<u32> {
        let mut body = notice.body.clone();
        let mut actions: Vec<&str> = Vec::new();
        if self.actions_supported {
            for (key, label) in &notice.actions {
                actions.push(key);
                actions.push(label);
            }
        } else if notice.actions.iter().any(|(k, _)| k == ACTION_RECORD) {
            body.push_str("\nRun: voxtype meeting start");
        } else if notice.actions.iter().any(|(k, _)| k == ACTION_STOP) {
            body.push_str("\nRun: voxtype meeting stop");
        }
        let mut hints: HashMap<&str, Value> = HashMap::new();
        hints.insert("urgency", Value::U8(1));
        hints.insert("desktop-entry", Value::from("voxtype"));
        // Never expires on its own: the prompt is closed when the call ends.
        let expire: i32 = 0;
        let id: u32 = self
            .proxy
            .call(
                "Notify",
                &(
                    "Voxtype",
                    0u32,
                    "audio-input-microphone",
                    notice.summary.as_str(),
                    body.as_str(),
                    actions,
                    hints,
                    expire,
                ),
            )
            .await?;
        Ok(id)
    }

    async fn close(&mut self, id: u32) {
        let _ = self
            .proxy
            .call::<_, _, ()>("CloseNotification", &(id,))
            .await;
    }
}
