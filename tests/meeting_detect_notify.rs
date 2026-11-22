//! The Record prompt end to end over D-Bus, on a private bus.
//!
//! Starts its own `dbus-daemon`, serves a fake
//! `org.freedesktop.Notifications` on it that answers every prompt the way
//! a user would, and checks that the answer reaches `voxtype meeting start`
//! (a stand-in executable that records its arguments). Nothing touches the
//! user's session bus or desktop.

#![cfg(target_os = "linux")]

mod common;

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use voxtype::meeting::detect::notify::BusNotifier;
use voxtype::meeting::detect::runner::{CliMeetingControl, Controller};
use voxtype::meeting::detect::{Call, Event};
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

/// A notification server that answers each prompt with `answer`: an action
/// key, or "closed" to dismiss it.
struct FakeServer {
    answer: String,
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
        _expire_timeout: i32,
    ) -> u32 {
        self.log.lock().unwrap().push(format!(
            "notify {} | {} | {}",
            summary,
            body,
            actions.join(",")
        ));
        let id = 7;
        let answer = self.answer.clone();
        let ctxt = ctxt.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if answer == "closed" {
                let _ = FakeServer::notification_closed(&ctxt, id, 2).await;
            } else {
                let _ = FakeServer::action_invoked(&ctxt, id, &answer).await;
            }
        });
        id
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into()]
    }

    fn close_notification(&self, id: u32) {
        self.log.lock().unwrap().push(format!("close {}", id));
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

/// A stand-in for the voxtype binary that writes its arguments to `out`.
fn fake_voxtype(dir: &Path, out: &Path) -> PathBuf {
    let exe = dir.join("voxtype");
    common::write_script(
        &exe,
        &format!("#!/bin/sh\nprintf '%s|' \"$@\" > '{}'\n", out.display()),
    );
    exe
}

/// Prompt for a Meet call, let the fake server answer, and return what the
/// server saw and the `voxtype` arguments, if it was run.
async fn prompt_answered_with(answer: &str) -> Option<(Vec<String>, Option<String>)> {
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
                answer: answer.to_string(),
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
    let (notifier, mut responses) = BusNotifier::connect(&client).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("args");
    let control = CliMeetingControl {
        exe: fake_voxtype(dir.path(), &out),
        config_path: None,
    };
    let mut controller = Controller::new(notifier, control);
    controller
        .on_event(&Event::Prompt(Call {
            app: "Google Meet".into(),
            title: "Meet - abc-defg-hij".into(),
            reason: "test".into(),
            auto_record: false,
            by_window: false,
        }))
        .await;

    let response = tokio::time::timeout(Duration::from_secs(5), responses.recv())
        .await
        .expect("the server's answer arrives")
        .expect("response channel open");
    controller.on_response(response).await;

    // CloseNotification is a method call; give the server a moment to log it.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let seen = log.lock().unwrap().clone();
    Some((seen, std::fs::read_to_string(&out).ok()))
}

#[tokio::test]
async fn record_on_the_notification_starts_the_meeting() {
    let Some((seen, args)) = prompt_answered_with("record").await else {
        return;
    };
    assert_eq!(
        seen[0],
        "notify Google Meet call detected | Record \"Meet - abc-defg-hij\" as a meeting?\nLet the others know \
         you're transcribing. | default,Record,record,Record,dismiss,Dismiss"
    );
    assert!(seen.contains(&"close 7".to_string()), "{seen:?}");
    assert_eq!(
        args.as_deref(),
        Some("meeting|start|--window-title|Meet - abc-defg-hij|")
    );
}

#[tokio::test]
async fn clicking_the_notification_body_starts_the_meeting() {
    let Some((_, args)) = prompt_answered_with("default").await else {
        return;
    };
    assert_eq!(
        args.as_deref(),
        Some("meeting|start|--window-title|Meet - abc-defg-hij|")
    );
}

#[tokio::test]
async fn dismissing_the_notification_starts_nothing() {
    let Some((_, args)) = prompt_answered_with("closed").await else {
        return;
    };
    assert_eq!(args, None);
}

#[tokio::test]
async fn the_dismiss_button_starts_nothing() {
    let Some((_, args)) = prompt_answered_with("dismiss").await else {
        return;
    };
    assert_eq!(args, None);
}

/// The automatic stop over the bus: the "Call ended" notice goes up, the
/// fake server answers it with `answer`, then the grace period runs out.
/// Returns what the server saw and the `voxtype` arguments run, if any.
async fn auto_stop_answered_with(answer: &str) -> Option<(Vec<String>, Option<String>)> {
    use voxtype::config::CallEnd;
    use voxtype::meeting::detect::runner::CallEndPolicy;
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
                answer: answer.to_string(),
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
    let (notifier, mut responses) = BusNotifier::connect(&client).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("args");
    let control = CliMeetingControl {
        exe: fake_voxtype(dir.path(), &out),
        config_path: None,
    };
    let mut controller = Controller::new(notifier, control).with_call_end(CallEndPolicy {
        mode: CallEnd::Stop,
        grace: Duration::from_millis(300),
    });
    controller
        .on_event(&Event::Ended {
            call: Call {
                app: "Zoom".into(),
                title: "Zoom call".into(),
                reason: "test".into(),
                auto_record: false,
                by_window: false,
            },
            offer_stop: true,
        })
        .await;
    let response = tokio::time::timeout(Duration::from_secs(5), responses.recv())
        .await
        .expect("the server's answer arrives")
        .expect("response channel open");
    controller.on_response(response).await;
    if let Some(at) = controller.stop_due_at() {
        tokio::time::sleep_until(at).await;
        controller.fire_due_stop(true).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let seen = log.lock().unwrap().clone();
    Some((seen, std::fs::read_to_string(&out).ok()))
}

#[tokio::test]
async fn a_call_ending_stops_the_meeting_after_the_grace_period() {
    let Some((seen, args)) = auto_stop_answered_with("closed").await else {
        return;
    };
    assert_eq!(
        seen[0],
        "notify Call ended | Zoom released the microphone. Stopping the recording in 0 s. \
         | keep,Keep recording"
    );
    assert_eq!(args.as_deref(), Some("meeting|stop|"));
}

#[tokio::test]
async fn keep_recording_on_the_notice_cancels_the_stop() {
    let Some((seen, args)) = auto_stop_answered_with("keep").await else {
        return;
    };
    assert!(seen.contains(&"close 7".to_string()), "{seen:?}");
    assert_eq!(args, None, "no stop");
}
