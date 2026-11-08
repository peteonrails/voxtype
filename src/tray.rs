//! Linux StatusNotifierItem and dbusmenu integration, adapted from PR #438.
//! The daemon owns one state channel. ksni owns registration and watcher recovery.
//! Nothing here opens an audio device or installs files at daemon startup.

use ksni::{menu::*, Status, ToolTip, Tray, TrayMethods};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};
use tokio::{
    process::Command,
    sync::{mpsc, watch},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Recording,
    Transcribing,
    Error,
    Stopped,
}

impl TrayState {
    pub fn from_state_name(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "idle" => Self::Idle,
            "recording" | "streaming" => Self::Recording,
            "transcribing" | "outputting" | "loading" => Self::Transcribing,
            "error" => Self::Error,
            "stopped" => Self::Stopped,
            other => {
                tracing::debug!(state = other, "Unknown tray state; showing ready");
                Self::Idle
            }
        }
    }
    fn sni_status(self) -> Status {
        match self {
            Self::Recording | Self::Error => Status::NeedsAttention,
            Self::Stopped => Status::Passive,
            _ => Status::Active,
        }
    }
    fn tooltip_body(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Recording => "Recording...",
            Self::Transcribing => "Transcribing...",
            Self::Error => "Dictation failed. Check the Voxtype logs; you can try recording again.",
            Self::Stopped => "Daemon not running",
        }
    }
}

/// Closed set of actions. No user-controlled command strings or shell evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenSettings,
    SelectLanguage(&'static str),
    ToggleRecording,
    Cancel,
}

type Actions = Arc<dyn Fn(TrayAction) + Send + Sync>;
struct VoxtypeTray {
    state: TrayState,
    actions: Actions,
    language: Option<Arc<Mutex<crate::language_selection::LanguageMenu>>>,
}

impl VoxtypeTray {
    fn language_menu(&self) -> ksni::MenuItem<Self> {
        let menu = self
            .language
            .as_ref()
            .map(|m| m.lock().unwrap().clone())
            .unwrap_or_default();
        let choices = menu.choices.clone();
        let mut children: Vec<ksni::MenuItem<Self>> = vec![
            StandardItem {
                label: menu.context.replace('_', "__"),
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: menu.explanation.replace('_', "__"),
                enabled: false,
                ..Default::default()
            }
            .into(),
        ];
        if !choices.is_empty() {
            children.push(
                RadioGroup {
                    selected: choices
                        .iter()
                        .position(|c| c.0 == menu.current)
                        .unwrap_or(usize::MAX),
                    options: choices
                        .iter()
                        .map(|(code, label)| RadioItem {
                            label: format!("{label} ({code})"),
                            ..Default::default()
                        })
                        .collect(),
                    select: Box::new(move |this: &mut Self, index| {
                        if let Some((code, _)) = choices.get(index) {
                            // No optimistic checkmark: daemon persists/readbacks before publishing.
                            (this.actions)(TrayAction::SelectLanguage(code));
                        }
                    }),
                }
                .into(),
            );
        }
        SubMenu {
            label: format!(
                "Language: {}",
                if menu.current.is_empty() {
                    "unavailable"
                } else {
                    &menu.current
                }
            ),
            enabled: !children.is_empty(),
            submenu: children,
            ..Default::default()
        }
        .into()
    }
}

