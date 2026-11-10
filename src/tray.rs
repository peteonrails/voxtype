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
    /// Open the model upgrade offers: Omarchy Settings on its Engine section
    /// when that plugin answers, otherwise `voxtype setup migrate` in a terminal.
    OpenModelUpgrade,
    /// `voxtype setup migrate --cleanup`: delete the files of the model an
    /// accepted upgrade replaced.
    CleanupOldModels,
}

/// The tray's reading of the model upgrade status object, the one
/// `voxtype setup migrate --json` prints and `voxtype status --format json
/// --extended` carries as `model_upgrade`. The tray only shows what that
/// object says; it never decides which model is better.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpgradeMenu {
    /// Offer titles, recommended first. Declined offers stay: they can still
    /// be accepted, so the menu keeps offering them.
    pub offers: Vec<String>,
    /// Old model files are waiting to be removed.
    pub cleanup: bool,
}

impl UpgradeMenu {
    /// Reads the contract object. Anything unexpected, including `null`,
    /// reads as "nothing to offer" rather than an error.
    pub fn from_status_value(value: &serde_json::Value) -> Self {
        let mut recommended = Vec::new();
        let mut rest = Vec::new();
        for offer in value
            .get("offers")
            .and_then(|o| o.as_array())
            .into_iter()
            .flatten()
        {
            let Some(id) = offer.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let title = ["title", "model"]
                .iter()
                .find_map(|k| offer.get(*k).and_then(|v| v.as_str()))
                .unwrap_or(id)
                .to_string();
            if offer.get("recommended").and_then(|v| v.as_bool()) == Some(true) {
                recommended.push(title);
            } else {
                rest.push(title);
            }
        }
        recommended.extend(rest);
        Self {
            offers: recommended,
            cleanup: value.get("cleanup").is_some_and(|c| c.is_object()),
        }
    }

    pub fn has_offers(&self) -> bool {
        !self.offers.is_empty()
    }
}

type Actions = Arc<dyn Fn(TrayAction) + Send + Sync>;
struct VoxtypeTray {
    state: TrayState,
    actions: Actions,
    language: Option<Arc<Mutex<crate::language_selection::LanguageMenu>>>,
    upgrade: UpgradeMenu,
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

