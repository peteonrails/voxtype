//! Window titles from Hyprland, used to confirm a call in a browser.
//!
//! Reads the same `j/clients` request `hyprctl clients -j` sends, straight
//! from Hyprland's command socket. Other compositors have no equivalent we
//! rely on, so detection runs without window titles there.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::Window;

#[derive(Deserialize)]
struct Client {
    #[serde(default)]
    class: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    pid: i64,
}

/// The windows in a `hyprctl clients -j` document.
pub fn parse_clients(json: &str) -> Result<Vec<Window>, serde_json::Error> {
    let clients: Vec<Client> = serde_json::from_str(json)?;
    Ok(clients
        .into_iter()
        .map(|c| Window {
            class: c.class,
            title: c.title,
            pid: u32::try_from(c.pid).ok().filter(|p| *p > 0),
        })
        .collect())
}

/// Hyprland's command socket: from `HYPRLAND_INSTANCE_SIGNATURE` when set,
/// otherwise the most recent instance under `$XDG_RUNTIME_DIR/hypr`, so a
/// daemon started without the variable still finds it.
pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    let hypr = runtime.join("hypr");
    if let Some(sig) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").filter(|s| !s.is_empty()) {
        let path = hypr.join(sig).join(".socket.sock");
        return path.exists().then_some(path);
    }
    newest_instance(&hypr)
}

fn newest_instance(hypr: &Path) -> Option<PathBuf> {
    std::fs::read_dir(hypr)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path().join(".socket.sock"))
        .filter(|p| p.exists())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

/// Ask Hyprland for its windows. `None` when Hyprland is not running.
#[cfg(unix)]
pub async fn query_windows() -> Option<Vec<Window>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let path = socket_path()?;
    let fetch = async {
        let mut sock = tokio::net::UnixStream::connect(&path).await.ok()?;
        sock.write_all(b"j/clients").await.ok()?;
        let mut buf = String::new();
        sock.read_to_string(&mut buf).await.ok()?;
        Some(buf)
    };
    let json = tokio::time::timeout(std::time::Duration::from_secs(2), fetch)
        .await
        .ok()??;
    match parse_clients(&json) {
        Ok(windows) => Some(windows),
        Err(e) => {
            tracing::debug!("Unreadable Hyprland clients reply: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_class_title_and_pid() {
        let json = r#"[{"class": "chromium", "title": "Meet - abc-defg-hij - Chromium",
                        "pid": 4242, "workspace": {"id": 1}},
                       {"class": "", "title": "", "pid": -1}]"#;
        let windows = parse_clients(json).unwrap();
        assert_eq!(windows[0].class, "chromium");
        assert_eq!(windows[0].pid, Some(4242));
        assert_eq!(windows[1].pid, None);
    }

    #[test]
    fn finds_the_newest_instance_socket() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(newest_instance(dir.path()), None);
        let inst = dir.path().join("abc_1_2");
        std::fs::create_dir(&inst).unwrap();
        std::fs::write(inst.join(".socket.sock"), "").unwrap();
        assert_eq!(newest_instance(dir.path()), Some(inst.join(".socket.sock")));
    }
}
