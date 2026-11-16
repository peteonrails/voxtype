//! fcitx5 text output
//!
//! Commits text into the focused text field through the fcitx5 input method,
//! the way a finished IME composition arrives: no synthesized key presses
//! and no clipboard. It needs the separate fcitx5-dictation add-on, which serves
//! `org.fcitx.Fcitx5.Dictation1` at `/dictation` on the session bus:
//!
//! - `CommitTo(as windows, s text) -> s` types into a focused input context
//!   whose program matches one of `windows` exactly
//! - `Focus() -> s` reports fcitx5's newest focused context without typing
//!
//! Each reply is `ok:<app>`, `password:<app>` (refused), `nofocus` or `empty`.
//!
//! fcitx5 alone cannot say which text field the user is looking at: some
//! apps never report losing focus (a shell can keep a hidden lock-screen
//! password box "focused"), and text committed there vanishes. So the
//! compositor is asked which window has focus, and `CommitTo` gets that
//! window's names (app id, class, program name). Hyprland, Sway and niri can
//! be asked. Elsewhere, or when the compositor doesn't answer, nothing is
//! typed and the driver fails like any other, so the chain moves on: the
//! add-on's plain `Commit` trusts fcitx5 alone, which such a hidden field
//! fools, so this driver never uses it.
//!
//! Known gap: switching windows in the few milliseconds between asking the
//! compositor and committing.
//!
//! When the compositor says no window has focus, or the add-on finds no
//! focused text field in that window, or it is a password field, this driver
//! fails with [`OutputError::NoTextField`] / [`OutputError::PasswordField`]
//! so the chain falls through, normally to the clipboard, and the daemon
//! reports that distinct outcome.
//!
//! Text is committed in one call; the add-on and fcitx5's frontend split it
//! into protocol-sized pieces, so there are no per-character delays.

use super::TextOutput;
use crate::error::OutputError;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// A reply from the dictation add-on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reply {
    /// Typed into (or, from `Focus`, would type into) this program.
    Ok(String),
    /// The focused field in this program is a password or sensitive field.
    Password(String),
    /// No text field has focus.
    NoFocus,
    /// Nothing to type.
    Empty,
    /// Anything else (a newer add-on, or a broken one).
    Unknown(String),
}

/// Parse a reply string from the add-on.
fn parse_reply(reply: &str) -> Reply {
    if let Some(app) = reply.strip_prefix("ok:") {
        Reply::Ok(app.to_string())
    } else if let Some(app) = reply.strip_prefix("password:") {
        Reply::Password(app.to_string())
    } else if reply == "nofocus" {
        Reply::NoFocus
    } else if reply == "empty" {
        Reply::Empty
    } else {
        Reply::Unknown(reply.to_string())
    }
}

/// What the compositor says about the focused window.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FocusedWindow {
    /// Names the focused window goes by, deduplicated.
    Names(Vec<String>),
    /// The compositor answered: no window has focus.
    NoneFocused,
    /// No compositor we know how to ask, or it didn't answer properly.
    Unknown,
}

/// The add-on's D-Bus methods. A trait so tests need no fcitx5.
#[async_trait::async_trait]
trait DictationBus: Send + Sync {
    async fn commit_to(&self, windows: &[String], text: &str) -> Result<String, String>;
    async fn focus(&self) -> Result<String, String>;
}

/// Where the focused window's names come from. A trait so tests need no
/// compositor.
#[async_trait::async_trait]
trait WindowSource: Send + Sync {
    async fn focused_window(&self) -> FocusedWindow;
}

/// fcitx5-based text output
pub struct Fcitx5Output {
    bus: Box<dyn DictationBus>,
    windows: Box<dyn WindowSource>,
    /// Text to append after transcription
    append_text: Option<String>,
}

impl Fcitx5Output {
    /// Create an fcitx5 output that talks to the add-on on the session bus
    /// and asks the running compositor for the focused window.
    /// A new output is built for every dictation, so the auto_submit
    /// warning is logged once per process.
    pub fn new(auto_submit: bool, append_text: Option<String>) -> Self {
        if auto_submit {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                tracing::warn!("fcitx5 output cannot press Enter; auto_submit is ignored")
            });
        }
        Self::with_backends(
            Box::new(SessionBus::default()),
            Box::new(CompositorWindows),
            append_text,
        )
    }

    fn with_backends(
        bus: Box<dyn DictationBus>,
        windows: Box<dyn WindowSource>,
        append_text: Option<String>,
    ) -> Self {
        Self {
            bus,
            windows,
            append_text,
        }
    }
}

