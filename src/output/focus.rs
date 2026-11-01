//! Remember and restore the window that had keyboard focus when a recording
//! started.
//!
//! [`PinnedWindow::capture`] snapshots the currently focused window using
//! whichever mechanism the session provides. [`PinnedWindow::refocus`] hands
//! focus back to that window right before text output, so dictation started
//! in one window lands there even when the user browsed to other windows or
//! desktops while speaking.
//!
//! ## Platform support (detected automatically at capture time)
//!
//! | Session                      | Mechanism                                  |
//! |------------------------------|--------------------------------------------|
//! | Hyprland                     | `hyprctl` (classic and 0.56+ Lua `hl.dsp.*` dispatchers) |
//! | sway                         | `swaymsg -t get_tree` / `[con_id] focus`   |
//! | i3                           | `i3-msg -t get_tree` / `[con_id] focus`    |
//! | X11                          | `xdotool getactivewindow` / `windowactivate` |
//! | macOS                        | System Events (frontmost application)      |
//! | GNOME / KDE / other Wayland  | not supported — feature stays inert        |
//!
//! Everything here is best-effort and fail-safe: when the session has no
//! supported focused-window mechanism, capture returns `None` and the caller
//! skips the feature entirely. Text output then behaves exactly as it would
//! with `return_to_start_window = false`.

use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// How long to wait for a helper command (`hyprctl`, `swaymsg`, `xdotool`,
/// `osascript`) before giving up. These are fast local IPC calls; the bound
/// exists only so a wedged compositor can never stall text output.
const HELPER_TIMEOUT: Duration = Duration::from_secs(2);

/// A window snapshot taken when recording started, together with the
/// knowledge of how to focus it again later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinnedWindow {
    /// Hyprland window. The `address` is Hyprland's stable window identity
    /// (survives title changes and workspace moves).
    Hyprland {
        address: String,
        /// Window center in layout coordinates, when the geometry was
        /// available. Used to warp the pointer onto the window before
        /// focusing it: under pointer-follows-focus behavior
        /// (Hyprland `follow_mouse`), a cursor left on another window or
        /// monitor can pull focus straight back after we set it.
        center: Option<(i32, i32)>,
        title: Option<String>,
    },
    /// sway / i3 container, focusable by container id. `tool` records which
    /// IPC client (`swaymsg` or `i3-msg`) matched the session.
    Sway {
        con_id: i64,
        name: Option<String>,
        tool: &'static str,
    },
    /// X11 window, focusable via xdotool.
    X11 {
        window_id: u32,
        name: Option<String>,
    },
    /// macOS application process, raisable via System Events.
    MacOS { pid: i32, name: Option<String> },
}

impl PinnedWindow {
    /// Snapshot the currently focused window, or `None` when the session has
    /// no supported focused-window mechanism (or the query failed — e.g. no
    /// window has focus, or the compositor helper is not installed).
    pub async fn capture() -> Option<PinnedWindow> {
        let get_env = |key: &str| std::env::var(key).ok();
        match select_provider(&get_env) {
            Provider::Hyprland => capture_hyprland().await,
            Provider::Sway => capture_sway("swaymsg").await,
            Provider::I3 => capture_sway("i3-msg").await,
            Provider::X11 => capture_x11().await,
            Provider::MacOS => capture_macos().await,
            Provider::Unsupported => None,
        }
    }

