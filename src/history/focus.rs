//! The focused window, for the history entry's `app` (and, when opted in,
//! `window_title`) and for `history.exclude_apps`.
//!
//! Asked straight over each compositor's own IPC socket, never through a
//! helper binary: Hyprland (`j/activewindow`), Sway and i3-compatible
//! compositors (`GET_TREE`), and niri (`"FocusedWindow"`). Other desktops
//! (GNOME, KDE, X11) give no answer, so the app is unknown there and
//! exclusions cannot apply; the docs say so.

use std::time::Duration;

use serde_json::Value;

/// How long a compositor gets to answer. A dictation is never held up by
/// this: the lookup runs alongside output.
const TIMEOUT: Duration = Duration::from_millis(500);

/// What the compositor says has focus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusedWindow {
    /// Class or app ID, such as `kitty` or `org.keepassxc.KeePassXC`.
    pub app: String,
    pub title: String,
}

impl FocusedWindow {
    fn from_parts(app: Option<&str>, title: Option<&str>) -> Option<Self> {
        let app = app.unwrap_or_default().trim().to_string();
        let title = title.unwrap_or_default().to_string();
        (!app.is_empty() || !title.is_empty()).then_some(Self { app, title })
    }
}

/// Hyprland's `j/activewindow` reply. `{}` when nothing has focus.
pub fn parse_hyprland(json: &str) -> Option<FocusedWindow> {
    let v: Value = serde_json::from_str(json).ok()?;
    FocusedWindow::from_parts(v["class"].as_str(), v["title"].as_str())
}

/// The focused node in a Sway `GET_TREE` reply: `app_id` for Wayland
/// windows, `window_properties.class` for Xwayland ones.
pub fn parse_sway_tree(json: &str) -> Option<FocusedWindow> {
    fn find(node: &Value) -> Option<&Value> {
        if node["focused"].as_bool() == Some(true) {
            return Some(node);
        }
        for key in ["nodes", "floating_nodes"] {
            if let Some(children) = node[key].as_array() {
                if let Some(found) = children.iter().find_map(find) {
                    return Some(found);
                }
            }
        }
        None
    }
    let v: Value = serde_json::from_str(json).ok()?;
    let node = find(&v)?;
    // A focused workspace or output is not a window.
    if !matches!(node["type"].as_str(), Some("con") | Some("floating_con")) {
        return None;
    }
    let app = node["app_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| node["window_properties"]["class"].as_str());
    FocusedWindow::from_parts(app, node["name"].as_str())
}

/// niri's reply to `"FocusedWindow"`: `{"Ok":{"FocusedWindow":{...}}}`, with
/// `null` inside when nothing has focus.
pub fn parse_niri(json: &str) -> Option<FocusedWindow> {
    let v: Value = serde_json::from_str(json).ok()?;
    let w = &v["Ok"]["FocusedWindow"];
    if w.is_null() {
        return None;
    }
    FocusedWindow::from_parts(w["app_id"].as_str(), w["title"].as_str())
}

#[cfg(unix)]
async fn hyprland() -> Option<FocusedWindow> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let path = crate::meeting::detect::hyprland::socket_path()?;
    let mut sock = tokio::net::UnixStream::connect(&path).await.ok()?;
    sock.write_all(b"j/activewindow").await.ok()?;
    let mut buf = String::new();
    sock.read_to_string(&mut buf).await.ok()?;
    parse_hyprland(&buf)
}

