//! Telling the user when output did not go where they expected.
//!
//! The fallback chain (wtype -> eitype -> dotool -> ydotool -> clipboard)
//! keeps dictation working when a typing tool is missing, but on its own it
//! hides the problem: the text quietly lands on the clipboard while the
//! "Transcribed" notification reads like it was typed. This module turns the
//! chain's outcome into notifications that say where the text went and how
//! to fix the driver that was skipped.
//!
//! The decision logic is pure (`fallback_notice`, `failure_notice`) so it can
//! be tested without a desktop; the async wrappers only gather facts from the
//! system and post the notification.

use super::session::DisplaySession;
use super::{is_keystroke_method, TextOutput};
use crate::config::{NotificationConfig, TranscriptionEngine};
use crate::error::OutputError;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Why a method in the chain did not deliver the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// `is_available()` said no (tool missing, daemon not running, wrong session).
    Unavailable,
    /// The tool ran and failed; the error message.
    Failed(String),
}

/// A method the chain passed over before delivering (or giving up).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedMethod {
    pub name: &'static str,
    pub reason: SkipReason,
}

/// What `output_with_fallback_report` did with the text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputReport {
    /// The method that delivered the text, or `None` when every method failed.
    pub delivered_by: Option<&'static str>,
    /// Methods tried (or probed) before `delivered_by`, in chain order.
    /// Methods skipped on purpose because a modifier was held are not listed.
    pub skipped: Vec<SkippedMethod>,
    /// Keystroke methods were skipped because a modifier key stayed held.
    pub modifier_skip: bool,
}

impl OutputReport {
    /// The text ended up on the clipboard although typing was attempted (or
    /// would have been, had the modifier guard not skipped it), so the user
    /// has to paste it.
    pub fn fell_back_to_clipboard(&self) -> bool {
        let Some(method) = self.delivered_by else {
            return false;
        };
        !is_keystroke_method(method)
            && (self.modifier_skip || self.skipped.iter().any(|s| is_keystroke_method(s.name)))
    }
}

/// Facts about the system the notice text depends on, gathered by the caller
/// so the text itself stays a pure function.
#[derive(Debug, Clone)]
pub struct FixEnv {
    pub session: DisplaySession,
    /// Contents of /etc/os-release, for the package manager command.
    pub os_release: Option<String>,
    /// Whether the `ydotool` binary is on PATH (an unavailable ydotool with
    /// the binary present means ydotoold is not running).
    pub ydotool_installed: bool,
}

impl FixEnv {
    fn gather() -> Self {
        Self {
            session: super::session::detect(),
            os_release: std::fs::read_to_string("/etc/os-release").ok(),
            ydotool_installed: which::which("ydotool").is_ok(),
        }
    }
}

/// A notification to post, with a key identifying the underlying problem so
/// the same problem is reported once per daemon run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub key: String,
    pub title: String,
    pub body: String,
}