    /// Give keyboard focus back to this window. Returns `true` when the
    /// focus command completed successfully.
    ///
    /// Never fatal: failures are logged and the caller simply proceeds with
    /// whatever window currently has focus.
    pub async fn refocus(&self) -> bool {
        match self {
            PinnedWindow::Hyprland {
                address, center, ..
            } => {
                let window_ref = format!("address:{address}");

                // Hyprland 0.56 replaced the classic
                // `dispatch <dispatcher> <arg>` CLI syntax with a Lua-based
                // dispatcher API (`hl.dsp.*`). Try the new syntax first and
                // fall back to the classic one for older releases.
                let focus_ok = run_command(
                    "hyprctl",
                    &[&format!(
                        r#"dispatch hl.dsp.focus({{ window = "{window_ref}" }})"#
                    )],
                )
                .await
                .is_some()
                    || run_command("hyprctl", &["dispatch", "focuswindow", &window_ref])
                        .await
                        .is_some();
                if !focus_ok {
                    tracing::debug!("hyprctl: could not focus {window_ref}");
                    return false;
                }

                // Raise the window (Lua API; classic `focuswindow` already
                // raises on older releases).
                let _ = run_command(
                    "hyprctl",
                    &[&format!(
                        r#"dispatch hl.dsp.window.bring_to_top({{ window = "{window_ref}" }})"#
                    )],
                )
                .await;

                // Warp the pointer onto the target: with pointer-follows-
                // focus behavior (Hyprland `follow_mouse`), a cursor left on
                // another window or monitor can pull focus straight back
                // after we set it.
                if let Some((x, y)) = center {
                    let cursor_ok = run_command(
                        "hyprctl",
                        &[&format!(
                            r#"dispatch hl.dsp.cursor.move({{ x = {x}, y = {y} }})"#
                        )],
                    )
                    .await
                    .is_some()
                        || run_command(
                            "hyprctl",
                            &["dispatch", "movecursor", &x.to_string(), &y.to_string()],
                        )
                        .await
                        .is_some();
                    if !cursor_ok {
                        tracing::debug!(
                            "could not warp cursor onto start window; on \
                             pointer-follows-focus setups focus may be pulled back"
                        );
                    }
                }

                true
            }
            PinnedWindow::Sway { con_id, tool, .. } => {
                run_command(tool, &[&format!("[con_id={con_id}]"), "focus"])
                    .await
                    .is_some()
            }
            PinnedWindow::X11 { window_id, .. } => run_command(
                "xdotool",
                &["windowactivate", "--sync", &window_id.to_string()],
            )
            .await
            .is_some(),
            PinnedWindow::MacOS { pid, .. } => {
                let script = format!(
                    "tell application \"System Events\" to set frontmost of \
                     (first application process whose unix id is {pid}) to true"
                );
                run_command("osascript", &["-e", &script]).await.is_some()
            }
        }
    }

    /// Short human-readable description for logs.
    pub fn describe(&self) -> String {
        match self {
            PinnedWindow::Hyprland { address, title, .. } => format!(
                "Hyprland window {:?} ({address})",
                title.as_deref().unwrap_or("<untitled>")
            ),
            PinnedWindow::Sway {
                con_id, name, tool, ..
            } => format!(
                "{tool} container {con_id} ({})",
                name.as_deref().unwrap_or("<unnamed>")
            ),
            PinnedWindow::X11 { window_id, name } => format!(
                "X11 window {window_id:#x} ({})",
                name.as_deref().unwrap_or("<unnamed>")
            ),
            PinnedWindow::MacOS { pid, name } => format!(
                "macOS process {pid} ({})",
                name.as_deref().unwrap_or("<unnamed>")
            ),
        }
    }
}

/// Which focused-window mechanism applies to the current session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Hyprland,
    Sway,
    I3,
    X11,
    MacOS,
    /// No supported mechanism; the feature stays inert.
    Unsupported,
}

