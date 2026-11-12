//! Resolve `[meeting] view` to the meeting view that opens.
//!
//! `auto` (the default) mirrors `[osd] frontend = "auto"`:
//!
//! 1. `quickshell` when the Quickshell OSD could run (its launcher is
//!    installed, `qs` is on PATH and a Wayland session is present): the
//!    Omarchy plugin or voxtype's standalone shell hosts the view there;
//! 2. else `gtk4` when `voxtype-meetings-gtk4` is installed, its shared
//!    libraries load, and a display is present;
//! 3. else none.
//!
//! `terminal` is never picked automatically, since it needs a terminal to
//! open in. An explicit choice is used as given when it can run at all.
//! System access goes through [`OsdProbe`], shared with the OSD resolver,
//! so every branch is testable with a fake.

use crate::osd::frontend::{quickshell_available, OsdProbe};
use std::path::PathBuf;

/// Environment variable that overrides `[meeting] view`.
pub const VIEW_ENV: &str = "VOXTYPE_MEETING_VIEW";

/// The GTK4 meetings window binary.
pub const GTK4_BINARY: &str = "voxtype-meetings-gtk4";

/// Values `[meeting] view` takes.
pub const VIEW_CHOICES: &[&str] = &["auto", "quickshell", "gtk4", "terminal", "none"];

/// A meeting view frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewFrontend {
    Auto,
    Quickshell,
    Gtk4,
    Terminal,
    None,
}

impl ViewFrontend {
    pub fn parse_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Some(ViewFrontend::Auto),
            "quickshell" | "qs" | "qml" => Some(ViewFrontend::Quickshell),
            "gtk4" | "gtk" => Some(ViewFrontend::Gtk4),
            "terminal" | "tui" => Some(ViewFrontend::Terminal),
            "none" | "off" | "disabled" => Some(ViewFrontend::None),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ViewFrontend::Auto => "auto",
            ViewFrontend::Quickshell => "quickshell",
            ViewFrontend::Gtk4 => "gtk4",
            ViewFrontend::Terminal => "terminal",
            ViewFrontend::None => "none",
        }
    }
}

/// The view a configured value resolves to, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewResolution {
    pub configured: ViewFrontend,
    /// `Quickshell`, `Gtk4`, `Terminal`, or `None` when nothing opens.
    pub active: ViewFrontend,
    /// The GTK4 binary, when that is what opens.
    pub path: Option<PathBuf>,
    /// One line for logs and messages.
    pub reason: String,
}

/// Why the GTK4 window can't open here, or its path when it can.
fn gtk4(probe: &dyn OsdProbe) -> Result<PathBuf, String> {
    let path = probe
        .find_voxtype_binary(GTK4_BINARY)
        .ok_or_else(|| format!("{GTK4_BINARY} is not installed"))?;
    if let Some(lib) = probe.missing_libraries(&path).first() {
        let name = lib.split(".so").next().unwrap_or(lib);
        return Err(format!("{GTK4_BINARY} is installed but {name} is missing"));
    }
    if !probe.display_present() {
        return Err("no display".into());
    }
    Ok(path)
}

/// Resolve `configured` against what this system can run.
pub fn resolve(configured: ViewFrontend, probe: &dyn OsdProbe) -> ViewResolution {
    let done = |active, path, reason: String| ViewResolution {
        configured,
        active,
        path,
        reason,
    };
    match configured {
        ViewFrontend::Auto => {
            if quickshell_available(probe) {
                return done(
                    ViewFrontend::Quickshell,
                    None,
                    "auto: Quickshell and a Wayland session are available".into(),
                );
            }
            match gtk4(probe) {
                Ok(path) => done(
                    ViewFrontend::Gtk4,
                    Some(path),
                    "auto: no Quickshell, using the GTK4 window".into(),
                ),
                Err(why) => done(
                    ViewFrontend::None,
                    None,
                    format!("auto: no Quickshell; {why}"),
                ),
            }
        }
        ViewFrontend::Gtk4 => match gtk4(probe) {
            Ok(path) => done(ViewFrontend::Gtk4, Some(path), "configured gtk4".into()),
            Err(why) => done(
                ViewFrontend::None,
                None,
                format!("configured gtk4, but {why}"),
            ),
        },
        ViewFrontend::Quickshell => done(
            ViewFrontend::Quickshell,
            None,
            "configured quickshell".into(),
        ),
        ViewFrontend::Terminal => done(ViewFrontend::Terminal, None, "configured terminal".into()),
        ViewFrontend::None => done(ViewFrontend::None, None, "configured none".into()),
    }
}

