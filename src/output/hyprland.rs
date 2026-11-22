//! Paste keystrokes sent through Hyprland's own dispatcher.
//!
//! wtype types a key by uploading a one-off XKB keymap and pressing evdev
//! keycode 1 for whatever keysym it needs first. XWayland clients and some
//! Chromium builds read that as Escape, so `shift+insert` arrives as
//! Shift+Escape (#938, Omarchy #16536). Hyprland can send a shortcut to the
//! focused window itself, with the real keymap, and needs no uinput access or
//! input-group membership.
//!
//! Opt-in only: `driver_order` must name `hyprland`, and it only ever sends
//! the paste keystroke and the auto-submit Enter. It cannot type text.
//!
//! Dispatcher syntax, per the Hyprland wiki (hyprwm/hyprland-wiki):
//! - Lua config, Hyprland 0.55.0 and later: `hyprctl dispatch` is shorthand
//!   for `eval 'hl.dispatch(...)'`, and the dispatcher is
//!   `hl.dsp.send_shortcut({ window?, mods, key })`, "Send a specific
//!   shortcut to a window" (content/configuring/core/dispatchers.md and
//!   content/configuring/core/advanced-configuration/using-hyprctl.md).
//! - hyprlang config, 0.54 and earlier: `sendshortcut` with params
//!   `mod, key[, window]`, "sends specified keys (with mods) to an
//!   optionally specified window", and `hyprctl dispatch` "Returns: `ok` on
//!   success, an error message on fail" (content/Configuring/Dispatchers.md
//!   and Using-hyprctl.md before the Lua rewrite, wiki commit 84b5e067).
//!
//! Each form fails harmlessly on the other release line (an invalid
//! dispatcher name, or a Lua syntax error) without sending a key, so the
//! Lua form is tried first and the legacy form only on its failure. The
//! form that worked is remembered for the rest of the process.

use crate::error::OutputError;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;
use tokio::process::Command;

/// How long one `hyprctl` call may take before the paste is given up.
const HYPRCTL_TIMEOUT: Duration = Duration::from_secs(2);

const SYNTAX_UNKNOWN: u8 = 0;
const SYNTAX_LUA: u8 = 1;
const SYNTAX_LEGACY: u8 = 2;

/// Whether this process runs inside a Hyprland session.
pub fn session_detected() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|v| !v.is_empty())
}

/// Quick, spawn-free check for status reports: a Hyprland session and
/// `hyprctl` on PATH.
pub fn available_quick() -> bool {
    session_detected() && which::which("hyprctl").is_ok()
}

/// Sends shortcuts through `hyprctl dispatch`.
#[derive(Debug)]
pub struct HyprlandDispatch {
    program: PathBuf,
    syntax: AtomicU8,
}

impl Default for HyprlandDispatch {
    fn default() -> Self {
        Self::with_program("hyprctl")
    }
}

impl HyprlandDispatch {
    /// Use `program` instead of `hyprctl` from PATH. Tests point this at a
    /// fake so no key ever reaches a real session.
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            syntax: AtomicU8::new(SYNTAX_UNKNOWN),
        }
    }

    /// A Hyprland session with `hyprctl` reachable. Anywhere else the driver
    /// is skipped and the next one in `driver_order` is tried.
    pub fn is_available(&self) -> bool {
        session_detected() && self.program_exists()
    }

    fn program_exists(&self) -> bool {
        if self.program.components().count() > 1 {
            self.program.is_file()
        } else {
            which::which(&self.program).is_ok()
        }
    }

    /// Send `mods` + `key` (voxtype's names, e.g. `["shift"]`, `"insert"`)
    /// to the focused window.
    pub async fn send_shortcut(&self, mods: &[String], key: &str) -> Result<(), OutputError> {
        let mods = hypr_mods(mods).map_err(OutputError::CtrlVFailed)?;
        let key = hypr_key(key).map_err(OutputError::CtrlVFailed)?;

        let lua = || vec!["dispatch".to_string(), lua_dispatch(&mods, &key)];
        let legacy = || legacy_dispatch_args(&mods, &key);

        match self.syntax.load(Ordering::Relaxed) {
            SYNTAX_LUA => return self.run(lua()).await,
            SYNTAX_LEGACY => return self.run(legacy()).await,
            _ => {}
        }
        match self.run(lua()).await {
            Ok(()) => {
                self.syntax.store(SYNTAX_LUA, Ordering::Relaxed);
                Ok(())
            }
            Err(lua_err) => {
                tracing::debug!(
                    "Lua-form hyprctl dispatch failed ({lua_err}); trying sendshortcut"
                );
                self.run(legacy()).await.map(|()| {
                    self.syntax.store(SYNTAX_LEGACY, Ordering::Relaxed);
                })
            }
        }
    }

    async fn run(&self, args: Vec<String>) -> Result<(), OutputError> {
        tracing::debug!("Running: {} {}", self.program.display(), args.join(" "));
        let child = Command::new(&self.program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output();
        let output = match tokio::time::timeout(HYPRCTL_TIMEOUT, child).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Err(OutputError::CtrlVFailed(format!(
                    "could not run {}: {}",
                    self.program.display(),
                    e
                )))
            }
            Err(_) => {
                return Err(OutputError::CtrlVFailed(format!(
                    "{} timed out after {:?}",
                    self.program.display(),
                    HYPRCTL_TIMEOUT
                )))
            }
        };
        // hyprctl reports dispatcher errors on stdout and can still exit 0,
        // so success is the literal reply "ok".
        let reply = String::from_utf8_lossy(&output.stdout);
        if output.status.success() && reply.trim() == "ok" {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(OutputError::CtrlVFailed(format!(
                "hyprctl dispatch failed: {}",
                format!("{} {}", reply.trim(), stderr.trim()).trim()
            )))
        }
    }
}