/// Decide which focused-window mechanism applies to this session, from the
/// most to the least specific signal.
///
/// Wayland compositors we cannot query (GNOME, KDE, ...) report
/// [`Provider::Unsupported`] rather than falling through to X11 tools that
/// would return nonsense under Wayland.
fn select_provider(get_env: &dyn Fn(&str) -> Option<String>) -> Provider {
    let env_nonempty =
        |key: &str| -> Option<String> { get_env(key).filter(|v| !v.trim().is_empty()) };

    if env_nonempty("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Provider::Hyprland;
    }
    if env_nonempty("SWAYSOCK").is_some() {
        return Provider::Sway;
    }
    if env_nonempty("I3SOCK").is_some() {
        return Provider::I3;
    }
    let is_wayland = env_nonempty("WAYLAND_DISPLAY").is_some()
        || env_nonempty("XDG_SESSION_TYPE").is_some_and(|v| v == "wayland");
    if is_wayland {
        return Provider::Unsupported;
    }
    let is_x11 = env_nonempty("DISPLAY").is_some()
        || env_nonempty("XDG_SESSION_TYPE").is_some_and(|v| v == "x11");
    if is_x11 {
        return Provider::X11;
    }
    if cfg!(target_os = "macos") {
        return Provider::MacOS;
    }
    Provider::Unsupported
}

/// Run a helper command and return its trimmed stdout, or `None` when the
/// command could not be spawned, failed, or timed out. Failures are logged
/// at debug level — this whole module is best-effort by contract.
async fn run_command(program: &str, args: &[&str]) -> Option<String> {
    let command = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match tokio::time::timeout(HELPER_TIMEOUT, command).await {
        Ok(Ok(out)) if out.status.success() => String::from_utf8(out.stdout)
            .ok()
            .map(|s| s.trim().to_string()),
        Ok(Ok(out)) => {
            tracing::debug!(
                "{} {:?} exited with {}: {}",
                program,
                args,
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
            None
        }
        Ok(Err(e)) => {
            tracing::debug!(
                "{} failed to spawn: {} (is the helper installed?)",
                program,
                e
            );
            None
        }
        Err(_) => {
            tracing::debug!("{} {:?} timed out", program, args);
            None
        }
    }
}

async fn capture_hyprland() -> Option<PinnedWindow> {
    let raw = run_command("hyprctl", &["activewindow", "-j"]).await?;
    parse_hyprland_activewindow(&raw)
}

/// Parse `hyprctl activewindow -j` output into a pinned window.
///
/// `hyprctl` prints the non-JSON string `Invalid` when no window has focus,
/// which fails JSON parsing and yields `None` — exactly the fail-safe we
/// want.
fn parse_hyprland_activewindow(raw: &str) -> Option<PinnedWindow> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let address = value.get("address")?.as_str()?.to_string();
    if address.is_empty() {
        return None;
    }
    let center = match (value.get("at"), value.get("size")) {
        (Some(at), Some(size)) => {
            let x = at.get(0)?.as_i64()?;
            let y = at.get(1)?.as_i64()?;
            let w = size.get(0)?.as_i64()?;
            let h = size.get(1)?.as_i64()?;
            // Guard against degenerate geometry; a division by two of
            // non-negative sizes is exact for our purposes.
            Some(((x + w / 2) as i32, (y + h / 2) as i32))
        }
        _ => None,
    };
    let title = value
        .get("title")
        .and_then(|t| t.as_str())
        .map(str::to_string);
    Some(PinnedWindow::Hyprland {
        address,
        center,
        title,
    })
}

async fn capture_sway(tool: &'static str) -> Option<PinnedWindow> {
    let raw = run_command(tool, &["-t", "get_tree", "-r"]).await?;
    let tree: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let node = find_focused_node(&tree)?;
    let con_id = node.get("id")?.as_i64()?;
    let name = node
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string);
    Some(PinnedWindow::Sway { con_id, name, tool })
}

/// Depth-first search for the container with `focused: true` in an i3/sway
/// layout tree.
fn find_focused_node(node: &serde_json::Value) -> Option<&serde_json::Value> {
    if node.get("focused").and_then(|f| f.as_bool()) == Some(true) {
        return Some(node);
    }
    for key in ["nodes", "floating_nodes"] {
        if let Some(children) = node.get(key).and_then(|c| c.as_array()) {
            for child in children {
                if let Some(found) = find_focused_node(child) {
                    return Some(found);
                }
            }
        }
    }
    None
}