    /// Persistent entries while an upgrade is on offer or old files are left
    /// over. No notification: the menu item (and the icon badge) is the whole
    /// prompt, and it stays until the offer is gone.
    fn upgrade_items(&self) -> Vec<ksni::MenuItem<Self>> {
        let mut items = Vec::new();
        if self.upgrade.has_offers() {
            items.push(
                StandardItem {
                    label: "Model update available...".into(),
                    icon_name: "software-update-available-symbolic".into(),
                    activate: Box::new(|this: &mut Self| {
                        (this.actions)(TrayAction::OpenModelUpgrade)
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        if self.upgrade.cleanup {
            items.push(
                StandardItem {
                    label: "Remove old model files".into(),
                    icon_name: "user-trash-symbolic".into(),
                    activate: Box::new(|this: &mut Self| {
                        (this.actions)(TrayAction::CleanupOldModels)
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        items
    }

    fn badged(&self) -> bool {
        self.upgrade.has_offers() && self.state == TrayState::Idle
    }

    fn tooltip_title(&self) -> String {
        if self.upgrade.has_offers() {
            format!(
                "VoxType: {} (model update available)",
                self.state.tooltip_body()
            )
        } else {
            format!("VoxType: {}", self.state.tooltip_body())
        }
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
        let mut icon = state_icon(self.state);
        if self.badged() {
            upgrade_badge(&mut icon);
        }
        vec![icon]
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
        let description = if self.upgrade.has_offers() {
            format!(
                "{}\n{}",
                self.state.tooltip_body(),
                self.upgrade.offers.join("\n")
            )
        } else {
            self.state.tooltip_body().into()
        };
        ToolTip {
            // Omarchy's compact tooltip displays the title, not the body.
            title: self.tooltip_title(),
            description,
            ..Default::default()
        }
    }
    // Hosts use ItemIsMenu to show the native menu. Legacy Activate is harmless.
    fn activate(&mut self, _x: i32, _y: i32) {}
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let mut menu: Vec<ksni::MenuItem<Self>> = vec![
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
        ];
        menu.extend(self.upgrade_items());
        menu.extend([
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
        ]);
        menu
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

/// A filled dot in the top-right corner, ringed in white so it reads on light
/// and dark bars alike. Only drawn on the idle icon: recording, transcribing
/// and error already carry their own marks, and those states matter more.
fn upgrade_badge(icon: &mut ksni::Icon) {
    let w = icon.width.max(0) as usize;
    let h = icon.height.max(0) as usize;
    if w < 16 || h < 16 || icon.data.len() < w * h * 4 {
        return;
    }
    let r = (w.min(h) / 8).max(4) as i64;
    let (cx, cy) = (w as i64 - r - 2, r + 2);
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let d2 = (x - cx).pow(2) + (y - cy).pow(2);
            if d2 > r * r {
                continue;
            }
            let ring = d2 > (r - 2).pow(2);
            let i = (y as usize * w + x as usize) * 4;
            // Network-order ARGB, as everywhere else in this file.
            icon.data[i..i + 4].copy_from_slice(if ring {
                &[255, 255, 255, 255]
            } else {
                &[255, 0x2f, 0x80, 0xed]
            });
        }
    }
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
    mut upgrade_rx: mpsc::UnboundedReceiver<UpgradeMenu>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let initial = *rx.borrow_and_update();
        let tray = VoxtypeTray {
            state: initial,
            actions,
            language,
            upgrade: UpgradeMenu::default(),
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
        // The state channel alone decides the tray's lifetime; upgrade senders
        // held by background refreshes must never keep it alive.
        let mut upgrades_open = true;
        loop {
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let state = *rx.borrow_and_update();
                    if handle.0.update(|tray| tray.state = state).await.is_none() {
                        break;
                    }
                }
                menu = upgrade_rx.recv(), if upgrades_open => match menu {
                    Some(menu) => {
                        if handle.0.update(|tray| tray.upgrade = menu).await.is_none() {
                            break;
                        }
                    }
                    None => upgrades_open = false,
                },
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
        mpsc::unbounded_channel().1,
    )
}

/// Spawn the daemon tray. Only explicit menu actions run the fixed CLI commands.
pub fn spawn(rx: watch::Receiver<TrayState>) -> tokio::task::JoinHandle<()> {
    spawn_inner(
        rx,
        Arc::new(dispatch_action),
        None,
        mpsc::unbounded_channel().1,
    )
}

fn dispatch_action(action: TrayAction) {
    if matches!(action, TrayAction::SelectLanguage(_)) {
        tracing::warn!("Language action requires the daemon control channel");
        return;
    }
    if action == TrayAction::OpenModelUpgrade {
        open_model_upgrade();
        return;
    }
    if action == TrayAction::CleanupOldModels {
        tokio::spawn(run_cleanup());
        return;
    }
    static OPENING: AtomicBool = AtomicBool::new(false);
    if action == TrayAction::OpenSettings && OPENING.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        // On Omarchy with the Settings panel loaded, open the panel; anything
        // else (no shell, panel not linked, hung shell) opens the TUI.
        if action == TrayAction::OpenSettings && open_settings_plugin(&["open"]).await {
            OPENING.store(false, Ordering::Release);
            return;
        }
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

// ------------------------------------------------------------ model upgrades

const SETTINGS_PLUGIN_TIMEOUT: Duration = Duration::from_secs(3);
const MIGRATE_STATUS_TIMEOUT: Duration = Duration::from_secs(30);
/// Read the offer after startup has settled, never in its way.
const MIGRATE_STATUS_DELAY: Duration = Duration::from_secs(3);

fn graphical_session() -> bool {
    ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

fn find_in_path(name: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if name.is_empty() {
        return None;
    }
    let candidate = std::path::Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// Terminals in the same order and with the same window class as
/// `voxtype-configure-launcher`, so compositor rules written for the TUI
/// (`class:^(voxtype)$`) float this window too. `$TERMINAL` wins when set.
const TERMINALS: &[&str] = &[
    "ghostty",
    "alacritty",
    "kitty",
    "foot",
    "wezterm",
    "konsole",
    "gnome-terminal",
    "xterm",
    "xfce4-terminal",
];

/// argv that opens `program` in `terminal`. The terminal's own name decides
/// the flag shape; an unknown one gets `-e`, as in the launcher.
fn terminal_argv(terminal: &str, program: &[String]) -> Vec<String> {
    let name = std::path::Path::new(terminal)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(terminal);
    let prefix: &[&str] = match name {
        "ghostty" => &["--class=voxtype", "-e"],
        "alacritty" => &["--class", "voxtype", "-e"],
        "kitty" => &["--class", "voxtype"],
        "foot" => &["--app-id", "voxtype"],
        "wezterm" => &["start", "--class", "voxtype", "--"],
        "xterm" => &["-class", "voxtype", "-e"],
        "gnome-terminal" => &["--"],
        "xfce4-terminal" => &["-x"],
        _ => &["-e"],
    };
    std::iter::once(terminal.to_string())
        .chain(prefix.iter().map(|s| s.to_string()))
        .chain(program.iter().cloned())
        .collect()
}

/// `voxtype setup migrate` under `sh`, held open afterwards so the outcome
/// stays readable. The binary path is passed as `$0`, never spliced into the
/// script, so a path with spaces or quotes cannot change what runs.
fn migrate_program(bin: &std::path::Path) -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        "\"$0\" setup migrate; status=$?; printf '\\nPress Enter to close. '; read -r _; exit $status"
            .into(),
        bin.to_string_lossy().into_owned(),
    ]
}

fn find_terminal() -> Option<std::path::PathBuf> {
    let preferred = std::env::var("TERMINAL").ok();
    preferred
        .iter()
        .map(String::as_str)
        .chain(TERMINALS.iter().copied())
        .find_map(find_in_path)
}

/// Asks the Omarchy Settings plugin to open on its Engine section.
async fn open_settings_engine_section() -> bool {
    open_settings_plugin(&["section", "Engine"]).await
}

/// Calls the Omarchy Settings plugin's IPC target (`omarchy-shell
/// io.voxtype.settings <args>`). Only a literal `ok` counts: a missing
/// shell, a missing plugin and a hung shell all fall through to the caller's
/// terminal fallback.
async fn open_settings_plugin(args: &[&str]) -> bool {
    open_settings_plugin_with(find_omarchy_shell(), args).await
}

/// `omarchy-shell` on PATH, then in Omarchy's bin directory: the daemon runs
/// under systemd, whose PATH usually lacks it.
fn find_omarchy_shell() -> Option<std::path::PathBuf> {
    find_in_path("omarchy-shell").or_else(|| find_in_path("/usr/share/omarchy/bin/omarchy-shell"))
}

async fn open_settings_plugin_with(shell: Option<std::path::PathBuf>, args: &[&str]) -> bool {
    let Some(shell) = shell else {
        return false;
    };
    let mut command = Command::new(shell);
    command
        .arg("io.voxtype.settings")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    match tokio::time::timeout(SETTINGS_PLUGIN_TIMEOUT, command.output()).await {
        Ok(Ok(out)) => out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "ok",
        _ => false,
    }
}

fn open_model_upgrade() {
    // One window at a time: held until the terminal closes, like the
    // launcher's lock for the TUI.
    static OPENING: AtomicBool = AtomicBool::new(false);
    if !graphical_session() {
        tracing::warn!("No graphical session; run `voxtype setup migrate` in a terminal");
        return;
    }
    if OPENING.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        let result: std::io::Result<()> = async {
            if open_settings_engine_section().await {
                return Ok(());
            }
            let bin = std::env::current_exe()?;
            let terminal = find_terminal().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no terminal emulator found")
            })?;
            let argv = terminal_argv(&terminal.to_string_lossy(), &migrate_program(&bin));
            Command::new(&argv[0])
                .args(&argv[1..])
                .stdin(std::process::Stdio::null())
                .status()
                .await
                .map(|_| ())
        }
        .await;
        OPENING.store(false, Ordering::Release);
        if let Err(error) = result {
            tracing::warn!(%error, "Could not open the model upgrade offer");
        }
    });
}

async fn run_cleanup() {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let result = async {
        Command::new(std::env::current_exe()?)
            .args(["setup", "migrate", "--cleanup"])
            .stdin(std::process::Stdio::null())
            .status()
            .await
    }
    .await;
    RUNNING.store(false, Ordering::Release);
    match result {
        Ok(status) if status.success() => {}
        error => tracing::warn!(?error, "Removing old model files failed"),
    }
}

/// `setup migrate --json` from this same binary. `None` when it could not be
/// read, including a build without the subcommand, which leaves the menu as
/// it was.
async fn fetch_upgrade_menu() -> Option<UpgradeMenu> {
    let mut command = Command::new(std::env::current_exe().ok()?);
    command
        .args(["setup", "migrate", "--json"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(MIGRATE_STATUS_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(UpgradeMenu::from_status_value(&value))
}

async fn refresh_upgrade(tx: mpsc::UnboundedSender<UpgradeMenu>) {
    if let Some(menu) = fetch_upgrade_menu().await {
        let _ = tx.send(menu);
    }
}

/// Daemon-owned guard. Early errors/cancellation also request service shutdown.
pub struct TrayRuntime {
    tx: Option<watch::Sender<TrayState>>,
    task: tokio::task::JoinHandle<()>,
    language: Option<Arc<Mutex<crate::language_selection::LanguageMenu>>>,
    language_rx: mpsc::UnboundedReceiver<&'static str>,
    upgrade_tx: mpsc::UnboundedSender<UpgradeMenu>,
    upgrade_refresh: Option<tokio::task::JoinHandle<()>>,
}
impl TrayRuntime {
    pub fn start(enabled: bool) -> Option<Self> {
        if !enabled {
            return None;
        }
        let (tx, rx) = watch::channel(TrayState::Idle);
        let (upgrade_tx, upgrade_rx) = mpsc::unbounded_channel();
        Some(Self {
            tx: Some(tx),
            task: spawn_inner(rx, Arc::new(dispatch_action), None, upgrade_rx),
            language: None,
            language_rx: mpsc::unbounded_channel().1,
            upgrade_tx,
            upgrade_refresh: None,
        })
    }
    /// The daemon's tray. Reads the model upgrade offer once startup has
    /// settled, and again after the tray removes old model files.
    pub fn start_for_config(config: &crate::config::Config) -> Option<Self> {
        let (upgrade_tx, upgrade_rx) = mpsc::unbounded_channel();
        let refresher = upgrade_tx.clone();
        let actions: Actions = Arc::new(move |action| {
            if action == TrayAction::CleanupOldModels {
                let refresher = refresher.clone();
                tokio::spawn(async move {
                    run_cleanup().await;
                    refresh_upgrade(refresher).await;
                });
            } else {
                dispatch_action(action);
            }
        });
        let mut runtime = Self::start_controlled(config, actions, upgrade_tx, upgrade_rx)?;
        let initial = runtime.upgrade_tx.clone();
        runtime.upgrade_refresh = Some(tokio::spawn(async move {
            tokio::time::sleep(MIGRATE_STATUS_DELAY).await;
            refresh_upgrade(initial).await;
        }));
        Some(runtime)
    }
    /// Recording-free host harness; non-language actions are delivered, never executed.
    pub fn start_for_config_with_actions(
        config: &crate::config::Config,
        actions: mpsc::Sender<TrayAction>,
    ) -> Option<Self> {
        let (upgrade_tx, upgrade_rx) = mpsc::unbounded_channel();
        Self::start_controlled(
            config,
            Arc::new(move |action| {
                let _ = actions.try_send(action);
            }),
            upgrade_tx,
            upgrade_rx,
        )
    }
    fn start_controlled(
        config: &crate::config::Config,
        other_actions: Actions,
        upgrade_tx: mpsc::UnboundedSender<UpgradeMenu>,
        upgrade_rx: mpsc::UnboundedReceiver<UpgradeMenu>,
    ) -> Option<Self> {
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
        let task = spawn_inner(rx, actions, Some(language.clone()), upgrade_rx);
        Some(Self {
            tx: Some(tx),
            task,
            language: Some(language),
            language_rx,
            upgrade_tx,
            upgrade_refresh: None,
        })
    }
    /// Replace the upgrade entries with what the status object now says, for
    /// a caller that already holds it (the daemon after computing it).
    pub fn update_upgrade(&self, menu: UpgradeMenu) {
        let _ = self.upgrade_tx.send(menu);
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
        if let Some(refresh) = self.upgrade_refresh.take() {
            refresh.abort();
        }
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
        if let Some(refresh) = self.upgrade_refresh.take() {
            refresh.abort();
        }
        self.tx.take();
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake `omarchy-shell` that logs its arguments and prints `answer`.
    fn fake_shell(dir: &std::path::Path, answer: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("omarchy-shell");
        std::fs::write(
            &path,
            format!("#!/bin/sh\necho \"$*\" >> \"$0.log\"\necho {answer}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn settings_opens_the_omarchy_panel_when_it_answers_ok() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "ok");
        assert!(open_settings_plugin_with(Some(shell.clone()), &["open"]).await);
        let log = std::fs::read_to_string(dir.path().join("omarchy-shell.log")).unwrap();
        assert_eq!(log.trim(), "io.voxtype.settings open");
    }

    #[tokio::test]
    async fn settings_falls_back_when_the_panel_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "unknown");
        assert!(!open_settings_plugin_with(Some(shell), &["open"]).await);
        assert!(!open_settings_plugin_with(None, &["open"]).await);
        assert!(!open_settings_plugin_with(Some(dir.path().join("missing")), &["open"]).await);
    }

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
            upgrade: UpgradeMenu::default(),
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
            upgrade: UpgradeMenu::default(),
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
            upgrade: UpgradeMenu::default(),
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
            upgrade: UpgradeMenu::default(),
        };
        let idle = tray.icon_pixmap();
        tray.state = TrayState::Recording;
        let recording = tray.icon_pixmap();
        tray.state = TrayState::Transcribing;
        assert_ne!(idle[0].data, recording[0].data);
        assert_ne!(recording[0].data, tray.icon_pixmap()[0].data);
        assert_ne!(idle[0].data, tray.icon_pixmap()[0].data);
    }

    // ── model upgrade offers ──────────────────────────────────────────────────

    fn contract(offers: serde_json::Value, cleanup: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "current": {"engine": "parakeet", "model": "parakeet-tdt-0.6b-v3", "format": "onnx"},
            "offers": offers,
            "removal_notice": null,
            "cleanup": cleanup,
        })
    }

    fn both_offers() -> serde_json::Value {
        // The format offer first on purpose: the recommended one must lead.
        serde_json::json!([
            {"id": "format-gguf", "kind": "format", "engine": "parakeet",
             "model": "parakeet-tdt-0.6b-v3-Q8_0.gguf",
             "title": "Your model format has been updated", "detail": "",
             "download_bytes": 739508576, "recommended": false, "declined": true},
            {"id": "parakeet-ultra", "kind": "model", "engine": "parakeet",
             "model": "parakeet-ultra-Q8_0.gguf", "title": "Parakeet Ultra is now available",
             "detail": "", "download_bytes": 740363168, "recommended": true, "declined": false}
        ])
    }

    fn tray_with(upgrade: UpgradeMenu, log: Arc<Mutex<Vec<TrayAction>>>) -> VoxtypeTray {
        VoxtypeTray {
            language: None,
            state: TrayState::Idle,
            actions: Arc::new(move |action| log.lock().unwrap().push(action)),
            upgrade,
        }
    }

    fn standard_items(tray: &VoxtypeTray) -> Vec<StandardItem<VoxtypeTray>> {
        tray.menu()
            .into_iter()
            .filter_map(|item| match item {
                ksni::MenuItem::Standard(item) => Some(item),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn upgrade_menu_reads_contract_recommended_first() {
        let menu = UpgradeMenu::from_status_value(&contract(
            both_offers(),
            serde_json::json!({"model": "parakeet-tdt-0.6b-v3", "bytes": 2600000000u64}),
        ));
        assert_eq!(
            menu.offers,
            vec![
                "Parakeet Ultra is now available".to_string(),
                "Your model format has been updated".to_string()
            ]
        );
        assert!(menu.cleanup);
        assert!(menu.has_offers());
    }

    #[test]
    fn upgrade_menu_is_empty_for_null_or_nothing_to_offer() {
        assert_eq!(
            UpgradeMenu::from_status_value(&serde_json::Value::Null),
            UpgradeMenu::default()
        );
        let none = UpgradeMenu::from_status_value(&contract(
            serde_json::json!([]),
            serde_json::Value::Null,
        ));
        assert!(!none.has_offers());
        assert!(!none.cleanup);
        // An offer without an id is not something the CLI could accept.
        let malformed = UpgradeMenu::from_status_value(&contract(
            serde_json::json!([{"title": "no id"}]),
            serde_json::Value::Null,
        ));
        assert!(!malformed.has_offers());
    }

    #[test]
    fn declined_offers_stay_in_the_menu() {
        let mut offers = both_offers();
        for offer in offers.as_array_mut().unwrap() {
            offer["declined"] = serde_json::json!(true);
        }
        let menu = UpgradeMenu::from_status_value(&contract(offers, serde_json::Value::Null));
        assert_eq!(menu.offers.len(), 2);
    }

    #[test]
    fn menu_has_no_upgrade_entries_without_offers() {
        let tray = tray_with(UpgradeMenu::default(), Arc::default());
        let labels: Vec<_> = standard_items(&tray).into_iter().map(|i| i.label).collect();
        assert!(!labels.iter().any(|l| l.starts_with("Model update")));
        assert!(!labels.iter().any(|l| l == "Remove old model files"));
        assert!(!tray.tool_tip().title.contains("model update"));
    }

    #[test]
    fn offer_entry_opens_model_upgrade() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut tray = tray_with(
            UpgradeMenu {
                offers: vec!["Parakeet Ultra is now available".into()],
                cleanup: false,
            },
            log.clone(),
        );
        let items = standard_items(&tray);
        let entry = items
            .iter()
            .find(|i| i.label == "Model update available...")
            .expect("persistent offer entry");
        assert!(entry.enabled);
        assert!(!items.iter().any(|i| i.label == "Remove old model files"));
        (entry.activate)(&mut tray);
        assert_eq!(*log.lock().unwrap(), vec![TrayAction::OpenModelUpgrade]);
        // Directly under Settings, above the language submenu.
        let menu = tray.menu();
        let position = |label: &str| {
            menu.iter()
                .position(|item| matches!(item, ksni::MenuItem::Standard(i) if i.label == label))
        };
        assert_eq!(
            position("Model update available..."),
            position("Settings").map(|p| p + 1)
        );
        let tip = tray.tool_tip();
        assert!(tip.title.contains("model update available"));
        assert!(tip.description.contains("Parakeet Ultra is now available"));
    }

    #[test]
    fn cleanup_entry_runs_cleanup_even_without_offers() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut tray = tray_with(
            UpgradeMenu {
                offers: vec![],
                cleanup: true,
            },
            log.clone(),
        );
        let items = standard_items(&tray);
        assert!(!items.iter().any(|i| i.label.starts_with("Model update")));
        let cleanup = items
            .iter()
            .find(|i| i.label == "Remove old model files")
            .expect("cleanup entry");
        (cleanup.activate)(&mut tray);
        assert_eq!(*log.lock().unwrap(), vec![TrayAction::CleanupOldModels]);
    }

    #[test]
    fn offer_badges_only_the_idle_icon() {
        let plain = tray_with(UpgradeMenu::default(), Arc::default());
        let mut offered = tray_with(
            UpgradeMenu {
                offers: vec!["x".into()],
                cleanup: false,
            },
            Arc::default(),
        );
        assert_ne!(plain.icon_pixmap()[0].data, offered.icon_pixmap()[0].data);
        // Recording keeps its own icon unmarked: that state matters more.
        offered.state = TrayState::Recording;
        let mut recording = tray_with(UpgradeMenu::default(), Arc::default());
        recording.state = TrayState::Recording;
        assert_eq!(
            offered.icon_pixmap()[0].data,
            recording.icon_pixmap()[0].data
        );
        // A badge, not a status change: hosts would otherwise flash the icon.
        assert!(matches!(offered.status(), Status::NeedsAttention));
        offered.state = TrayState::Idle;
        assert!(matches!(offered.status(), Status::Active));
    }

    #[test]
    fn terminal_argv_matches_configure_launcher_shapes() {
        let program = vec![
            "sh".to_string(),
            "-c".into(),
            "x".into(),
            "/bin/voxtype".into(),
        ];
        let argv = |t: &str| terminal_argv(t, &program);
        assert_eq!(
            argv("ghostty")[..3],
            ["ghostty", "--class=voxtype", "-e"].map(String::from)
        );
        assert_eq!(
            argv("/usr/bin/foot")[..3],
            ["/usr/bin/foot", "--app-id", "voxtype"].map(String::from)
        );
        assert_eq!(
            argv("kitty")[1..3],
            ["--class", "voxtype"].map(String::from)
        );
        assert_eq!(
            argv("wezterm")[1..5],
            ["start", "--class", "voxtype", "--"].map(String::from)
        );
        assert_eq!(argv("gnome-terminal")[1], "--");
        assert_eq!(argv("xfce4-terminal")[1], "-x");
        assert_eq!(argv("some-term")[1], "-e");
        for t in [
            "ghostty",
            "alacritty",
            "kitty",
            "foot",
            "wezterm",
            "xterm",
            "konsole",
        ] {
            assert!(
                argv(t).ends_with(&program),
                "{t} must pass the program as argv"
            );
        }
    }

    #[test]
    fn migrate_program_passes_binary_as_argument_not_script() {
        let bin = std::path::Path::new("/opt/odd path/vox'type");
        let program = migrate_program(bin);
        assert_eq!(program[..2], ["sh", "-c"].map(String::from));
        assert!(program[2].starts_with("\"$0\" setup migrate;"));
        assert!(!program[2].contains("odd path"));
        assert_eq!(program[3], "/opt/odd path/vox'type");
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
            upgrade: UpgradeMenu::default(),
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
            upgrade: UpgradeMenu::default(),
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
