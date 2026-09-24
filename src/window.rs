//! Focused-window queries for per-application output rules.
//!
//! Wayland has no universal "what is focused" protocol, so detection shells
//! out to the active compositor's IPC tool (hyprctl, swaymsg, niri msg) or,
//! on macOS, to osascript. A short timeout guards against a hung compositor
//! socket stalling the output path: on timeout or parse failure we report
//! no focused window and per-app rules simply don't apply for that cycle.

use crate::config::OutputConfig;
#[cfg(not(target_os = "macos"))]
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

/// How long to wait for the compositor query before giving up.
fn query_timeout() -> Duration {
    Duration::from_millis(750)
}

/// The currently focused window, as the compositor reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedWindow {
    /// Window class / app_id (e.g. "Slack", "kitty", "org.qutebrowser.qutebrowser")
    pub class: String,
    /// Window title (may be empty when the compositor doesn't provide one)
    pub title: String,
}

/// Resolve `[output.auto_submit_apps]` against the focused window.
///
/// Returns the matched rule's value, or `None` when no rules are configured
/// (the compositor is not queried at all), the focused window can't be
/// determined, or no pattern matches. Callers keep the global `auto_submit`
/// in every `None` case.
pub async fn auto_submit_override(config: &OutputConfig) -> Option<bool> {
    if config.auto_submit_apps.is_empty() {
        return None;
    }
    let Some(focused) = focused_window().await else {
        tracing::debug!(
            "auto_submit_apps configured but focused window could not be determined; \
             using global auto_submit"
        );
        return None;
    };
    match config.resolve_auto_submit_apps(&focused.class, &focused.title) {
        Some(submit) => {
            tracing::info!(
                class = %focused.class,
                title = %focused.title,
                submit,
                "Per-app auto-submit rule applied"
            );
            Some(submit)
        }
        None => {
            tracing::debug!(
                class = %focused.class,
                "No auto_submit_apps rule matched focused window"
            );
            None
        }
    }
}

/// Query the compositor for the focused window.
///
/// Returns `None` when no supported compositor is detected, the query
/// fails or times out, or the compositor reports no focused window.
pub async fn focused_window() -> Option<FocusedWindow> {
    #[cfg(target_os = "macos")]
    {
        return query_macos().await;
    }
    #[cfg(not(target_os = "macos"))]
    {
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return query_hyprland().await;
        }
        if std::env::var_os("SWAYSOCK").is_some() {
            return query_sway().await;
        }
        if std::env::var_os("NIRI_SOCKET").is_some() {
            return query_niri().await;
        }
        tracing::debug!(
            "No supported compositor detected (Hyprland, Sway, Niri); \
             per-app output rules cannot be applied"
        );
        None
    }
}

/// Whether `focused_window()` has a backend to talk to in this environment.
///
/// The Linux backends are detected from the compositor's socket variable,
/// which a systemd user service only sees if the compositor exported it
/// (`systemctl --user import-environment ...`). The daemon warns at startup
/// when rules are configured but this returns false.
pub fn compositor_supported() -> bool {
    if cfg!(target_os = "macos") {
        return true;
    }
    ["HYPRLAND_INSTANCE_SIGNATURE", "SWAYSOCK", "NIRI_SOCKET"]
        .iter()
        .any(|var| std::env::var_os(var).is_some())
}

