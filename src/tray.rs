//! Linux system tray via StatusNotifierItem (SNI) D-Bus protocol.
//!
//! Uses the `ksni` crate which implements both the SNI spec and the
//! `com.canonical.dbusmenu` context menu protocol. Works with KDE Plasma,
//! GNOME (AppIndicator/KStatusNotifierItem extension), waybar, swaybar, and
//! any SNI-compatible panel.
//!
//! Enable with `[tray] enabled = true` in config (default on Linux). The
//! daemon spawns a tokio task that watches an in-process watch channel and
//! reflects state changes in the icon, tooltip, and context menu.

use ksni::{menu::*, Status, ToolTip, Tray, TrayMethods};
use std::path::PathBuf;
use tokio::sync::watch;

// ── State ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Recording,
    Transcribing,
    Stopped,
}

impl TrayState {
    pub fn from_state_name(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "idle" => TrayState::Idle,
            "recording" => TrayState::Recording,
            "transcribing" => TrayState::Transcribing,
            _ => TrayState::Stopped,
        }
    }

    fn sni_status(self) -> Status {
        match self {
            TrayState::Recording => Status::NeedsAttention,
            TrayState::Stopped => Status::Passive,
            _ => Status::Active,
        }
    }

    fn tooltip_body(self) -> &'static str {
        match self {
            TrayState::Idle => "Ready",
            TrayState::Recording => "Recording...",
            TrayState::Transcribing => "Transcribing...",
            TrayState::Stopped => "Daemon not running",
        }
    }
}

// ── ksni::Tray implementation ─────────────────────────────────────────────────

struct VoxtypeTray {
    state: TrayState,
}

impl Tray for VoxtypeTray {
    fn id(&self) -> String {
        "voxtype".into()
    }

    fn title(&self) -> String {
        "Voxtype".into()
    }

    // Icon name used when status is Active or Transcribing.
    fn icon_name(&self) -> String {
        "voxtype".into()
    }

    // Icon name used when status is NeedsAttention (Recording).
    fn attention_icon_name(&self) -> String {
        "voxtype-recording".into()
    }

    fn status(&self) -> Status {
        self.state.sni_status()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Voxtype".into(),
            description: self.state.tooltip_body().into(),
            ..Default::default()
        }
    }

    /// Left-click toggles recording.
    fn activate(&mut self, _x: i32, _y: i32) {
        spawn_voxtype(&["record", "toggle"]);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let is_active = matches!(self.state, TrayState::Recording | TrayState::Transcribing);
        vec![
            StandardItem {
                label: "Toggle Recording".into(),
                activate: Box::new(|_| spawn_voxtype(&["record", "toggle"])),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Cancel".into(),
                enabled: is_active,
                activate: Box::new(|_| spawn_voxtype(&["record", "cancel"])),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Restart Daemon".into(),
                activate: Box::new(|_| {
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "restart", "voxtype"])
                        .spawn();
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Spawn a voxtype sub-command, finding the binary next to the current exe.
fn spawn_voxtype(args: &[&str]) {
    let bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("voxtype")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("voxtype"));
    let _ = std::process::Command::new(bin).args(args).spawn();
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Spawn the SNI tray as a tokio task. The task watches the in-process state
/// channel and reflects changes via ksni's handle. Exits when the sender is
/// dropped (daemon shutting down).
pub fn spawn(rx: watch::Receiver<TrayState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let tray = VoxtypeTray {
            state: *rx.borrow(),
        };
        let handle = match tray.spawn().await {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("Failed to start tray icon: {e}");
                return;
            }
        };
        tracing::info!("Tray icon active (SNI via ksni)");

        let mut rx = rx;
        loop {
            match rx.changed().await {
                Ok(_) => {
                    let new_state = *rx.borrow_and_update();
                    handle.update(|t| t.state = new_state).await;
                }
                Err(_) => {
                    tracing::debug!("Tray state channel closed, exiting.");
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TrayState::from_state_name ────────────────────────────────────────────

    #[test]
    fn from_str_idle() {
        assert_eq!(TrayState::from_state_name("idle"), TrayState::Idle);
    }

    #[test]
    fn from_str_recording() {
        assert_eq!(
            TrayState::from_state_name("recording"),
            TrayState::Recording
        );
    }

    #[test]
    fn from_str_transcribing() {
        assert_eq!(
            TrayState::from_state_name("transcribing"),
            TrayState::Transcribing
        );
    }

    #[test]
    fn from_str_unknown_is_stopped() {
        assert_eq!(TrayState::from_state_name("whatever"), TrayState::Stopped);
        assert_eq!(TrayState::from_state_name(""), TrayState::Stopped);
    }

    #[test]
    fn from_str_case_insensitive() {
        assert_eq!(TrayState::from_state_name("IDLE"), TrayState::Idle);
        assert_eq!(
            TrayState::from_state_name("Recording"),
            TrayState::Recording
        );
        assert_eq!(
            TrayState::from_state_name("TRANSCRIBING"),
            TrayState::Transcribing
        );
    }

    #[test]
    fn from_str_trims_whitespace() {
        assert_eq!(TrayState::from_state_name("  idle  "), TrayState::Idle);
        assert_eq!(
            TrayState::from_state_name("recording\n"),
            TrayState::Recording
        );
    }

    // ── icon names ────────────────────────────────────────────────────────────

    #[test]
    fn idle_icon_name_matches_installed_theme_name() {
        use ksni::Tray;
        // Must match the filename stem written by setup::icons::install().
        let t = VoxtypeTray {
            state: TrayState::Idle,
        };
        assert_eq!(t.icon_name(), "voxtype");
    }

    #[test]
    fn attention_icon_name_matches_installed_theme_name() {
        use ksni::Tray;
        let t = VoxtypeTray {
            state: TrayState::Idle,
        };
        assert_eq!(t.attention_icon_name(), "voxtype-recording");
    }

    // ── sni_status ────────────────────────────────────────────────────────────

    #[test]
    fn sni_status_recording_is_needs_attention() {
        assert!(matches!(
            TrayState::Recording.sni_status(),
            Status::NeedsAttention
        ));
    }

    #[test]
    fn sni_status_stopped_is_passive() {
        assert!(matches!(TrayState::Stopped.sni_status(), Status::Passive));
    }

    #[test]
    fn sni_status_idle_and_transcribing_are_active() {
        assert!(matches!(TrayState::Idle.sni_status(), Status::Active));
        assert!(matches!(
            TrayState::Transcribing.sni_status(),
            Status::Active
        ));
    }

    // ── tooltip_body ──────────────────────────────────────────────────────────

    #[test]
    fn tooltip_body_non_empty_for_all_states() {
        for state in [
            TrayState::Idle,
            TrayState::Recording,
            TrayState::Transcribing,
            TrayState::Stopped,
        ] {
            assert!(
                !state.tooltip_body().is_empty(),
                "{:?} tooltip was empty",
                state
            );
        }
    }
}
