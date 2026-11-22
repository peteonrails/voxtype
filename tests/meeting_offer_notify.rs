//! The meeting-mode offer notification end to end, on a private bus.
//!
//! Starts its own `dbus-daemon` and serves a fake
//! `org.freedesktop.Notifications` that answers the offer the way a user
//! would, then checks the answer `ask` reports. Nothing touches the user's
//! session bus or desktop.

#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use voxtype::meeting_offer::{notify::ask, Answer, OfferKind};
use zbus::zvariant::OwnedValue;
use zbus::SignalContext;

struct PrivateBus {
    child: std::process::Child,
    address: String,
}

impl PrivateBus {
    fn start() -> Option<Self> {
        let mut child = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take()?)
            .read_line(&mut line)
            .ok()?;
        let address = line.trim().to_string();
        (!address.is_empty()).then_some(Self { child, address })
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Answers each notification with `answer` (an action key, or "closed").
/// A server without action support leaves it out of its capabilities.
struct FakeServer {
    answer: String,
    actions: bool,
    log: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl FakeServer {
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        _app_name: String,
        _replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        _hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        self.log.lock().unwrap().push(format!(
            "{summary}|{body}|{}|{expire_timeout}",
            actions.join(",")
        ));
        let id = 41;
        let answer = self.answer.clone();
        let ctxt = ctxt.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            // An unrelated notification's answer must not count.
            let _ = FakeServer::action_invoked(&ctxt, 7, "turn-on").await;
            if answer == "closed" {
                let _ = FakeServer::notification_closed(&ctxt, id, 2).await;
            } else {
                let _ = FakeServer::action_invoked(&ctxt, id, &answer).await;
            }
        });
        id
    }

    fn get_capabilities(&self) -> Vec<String> {
        if self.actions {
            vec!["actions".into(), "body".into()]
        } else {
            vec!["body".into()]
        }
    }

    fn close_notification(&self, id: u32) {
        self.log.lock().unwrap().push(format!("close {id}"));
    }

    #[zbus(signal)]
    async fn action_invoked(
        ctxt: &SignalContext<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn notification_closed(
        ctxt: &SignalContext<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;
}

async fn answered(answer: &str, kind: OfferKind, actions: bool) -> Option<(Answer, Vec<String>)> {
    let Some(bus) = PrivateBus::start() else {
        eprintln!("skipping: dbus-daemon is not available");
        return None;
    };
    let log = Arc::new(Mutex::new(Vec::new()));
    let _server = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("org.freedesktop.Notifications")
        .unwrap()
        .serve_at(
            "/org/freedesktop/Notifications",
            FakeServer {
                answer: answer.into(),
                actions,
                log: log.clone(),
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let client = zbus::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), ask(&client, kind))
        .await
        .expect("the answer arrives")
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let seen = log.lock().unwrap().clone();
    Some((got, seen))
}

#[tokio::test]
async fn each_button_maps_to_its_answer() {
    for (key, want) in [
        ("turn-on", Answer::TurnOn),
        ("default", Answer::TurnOn),
        ("not-now", Answer::NotNow),
        ("never", Answer::Never),
        ("closed", Answer::NotNow),
    ] {
        let Some((got, _)) = answered(key, OfferKind::Plain, true).await else {
            return;
        };
        assert_eq!(got, want, "{key}");
    }
}

#[tokio::test]
async fn the_offer_carries_its_wording_and_buttons_and_stays_up() {
    let Some((_, seen)) = answered("turn-on", OfferKind::MeetingRecorder, true).await else {
        return;
    };
    assert_eq!(
        seen[0],
        "Turn on Voxtype meetings?|You record meetings with Meeting Recorder. Voxtype can \
         transcribe meetings live too, with your side and the other side of the call \
         labelled. Turn on meeting mode?|turn-on,Turn on,not-now,Not now,never,No, don't \
         ask again|0"
    );
    assert!(seen.contains(&"close 41".to_string()), "{seen:?}");
}

#[tokio::test]
async fn a_server_without_buttons_points_at_the_tray() {
    let Some((got, seen)) = answered("closed", OfferKind::Plain, false).await else {
        return;
    };
    assert_eq!(got, Answer::NotNow);
    assert!(seen[0].contains("\"Turn on meetings...\" in the Voxtype tray menu"));
    assert!(seen[0].contains("||0"), "no actions sent: {}", seen[0]);
}