#[async_trait::async_trait]
impl TextOutput for Fcitx5Output {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        let text = match &self.append_text {
            Some(append) => format!("{}{}", text, append),
            None => text.to_string(),
        };
        if text.is_empty() {
            return Ok(());
        }

        let reply = match self.windows.focused_window().await {
            FocusedWindow::Names(names) => {
                tracing::debug!("fcitx5: focused window goes by {:?}", names);
                self.bus.commit_to(&names, &text).await
            }
            FocusedWindow::NoneFocused => return Err(OutputError::NoTextField),
            // Focus can't be confirmed: fail like any driver, rather than
            // guess or claim there is no text field.
            FocusedWindow::Unknown => {
                static WARNED: std::sync::Once = std::sync::Once::new();
                WARNED.call_once(|| {
                    tracing::warn!(
                        "fcitx5 output needs Hyprland, Sway or niri to confirm the focused \
                         window; falling back to the next output driver"
                    )
                });
                return Err(OutputError::InjectionFailed(
                    "fcitx5: can't confirm the focused window on this compositor".to_string(),
                ));
            }
        }
        .map_err(|e| OutputError::InjectionFailed(format!("fcitx5: {}", e)))?;

        match parse_reply(&reply) {
            Reply::Ok(app) => {
                tracing::debug!("fcitx5 committed {} chars to {}", text.len(), app);
                Ok(())
            }
            // Text was checked non-empty above, so "empty" means it was lost.
            Reply::Empty => Err(OutputError::InjectionFailed(
                "fcitx5: add-on received no text".to_string(),
            )),
            Reply::NoFocus => Err(OutputError::NoTextField),
            Reply::Password(app) => Err(OutputError::PasswordField(app)),
            Reply::Unknown(other) => Err(OutputError::InjectionFailed(format!(
                "fcitx5: unexpected reply {:?}",
                other
            ))),
        }
    }

    /// Always worth trying on Linux, without a D-Bus round trip: an
    /// unreachable add-on makes `output` fail, and the chain moves on.
    async fn is_available(&self) -> bool {
        cfg!(target_os = "linux")
    }

    fn name(&self) -> &'static str {
        "fcitx5"
    }
}

/// Whether the fcitx5-dictation add-on answers on the session bus, for
/// `voxtype setup`.
pub(crate) async fn add_on_reachable() -> bool {
    match SessionBus::default().focus().await {
        Ok(_) => true,
        Err(e) => {
            tracing::debug!("fcitx5-dictation add-on not reachable: {}", e);
            false
        }
    }
}

/// The add-on on the user's session bus.
#[derive(Default)]
struct SessionBus {
    #[cfg(target_os = "linux")]
    proxy: tokio::sync::OnceCell<zbus::Proxy<'static>>,
}

#[cfg(target_os = "linux")]
impl SessionBus {
    const DEST: &'static str = "org.fcitx.Fcitx5";
    const PATH: &'static str = "/dictation";
    const IFACE: &'static str = "org.fcitx.Fcitx5.Dictation1";
    /// Bound on each D-Bus call, connecting included, so a wedged fcitx5 or
    /// session bus can't hold up the chain's fallback to the clipboard. A
    /// dictation makes one call (CommitTo) after the compositor query (at
    /// most 2 s), so the driver gives up within about 5 s.
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

    async fn proxy(&self) -> zbus::Result<&zbus::Proxy<'static>> {
        self.proxy
            .get_or_try_init(|| async {
                let conn = zbus::Connection::session().await?;
                zbus::ProxyBuilder::new(&conn)
                    .destination(Self::DEST)?
                    .path(Self::PATH)?
                    .interface(Self::IFACE)?
                    .cache_properties(zbus::CacheProperties::No)
                    .build()
                    .await
            })
            .await
    }

    async fn call<B>(&self, method: &str, body: &B) -> Result<String, String>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let call = async {
            let proxy = self.proxy().await.map_err(|e| e.to_string())?;
            // NoAutoStart: fcitx5 is D-Bus activatable, and probing for the
            // add-on must never launch an input method the user isn't running.
            proxy
                .call_with_flags::<_, _, String>(
                    method,
                    zbus::MethodFlags::NoAutoStart.into(),
                    body,
                )
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("{} returned no reply", method))
        };
        tokio::time::timeout(Self::TIMEOUT, call)
            .await
            .map_err(|_| format!("{} timed out", method))?
    }
}