/// The package manager command that installs `pkg`, from /etc/os-release.
fn install_command(os_release: Option<&str>, pkg: &str) -> String {
    let ids: Vec<String> = os_release
        .unwrap_or("")
        .lines()
        .filter_map(|l| {
            l.strip_prefix("ID=")
                .or_else(|| l.strip_prefix("ID_LIKE="))
                .map(|v| v.trim_matches('"').to_lowercase())
        })
        .flat_map(|v| v.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .collect();
    let has = |id: &str| ids.iter().any(|i| i == id);
    if has("arch") {
        format!("sudo pacman -S {pkg}")
    } else if has("debian") || has("ubuntu") {
        format!("sudo apt install {pkg}")
    } else if has("fedora") || has("rhel") {
        format!("sudo dnf install {pkg}")
    } else if has("suse") || has("opensuse") {
        format!("sudo zypper install {pkg}")
    } else if has("nixos") {
        format!("add {pkg} to environment.systemPackages")
    } else {
        format!("install the {pkg} package")
    }
}

/// First line of an error, shortened for a notification body.
fn short_error(err: &str) -> String {
    let line = err.lines().next().unwrap_or("").trim();
    if line.chars().count() > 100 {
        format!("{}...", line.chars().take(100).collect::<String>())
    } else {
        line.to_string()
    }
}

/// One line per skipped typing tool: what went wrong and the fix. `None` for
/// methods not worth a line (clipboard methods, or Wayland-only tools that
/// were merely unavailable in an X11 session).
fn driver_line(skip: &SkippedMethod, env: &FixEnv) -> Option<String> {
    if !is_keystroke_method(skip.name) {
        return None;
    }
    method_line(skip, env)
}

/// `driver_line` for any method, clipboard tools included: used when nothing
/// delivered the text, so a missing clipboard tool is part of the story.
fn method_line(skip: &SkippedMethod, env: &FixEnv) -> Option<String> {
    let os = env.os_release.as_deref();
    match &skip.reason {
        SkipReason::Unavailable => {
            let line = match skip.name {
                "wtype" | "eitype" if env.session == DisplaySession::X11 => return None,
                "wtype" => format!(
                    "wtype: not installed. Fix: {}",
                    install_command(os, "wtype")
                ),
                "eitype" => "eitype: not installed. Fix: cargo install eitype".to_string(),
                "dotool" => {
                    "dotool: not installed. Fix: install it from https://sr.ht/~geb/dotool/"
                        .to_string()
                }
                "ydotool" if env.ydotool_installed => {
                    "ydotool: ydotoold is not running. Fix: systemctl --user enable --now ydotool"
                        .to_string()
                }
                "ydotool" => format!(
                    "ydotool: not installed. Fix: {}",
                    install_command(os, "ydotool")
                ),
                "clipboard (wl-copy)" if env.session == DisplaySession::Wayland => format!(
                    "wl-copy: not installed. Fix: {}",
                    install_command(os, "wl-clipboard")
                ),
                "clipboard (xclip/xsel)" if env.session == DisplaySession::X11 => format!(
                    "xclip: not installed. Fix: {}",
                    install_command(os, "xclip")
                ),
                n if n.starts_with("paste") && env.session == DisplaySession::X11 => format!(
                    "paste: needs xclip or xsel plus ydotool. Fix: {}",
                    install_command(os, "xclip ydotool")
                ),
                n if n.starts_with("paste") => format!(
                    "paste: needs wl-copy plus wtype, eitype or ydotool. Fix: {}",
                    install_command(os, "wl-clipboard wtype")
                ),
                // A clipboard tool for the other display server: expected.
                "clipboard (wl-copy)" | "clipboard (xclip/xsel)" => return None,
                other => format!("{other}: not available"),
            };
            Some(line)
        }
        SkipReason::Failed(err) => {
            let mut line = format!("{}: {}", skip.name, short_error(err));
            let lower = err.to_lowercase();
            if matches!(skip.name, "dotool" | "ydotool")
                && (lower.contains("uinput") || lower.contains("permission denied"))
            {
                line.push_str(". Fix: sudo usermod -aG input $USER, then log out and back in");
            }
            Some(line)
        }
    }
}

/// The once-per-problem warning for a run that fell back to the clipboard
/// because the typing tools could not be used. `None` when the text was typed,
/// when the user chose clipboard output, or when the only reason was a held
/// modifier key (which the chain already reports on its own).
pub fn fallback_notice(report: &OutputReport, env: &FixEnv) -> Option<Notice> {
    if !report.fell_back_to_clipboard() {
        return None;
    }
    let lines: Vec<String> = report
        .skipped
        .iter()
        .filter_map(|s| driver_line(s, env))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let key = report
        .skipped
        .iter()
        .filter(|s| is_keystroke_method(s.name))
        .map(|s| match &s.reason {
            SkipReason::Unavailable => format!("{}:unavailable", s.name),
            SkipReason::Failed(_) => format!("{}:failed", s.name),
        })
        .collect::<Vec<_>>()
        .join(",");
    Some(Notice {
        key,
        title: "Voxtype: copied to clipboard, paste manually".to_string(),
        body: format!(
            "Voxtype could not type the text, so it is on the clipboard.\n{}",
            lines.join("\n")
        ),
    })
}

/// The notification for a run where no method delivered the text. The text
/// itself goes in the body so it is never lost, and `saved_to` says where the
/// full copy is.
pub fn failure_notice(
    report: &OutputReport,
    err: &OutputError,
    text: &str,
    saved_to: Option<&Path>,
    env: &FixEnv,
) -> Notice {
    let mut body = short_error(&err.to_string());
    let lines: Vec<String> = report
        .skipped
        .iter()
        .filter_map(|s| method_line(s, env))
        .collect();
    if !lines.is_empty() {
        body.push('\n');
        body.push_str(&lines.join("\n"));
    }
    if let Some(path) = saved_to {
        body.push_str(&format!("\nText saved to {}", path.display()));
    }
    let preview = if text.chars().count() > 300 {
        format!("{}...", text.chars().take(300).collect::<String>())
    } else {
        text.to_string()
    };
    body.push_str(&format!("\nText: {preview}"));
    Notice {
        key: "all-failed".to_string(),
        title: "Voxtype: transcription not delivered".to_string(),
        body,
    }
}

/// Problems already reported this run, so a missing tool is reported once
/// rather than on every dictation.
static NOTIFIED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// True the first time `key` is seen in this process.
fn first_time(key: &str) -> bool {
    let mut guard = NOTIFIED.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashSet::new)
        .insert(key.to_string())
}