impl Tray for VoxtypeTray {
    const MENU_ON_ACTIVATE: bool = true;
    fn id(&self) -> String {
        "voxtype".into()
    }
    fn title(&self) -> String {
        "VoxType".into()
    }
    // Quickshell prefers a nonempty theme name even when it cannot resolve it.
    // Always publish embedded pixels, independent of optional icon installation.
    fn icon_name(&self) -> String {
        String::new()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![state_icon(self.state)]
    }
    fn attention_icon_name(&self) -> String {
        String::new()
    }
    fn attention_icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon_pixmap()
    }
    fn status(&self) -> Status {
        self.state.sni_status()
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            // Omarchy's compact tooltip displays the title, not the body.
            title: format!("VoxType: {}", self.state.tooltip_body()),
            description: self.state.tooltip_body().into(),
            ..Default::default()
        }
    }
    // Hosts use ItemIsMenu to show the native menu. Legacy Activate is harmless.
    fn activate(&mut self, _x: i32, _y: i32) {}
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem {
                label: format!("VoxType: {}", self.state.tooltip_body()),
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Settings".into(),
                icon_name: "preferences-system-symbolic".into(),
                activate: Box::new(|this: &mut Self| (this.actions)(TrayAction::OpenSettings)),
                ..Default::default()
            }
            .into(),
            self.language_menu(),
            MenuItem::Separator,
            StandardItem {
                label: if self.state == TrayState::Recording {
                    "Stop Recording"
                } else {
                    "Start Recording"
                }
                .into(),
                enabled: !matches!(self.state, TrayState::Transcribing | TrayState::Stopped),
                activate: Box::new(|this: &mut Self| {
                    if !matches!(this.state, TrayState::Transcribing | TrayState::Stopped) {
                        (this.actions)(TrayAction::ToggleRecording);
                    }
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Cancel".into(),
                enabled: matches!(self.state, TrayState::Recording | TrayState::Transcribing),
                activate: Box::new(|this: &mut Self| {
                    if matches!(this.state, TrayState::Recording | TrayState::Transcribing) {
                        (this.actions)(TrayAction::Cancel);
                    }
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        tracing::debug!(?reason, "Tray host offline; waiting for it to return");
        true
    }
}

// Embedded fallback means a read-only home or a missing icon installer is harmless.
// PNG pixels are converted to SNI's network-order ARGB, not native-endian RGBA.
fn state_icon(state: TrayState) -> ksni::Icon {
    static IDLE: OnceLock<ksni::Icon> = OnceLock::new();
    static RECORDING: OnceLock<ksni::Icon> = OnceLock::new();
    let recording = matches!(state, TrayState::Recording | TrayState::Error);
    let icon = if recording { &RECORDING } else { &IDLE }.get_or_init(|| {
        let bytes: &[u8] = if recording {
            include_bytes!("../assets/icon-recording.png")
        } else {
            include_bytes!("../assets/icon.png")
        };
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        // Bundled assets are checked by tests, not read from external files.
        let mut reader = decoder.read_info().expect("bundled tray PNG");
        let mut pixels = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut pixels)
            .expect("bundled tray PNG pixels");
        let channels = info.color_type.samples();
        let mut data = Vec::with_capacity(info.width as usize * info.height as usize * 4);
        for p in pixels[..info.buffer_size()].chunks_exact(channels) {
            data.extend_from_slice(&[if channels == 4 { p[3] } else { 255 }, p[0], p[1], p[2]]);
        }
        ksni::Icon {
            width: info.width as i32,
            height: info.height as i32,
            data,
        }
    });
    let mut icon = icon.clone();
    // Distinct high-contrast badges also distinguish busy/error without color.
    if matches!(state, TrayState::Transcribing | TrayState::Error) {
        let w = icon.width as usize;
        for y in 4..36 {
            for x in (w - 36)..(w - 4) {
                let i = (y * w + x) * 4;
                let mark = if state == TrayState::Error {
                    (x > w - 23 && x < w - 17) && !(24..=28).contains(&y)
                } else {
                    y == 9 || y == 30 || (x + y) % 16 < 3
                };
                icon.data[i..i + 4].copy_from_slice(if mark {
                    &[255, 255, 255, 255]
                } else {
                    &[255, 30, 30, 30]
                });
            }
        }
    }
    icon
}

/// Explicit shutdown is essential: dropping a ksni Handle alone does not close it.
struct ServiceGuard(ksni::Handle<VoxtypeTray>);
impl Drop for ServiceGuard {
    fn drop(&mut self) {
        drop(self.0.shutdown());
    }
}

fn spawn_inner(
    mut rx: watch::Receiver<TrayState>,
    actions: Actions,
    language: Option<Arc<Mutex<crate::language_selection::LanguageMenu>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let initial = *rx.borrow_and_update();
        let tray = VoxtypeTray {
            state: initial,
            actions,
            language,
        };
        let start = tray.assume_sni_available(true).spawn();
        // Bounded, cancellable startup. D-Bus must never hold dictation startup hostage.
        let handle = tokio::select! {
            _ = rx.wait_for(|_| false) => return,
            result = tokio::time::timeout(Duration::from_secs(5), start) => match result {
                Ok(Ok(h)) => ServiceGuard(h),
                Ok(Err(error)) => { tracing::warn!(?error, "Native tray unavailable; dictation continues"); return; }
                Err(_) => { tracing::warn!("Native tray startup timed out; dictation continues"); return; }
            }
        };
        // Reflect a state that arrived during registration.
        let state = *rx.borrow_and_update();
        handle.0.update(|tray| tray.state = state).await;
        while rx.changed().await.is_ok() {
            let state = *rx.borrow_and_update();
            if handle.0.update(|tray| tray.state = state).await.is_none() {
                break;
            }
        }
        handle.0.shutdown().await;
    })
}