#[cfg(target_os = "linux")]
#[async_trait::async_trait]
impl DictationBus for SessionBus {
    async fn commit_to(&self, windows: &[String], text: &str) -> Result<String, String> {
        self.call("CommitTo", &(windows, text)).await
    }

    async fn focus(&self) -> Result<String, String> {
        self.call("Focus", &()).await
    }
}

#[cfg(not(target_os = "linux"))]
#[async_trait::async_trait]
impl DictationBus for SessionBus {
    async fn commit_to(&self, _windows: &[String], _text: &str) -> Result<String, String> {
        Err("fcitx5 output is Linux-only".to_string())
    }

    async fn focus(&self) -> Result<String, String> {
        Err("fcitx5 output is Linux-only".to_string())
    }
}

/// Asks Hyprland (`hyprctl`), Sway (`swaymsg`) or niri (`niri msg`) for the
/// focused window.
struct CompositorWindows;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Compositor {
    Hyprland,
    Sway,
    Niri,
}

impl Compositor {
    fn detect() -> Option<Self> {
        let set = |var| std::env::var_os(var).is_some();
        if set("HYPRLAND_INSTANCE_SIGNATURE") {
            Some(Compositor::Hyprland)
        } else if set("SWAYSOCK") {
            Some(Compositor::Sway)
        } else if set("NIRI_SOCKET") {
            Some(Compositor::Niri)
        } else {
            None
        }
    }

    fn command(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Compositor::Hyprland => ("hyprctl", &["activewindow", "-j"]),
            Compositor::Sway => ("swaymsg", &["-t", "get_tree"]),
            Compositor::Niri => ("niri", &["msg", "-j", "focused-window"]),
        }
    }
}

#[async_trait::async_trait]
impl WindowSource for CompositorWindows {
    async fn focused_window(&self) -> FocusedWindow {
        let Some(compositor) = Compositor::detect() else {
            return FocusedWindow::Unknown;
        };
        let (program, args) = compositor.command();
        match ask(program, args).await {
            Some(stdout) => focused_window_from(compositor, &stdout, Path::new("/proc")),
            None => FocusedWindow::Unknown,
        }
    }
}

/// Run a compositor query; its output only if it ran and succeeded.
async fn ask(program: &str, args: &[&str]) -> Option<Vec<u8>> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    match output {
        Ok(Ok(out)) if out.status.success() => Some(out.stdout),
        Ok(Ok(out)) => {
            tracing::warn!("fcitx5: {} failed ({})", program, out.status);
            None
        }
        Ok(Err(e)) => {
            tracing::warn!("fcitx5: couldn't run {}: {}", program, e);
            None
        }
        Err(_) => {
            tracing::warn!("fcitx5: {} did not answer within 2s", program);
            None
        }
    }
}

/// Read a compositor's successful answer. An empty answer means no window
/// has focus; anything that isn't JSON can't be trusted.
fn focused_window_from(compositor: Compositor, stdout: &[u8], proc_root: &Path) -> FocusedWindow {
    let json: serde_json::Value = match serde_json::from_slice(stdout) {
        Ok(v) => v,
        Err(_) if stdout.trim_ascii().is_empty() => return FocusedWindow::NoneFocused,
        Err(e) => {
            tracing::warn!("fcitx5: unreadable answer from {:?}: {}", compositor, e);
            return FocusedWindow::Unknown;
        }
    };
    let (names, pid) = match compositor {
        Compositor::Hyprland => hyprland_window(&json),
        Compositor::Sway => sway_focused_window(&json),
        Compositor::Niri => niri_window(&json),
    };
    let names = with_program_names(names, pid, proc_root);
    if names.is_empty() {
        FocusedWindow::NoneFocused
    } else {
        FocusedWindow::Names(names)
    }
}

/// Class, initial class and pid from `hyprctl activewindow -j`.
fn hyprland_window(json: &serde_json::Value) -> (Vec<String>, Option<u32>) {
    let names = ["class", "initialClass"]
        .iter()
        .filter_map(|key| json.get(key).and_then(|v| v.as_str()))
        .map(str::to_string)
        .collect();
    (names, json_pid(json))
}

