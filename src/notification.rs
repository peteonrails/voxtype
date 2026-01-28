//! Platform-specific desktop notifications
//!
//! Provides a unified interface for sending desktop notifications on
//! different platforms:
//! - Linux: Uses notify-send (libnotify)
//! - macOS: Uses osascript (AppleScript)

use std::process::Stdio;
use tokio::process::Command;

/// Send a desktop notification with the given title and body.
///
/// This function is async and non-blocking. Notification failures are
/// logged but don't propagate errors (notifications are best-effort).
pub async fn send(title: &str, body: &str) {
    #[cfg(target_os = "linux")]
    send_linux(title, body).await;

    #[cfg(target_os = "macos")]
    send_macos(title, body).await;

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        tracing::debug!("Notifications not supported on this platform");
        let _ = (title, body); // Suppress unused warnings
    }
}

/// Send a notification on Linux using notify-send
#[cfg(target_os = "linux")]
async fn send_linux(title: &str, body: &str) {
    let result = Command::new("notify-send")
        .args([
            "--app-name=Voxtype",
            "--expire-time=2000",
            title,
            body,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    if let Err(e) = result {
        tracing::debug!("Failed to send notification: {}", e);
    }
}

/// Send a notification on macOS
/// Prefers terminal-notifier (supports custom icons) with fallback to osascript
#[cfg(target_os = "macos")]
async fn send_macos(title: &str, body: &str) {
    // Try terminal-notifier first (supports custom icons)
    if send_macos_terminal_notifier(title, body).await {
        return;
    }

    // Fallback to osascript
    send_macos_osascript(title, body).await;
}

/// Send notification via terminal-notifier (supports custom icons)
#[cfg(target_os = "macos")]
async fn send_macos_terminal_notifier(title: &str, body: &str) -> bool {
    let mut args = vec![
        "-title".to_string(),
        title.to_string(),
        "-message".to_string(),
        body.to_string(),
        "-group".to_string(),
        "voxtype".to_string(),
    ];

    // Add custom icon if available
    if let Some(icon_path) = find_notification_icon() {
        args.push("-appIcon".to_string());
        args.push(icon_path);
    }

    let result = Command::new("terminal-notifier")
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    match result {
        Ok(status) => status.success(),
        Err(_) => false, // terminal-notifier not available
    }
}

/// Send notification via osascript (fallback, no custom icon support)
#[cfg(target_os = "macos")]
async fn send_macos_osascript(title: &str, body: &str) {
    let escaped_title = title.replace('"', "\\\"");
    let escaped_body = body.replace('"', "\\\"");

    let script = format!(
        r#"display notification "{}" with title "{}""#,
        escaped_body, escaped_title
    );

    let result = Command::new("osascript")
        .args(["-e", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    if let Err(e) = result {
        tracing::debug!("Failed to send notification: {}", e);
    }
}

/// Find the notification icon path (returns file:// URL for terminal-notifier)
#[cfg(target_os = "macos")]
fn find_notification_icon() -> Option<String> {
    // Check common locations for the voxtype icon
    let candidates = [
        // User-installed icon
        dirs::data_dir().map(|d| d.join("voxtype/icon.png")),
        // Config directory
        dirs::config_dir().map(|d| d.join("voxtype/icon.png")),
        // Homebrew installation
        Some(std::path::PathBuf::from("/opt/homebrew/share/voxtype/icon.png")),
        // System-wide
        Some(std::path::PathBuf::from("/usr/local/share/voxtype/icon.png")),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() {
            // Return as file:// URL to handle paths with spaces
            let path_str = candidate.to_string_lossy();
            // URL-encode spaces and special characters
            let encoded = path_str.replace(' ', "%20");
            return Some(format!("file://{}", encoded));
        }
    }

    None
}

/// Send a notification synchronously (blocking).
///
/// Used in non-async contexts like early startup warnings.
pub fn send_sync(title: &str, body: &str) {
    #[cfg(target_os = "linux")]
    send_linux_sync(title, body);

    #[cfg(target_os = "macos")]
    send_macos_sync(title, body);

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (title, body); // Suppress unused warnings
    }
}

/// Send a notification on Linux using notify-send (synchronous)
#[cfg(target_os = "linux")]
fn send_linux_sync(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args([
            "--app-name=Voxtype",
            "--expire-time=5000",
            title,
            body,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Send a notification on macOS (synchronous)
#[cfg(target_os = "macos")]
fn send_macos_sync(title: &str, body: &str) {
    // Try terminal-notifier first
    let mut args = vec!["-title", title, "-message", body, "-group", "voxtype"];

    let icon_path = find_notification_icon();
    if let Some(ref path) = icon_path {
        args.push("-appIcon");
        args.push(path);
    }

    let result = std::process::Command::new("terminal-notifier")
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if result.map(|s| s.success()).unwrap_or(false) {
        return;
    }

    // Fallback to osascript
    let escaped_title = title.replace('"', "\\\"");
    let escaped_body = body.replace('"', "\\\"");

    let script = format!(
        r#"display notification "{}" with title "{}""#,
        escaped_body, escaped_title
    );

    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_escaping() {
        // Test that quotes are properly escaped for AppleScript
        let title = r#"Test "title""#;
        let escaped = title.replace('"', "\\\"");
        assert_eq!(escaped, r#"Test \"title\""#);
    }
}
