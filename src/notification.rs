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

/// The freedesktop notification ID of the last notification we posted, so the
/// next one can replace it instead of stacking beside it. 0 means "none yet".
///
/// The hints below (`x-canonical-private-synchronous`) only suppress stacking
/// on GNOME and Canonical's notify-osd. KDE's notification daemon ignores them
/// entirely, which is why notifications kept stacking on Plasma long after
/// #345 (see #532). `--replace-id` is the mechanism in the freedesktop
/// specification, and every conforming server honours it.
#[cfg(target_os = "linux")]
static LAST_NOTIFICATION_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Set once `notify-send` has told us it does not understand `--print-id`, so
/// we stop paying for a failed invocation on every notification. libnotify
/// gained `--print-id` in 0.7.9; older builds still get notifications, they
/// just stack.
#[cfg(target_os = "linux")]
static REPLACE_UNSUPPORTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Arguments shared by both Linux paths: identity, lifetime, and the GNOME
/// hints, which stay because they are still what suppresses stacking there.
#[cfg(target_os = "linux")]
fn linux_common_args(expire_ms: &str) -> Vec<String> {
    vec![
        "--app-name=Voxtype".to_string(),
        format!("--expire-time={}", expire_ms),
        "-h".to_string(),
        "string:x-canonical-private-synchronous:voxtype".to_string(),
        "-h".to_string(),
        "int:transient:1".to_string(),
    ]
}

/// `--replace-id <n>` for the notification we last posted, or nothing if we
/// have not posted one yet. A stale ID is harmless: the specification has the
/// server create a new notification when the ID is unknown.
#[cfg(target_os = "linux")]
fn replace_args() -> Vec<String> {
    use std::sync::atomic::Ordering;
    match LAST_NOTIFICATION_ID.load(Ordering::Relaxed) {
        0 => Vec::new(),
        id => vec!["--replace-id".to_string(), id.to_string()],
    }
}

/// Remember the ID `notify-send -p` printed so the next notification can
/// replace this one.
#[cfg(target_os = "linux")]
fn remember_notification_id(stdout: &[u8]) {
    use std::sync::atomic::Ordering;
    if let Ok(id) = String::from_utf8_lossy(stdout).trim().parse::<u32>() {
        LAST_NOTIFICATION_ID.store(id, Ordering::Relaxed);
    }
}

/// Send a notification on Linux using notify-send
#[cfg(target_os = "linux")]
async fn send_linux(title: &str, body: &str) {
    // Synchronous + transient hints ([#345]) keep this in the same
    // overwrite slot as the daemon's recording/transcribing notifications
    // and prevent stacking in the GNOME/Ubuntu notification history.
    use std::sync::atomic::Ordering;

    if !REPLACE_UNSUPPORTED.load(Ordering::Relaxed) {
        let mut args = linux_common_args("2000");
        args.push("--print-id".to_string());
        args.extend(replace_args());
        args.push(title.to_string());
        args.push(body.to_string());

        match Command::new("notify-send")
            .args(&args)
            .stderr(Stdio::null())
            .output()
            .await
        {
            Ok(out) if out.status.success() => {
                remember_notification_id(&out.stdout);
                return;
            }
            Ok(_) => {
                // Almost certainly a libnotify too old for --print-id. Fall
                // through once, then stop trying.
                REPLACE_UNSUPPORTED.store(true, Ordering::Relaxed);
            }
            Err(e) => {
                tracing::debug!("Failed to send notification: {}", e);
                return;
            }
        }
    }

    let mut args = linux_common_args("2000");
    args.push(title.to_string());
    args.push(body.to_string());
    if let Err(e) = Command::new("notify-send")
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
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
        | TranscriptionEngine::Omnilingual
        | TranscriptionEngine::Cohere
        | TranscriptionEngine::Gigaam
        | TranscriptionEngine::Soniox => None,
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
    // Same overwrite-and-transient hints as the async path ([#345]).
    use std::sync::atomic::Ordering;

    if !REPLACE_UNSUPPORTED.load(Ordering::Relaxed) {
        let mut args = linux_common_args("5000");
        args.push("--print-id".to_string());
        args.extend(replace_args());
        args.push(title.to_string());
        args.push(body.to_string());

        // Waits for the child, unlike the old spawn-and-forget, because the
        // printed ID is the whole point. notify-send returns immediately once
        // the server has accepted the notification.
        match std::process::Command::new("notify-send")
            .args(&args)
            .stderr(Stdio::null())
            .output()
        {
            Ok(out) if out.status.success() => {
                remember_notification_id(&out.stdout);
                return;
            }
            Ok(_) => REPLACE_UNSUPPORTED.store(true, Ordering::Relaxed),
            Err(_) => return,
        }
    }

    let mut args = linux_common_args("5000");
    args.push(title.to_string());
    args.push(body.to_string());
    let _ = std::process::Command::new("notify-send")
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// One test for the whole round trip: LAST_NOTIFICATION_ID is a process
    /// global, so splitting these would let them race each other.
    #[test]
    fn replace_id_round_trip() {
        LAST_NOTIFICATION_ID.store(0, Ordering::Relaxed);

        // Nothing posted yet: no --replace-id, so the server assigns a fresh
        // notification rather than being handed an ID it never issued.
        assert!(replace_args().is_empty());

        remember_notification_id(b"778\n");
        assert_eq!(
            replace_args(),
            vec!["--replace-id".to_string(), "778".to_string()]
        );

        // A later notification replaces the newer ID, not the first one.
        remember_notification_id(b"779");
        assert_eq!(
            replace_args(),
            vec!["--replace-id".to_string(), "779".to_string()]
        );

        // Garbage on stdout must not clobber a good ID: better to replace the
        // previous notification than to start stacking again.
        remember_notification_id(b"not an id");
        assert_eq!(
            replace_args(),
            vec!["--replace-id".to_string(), "779".to_string()]
        );

        LAST_NOTIFICATION_ID.store(0, Ordering::Relaxed);
    }

    #[test]
    fn common_args_carry_identity_and_gnome_hints() {
        let args = linux_common_args("2000");
        assert!(args.contains(&"--app-name=Voxtype".to_string()));
        assert!(args.contains(&"--expire-time=2000".to_string()));
        // The GNOME hint stays: it is what suppresses stacking there, while
        // --replace-id is what does it on KDE.
        assert!(args.contains(&"string:x-canonical-private-synchronous:voxtype".to_string()));
    }
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