/// Starts a tray with recording-free actions for protocol and host probes.
pub fn spawn_with_actions(
    rx: watch::Receiver<TrayState>,
    actions: mpsc::Sender<TrayAction>,
) -> tokio::task::JoinHandle<()> {
    spawn_inner(
        rx,
        Arc::new(move |action| {
            let _ = actions.try_send(action);
        }),
        None,
    )
}

/// Spawn the daemon tray. Only explicit menu actions run the fixed CLI commands.
pub fn spawn(rx: watch::Receiver<TrayState>) -> tokio::task::JoinHandle<()> {
    spawn_inner(rx, Arc::new(dispatch_action), None)
}

fn dispatch_action(action: TrayAction) {
    if matches!(action, TrayAction::SelectLanguage(_)) {
        tracing::warn!("Language action requires the daemon control channel");
        return;
    }
    static OPENING: AtomicBool = AtomicBool::new(false);
    if action == TrayAction::OpenSettings && OPENING.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        let result = async {
            let bin = std::env::current_exe()?;
            let mut command = if action == TrayAction::OpenSettings {
                let companion = bin
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("voxtype-configure-launcher");
                Command::new(if companion.is_file() {
                    companion
                } else {
                    std::path::PathBuf::from("voxtype-configure-launcher")
                })
            } else {
                let mut command = Command::new(bin);
                command.args([
                    "record",
                    if action == TrayAction::Cancel {
                        "cancel"
                    } else {
                        "toggle"
                    },
                ]);
                command
            };
            command.stdin(std::process::Stdio::null()).status().await
        }
        .await;
        if action == TrayAction::OpenSettings {
            OPENING.store(false, Ordering::Release);
        }
        match result {
            Ok(status) if status.success() => {}
            error => tracing::warn!(?action, ?error, "Tray action failed"),
        }
    });
}