async fn run_command(program: &str, args: &[&str]) -> Option<String> {
    // kill_on_drop: when the timeout below fires, the future is dropped and
    // the child must die with it, or a wedged compositor socket leaks one
    // blocked process per dictation.
    let future = Command::new(program).args(args).kill_on_drop(true).output();
    let output = match tokio::time::timeout(query_timeout(), future).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            tracing::warn!("{} query failed: {}", program, e);
            return None;
        }
        Err(_) => {
            tracing::warn!("{} query timed out", program);
            return None;
        }
    };

    if !output.status.success() {
        tracing::warn!(
            "{} exited with {}: {}",
            program,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(not(target_os = "macos"))]
async fn query_hyprland() -> Option<FocusedWindow> {
    let stdout = run_command("hyprctl", &["-j", "activewindow"]).await?;
    parse_hyprland_window(&stdout)
}

#[cfg(not(target_os = "macos"))]
/// Parse `hyprctl -j activewindow` output. Returns `None` for empty
/// desktops (empty JSON body) or the non-JSON "Invalid" response some
/// Hyprland versions emit when nothing is focused.
fn parse_hyprland_window(stdout: &str) -> Option<FocusedWindow> {
    let json: Value = serde_json::from_str(stdout.trim()).ok()?;
    let class = json.get("class")?.as_str()?.to_string();
    if class.is_empty() {
        return None;
    }
    let title = json
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(FocusedWindow { class, title })
}

#[cfg(not(target_os = "macos"))]
async fn query_sway() -> Option<FocusedWindow> {
    let stdout = run_command("swaymsg", &["-t", "get_tree", "-r"]).await?;
    let json: Value = serde_json::from_str(&stdout).ok()?;
    let node = find_focused_node(&json)?;
    parse_node(node)
}

#[cfg(not(target_os = "macos"))]
/// Depth-first search for the tree node with `focused == true`.
fn find_focused_node(value: &Value) -> Option<&Value> {
    if value.get("focused").and_then(Value::as_bool) == Some(true) {
        return Some(value);
    }
    for key in ["nodes", "floating_nodes"] {
        if let Some(children) = value.get(key).and_then(Value::as_array) {
            for child in children {
                if let Some(found) = find_focused_node(child) {
                    return Some(found);
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
/// Extract class/title from a Sway or Niri window node.
fn parse_node(node: &Value) -> Option<FocusedWindow> {
    let class = node
        .get("app_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            // XWayland windows under Sway have no app_id; fall back to the
            // X11 class nested in the window_properties object.
            node.get("window_properties")
                .and_then(|p| p.get("class"))
                .and_then(Value::as_str)
        })?
        .to_string();
    // Sway exposes the title as "name"; Niri as "title".
    let title = ["title", "name"]
        .iter()
        .find_map(|key| node.get(*key).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    Some(FocusedWindow { class, title })
}

#[cfg(not(target_os = "macos"))]
async fn query_niri() -> Option<FocusedWindow> {
    let stdout = run_command("niri", &["msg", "--json", "focused-window"]).await?;
    let json: Value = serde_json::from_str(stdout.trim()).ok()?;
    parse_node(&json)
}

#[cfg(target_os = "macos")]
async fn query_macos() -> Option<FocusedWindow> {
    // osascript accepts a multi-line script in a single -e argument.
    let script = r#"
        tell application "System Events"
            get name of first application process whose frontmost is true
        end tell
    "#;
    let stdout = run_command("osascript", &["-e", script.trim()]).await?;
    let class = stdout.trim().to_string();
    if class.is_empty() {
        return None;
    }
    Some(FocusedWindow {
        class,
        title: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_parse_hyprland_window() {
        let json = r#"{"address":"0x123","class":"kitty","title":"zsh"}"#;
        let fw = parse_hyprland_window(json).unwrap();
        assert_eq!(fw.class, "kitty");
        assert_eq!(fw.title, "zsh");
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_parse_hyprland_window_invalid() {
        // Empty desktop: older Hyprland answers "Invalid", newer ones send
        // an empty class.
        assert!(parse_hyprland_window("Invalid").is_none());
        assert!(parse_hyprland_window(r#"{"class":"","title":""}"#).is_none());
        assert!(parse_hyprland_window("").is_none());
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_find_focused_node() {
        let tree = serde_json::json!({
            "nodes": [
                {"id": 1, "focused": false},
                {"id": 2, "nodes": [
                    {"id": 3, "floating_nodes": []},
                    {"id": 4, "focused": true, "app_id": "foot"}
                ]}
            ]
        });
        let node = find_focused_node(&tree).unwrap();
        assert_eq!(node.get("app_id").and_then(Value::as_str), Some("foot"));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_find_focused_node_none() {
        let tree = serde_json::json!({"nodes": [{"id": 1, "focused": false}]});
        assert!(find_focused_node(&tree).is_none());
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_parse_node_sway_xwayland() {
        let node = serde_json::json!({
            "app_id": null,
            "name": "Steam",
            "window_properties": {"class": "Steam"}
        });
        let fw = parse_node(&node).unwrap();
        assert_eq!(fw.class, "Steam");
        assert_eq!(fw.title, "Steam");
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_parse_node_niri() {
        let node = serde_json::json!({
            "app_id": "Slack",
            "title": "Slack | #dev"
        });
        let fw = parse_node(&node).unwrap();
        assert_eq!(fw.class, "Slack");
        assert_eq!(fw.title, "Slack | #dev");
    }
}