/// What the daemon does about a view when a meeting starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartAction {
    /// Nothing: `show_view_on_start` is off, nothing can open, or the
    /// Quickshell view, which opens itself, is the one in use.
    Nothing,
    /// Launch the GTK4 window at this path.
    Gtk4(PathBuf),
    /// Run `voxtype meeting watch` in a terminal through this
    /// `xdg-terminal-exec`. Only for an explicit `view = "terminal"`.
    Terminal(PathBuf),
}

/// Decide what to open when a meeting starts. `terminal_exec` is
/// `xdg-terminal-exec` when it is installed.
pub fn start_action(
    show_view_on_start: bool,
    resolution: &ViewResolution,
    terminal_exec: Option<PathBuf>,
) -> StartAction {
    if !show_view_on_start {
        return StartAction::Nothing;
    }
    match (resolution.active, &resolution.path) {
        (ViewFrontend::Gtk4, Some(path)) => StartAction::Gtk4(path.clone()),
        (ViewFrontend::Terminal, _) => terminal_exec
            .map(StartAction::Terminal)
            .unwrap_or(StartAction::Nothing),
        _ => StartAction::Nothing,
    }
}

/// Open the meeting view for a meeting that just started, as
/// `[meeting] show_view_on_start` and `[meeting] view` ask, from the daemon.
/// Returns a line for the log. Blocking: it probes the system and spawns.
pub fn open_on_start(view: &str, show_view_on_start: bool) -> String {
    use crate::osd::frontend::SystemProbe;
    use std::process::{Command, Stdio};

    if !show_view_on_start {
        return "show_view_on_start is off".into();
    }
    let resolution = resolve(effective_configured(view), &SystemProbe);
    let terminal = which::which("xdg-terminal-exec").ok();
    let config_args: Vec<std::ffi::OsString> = crate::config::Config::path_override()
        .map(|p| vec!["--config".into(), p.into_os_string()])
        .unwrap_or_default();
    let mut cmd = match start_action(true, &resolution, terminal) {
        StartAction::Nothing => {
            return format!("nothing to open ({})", resolution.reason);
        }
        StartAction::Gtk4(path) => {
            let mut cmd = Command::new(path);
            cmd.args(&config_args);
            cmd
        }
        StartAction::Terminal(term) => {
            let Ok(exe) = std::env::current_exe() else {
                return "cannot find the voxtype binary for the terminal view".into();
            };
            let mut cmd = Command::new(term);
            cmd.arg(exe).args(&config_args).args(["meeting", "watch"]);
            cmd
        }
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: a signal meant for the daemon's group
        // (Ctrl+C in a foreground daemon) does not close the window.
        cmd.process_group(0);
    }
    match cmd.spawn() {
        Ok(mut child) => {
            // Reap it whenever it closes.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            format!("opened ({})", resolution.reason)
        }
        Err(e) => format!("could not open it: {e} ({})", resolution.reason),
    }
}

