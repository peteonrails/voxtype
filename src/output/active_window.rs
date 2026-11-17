//! Focused-window lookup for terminal-aware paste.
//!
//! Paste mode's `auto` keystroke sends a different shortcut to terminal
//! emulators than to GUI apps, because Ctrl+V inside a terminal reaches the
//! running program as a raw ^V (readline's literal-insert, Codex CLI's
//! "paste image") instead of pasting. This module asks the compositor which
//! window has focus and decides whether it is a terminal.
//!
//! Supported compositors, picked by the socket variable each one exports:
//! - Hyprland: `hyprctl activewindow -j`, using Omarchy's `terminal` window
//!   tag when present and the window class otherwise
//! - Sway: `swaymsg -t get_tree`, using the focused node's app_id/class
//! - niri: `niri msg --json focused-window`, using its app_id
//!
//! Anywhere else, and whenever the query fails or times out, the answer is
//! "unknown" and paste mode sends its non-terminal keystroke, which is what
//! it sent before terminal detection existed.

use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Upper bound on the compositor query. hyprctl answers in a few
/// milliseconds; the query also runs alongside paste mode's pre-paste delay,
/// so in practice it adds no latency.
const QUERY_TIMEOUT: Duration = Duration::from_millis(250);

/// Window classes / Wayland app_ids of terminal emulators, compared
/// case-insensitively. Mirrors Omarchy's terminal tag rule
/// (default/hypr/apps/terminals.lua) plus other common terminals.
const TERMINAL_CLASSES: &[&str] = &[
    "alacritty",
    "kitty",
    "com.mitchellh.ghostty",
    "ghostty",
    "foot",
    "footclient",
    "org.codeberg.dnkl.foot",
    "wezterm",
    "org.wezfurlong.wezterm",
    "konsole",
    "org.kde.konsole",
    "gnome-terminal-server",
    "org.gnome.terminal",
    "org.gnome.console",
    "kgx",
    "org.gnome.ptyxis",
    "ptyxis",
    "com.raggesilver.blackbox",
    "xterm",
    "uxterm",
    "urxvt",
    "rxvt",
    "st",
    "st-256color",
    "tilix",
    "com.gexperts.tilix",
    "terminator",
    "xfce4-terminal",
    "qterminal",
    "lxterminal",
    "sakura",
    "terminology",
    "rio",
    "contour",
    "dev.warp.warp",
    "warp",
    "tabby",
    "cool-retro-term",
    "zutty",
];

/// Class prefixes that mark a terminal. Omarchy launches its TUIs and its own
/// terminal windows under these app_ids (org.omarchy.btop, TUI.float, ...).
const TERMINAL_CLASS_PREFIXES: &[&str] = &["org.omarchy.", "tui."];