/// voxtype modifier names to Hyprland's, space-separated (`"CTRL SHIFT"`).
/// Hyprland matches modifier names within the string, so this form is read
/// the same by the Lua and hyprlang parsers.
fn hypr_mods(mods: &[String]) -> Result<String, String> {
    let mut out: Vec<&'static str> = Vec::new();
    for m in mods {
        let name = match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "leftctrl" | "rightctrl" => "CTRL",
            "shift" | "leftshift" | "rightshift" => "SHIFT",
            "alt" | "leftalt" => "ALT",
            "super" | "meta" | "win" | "logo" | "leftmeta" => "SUPER",
            other => return Err(format!("unsupported modifier for Hyprland: {other}")),
        };
        if !out.contains(&name) {
            out.push(name);
        }
    }
    Ok(out.join(" "))
}

/// voxtype key names to XKB keysym names, which is what Hyprland resolves.
/// Anything outside `[A-Za-z0-9_]` is rejected: the name ends up inside a
/// Lua string literal, and no keysym needs other characters.
fn hypr_key(key: &str) -> Result<String, String> {
    let lower = key.to_ascii_lowercase();
    let name = match lower.as_str() {
        "insert" | "ins" => "Insert".to_string(),
        "enter" | "return" => "Return".to_string(),
        "tab" => "Tab".to_string(),
        "space" => "space".to_string(),
        _ => key.to_string(),
    };
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(format!("unsupported key for Hyprland: {key:?}"));
    }
    Ok(name)
}

/// `hl.dsp.send_shortcut({ mods = "SHIFT", key = "Insert" })`, with no
/// window so the focused window gets it.
fn lua_dispatch(mods: &str, key: &str) -> String {
    format!("hl.dsp.send_shortcut({{ mods = \"{mods}\", key = \"{key}\" }})")
}