/// `[meeting] view` after the `VOXTYPE_MEETING_VIEW` override. An unknown
/// value reads as `auto`.
pub fn effective_configured(config_value: &str) -> ViewFrontend {
    std::env::var(VIEW_ENV)
        .ok()
        .and_then(|v| ViewFrontend::parse_str(&v))
        .or_else(|| ViewFrontend::parse_str(config_value))
        .unwrap_or(ViewFrontend::Auto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[derive(Default)]
    struct Fake {
        binaries: Vec<&'static str>,
        programs: Vec<&'static str>,
        wayland: bool,
        display: bool,
        missing: Vec<String>,
    }

    impl OsdProbe for Fake {
        fn find_voxtype_binary(&self, name: &str) -> Option<PathBuf> {
            self.binaries
                .contains(&name)
                .then(|| PathBuf::from(format!("/usr/lib/voxtype/{name}")))
        }
        fn on_path(&self, name: &str) -> bool {
            self.programs.contains(&name)
        }
        fn wayland_session(&self) -> bool {
            self.wayland
        }
        fn display_present(&self) -> bool {
            self.display
        }
        fn missing_libraries(&self, _binary: &Path) -> Vec<String> {
            self.missing.clone()
        }
    }

    fn quickshell_desktop() -> Fake {
        Fake {
            binaries: vec!["voxtype-osd-quickshell", GTK4_BINARY],
            programs: vec!["qs"],
            wayland: true,
            display: true,
            ..Default::default()
        }
    }

    #[test]
    fn auto_prefers_quickshell() {
        let r = resolve(ViewFrontend::Auto, &quickshell_desktop());
        assert_eq!(r.active, ViewFrontend::Quickshell);
    }

    #[test]
    fn auto_falls_back_to_gtk4_without_quickshell() {
        let mut p = quickshell_desktop();
        p.programs.clear();
        let r = resolve(ViewFrontend::Auto, &p);
        assert_eq!(r.active, ViewFrontend::Gtk4);
        assert_eq!(
            r.path.as_deref(),
            Some(Path::new("/usr/lib/voxtype/voxtype-meetings-gtk4"))
        );
    }

    #[test]
    fn gtk4_needs_its_libraries_and_a_display() {
        let mut p = quickshell_desktop();
        p.programs.clear();
        p.missing = vec!["libgtk-4.so.1".into()];
        let r = resolve(ViewFrontend::Auto, &p);
        assert_eq!(r.active, ViewFrontend::None);
        assert!(r.reason.contains("libgtk-4 is missing"), "{}", r.reason);

        p.missing.clear();
        p.display = false;
        p.wayland = false;
        let r = resolve(ViewFrontend::Gtk4, &p);
        assert_eq!(r.active, ViewFrontend::None);
        assert!(r.reason.contains("no display"), "{}", r.reason);

        let r = resolve(ViewFrontend::Gtk4, &Fake::default());
        assert!(r.reason.contains("not installed"), "{}", r.reason);
    }

    #[test]
    fn terminal_is_never_automatic_but_is_honoured() {
        let r = resolve(ViewFrontend::Auto, &Fake::default());
        assert_eq!(r.active, ViewFrontend::None);
        let r = resolve(ViewFrontend::Terminal, &Fake::default());
        assert_eq!(r.active, ViewFrontend::Terminal);
        let r = resolve(ViewFrontend::None, &quickshell_desktop());
        assert_eq!(r.active, ViewFrontend::None);
        let r = resolve(ViewFrontend::Quickshell, &Fake::default());
        assert_eq!(r.active, ViewFrontend::Quickshell);
    }

    #[test]
    fn a_meeting_start_opens_the_gtk4_window_or_an_explicit_terminal_only() {
        let mut p = quickshell_desktop();
        p.programs.clear();
        let gtk = resolve(ViewFrontend::Auto, &p);
        let exec = PathBuf::from("/usr/bin/xdg-terminal-exec");
        let term = Some(exec.clone());
        assert_eq!(
            start_action(true, &gtk, None),
            StartAction::Gtk4(PathBuf::from("/usr/lib/voxtype/voxtype-meetings-gtk4"))
        );
        assert_eq!(start_action(false, &gtk, None), StartAction::Nothing);
        // Quickshell opens its own view.
        let qs = resolve(ViewFrontend::Auto, &quickshell_desktop());
        assert_eq!(start_action(true, &qs, term.clone()), StartAction::Nothing);
        // Nothing to open, and auto never picks the terminal.
        let none = resolve(ViewFrontend::Auto, &Fake::default());
        assert_eq!(
            start_action(true, &none, term.clone()),
            StartAction::Nothing
        );
        // An explicit terminal opens one when it can.
        let t = resolve(ViewFrontend::Terminal, &Fake::default());
        assert_eq!(
            start_action(true, &t, term.clone()),
            StartAction::Terminal(exec)
        );
        assert_eq!(start_action(true, &t, None), StartAction::Nothing);
    }

    #[test]
    fn spellings_parse() {
        for v in VIEW_CHOICES {
            assert_eq!(ViewFrontend::parse_str(v).unwrap().as_str(), *v);
        }
        assert_eq!(ViewFrontend::parse_str("GTK"), Some(ViewFrontend::Gtk4));
        assert_eq!(ViewFrontend::parse_str("bogus"), None);
    }
}