/// Whether a window class or app_id belongs to a known terminal emulator.
pub fn is_terminal_class(class: &str) -> bool {
    let class = class.trim().to_ascii_lowercase();
    if class.is_empty() {
        return false;
    }
    TERMINAL_CLASSES.contains(&class.as_str())
        || TERMINAL_CLASS_PREFIXES.iter().any(|p| class.starts_with(p))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compositor {
    Hyprland,
    Sway,
    Niri,
}

fn detect_compositor() -> Option<Compositor> {
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
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

/// Ask the compositor whether the focused window is a terminal.
///
/// Returns `None` when that can't be determined: unsupported compositor,
/// missing tool, timeout, nothing focused, or unparseable output.
pub async fn focused_window_is_terminal() -> Option<bool> {
    let compositor = detect_compositor()?;
    let (program, args): (&str, &[&str]) = match compositor {
        Compositor::Hyprland => ("hyprctl", &["activewindow", "-j"]),
        Compositor::Sway => ("swaymsg", &["-t", "get_tree"]),
        Compositor::Niri => ("niri", &["msg", "--json", "focused-window"]),
    };

    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();

    let output = match tokio::time::timeout(QUERY_TIMEOUT, child).await {
        Ok(Ok(output)) if output.status.success() => output,
        Ok(Ok(output)) => {
            tracing::debug!("{} exited with {}", program, output.status);
            return None;
        }
        Ok(Err(e)) => {
            tracing::debug!("Could not run {}: {}", program, e);
            return None;
        }
        Err(_) => {
            tracing::debug!("{} timed out after {:?}", program, QUERY_TIMEOUT);
            return None;
        }
    };

    let json = String::from_utf8_lossy(&output.stdout);
    let result = match compositor {
        Compositor::Hyprland => hyprland_is_terminal(&json),
        Compositor::Sway => sway_is_terminal(&json),
        Compositor::Niri => niri_is_terminal(&json),
    };
    tracing::debug!(
        "Focused window terminal check via {}: {:?}",
        program,
        result
    );
    result
}

/// Parse `hyprctl activewindow -j`. A window tagged `terminal` (dynamic
/// tags carry a trailing `*`, e.g. `terminal*`) is a terminal; otherwise the
/// class decides.
fn hyprland_is_terminal(json: &str) -> Option<bool> {
    let window: Value = serde_json::from_str(json).ok()?;
    let window = window.as_object()?;

    let tagged = window
        .get("tags")
        .and_then(Value::as_array)
        .is_some_and(|tags| {
            tags.iter()
                .filter_map(Value::as_str)
                .any(|tag| tag.trim_end_matches('*') == "terminal")
        });
    if tagged {
        return Some(true);
    }

    // hyprctl prints `{}` when no window has focus.
    let class = window.get("class").and_then(Value::as_str)?;
    let initial_class = window
        .get("initialClass")
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(is_terminal_class(class) || is_terminal_class(initial_class))
}

/// Parse `swaymsg -t get_tree`, find the focused node, and check its
/// app_id (Wayland) or window class (XWayland).
fn sway_is_terminal(json: &str) -> Option<bool> {
    let tree: Value = serde_json::from_str(json).ok()?;
    let node = find_focused_sway_node(&tree)?;
    let app_id = node.get("app_id").and_then(Value::as_str).unwrap_or("");
    let class = node
        .get("window_properties")
        .and_then(|p| p.get("class"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if app_id.is_empty() && class.is_empty() {
        // Focus is on a workspace or output, not a window.
        return None;
    }
    Some(is_terminal_class(app_id) || is_terminal_class(class))
}

fn find_focused_sway_node(node: &Value) -> Option<&Value> {
    if node.get("focused").and_then(Value::as_bool) == Some(true) {
        return Some(node);
    }
    ["nodes", "floating_nodes"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .find_map(find_focused_sway_node)
}

/// Parse `niri msg --json focused-window`, which prints a window object or
/// `null`.
fn niri_is_terminal(json: &str) -> Option<bool> {
    let window: Value = serde_json::from_str(json).ok()?;
    let app_id = window.get("app_id").and_then(Value::as_str)?;
    Some(is_terminal_class(app_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyprland_dynamic_terminal_tag_marks_a_terminal() {
        let json = r#"{"class": "something-custom", "tags": ["default-opacity*", "terminal*"]}"#;
        assert_eq!(hyprland_is_terminal(json), Some(true));
    }

    #[test]
    fn hyprland_static_terminal_tag_marks_a_terminal() {
        let json = r#"{"class": "something-custom", "tags": ["terminal"]}"#;
        assert_eq!(hyprland_is_terminal(json), Some(true));
    }

    #[test]
    fn hyprland_tag_must_match_exactly() {
        let json =
            r#"{"class": "firefox", "tags": ["terminals*", "not-terminal", "terminal-ish"]}"#;
        assert_eq!(hyprland_is_terminal(json), Some(false));
    }

    #[test]
    fn hyprland_falls_back_to_class_without_tags() {
        let ghostty = r#"{"class": "com.mitchellh.ghostty", "tags": []}"#;
        assert_eq!(hyprland_is_terminal(ghostty), Some(true));

        let no_tags_field = r#"{"class": "Alacritty"}"#;
        assert_eq!(hyprland_is_terminal(no_tags_field), Some(true));

        let browser = r#"{"class": "chromium", "tags": ["default-opacity*"]}"#;
        assert_eq!(hyprland_is_terminal(browser), Some(false));
    }

    #[test]
    fn hyprland_checks_initial_class_too() {
        let json = r#"{"class": "", "initialClass": "kitty", "tags": []}"#;
        assert_eq!(hyprland_is_terminal(json), Some(true));
    }

    #[test]
    fn hyprland_full_activewindow_output() {
        let json = r#"{
            "address": "0x55ddf20557b0",
            "mapped": true,
            "class": "com.mitchellh.ghostty",
            "title": "codex",
            "initialClass": "com.mitchellh.ghostty",
            "pid": 6015,
            "xwayland": false,
            "tags": ["default-opacity*", "terminal*"],
            "contentType": "none"
        }"#;
        assert_eq!(hyprland_is_terminal(json), Some(true));
    }

    #[test]
    fn hyprland_no_focused_window_is_unknown() {
        assert_eq!(hyprland_is_terminal("{}"), None);
        assert_eq!(hyprland_is_terminal("Invalid"), None);
        assert_eq!(hyprland_is_terminal(""), None);
    }

    #[test]
    fn sway_finds_focused_node_in_tiled_and_floating_trees() {
        let tiled = r#"{"focused": false, "nodes": [
            {"focused": false, "nodes": [
                {"focused": false, "app_id": "firefox", "nodes": []},
                {"focused": true, "app_id": "foot", "nodes": []}
            ], "floating_nodes": []}
        ], "floating_nodes": []}"#;
        assert_eq!(sway_is_terminal(tiled), Some(true));

        let floating_xwayland = r#"{"focused": false, "nodes": [
            {"focused": false, "nodes": [], "floating_nodes": [
                {"focused": true, "app_id": null,
                 "window_properties": {"class": "URxvt"}, "nodes": []}
            ]}
        ]}"#;
        assert_eq!(sway_is_terminal(floating_xwayland), Some(true));

        let gui = r#"{"focused": false, "nodes": [
            {"focused": true, "app_id": "org.gnome.Nautilus", "nodes": []}
        ]}"#;
        assert_eq!(sway_is_terminal(gui), Some(false));
    }

    #[test]
    fn sway_focus_on_empty_workspace_is_unknown() {
        let json = r#"{"focused": false, "nodes": [
            {"focused": true, "type": "workspace", "nodes": []}
        ]}"#;
        assert_eq!(sway_is_terminal(json), None);
    }

    #[test]
    fn niri_reads_app_id() {
        assert_eq!(
            niri_is_terminal(r#"{"id": 3, "app_id": "org.wezfurlong.wezterm"}"#),
            Some(true)
        );
        assert_eq!(
            niri_is_terminal(r#"{"id": 4, "app_id": "firefox"}"#),
            Some(false)
        );
        assert_eq!(niri_is_terminal("null"), None);
    }

    #[test]
    fn terminal_classes_match_case_insensitively_and_by_omarchy_prefix() {
        assert!(is_terminal_class("Alacritty"));
        assert!(is_terminal_class("org.codeberg.dnkl.foot"));
        assert!(is_terminal_class("org.omarchy.btop"));
        assert!(is_terminal_class("TUI.float"));
        assert!(!is_terminal_class("firefox"));
        assert!(!is_terminal_class(""));
        assert!(!is_terminal_class("stalonetray"));
    }

    /// Queries the running compositor (read-only). Run from a terminal
    /// window with `cargo test live_focused_window -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs a running Hyprland, Sway or niri session"]
    async fn live_focused_window_query() {
        let started = std::time::Instant::now();
        let result = focused_window_is_terminal().await;
        println!(
            "compositor: {:?}, terminal: {:?}, took {:?}",
            detect_compositor(),
            result,
            started.elapsed()
        );
        assert!(result.is_some());
    }
}