async fn capture_x11() -> Option<PinnedWindow> {
    let id_raw = run_command("xdotool", &["getactivewindow"]).await?;
    let window_id: u32 = id_raw.trim().parse().ok()?;
    let name = run_command("xdotool", &["getwindowname", &window_id.to_string()]).await;
    Some(PinnedWindow::X11 { window_id, name })
}

async fn capture_macos() -> Option<PinnedWindow> {
    let raw = run_command(
        "osascript",
        &[
            "-e",
            "tell application \"System Events\" to get {unix id, name} of \
           first application process whose frontmost is true",
        ],
    )
    .await?;
    parse_macos_frontmost(&raw)
}

/// Parse `osascript` list output of the form `1234, Finder`
/// (unix id, process name). `split_once` keeps process names that themselves
/// contain a comma intact.
fn parse_macos_frontmost(raw: &str) -> Option<PinnedWindow> {
    let (pid_raw, name) = raw.trim().split_once(',')?;
    let pid: i32 = pid_raw.trim().parse().ok()?;
    let name = name.trim();
    Some(PinnedWindow::MacOS {
        pid,
        name: (!name.is_empty()).then(|| name.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an env lookup from a fixed table (order-independent).
    fn env_table<'a>(table: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key: &str| {
            table
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn test_select_provider_hyprland_wins() {
        let get_env = env_table(&[
            ("HYPRLAND_INSTANCE_SIGNATURE", "sig_123"),
            ("WAYLAND_DISPLAY", "wayland-1"),
        ]);
        assert_eq!(select_provider(&get_env), Provider::Hyprland);
    }

    #[test]
    fn test_select_provider_sway_and_i3() {
        let get_env = env_table(&[("SWAYSOCK", "/run/user/1000/sway-ipc.sock")]);
        assert_eq!(select_provider(&get_env), Provider::Sway);

        let get_env = env_table(&[("I3SOCK", "/tmp/i3.sock"), ("DISPLAY", ":0")]);
        assert_eq!(select_provider(&get_env), Provider::I3);
    }

    #[test]
    fn test_select_provider_unknown_wayland_is_unsupported() {
        // GNOME / KDE / river / ... — we must NOT fall through to X11 tools.
        let get_env = env_table(&[("WAYLAND_DISPLAY", "wayland-0")]);
        assert_eq!(select_provider(&get_env), Provider::Unsupported);

        let get_env = env_table(&[("XDG_SESSION_TYPE", "wayland")]);
        assert_eq!(select_provider(&get_env), Provider::Unsupported);
    }

    #[test]
    fn test_select_provider_x11() {
        let get_env = env_table(&[("XDG_SESSION_TYPE", "x11")]);
        assert_eq!(select_provider(&get_env), Provider::X11);

        let get_env = env_table(&[("DISPLAY", ":0")]);
        assert_eq!(select_provider(&get_env), Provider::X11);
    }

    #[test]
    fn test_select_provider_empty_env_values_are_ignored() {
        // A set-but-empty variable must not select a provider.
        let get_env = env_table(&[
            ("HYPRLAND_INSTANCE_SIGNATURE", ""),
            ("SWAYSOCK", "  "),
            ("WAYLAND_DISPLAY", ""),
            ("DISPLAY", ""),
        ]);
        assert_eq!(select_provider(&get_env), Provider::Unsupported);
    }

    #[test]
    fn test_select_provider_tty_is_unsupported() {
        let get_env = env_table(&[("XDG_SESSION_TYPE", "tty")]);
        assert_eq!(select_provider(&get_env), Provider::Unsupported);
    }

    #[test]
    fn test_parse_hyprland_activewindow() {
        let raw = r#"{
            "address": "0x5612ab",
            "at": [1920, 0],
            "size": [960, 540],
            "title": "foot — Work",
            "class": "foot",
            "pid": 4242,
            "focused": true
        }"#;
        let pinned = parse_hyprland_activewindow(raw).unwrap();
        match pinned {
            PinnedWindow::Hyprland {
                address,
                center,
                title,
            } => {
                assert_eq!(address, "0x5612ab");
                assert_eq!(center, Some((2400, 270)));
                assert_eq!(title.as_deref(), Some("foot — Work"));
            }
            other => panic!("expected Hyprland variant, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_hyprland_activewindow_invalid() {
        // `hyprctl activewindow -j` prints this when no window has focus.
        assert!(parse_hyprland_activewindow("Invalid").is_none());
    }

    #[test]
    fn test_parse_hyprland_activewindow_missing_geometry_is_ok() {
        let raw = r#"{ "address": "0xdeadbeef", "title": "Terminal" }"#;
        match parse_hyprland_activewindow(raw).unwrap() {
            PinnedWindow::Hyprland {
                address, center, ..
            } => {
                assert_eq!(address, "0xdeadbeef");
                assert_eq!(center, None);
            }
            other => panic!("expected Hyprland variant, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_hyprland_activewindow_empty_address_is_none() {
        let raw = r#"{ "address": "", "title": "" }"#;
        assert!(parse_hyprland_activewindow(raw).is_none());
    }

    #[test]
    fn test_find_focused_node_nested() {
        let tree: serde_json::Value = serde_json::from_str(
            r#"{
                "id": 1, "name": "root", "focused": false,
                "nodes": [
                    { "id": 2, "name": "ws1", "focused": false, "nodes": [
                        { "id": 3, "name": "foot", "focused": true }
                    ]},
                    { "id": 4, "name": "ws2", "focused": false, "nodes": [] }
                ],
                "floating_nodes": []
            }"#,
        )
        .unwrap();
        let node = find_focused_node(&tree).unwrap();
        assert_eq!(node["id"].as_i64(), Some(3));
        assert_eq!(node["name"].as_str(), Some("foot"));
    }

    #[test]
    fn test_find_focused_node_in_floating_nodes() {
        let tree: serde_json::Value = serde_json::from_str(
            r#"{
                "id": 1, "focused": false,
                "nodes": [],
                "floating_nodes": [ { "id": 9, "name": "pavucontrol", "focused": true } ]
            }"#,
        )
        .unwrap();
        let node = find_focused_node(&tree).unwrap();
        assert_eq!(node["id"].as_i64(), Some(9));
    }

    #[test]
    fn test_find_focused_node_none_focused() {
        let tree: serde_json::Value = serde_json::from_str(
            r#"{ "id": 1, "focused": false, "nodes": [ { "id": 2, "focused": false } ] }"#,
        )
        .unwrap();
        assert!(find_focused_node(&tree).is_none());
    }

    #[test]
    fn test_parse_macos_frontmost() {
        match parse_macos_frontmost("1234, Finder\n").unwrap() {
            PinnedWindow::MacOS { pid, name } => {
                assert_eq!(pid, 1234);
                assert_eq!(name.as_deref(), Some("Finder"));
            }
            other => panic!("expected MacOS variant, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_macos_frontmost_name_with_comma() {
        match parse_macos_frontmost("5678, Code, main.rs").unwrap() {
            PinnedWindow::MacOS { pid, name } => {
                assert_eq!(pid, 5678);
                assert_eq!(name.as_deref(), Some("Code, main.rs"));
            }
            other => panic!("expected MacOS variant, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_macos_frontmost_garbage() {
        assert!(parse_macos_frontmost("").is_none());
        assert!(parse_macos_frontmost("not a pid").is_none());
    }

    #[test]
    fn test_describe_mentions_platform() {
        let hypr = PinnedWindow::Hyprland {
            address: "0x5612ab".to_string(),
            center: Some((10, 20)),
            title: Some("foot".to_string()),
        };
        assert!(hypr.describe().contains("Hyprland"));
        assert!(hypr.describe().contains("0x5612ab"));

        let sway = PinnedWindow::Sway {
            con_id: 42,
            name: Some("foot".to_string()),
            tool: "swaymsg",
        };
        assert!(sway.describe().contains("swaymsg"));
        assert!(sway.describe().contains("42"));
    }
}
