//! Platform-specific desktop notifications
//!
//! Provides a unified interface for sending desktop notifications on
//! different platforms:
//! - Linux: Uses notify-send (libnotify)
//! - macOS: Uses terminal-notifier with engine-specific icons

use std::process::Stdio;

#[cfg(target_os = "linux")]
use tokio::process::Command;

use crate::config::TranscriptionEngine;

/// Send a desktop notification with the given title and body.
///
/// This function is async and non-blocking. Notification failures are
/// logged but don't propagate errors (notifications are best-effort).
pub async fn send(title: &str, body: &str) {
    send_with_engine(title, body, None).await;
}

/// Send a desktop notification with optional engine icon.
///
/// On macOS, when an engine is provided, the engine-specific icon is shown
/// as a content image in the notification.
pub async fn send_with_engine(title: &str, body: &str, engine: Option<TranscriptionEngine>) {
    #[cfg(target_os = "linux")]
    {
        let _ = engine; // Linux doesn't use engine icons in notifications
        send_linux(title, body).await;
    }

    #[cfg(target_os = "macos")]
    send_macos_native(title, body, engine);

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        tracing::debug!("Notifications not supported on this platform");
        let _ = (title, body, engine); // Suppress unused warnings
    }
}

/// Send a notification on Linux using notify-send
#[cfg(target_os = "linux")]
async fn send_linux(title: &str, body: &str) {
    let result = Command::new("notify-send")
        .args(["--app-name=Voxtype", "--expire-time=2000", title, body])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    if let Err(e) = result {
        tracing::debug!("Failed to send notification: {}", e);
    }
}

/// Send a macOS notification using terminal-notifier
/// Falls back to osascript if terminal-notifier is not installed
#[cfg(target_os = "macos")]
fn send_macos_native(title: &str, body: &str, engine: Option<TranscriptionEngine>) {
    // Try bundled terminal-notifier first, then system PATH, then osascript
    let bundled_path =
        "/Applications/Voxtype.app/Contents/Resources/terminal-notifier.app/Contents/MacOS/terminal-notifier";

    let notifier_paths = [bundled_path, "terminal-notifier"];

    // Engine-specific content images
    let content_image = engine.and_then(|e| match e {
        TranscriptionEngine::Parakeet => {
            Some("/Applications/Voxtype.app/Contents/Resources/parakeet.png")
        }
        TranscriptionEngine::Whisper => {
            Some("/Applications/Voxtype.app/Contents/Resources/whisper.png")
        }
        TranscriptionEngine::Moonshine
        | TranscriptionEngine::SenseVoice
        | TranscriptionEngine::Paraformer
        | TranscriptionEngine::Dolphin
        | TranscriptionEngine::Omnilingual => None,
    });

    for notifier in notifier_paths {
        let mut cmd = std::process::Command::new(notifier);
        cmd.args([
            "-title",
            title,
            "-message",
            body,
            "-sender",
            "io.voxtype.menubar",
        ]);

        if let Some(image_path) = content_image {
            // Only add content image if the file exists
            if std::path::Path::new(image_path).exists() {
                cmd.args(["-contentImage", image_path]);
            }
        }

        let result = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();

        match result {
            Ok(status) if status.success() => {
                tracing::debug!("Sent notification via {}", notifier);
                return;
            }
            _ => continue,
        }
    }

    // Fallback to osascript
    tracing::debug!("terminal-notifier not available, using osascript");
    send_macos_osascript_sync(title, body);
}

/// Fallback notification via osascript (if native fails)
#[cfg(target_os = "macos")]
fn send_macos_osascript_sync(title: &str, body: &str) {
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

/// Send a notification synchronously (blocking).
///
/// Used in non-async contexts like early startup warnings.
pub fn send_sync(title: &str, body: &str) {
    send_sync_with_engine(title, body, None);
}

/// Send a notification synchronously with optional engine icon.
pub fn send_sync_with_engine(title: &str, body: &str, engine: Option<TranscriptionEngine>) {
    #[cfg(target_os = "linux")]
    {
        let _ = engine;
        send_linux_sync(title, body);
    }

    #[cfg(target_os = "macos")]
    send_macos_native(title, body, engine);

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (title, body, engine); // Suppress unused warnings
    }
}

/// Send a notification on Linux using notify-send (synchronous)
#[cfg(target_os = "linux")]
fn send_linux_sync(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name=Voxtype", "--expire-time=5000", title, body])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_quote_escaping() {
        // Test that quotes are properly escaped for AppleScript
        let title = r#"Test "title""#;
        let escaped = title.replace('"', "\\\"");
        assert_eq!(escaped, r#"Test \"title\""#);
    }
}