/// App id, X11 class and pid of the focused node in `swaymsg -t get_tree`.
fn sway_focused_window(json: &serde_json::Value) -> (Vec<String>, Option<u32>) {
    fn find(node: &serde_json::Value) -> Option<&serde_json::Value> {
        if node.get("focused").and_then(|f| f.as_bool()) == Some(true) {
            return Some(node);
        }
        ["nodes", "floating_nodes"]
            .iter()
            .filter_map(|key| node.get(key).and_then(|n| n.as_array()))
            .flatten()
            .find_map(find)
    }
    let Some(node) = find(json) else {
        return (Vec::new(), None);
    };
    let names = [
        node.get("app_id"),
        node.get("window_properties").and_then(|p| p.get("class")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|v| v.as_str())
    .map(str::to_string)
    .collect();
    (names, json_pid(node))
}

/// App id and pid from `niri msg -j focused-window` (`null` when no window
/// has focus).
fn niri_window(json: &serde_json::Value) -> (Vec<String>, Option<u32>) {
    let names = json
        .get("app_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .into_iter()
        .collect();
    (names, json_pid(json))
}

fn json_pid(json: &serde_json::Value) -> Option<u32> {
    json.get("pid")
        .and_then(|p| p.as_u64())
        .and_then(|p| u32::try_from(p).ok())
        .filter(|&p| p > 0)
}

/// Add the file name of the executable behind `pid`, then drop empty names
/// and duplicates. fcitx5 knows a Qt app (D-Bus frontend) by that file name,
/// which often differs from its Wayland app id. Shared runtimes are skipped:
/// "python3" or "quickshell" could just as well name another app's hidden
/// "focused" field.
fn with_program_names(mut names: Vec<String>, pid: Option<u32>, proc_root: &Path) -> Vec<String> {
    if let Some(pid) = pid {
        if let Ok(exe) = std::fs::read_link(proc_root.join(pid.to_string()).join("exe")) {
            if let Some(file) = exe.file_name() {
                let file = file.to_string_lossy();
                // An executable upgraded while running reads "name (deleted)".
                let file = file.strip_suffix(" (deleted)").unwrap_or(&file);
                if !is_shared_runtime(file) {
                    names.push(file.to_string());
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    names.retain(|n| !n.is_empty() && seen.insert(n.clone()));
    names
}

/// Interpreters and launchers many unrelated apps run under.
fn is_shared_runtime(file: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "python", "perl", "ruby", "node", "electron", "java", "wine", "flatpak", "AppRun",
    ];
    const EXACT: &[&str] = &["qs", "quickshell", "bwrap", "bash", "sh"];
    EXACT.contains(&file) || PREFIXES.iter().any(|p| file.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn parses_every_reply() {
        assert_eq!(parse_reply("ok:firefox"), Reply::Ok("firefox".into()));
        assert_eq!(parse_reply("ok:"), Reply::Ok(String::new()));
        assert_eq!(
            parse_reply("password:keepassxc"),
            Reply::Password("keepassxc".into())
        );
        assert_eq!(parse_reply("nofocus"), Reply::NoFocus);
        assert_eq!(parse_reply("empty"), Reply::Empty);
        assert_eq!(parse_reply("weird"), Reply::Unknown("weird".into()));
        assert_eq!(parse_reply(""), Reply::Unknown(String::new()));
    }

    /// Records calls and answers each with a fixed reply.
    #[derive(Clone, Default)]
    struct FakeBus {
        reply: Option<String>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeBus {
        fn replying(reply: &str) -> Self {
            Self {
                reply: Some(reply.to_string()),
                ..Default::default()
            }
        }
        fn answer(&self, call: String) -> Result<String, String> {
            self.calls.lock().unwrap().push(call);
            self.reply
                .clone()
                .ok_or_else(|| "ServiceUnknown".to_string())
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl DictationBus for FakeBus {
        async fn commit_to(&self, windows: &[String], text: &str) -> Result<String, String> {
            self.answer(format!("CommitTo {:?} {}", windows, text))
        }
        async fn focus(&self) -> Result<String, String> {
            self.answer("Focus".to_string())
        }
    }

    struct FixedWindow(FocusedWindow);

    #[async_trait::async_trait]
    impl WindowSource for FixedWindow {
        async fn focused_window(&self) -> FocusedWindow {
            self.0.clone()
        }
    }

    fn driver(bus: &FakeBus, window: FocusedWindow) -> Fcitx5Output {
        Fcitx5Output::with_backends(Box::new(bus.clone()), Box::new(FixedWindow(window)), None)
    }

    fn names(n: &[&str]) -> FocusedWindow {
        FocusedWindow::Names(n.iter().map(|s| s.to_string()).collect())
    }

    #[tokio::test]
    async fn commits_to_the_focused_window() {
        let bus = FakeBus::replying("ok:firefox");
        let out = driver(&bus, names(&["firefox"]));
        out.output("hello").await.unwrap();
        assert_eq!(bus.calls(), vec![r#"CommitTo ["firefox"] hello"#]);
    }

    #[tokio::test]
    async fn unconfirmed_focus_is_an_ordinary_failure() {
        // Not "no text field": on a compositor we can't ask there may well be
        // one, so the chain must fall through without a copied outcome.
        let bus = FakeBus::replying("ok:foot");
        let err = driver(&bus, FocusedWindow::Unknown)
            .output("hi")
            .await
            .unwrap_err();
        assert!(matches!(err, OutputError::InjectionFailed(_)), "{err:?}");
        assert!(bus.calls().is_empty());
    }

    #[tokio::test]
    async fn no_focused_window_is_no_text_field_without_calling_fcitx5() {
        let bus = FakeBus::replying("ok:shell");
        let err = driver(&bus, FocusedWindow::NoneFocused)
            .output("hi")
            .await
            .unwrap_err();
        assert!(matches!(err, OutputError::NoTextField));
        assert!(bus.calls().is_empty());
    }

    #[tokio::test]
    async fn refusals_become_distinct_errors() {
        let bus = FakeBus::replying("nofocus");
        let err = driver(&bus, names(&["x"])).output("a").await.unwrap_err();
        assert!(matches!(err, OutputError::NoTextField));

        let bus = FakeBus::replying("password:keepassxc");
        let err = driver(&bus, names(&["x"])).output("a").await.unwrap_err();
        assert!(matches!(err, OutputError::PasswordField(app) if app == "keepassxc"));

        let bus = FakeBus::replying("surprise");
        let err = driver(&bus, names(&["x"])).output("a").await.unwrap_err();
        assert!(matches!(err, OutputError::InjectionFailed(_)));

        let bus = FakeBus::default(); // add-on gone
        let err = driver(&bus, names(&["x"])).output("a").await.unwrap_err();
        assert!(matches!(err, OutputError::InjectionFailed(_)));
    }

    #[tokio::test]
    async fn appends_text_and_skips_empty_output() {
        let bus = FakeBus::replying("ok:foot");
        let out = Fcitx5Output::with_backends(
            Box::new(bus.clone()),
            Box::new(FixedWindow(names(&["foot"]))),
            Some(" ".to_string()),
        );
        out.output("hi").await.unwrap();
        assert_eq!(bus.calls(), vec![r#"CommitTo ["foot"] hi "#]);

        let bus = FakeBus::replying("ok:foot");
        driver(&bus, names(&["foot"])).output("").await.unwrap();
        assert!(bus.calls().is_empty());
    }

    #[tokio::test]
    async fn availability_costs_no_d_bus_call() {
        let down = FakeBus::default();
        assert!(driver(&down, FocusedWindow::Unknown).is_available().await);
        assert!(down.calls().is_empty());
    }

    #[tokio::test]
    async fn an_empty_reply_to_real_text_is_a_failure() {
        let bus = FakeBus::replying("empty");
        let err = driver(&bus, names(&["x"])).output("a").await.unwrap_err();
        assert!(matches!(err, OutputError::InjectionFailed(_)), "{err:?}");
    }

    #[test]
    fn reads_hyprland_active_window() {
        let json = serde_json::json!({
            "class": "org.mozilla.firefox",
            "initialClass": "org.mozilla.firefox",
            "pid": 4242
        });
        let (names, pid) = hyprland_window(&json);
        assert_eq!(names, vec!["org.mozilla.firefox", "org.mozilla.firefox"]);
        assert_eq!(pid, Some(4242));
        assert_eq!(hyprland_window(&serde_json::json!({})), (vec![], None));
    }

    #[test]
    fn reads_sways_focused_node() {
        let tree = serde_json::json!({
            "focused": false,
            "nodes": [{
                "focused": false,
                "nodes": [
                    {"focused": false, "app_id": "foot", "pid": 1},
                ],
                "floating_nodes": [
                    {"focused": true, "app_id": null,
                     "window_properties": {"class": "Code"}, "pid": 77}
                ]
            }]
        });
        assert_eq!(
            sway_focused_window(&tree),
            (vec!["Code".to_string()], Some(77))
        );

        // A focused empty workspace has no app: nothing to type into.
        let empty = serde_json::json!({"nodes": [{"focused": true, "type": "workspace"}]});
        assert_eq!(sway_focused_window(&empty), (vec![], None));
    }

    #[tokio::test]
    async fn a_failed_compositor_query_is_no_answer() {
        assert_eq!(
            ask("sh", &["-c", "echo '{}'"]).await.as_deref(),
            Some(&b"{}\n"[..])
        );
        // Exits non-zero, even with JSON on stdout (e.g. a lost socket).
        assert_eq!(ask("sh", &["-c", "echo '{}'; exit 1"]).await, None);
        assert_eq!(ask("voxtype-no-such-program", &[]).await, None);
    }

    #[test]
    fn reads_niris_focused_window() {
        let json = serde_json::json!({
            "id": 12, "title": "~", "app_id": "foot", "pid": 3141,
            "workspace_id": 1, "is_focused": true
        });
        assert_eq!(niri_window(&json), (vec!["foot".to_string()], Some(3141)));
        assert_eq!(niri_window(&serde_json::Value::Null), (vec![], None));
    }

    #[test]
    fn only_a_clean_empty_answer_means_nothing_has_focus() {
        let no_proc = Path::new("/nonexistent");
        let read = |c, out: &str| focused_window_from(c, out.as_bytes(), no_proc);
        assert_eq!(
            read(Compositor::Hyprland, r#"{"class":"foot","pid":1}"#),
            names(&["foot"])
        );
        assert_eq!(read(Compositor::Hyprland, "{}"), FocusedWindow::NoneFocused);
        assert_eq!(read(Compositor::Niri, "null\n"), FocusedWindow::NoneFocused);
        assert_eq!(read(Compositor::Sway, ""), FocusedWindow::NoneFocused);
        // Garbage is not an answer: focus can't be confirmed.
        assert_eq!(
            read(Compositor::Hyprland, "Invalid"),
            FocusedWindow::Unknown
        );
        assert_eq!(
            read(Compositor::Niri, "Error: no socket"),
            FocusedWindow::Unknown
        );
    }

    #[test]
    fn adds_program_names_and_dedupes() {
        let root = tempfile::tempdir().unwrap();
        let proc_dir = root.path().join("77");
        std::fs::create_dir(&proc_dir).unwrap();
        std::os::unix::fs::symlink("/usr/lib/firefox/firefox", proc_dir.join("exe")).unwrap();

        let names = with_program_names(
            vec![
                "firefox".into(),
                "".into(),
                "firefox".into(),
                "org.mozilla.firefox".into(),
            ],
            Some(77),
            root.path(),
        );
        assert_eq!(names, vec!["firefox", "org.mozilla.firefox"]);

        // Upgraded while running: the " (deleted)" suffix is dropped.
        let kate = root.path().join("78");
        std::fs::create_dir(&kate).unwrap();
        std::os::unix::fs::symlink("/usr/bin/kate (deleted)", kate.join("exe")).unwrap();
        assert_eq!(
            with_program_names(vec!["org.kde.kate".into()], Some(78), root.path()),
            vec!["org.kde.kate", "kate"]
        );

        // Shared runtimes never count as the window's name.
        let py = root.path().join("79");
        std::fs::create_dir(&py).unwrap();
        std::os::unix::fs::symlink("/usr/bin/python3.14", py.join("exe")).unwrap();
        assert_eq!(
            with_program_names(vec!["gtkentry.py".into()], Some(79), root.path()),
            vec!["gtkentry.py"]
        );

        // A pid that has gone away adds nothing.
        assert_eq!(
            with_program_names(vec!["foot".into()], Some(1), root.path()),
            vec!["foot"]
        );
    }

    /// Talks to a running fcitx5 with the dictation add-on. Types nothing:
    /// CommitTo is given a window name no input context has.
    /// Run with: cargo test fcitx5::tests::live -- --ignored
    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "needs a running fcitx5 with the dictation add-on"]
    async fn live_add_on_round_trip() {
        let bus = SessionBus::default();
        let focus = bus.focus().await.expect("add-on answers Focus");
        assert!(!matches!(parse_reply(&focus), Reply::Unknown(_)), "{focus}");
        let reply = bus
            .commit_to(&["voxtype-no-such-window".to_string()], "x")
            .await
            .expect("add-on answers CommitTo");
        assert_eq!(parse_reply(&reply), Reply::NoFocus);
    }
}