/// Post the notification for a successful output: the one-time driver warning
/// if this run fell back to the clipboard for a new reason, otherwise the
/// usual transcription notification, retitled when the text has to be pasted.
pub async fn notify_delivered(
    report: &OutputReport,
    text: &str,
    notification: &NotificationConfig,
    engine: TranscriptionEngine,
) {
    if report.fell_back_to_clipboard() && !report.modifier_skip {
        if let Some(notice) = fallback_notice(report, &FixEnv::gather()) {
            tracing::warn!("{}: {}", notice.title, notice.body.replace('\n', " | "));
            if first_time(&notice.key) {
                crate::notification::send_status(
                    &notice.title,
                    &notice.body,
                    "normal",
                    crate::notification::Lifetime::Millis(10000),
                )
                .await;
                return;
            }
        }
    }

    if notification.on_transcription {
        super::send_transcription_notification_titled(
            text,
            notification.show_engine_icon,
            engine,
            &notification.urgency,
            if report.fell_back_to_clipboard() {
                "Copied to clipboard, paste manually"
            } else {
                "Transcribed"
            },
        )
        .await;
    }
}

/// How many undelivered transcriptions to keep. Each is a short text file,
/// so this is generous; it only exists so a long-broken setup can't fill the
/// disk.
const UNDELIVERED_KEEP: usize = 200;

/// Directory holding transcriptions no output method could deliver:
/// `$XDG_STATE_HOME/voxtype/undelivered` (default `~/.local/state`). State,
/// not runtime: a dictation must survive a logout or reboot.
pub fn undelivered_dir() -> PathBuf {
    crate::config::Config::state_dir().join("undelivered")
}

/// Every output method failed: save the text where the user can get it back
/// and say so in a notification, along with what failed.
pub async fn notify_failed(report: &OutputReport, err: &OutputError, text: &str) {
    let dir = undelivered_dir();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f").to_string();
    let saved = match save_undelivered(&dir, &stamp, text, UNDELIVERED_KEEP) {
        Ok(path) => Some(path),
        Err(e) => {
            tracing::warn!(
                "Could not save the undelivered transcription in {:?}: {}",
                dir,
                e
            );
            None
        }
    };
    let notice = failure_notice(report, err, text, saved.as_deref(), &FixEnv::gather());
    tracing::error!("{}: {}", notice.title, notice.body.replace('\n', " | "));
    crate::notification::send_status(
        &notice.title,
        &notice.body,
        "critical",
        crate::notification::Lifetime::Millis(15000),
    )
    .await;
}