#[cfg(unix)]
async fn sway() -> Option<FocusedWindow> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    const GET_TREE: u32 = 4;
    let path = std::env::var_os("SWAYSOCK").or_else(|| std::env::var_os("I3SOCK"))?;
    let mut sock = tokio::net::UnixStream::connect(&path).await.ok()?;
    let mut request = b"i3-ipc".to_vec();
    request.extend_from_slice(&0u32.to_ne_bytes());
    request.extend_from_slice(&GET_TREE.to_ne_bytes());
    sock.write_all(&request).await.ok()?;
    let mut header = [0u8; 14];
    sock.read_exact(&mut header).await.ok()?;
    if &header[..6] != b"i3-ipc" {
        return None;
    }
    let len = u32::from_ne_bytes(header[6..10].try_into().ok()?) as usize;
    // A tree is tens of kilobytes; refuse anything absurd.
    if len > 16 * 1024 * 1024 {
        return None;
    }
    let mut body = vec![0u8; len];
    sock.read_exact(&mut body).await.ok()?;
    parse_sway_tree(std::str::from_utf8(&body).ok()?)
}

#[cfg(unix)]
async fn niri() -> Option<FocusedWindow> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let path = std::env::var_os("NIRI_SOCKET")?;
    let mut sock = tokio::net::UnixStream::connect(&path).await.ok()?;
    sock.write_all(b"\"FocusedWindow\"\n").await.ok()?;
    let mut line = String::new();
    BufReader::new(sock).read_line(&mut line).await.ok()?;
    parse_niri(&line)
}

/// Ask whichever compositor is running what has focus. `None` when no
/// supported compositor answers in time.
pub async fn focused_window() -> Option<FocusedWindow> {
    #[cfg(unix)]
    {
        let ask = async {
            if std::env::var_os("NIRI_SOCKET").is_some() {
                if let Some(w) = niri().await {
                    return Some(w);
                }
            }
            if std::env::var_os("SWAYSOCK").is_some() || std::env::var_os("I3SOCK").is_some() {
                if let Some(w) = sway().await {
                    return Some(w);
                }
            }
            hyprland().await
        };
        tokio::time::timeout(TIMEOUT, ask).await.ok().flatten()
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hyprland_activewindow() {
        let w = parse_hyprland(
            r#"{"address":"0x1","class":"org.keepassxc.KeePassXC","title":"Passwords - KeePassXC","pid":42}"#,
        )
        .unwrap();
        assert_eq!(w.app, "org.keepassxc.KeePassXC");
        assert_eq!(w.title, "Passwords - KeePassXC");
        assert_eq!(parse_hyprland("{}"), None);
        assert_eq!(parse_hyprland("not json"), None);
    }

    #[test]
    fn reads_the_focused_sway_window_wayland_or_xwayland() {
        let tree = r#"{"type":"root","focused":false,"nodes":[
            {"type":"output","focused":false,"nodes":[
              {"type":"workspace","focused":false,"nodes":[
                {"type":"con","focused":false,"app_id":"firefox","name":"Mozilla Firefox","nodes":[]},
                {"type":"con","focused":true,"app_id":"kitty","name":"~/notes","nodes":[]}
              ],"floating_nodes":[]}
            ]}]}"#;
        assert_eq!(
            parse_sway_tree(tree),
            Some(FocusedWindow {
                app: "kitty".into(),
                title: "~/notes".into()
            })
        );
        let x = r#"{"type":"root","nodes":[{"type":"workspace","nodes":[],"floating_nodes":[
            {"type":"floating_con","focused":true,"app_id":null,
             "window_properties":{"class":"Bitwarden"},"name":"Bitwarden","nodes":[]}]}]}"#;
        assert_eq!(parse_sway_tree(x).unwrap().app, "Bitwarden");
        let workspace =
            r#"{"type":"root","nodes":[{"type":"workspace","focused":true,"nodes":[]}]}"#;
        assert_eq!(parse_sway_tree(workspace), None);
    }

    #[test]
    fn reads_niri_focused_window() {
        let w = parse_niri(
            r#"{"Ok":{"FocusedWindow":{"id":3,"title":"Inbox","app_id":"thunderbird"}}}"#,
        )
        .unwrap();
        assert_eq!(w.app, "thunderbird");
        assert_eq!(w.title, "Inbox");
        assert_eq!(parse_niri(r#"{"Ok":{"FocusedWindow":null}}"#), None);
    }
}