/// Daemon-owned guard. Early errors/cancellation also request service shutdown.
pub struct TrayRuntime {
    tx: Option<watch::Sender<TrayState>>,
    task: tokio::task::JoinHandle<()>,
    language: Option<Arc<Mutex<crate::language_selection::LanguageMenu>>>,
    language_rx: mpsc::UnboundedReceiver<&'static str>,
}
impl TrayRuntime {
    pub fn start(enabled: bool) -> Option<Self> {
        if !enabled {
            return None;
        }
        let (tx, rx) = watch::channel(TrayState::Idle);
        Some(Self {
            tx: Some(tx),
            task: spawn(rx),
            language: None,
            language_rx: mpsc::unbounded_channel().1,
        })
    }
    pub fn start_for_config(config: &crate::config::Config) -> Option<Self> {
        Self::start_controlled(config, Arc::new(dispatch_action))
    }
    /// Recording-free host harness; non-language actions are delivered, never executed.
    pub fn start_for_config_with_actions(
        config: &crate::config::Config,
        actions: mpsc::Sender<TrayAction>,
    ) -> Option<Self> {
        Self::start_controlled(
            config,
            Arc::new(move |action| {
                let _ = actions.try_send(action);
            }),
        )
    }
    fn start_controlled(config: &crate::config::Config, other_actions: Actions) -> Option<Self> {
        if !config.tray.enabled {
            return None;
        }
        let (tx, rx) = watch::channel(TrayState::Idle);
        let language = Arc::new(Mutex::new(
            crate::language_selection::LanguageMenu::for_config(config),
        ));
        let (language_tx, language_rx) = mpsc::unbounded_channel();
        let actions = Arc::new(move |action| match action {
            TrayAction::SelectLanguage(code) => {
                let _ = language_tx.send(code);
            }
            action => other_actions(action),
        });
        let task = spawn_inner(rx, actions, Some(language.clone()));
        Some(Self {
            tx: Some(tx),
            task,
            language: Some(language),
            language_rx,
        })
    }
    pub async fn next_language(&mut self) -> Option<&'static str> {
        self.language_rx.recv().await
    }
    pub fn update_language(&self, menu: crate::language_selection::LanguageMenu) {
        if let Some(language) = &self.language {
            *language.lock().unwrap() = menu;
        }
        if let Some(tx) = &self.tx {
            tx.send_modify(|_| {});
        }
    }
    pub fn update(&self, state: TrayState) {
        if let Some(tx) = &self.tx {
            tx.send_if_modified(|old| {
                if *old == state {
                    false
                } else {
                    *old = state;
                    true
                }
            });
        }
    }
    pub async fn shutdown(mut self) {
        self.tx.take();
        if tokio::time::timeout(Duration::from_secs(2), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
        }
    }
}
impl Drop for TrayRuntime {
    fn drop(&mut self) {
        self.tx.take();
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_context_preserves_model_identifier_underscores() {
        let mut config = crate::config::Config::default();
        config.set_model_for(
            crate::config::TranscriptionEngine::Cohere,
            "cohere-transcribe-03-2026-Q4_K_M.gguf",
        );
        let tray = VoxtypeTray {
            state: TrayState::Idle,
            actions: Arc::new(|_| {}),
            language: Some(Arc::new(Mutex::new(
                crate::language_selection::LanguageMenu::for_config(&config),
            ))),
        };
        let ksni::MenuItem::SubMenu(menu) = tray.language_menu() else {
            panic!("submenu missing")
        };
        let ksni::MenuItem::Standard(context) = &menu.submenu[0] else {
            panic!("context missing")
        };
        assert!(
            context.label.contains("Q4__K__M.gguf"),
            "SNI mnemonic escaping: {}",
            context.label
        );
    }

    #[test]
    fn language_submenu_is_present() {
        let tray = VoxtypeTray {
            language: None,
            state: TrayState::Idle,
            actions: Arc::new(|_| {}),
        };
        assert!(tray.menu().iter().any(|item| matches!(item,
            ksni::MenuItem::SubMenu(menu) if menu.label.starts_with("Language"))));
    }

    #[test]
    fn menu_offers_app_without_duplicate_recording_actions() {
        let tray = VoxtypeTray {
            language: None,
            state: TrayState::Transcribing,
            actions: std::sync::Arc::new(|_| {}),
        };
        let menu = tray.menu();
        let items: Vec<_> = menu
            .iter()
            .filter_map(|item| match item {
                ksni::MenuItem::Standard(item) => Some(item),
                _ => None,
            })
            .collect();
        assert!(items.iter().any(|item| item.label == "Settings"));
        assert_eq!(
            items
                .iter()
                .filter(|item| item.label.contains("Recording"))
                .count(),
            1
        );
        assert!(
            !items
                .iter()
                .find(|item| item.label.contains("Recording"))
                .unwrap()
                .enabled
        );
        assert!(
            !tray.icon_pixmap().is_empty(),
            "no icon installation must be required"
        );
        assert_eq!(
            format!("{:?}", TrayState::from_state_name("error")),
            "Error"
        );
    }

    #[test]
    fn recording_and_transcription_have_distinct_icons() {
        let mut tray = VoxtypeTray {
            language: None,
            state: TrayState::Idle,
            actions: std::sync::Arc::new(|_| {}),
        };
        let idle = tray.icon_pixmap();
        tray.state = TrayState::Recording;
        let recording = tray.icon_pixmap();
        tray.state = TrayState::Transcribing;
        assert_ne!(idle[0].data, recording[0].data);
        assert_ne!(recording[0].data, tray.icon_pixmap()[0].data);
        assert_ne!(idle[0].data, tray.icon_pixmap()[0].data);
    }

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
    fn from_str_unknown_is_idle() {
        // Unknown state names default to Idle (not Stopped) so the tray does
        // not falsely report "Daemon not running" if a new state is added.
        assert_eq!(TrayState::from_state_name("whatever"), TrayState::Idle);
        assert_eq!(TrayState::from_state_name(""), TrayState::Idle);
    }

    #[test]
    fn from_str_stopped_is_explicit() {
        assert_eq!(TrayState::from_state_name("stopped"), TrayState::Stopped);
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
    fn idle_pixmap_is_not_shadowed_by_missing_theme() {
        use ksni::Tray;
        // Must match the filename stem written by setup::icons::install().
        let t = VoxtypeTray {
            language: None,
            state: TrayState::Idle,
            actions: std::sync::Arc::new(|_| {}),
        };
        assert!(t.icon_name().is_empty());
    }

    #[test]
    fn attention_pixmap_is_not_shadowed_by_missing_theme() {
        use ksni::Tray;
        let t = VoxtypeTray {
            language: None,
            state: TrayState::Idle,
            actions: std::sync::Arc::new(|_| {}),
        };
        assert!(t.attention_icon_name().is_empty());
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