/// Save `text` as `<dir>/<stamp>.txt`, readable only by the user (a
/// transcript can be private), then prune the directory to the newest
/// `keep` files. Stamps sort chronologically, so name order is age order.
fn save_undelivered(dir: &Path, stamp: &str, text: &str, keep: usize) -> std::io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;

    // Two failures within the same millisecond must not overwrite each other.
    // `_` sorts after `.`, so the suffixed file still sorts as the newer one.
    let mut path = dir.join(format!("{stamp}.txt"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{stamp}_{n}.txt"));
        n += 1;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(text.as_bytes())?;
    file.write_all(b"\n")?;

    let mut saved: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .collect();
    if saved.len() > keep {
        saved.sort();
        for old in &saved[..saved.len() - keep] {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(path)
}

/// Run each method in `chain` until one delivers `text`, recording what was
/// skipped and why. Shared by `output_with_fallback_report`.
pub(super) async fn run_chain(
    chain: &[Box<dyn TextOutput>],
    text: &str,
    skip_keystroke_methods: bool,
) -> (OutputReport, Result<(), OutputError>) {
    let mut report = OutputReport {
        modifier_skip: skip_keystroke_methods,
        ..Default::default()
    };
    let mut result = Err(OutputError::AllMethodsFailed);
    for output in chain {
        if skip_keystroke_methods && is_keystroke_method(output.name()) {
            tracing::debug!(
                "{} skipped (modifier still held), trying next",
                output.name()
            );
            continue;
        }

        if !output.is_available().await {
            tracing::debug!("{} not available, trying next", output.name());
            report.skipped.push(SkippedMethod {
                name: output.name(),
                reason: SkipReason::Unavailable,
            });
            continue;
        }

        match output.output(text).await {
            Ok(()) => {
                tracing::debug!("Text output via {}", output.name());
                report.delivered_by = Some(output.name());
                result = Ok(());
                break;
            }
            Err(e) => {
                tracing::warn!("{} failed: {}, trying next", output.name(), e);
                report.skipped.push(SkippedMethod {
                    name: output.name(),
                    reason: SkipReason::Failed(e.to_string()),
                });
            }
        }
    }
    (report, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARCH: &str = "NAME=\"Arch Linux\"\nID=arch\n";
    const UBUNTU: &str = "ID=ubuntu\nID_LIKE=debian\n";
    const CACHY: &str = "ID=cachyos\nID_LIKE=\"arch\"\n";

    fn env(session: DisplaySession, ydotool_installed: bool) -> FixEnv {
        FixEnv {
            session,
            os_release: Some(ARCH.to_string()),
            ydotool_installed,
        }
    }

    fn unavailable(name: &'static str) -> SkippedMethod {
        SkippedMethod {
            name,
            reason: SkipReason::Unavailable,
        }
    }

    fn default_chain_fallback() -> OutputReport {
        OutputReport {
            delivered_by: Some("clipboard (wl-copy)"),
            skipped: vec![
                unavailable("wtype"),
                unavailable("eitype"),
                unavailable("dotool"),
                unavailable("ydotool"),
            ],
            modifier_skip: false,
        }
    }

    #[test]
    fn install_command_follows_the_distro() {
        assert_eq!(install_command(Some(ARCH), "wtype"), "sudo pacman -S wtype");
        assert_eq!(
            install_command(Some(CACHY), "wtype"),
            "sudo pacman -S wtype"
        );
        assert_eq!(
            install_command(Some(UBUNTU), "ydotool"),
            "sudo apt install ydotool"
        );
        assert_eq!(
            install_command(Some("ID=fedora\n"), "wtype"),
            "sudo dnf install wtype"
        );
        assert_eq!(install_command(None, "wtype"), "install the wtype package");
    }

    #[test]
    fn typed_text_needs_no_notice() {
        let report = OutputReport {
            delivered_by: Some("dotool"),
            skipped: vec![unavailable("wtype"), unavailable("eitype")],
            modifier_skip: false,
        };
        assert!(!report.fell_back_to_clipboard());
        assert!(fallback_notice(&report, &env(DisplaySession::Wayland, false)).is_none());
    }

    #[test]
    fn chosen_clipboard_mode_needs_no_notice() {
        // mode = "clipboard" on X11: wl-copy skipped, xclip delivers.
        let report = OutputReport {
            delivered_by: Some("clipboard (xclip/xsel)"),
            skipped: vec![unavailable("clipboard (wl-copy)")],
            modifier_skip: false,
        };
        assert!(!report.fell_back_to_clipboard());
        assert!(fallback_notice(&report, &env(DisplaySession::X11, false)).is_none());
    }

    #[test]
    fn modifier_skip_counts_as_clipboard_but_not_as_a_driver_problem() {
        let report = OutputReport {
            delivered_by: Some("clipboard (wl-copy)"),
            skipped: vec![],
            modifier_skip: true,
        };
        assert!(report.fell_back_to_clipboard());
        assert!(fallback_notice(&report, &env(DisplaySession::Wayland, false)).is_none());
    }

    #[test]
    fn missing_tools_on_wayland_list_every_fix() {
        let notice = fallback_notice(
            &default_chain_fallback(),
            &env(DisplaySession::Wayland, true),
        )
        .unwrap();
        assert_eq!(notice.title, "Voxtype: copied to clipboard, paste manually");
        assert_eq!(
            notice.body,
            "Voxtype could not type the text, so it is on the clipboard.\n\
             wtype: not installed. Fix: sudo pacman -S wtype\n\
             eitype: not installed. Fix: cargo install eitype\n\
             dotool: not installed. Fix: install it from https://sr.ht/~geb/dotool/\n\
             ydotool: ydotoold is not running. Fix: systemctl --user enable --now ydotool"
        );
        assert_eq!(
            notice.key,
            "wtype:unavailable,eitype:unavailable,dotool:unavailable,ydotool:unavailable"
        );
    }

    #[test]
    fn x11_omits_wayland_only_tools() {
        let mut report = default_chain_fallback();
        report.delivered_by = Some("clipboard (xclip/xsel)");
        report.skipped.push(unavailable("clipboard (wl-copy)"));
        let notice = fallback_notice(&report, &env(DisplaySession::X11, false)).unwrap();
        assert!(!notice.body.contains("wtype"));
        assert!(!notice.body.contains("eitype"));
        assert!(notice
            .body
            .contains("ydotool: not installed. Fix: sudo pacman -S ydotool"));
    }

    #[test]
    fn failures_show_the_error_and_uinput_fix() {
        let report = OutputReport {
            delivered_by: Some("clipboard (wl-copy)"),
            skipped: vec![
                SkippedMethod {
                    name: "wtype",
                    reason: SkipReason::Failed(
                        "Text injection failed: wtype exited: Compositor does not support the virtual keyboard protocol\nmore"
                            .to_string(),
                    ),
                },
                SkippedMethod {
                    name: "dotool",
                    reason: SkipReason::Failed(
                        "Text injection failed: /dev/uinput: Permission denied".to_string(),
                    ),
                },
            ],
            modifier_skip: false,
        };
        let notice = fallback_notice(&report, &env(DisplaySession::Wayland, false)).unwrap();
        assert!(notice.body.contains(
            "wtype: Text injection failed: wtype exited: Compositor does not support the virtual keyboard protocol"
        ));
        assert!(!notice.body.contains("more"));
        assert!(notice.body.contains(
            "dotool: Text injection failed: /dev/uinput: Permission denied. Fix: sudo usermod -aG input $USER"
        ));
        assert_eq!(notice.key, "wtype:failed,dotool:failed");
    }

    #[test]
    fn failure_notice_keeps_the_text() {
        let report = OutputReport {
            delivered_by: None,
            skipped: vec![SkippedMethod {
                name: "paste (clipboard + keystroke)",
                reason: SkipReason::Unavailable,
            }],
            modifier_skip: false,
        };
        let notice = failure_notice(
            &report,
            &OutputError::AllMethodsFailed,
            "hello world",
            Some(Path::new(
                "/home/u/.local/state/voxtype/undelivered/20261020-093900-123.txt",
            )),
            &env(DisplaySession::Wayland, false),
        );
        assert_eq!(notice.title, "Voxtype: transcription not delivered");
        assert!(notice.body.starts_with("All output methods failed."));
        assert!(notice.body.contains(
            "paste: needs wl-copy plus wtype, eitype or ydotool. Fix: sudo pacman -S wl-clipboard wtype"
        ));
        assert!(notice.body.contains(
            "Text saved to /home/u/.local/state/voxtype/undelivered/20261020-093900-123.txt"
        ));
        assert!(notice.body.ends_with("Text: hello world"));
    }

    #[test]
    fn failure_notice_names_the_missing_clipboard_tool() {
        let report = OutputReport {
            delivered_by: None,
            skipped: vec![
                unavailable("wtype"),
                unavailable("clipboard (wl-copy)"),
                unavailable("clipboard (xclip/xsel)"),
            ],
            modifier_skip: false,
        };
        let notice = failure_notice(
            &report,
            &OutputError::AllMethodsFailed,
            "hi",
            None,
            &env(DisplaySession::Wayland, false),
        );
        assert!(notice
            .body
            .contains("wl-copy: not installed. Fix: sudo pacman -S wl-clipboard"));
        assert!(!notice.body.contains("xclip:"));
        assert!(!notice.body.contains("Text saved"));
    }

    #[test]
    fn first_time_reports_each_key_once() {
        assert!(first_time("test-key-a"));
        assert!(!first_time("test-key-a"));
        assert!(first_time("test-key-b"));
    }

    #[test]
    fn save_undelivered_is_user_only_and_pruned() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("state/voxtype/undelivered");

        let first = save_undelivered(&dir, "20261020-093900-001", "secret words", 2).unwrap();
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "secret words\n");
        let mode = std::fs::metadata(&first).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);

        // Same stamp: a second file, not an overwrite.
        let twin = save_undelivered(&dir, "20261020-093900-001", "other", 2).unwrap();
        assert_ne!(twin, first);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "secret words\n");

        // A third file prunes the oldest, keeping the newest two.
        let third = save_undelivered(&dir, "20261020-094000-000", "newest", 2).unwrap();
        assert!(!first.exists());
        assert!(twin.exists());
        assert!(third.exists());
    }
}
