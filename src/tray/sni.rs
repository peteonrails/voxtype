//! StatusNotifierItem implementation using ksni

use super::{TrayEvent, TrayState};
use std::sync::{Arc, Mutex};

/// Mutable state shared with ksni callbacks (runs on ksni's thread).
/// Only `state` needs the mutex — it's updated from the tokio watcher task.
struct SharedState {
    state: TrayState,
}

struct VoxtypeTray {
    shared: Arc<Mutex<SharedState>>,
    /// Event sender lives outside the mutex — it's Clone+Send and never mutated.
    event_tx: std::sync::mpsc::Sender<TrayEvent>,
}

impl VoxtypeTray {
    fn send_event(&self, event: TrayEvent) {
        let _ = self.event_tx.send(event);
    }
}

impl ksni::Tray for VoxtypeTray {
    fn id(&self) -> String {
        "voxtype".to_string()
    }

    fn title(&self) -> String {
        "Voxtype".to_string()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_name(&self) -> String {
        let state = self.shared.lock().unwrap().state;
        match state {
            TrayState::Idle => "microphone-sensitivity-high".to_string(),
            TrayState::Recording => "media-record".to_string(),
            TrayState::Transcribing => "content-loading-symbolic".to_string(),
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let state = self.shared.lock().unwrap().state;
        let description = match state {
            TrayState::Idle => "Voxtype: Ready",
            TrayState::Recording => "Voxtype: Recording...",
            TrayState::Transcribing => "Voxtype: Transcribing...",
        };
        ksni::ToolTip {
            title: description.to_string(),
            description: String::new(),
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send_event(TrayEvent::ToggleRecording);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let state = self.shared.lock().unwrap().state;

        let status_label = match state {
            TrayState::Idle => "Status: Idle",
            TrayState::Recording => "Status: Recording",
            TrayState::Transcribing => "Status: Transcribing",
        };

        vec![
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: "Toggle Recording".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    tray.send_event(TrayEvent::ToggleRecording);
                }),
                ..Default::default()
            }),
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: "Cancel".to_string(),
                enabled: state == TrayState::Transcribing,
                activate: Box::new(|tray: &mut Self| {
                    tray.send_event(TrayEvent::CancelTranscription);
                }),
                ..Default::default()
            }),
            ksni::MenuItem::Separator,
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: status_label.to_string(),
                enabled: false,
                ..Default::default()
            }),
            ksni::MenuItem::Separator,
            ksni::MenuItem::Standard(ksni::menu::StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    tray.send_event(TrayEvent::Quit);
                }),
                ..Default::default()
            }),
        ]
    }
}

/// Spawn the tray service and return communication channels.
///
/// Returns `None` if the tray service fails to start (e.g., no DBus session bus).
pub fn spawn() -> Option<(
    tokio::sync::mpsc::Receiver<TrayEvent>,
    tokio::sync::watch::Sender<TrayState>,
)> {
    // Channels: daemon <-> tray
    let (state_tx, mut state_rx) = tokio::sync::watch::channel(TrayState::Idle);
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<TrayEvent>(8);

    // Bridge channel: ksni thread (std::sync) -> tokio task -> tokio mpsc
    let (std_event_tx, std_event_rx) = std::sync::mpsc::channel::<TrayEvent>();

    let shared = Arc::new(Mutex::new(SharedState {
        state: TrayState::Idle,
    }));

    let tray = VoxtypeTray {
        shared: shared.clone(),
        event_tx: std_event_tx,
    };

    // Preflight: check DBus session bus is available before spawning ksni,
    // since ksni::TrayService::spawn() panics on DBus failure.
    match std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        Err(_) => {
            tracing::warn!("DBUS_SESSION_BUS_ADDRESS not set, skipping tray");
            return None;
        }
        Ok(val) if val.trim().is_empty() => {
            tracing::warn!("DBUS_SESSION_BUS_ADDRESS is empty, skipping tray");
            return None;
        }
        Ok(_) => {}
    }

    // Spawn the bridge thread first, before ksni service, so we don't
    // leave an orphaned tray icon if thread creation fails.
    if std::thread::Builder::new()
        .name("tray-event-bridge".to_string())
        .spawn(move || {
            while let Ok(event) = std_event_rx.recv() {
                if event_tx.blocking_send(event).is_err() {
                    break; // receiver dropped, daemon shutting down
                }
            }
        })
        .is_err()
    {
        tracing::warn!("Failed to spawn tray event bridge thread");
        return None;
    }

    // Start the ksni service (runs its own event loop on a separate thread)
    let service = ksni::TrayService::new(tray);
    let handle = service.handle();
    service.spawn();

    // Task: watch for state changes and update the tray
    tokio::spawn(async move {
        while state_rx.changed().await.is_ok() {
            let new_state = *state_rx.borrow();
            {
                let mut s = shared.lock().unwrap();
                s.state = new_state;
            }
            handle.update(|_tray| {
                // The tray reads from shared state, so just triggering
                // an update is enough to refresh icon/tooltip/menu
            });
        }
    });

    Some((event_rx, state_tx))
}