/// `dispatch sendshortcut SHIFT, Insert` as argv words, which hyprctl joins
/// with spaces. The window parameter is optional and left out, so the
/// shortcut goes to the focused window.
fn legacy_dispatch_args(mods: &str, key: &str) -> Vec<String> {
    vec![
        "dispatch".to_string(),
        "sendshortcut".to_string(),
        format!("{mods},"),
        key.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_map_to_hyprland_names() {
        let m = |v: &[&str]| hypr_mods(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(m(&["shift"]).unwrap(), "SHIFT");
        assert_eq!(m(&["ctrl", "shift"]).unwrap(), "CTRL SHIFT");
        assert_eq!(m(&["super", "logo"]).unwrap(), "SUPER");
        assert_eq!(m(&[]).unwrap(), "");
        assert!(m(&["hyper"]).is_err());
    }

    #[test]
    fn keys_map_to_keysym_names_and_reject_injection() {
        assert_eq!(hypr_key("insert").unwrap(), "Insert");
        assert_eq!(hypr_key("enter").unwrap(), "Return");
        assert_eq!(hypr_key("v").unwrap(), "v");
        assert_eq!(hypr_key("F4").unwrap(), "F4");
        for bad in ["", "v\" }) os.execute(\"x", "a b", "v,", "é"] {
            assert!(hypr_key(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn dispatch_forms_match_the_documented_syntax() {
        assert_eq!(
            lua_dispatch("SHIFT", "Insert"),
            r#"hl.dsp.send_shortcut({ mods = "SHIFT", key = "Insert" })"#
        );
        assert_eq!(
            legacy_dispatch_args("SHIFT", "Insert"),
            ["dispatch", "sendshortcut", "SHIFT,", "Insert"]
        );
        // Enter for auto_submit has no modifiers.
        assert_eq!(
            legacy_dispatch_args("", "Return"),
            ["dispatch", "sendshortcut", ",", "Return"]
        );
    }

    /// A fake hyprctl that logs each argv to `log` (one line, words joined
    /// by `|`) and replies with the script's `reply` logic.
    fn fake_hyprctl(dir: &std::path::Path, body: &str) -> (PathBuf, PathBuf) {
        let log = dir.join("calls.log");
        let script = dir.join("hyprctl");
        crate::test_support::write_script(
            &script,
            &format!(
                "#!/bin/sh\nIFS='|'; echo \"$*\" >> '{}'\n{}\n",
                log.display(),
                body
            ),
        );
        (script, log)
    }

    fn calls(log: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[tokio::test]
    async fn lua_era_hyprland_gets_one_lua_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let (prog, log) = fake_hyprctl(dir.path(), "echo ok");
        let h = HyprlandDispatch::with_program(&prog);
        h.send_shortcut(&["shift".into()], "insert").await.unwrap();
        h.send_shortcut(&[], "enter").await.unwrap();
        assert_eq!(
            calls(&log),
            [
                r#"dispatch|hl.dsp.send_shortcut({ mods = "SHIFT", key = "Insert" })"#,
                r#"dispatch|hl.dsp.send_shortcut({ mods = "", key = "Return" })"#,
            ]
        );
    }

    /// Older Hyprland rejects the Lua form as an unknown dispatcher (and
    /// still exits 0), so the legacy form follows, and is then used alone.
    #[tokio::test]
    async fn hyprlang_era_hyprland_falls_back_to_sendshortcut_once() {
        let dir = tempfile::tempdir().unwrap();
        let (prog, log) = fake_hyprctl(
            dir.path(),
            "case \"$2\" in sendshortcut) echo ok ;; *) echo 'Invalid dispatcher' ;; esac",
        );
        let h = HyprlandDispatch::with_program(&prog);
        h.send_shortcut(&["ctrl".into(), "shift".into()], "v")
            .await
            .unwrap();
        h.send_shortcut(&["shift".into()], "insert").await.unwrap();
        assert_eq!(
            calls(&log),
            [
                r#"dispatch|hl.dsp.send_shortcut({ mods = "CTRL SHIFT", key = "v" })"#,
                "dispatch|sendshortcut|CTRL SHIFT,|v",
                "dispatch|sendshortcut|SHIFT,|Insert",
            ]
        );
    }

    #[tokio::test]
    async fn an_error_reply_is_a_failure_even_with_exit_zero() {
        let dir = tempfile::tempdir().unwrap();
        let (prog, log) = fake_hyprctl(dir.path(), "echo 'HYPRLAND_INSTANCE_SIGNATURE not set'");
        let h = HyprlandDispatch::with_program(&prog);
        let err = h.send_shortcut(&[], "v").await.unwrap_err();
        assert!(err.to_string().contains("not set"), "{err}");
        assert_eq!(calls(&log).len(), 2, "both forms tried, nothing more");
    }

    #[tokio::test]
    async fn invalid_keys_never_reach_hyprctl() {
        let dir = tempfile::tempdir().unwrap();
        let (prog, log) = fake_hyprctl(dir.path(), "echo ok");
        let h = HyprlandDispatch::with_program(&prog);
        assert!(h.send_shortcut(&[], "v\"x").await.is_err());
        assert!(h.send_shortcut(&["hyper".into()], "v").await.is_err());
        assert!(calls(&log).is_empty());
    }

    #[test]
    fn a_missing_program_is_unavailable() {
        let h = HyprlandDispatch::with_program("/nonexistent/hyprctl");
        assert!(!h.is_available());
    }
}
