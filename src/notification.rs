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

/// How long the notification server keeps a notification on screen.
///
/// Most notifications are a message: once it has been on screen for a moment
/// it has done its job. A few describe state instead, and state outlives a
/// banner. A "recording" popup that disappears after two seconds tells the
/// user the microphone is off while it is still live, so those stay up until
/// something takes them down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifetime {
    /// Let the server expire the notification after this many milliseconds.
    Millis(u32),
    /// Keep it up until it is replaced or closed. Requires a known
    /// notification ID, since closing it is the only way it ever goes away.
    UntilClosed,
}

#[cfg(target_os = "linux")]
impl Lifetime {
    /// The freedesktop `expire-timeout` for this lifetime. 0 is the
    /// specification's "never expire".
    fn expire_ms(self) -> u32 {
        match self {
            Lifetime::Millis(ms) => ms,
            Lifetime::UntilClosed => 0,
        }
    }

    /// Downgrade to a timed lifetime for the paths where we cannot learn the
    /// notification's ID. Nothing may stay up that we have no way to take
    /// down: an unclosable recording banner would outlive the recording, and
    /// every later one would pile up behind it.
    fn bounded(self, fallback_ms: u32) -> Self {
        match self {
            Lifetime::UntilClosed => Lifetime::Millis(fallback_ms),
            timed => timed,
        }
    }
}

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
        send_linux(title, body, Lifetime::Millis(2000), None).await;
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

/// Whether the notification currently occupying the slot was posted with
/// `Lifetime::UntilClosed`. Only such a notification is closed by
/// `close_persistent`; a timed one is already on its way out, and closing it
/// early would cut short a message the user is still reading.
#[cfg(target_os = "linux")]
static PERSISTENT_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Arguments shared by both Linux paths: identity, lifetime, urgency, and the
/// GNOME hints, which stay because they are still what suppresses stacking
/// there.
#[cfg(target_os = "linux")]
fn linux_common_args(lifetime: Lifetime, urgency: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--app-name=Voxtype".to_string(),
        format!("--expire-time={}", lifetime.expire_ms()),
        "-h".to_string(),
        "string:x-canonical-private-synchronous:voxtype".to_string(),
        "-h".to_string(),
        "int:transient:1".to_string(),
    ];
    if let Some(urgency) = urgency {
        args.push(format!(
            "--urgency={}",
            crate::output::sanitize_urgency(urgency)
        ));
    }
    args
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
/// replace this one, and whether that notification needs closing to go away.
#[cfg(target_os = "linux")]
fn remember_notification_id(stdout: &[u8], lifetime: Lifetime) {
    use std::sync::atomic::Ordering;
    if let Ok(id) = String::from_utf8_lossy(stdout).trim().parse::<u32>() {
        LAST_NOTIFICATION_ID.store(id, Ordering::Relaxed);
        PERSISTENT_ACTIVE.store(lifetime == Lifetime::UntilClosed, Ordering::Relaxed);
    }
}

/// Send a notification on Linux using notify-send
#[cfg(target_os = "linux")]
async fn send_linux(title: &str, body: &str, lifetime: Lifetime, urgency: Option<&str>) {
    // Synchronous + transient hints ([#345]) keep this in the same
    // overwrite slot as the daemon's recording/transcribing notifications
    // and prevent stacking in the GNOME/Ubuntu notification history.
    use std::sync::atomic::Ordering;

    if !REPLACE_UNSUPPORTED.load(Ordering::Relaxed) {
        let mut args = linux_common_args(lifetime, urgency);
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
                remember_notification_id(&out.stdout, lifetime);
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

    let mut args = linux_common_args(lifetime.bounded(2000), urgency);
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

/// Send a status notification: the same single-slot behaviour as `send`, plus
/// the configured urgency and a caller-chosen lifetime.
///
/// Every Voxtype notification on Linux belongs here. A caller that shells out
/// to `notify-send` itself gets a fresh ID from the server and stacks beside
/// the previous notification instead of replacing it, which is how the daemon
/// kept stacking on KDE long after the fix in #532.
#[cfg(target_os = "linux")]
pub async fn send_status(title: &str, body: &str, urgency: &str, lifetime: Lifetime) {
    send_linux(title, body, lifetime, Some(urgency)).await;
}

/// No-op off Linux: the macOS output drivers post their own completion
/// notifications through terminal-notifier (see `output/cgevent.rs`), and
/// there is no freedesktop server to talk to.
#[cfg(not(target_os = "linux"))]
pub async fn send_status(title: &str, body: &str, urgency: &str, lifetime: Lifetime) {
    let _ = (title, body, urgency, lifetime);
}

/// Close the notification posted with `Lifetime::UntilClosed`, if one is still
/// up. Does nothing when the slot holds a timed notification.
///
/// `notify-send` cannot close a notification, so this goes to the server
/// directly. Best effort: if the call fails the banner stays until the next
/// notification replaces it.
#[cfg(target_os = "linux")]
pub async fn close_persistent() {
    use std::sync::atomic::Ordering;

    if !PERSISTENT_ACTIVE.swap(false, Ordering::Relaxed) {
        return;
    }
    let id = LAST_NOTIFICATION_ID.swap(0, Ordering::Relaxed);
    if id == 0 {
        return;
    }

    let conn = match zbus::Connection::session().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::debug!("No session bus to close notification {}: {}", id, e);
            return;
        }
    };
    if let Err(e) = conn
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "CloseNotification",
            &(id,),
        )
        .await
    {
        tracing::debug!("Failed to close notification {}: {}", id, e);
    }
}

/// No-op off Linux, where nothing posts a notification without an expiry.
#[cfg(not(target_os = "linux"))]
pub async fn close_persistent() {}

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
        | TranscriptionEngine::Soniox
        | TranscriptionEngine::OpenVino => None,
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
        let mut args = linux_common_args(Lifetime::Millis(5000), None);
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
                remember_notification_id(&out.stdout, Lifetime::Millis(5000));
                return;
            }
            Ok(_) => REPLACE_UNSUPPORTED.store(true, Ordering::Relaxed),
            Err(_) => return,
        }
    }

    let mut args = linux_common_args(Lifetime::Millis(5000), None);
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

        remember_notification_id(b"778\n", Lifetime::Millis(2000));
        assert_eq!(
            replace_args(),
            vec!["--replace-id".to_string(), "778".to_string()]
        );

        // A later notification replaces the newer ID, not the first one.
        remember_notification_id(b"779", Lifetime::Millis(2000));
        assert_eq!(
            replace_args(),
            vec!["--replace-id".to_string(), "779".to_string()]
        );

        // Garbage on stdout must not clobber a good ID: better to replace the
        // previous notification than to start stacking again.
        remember_notification_id(b"not an id", Lifetime::Millis(2000));
        assert_eq!(
            replace_args(),
            vec!["--replace-id".to_string(), "779".to_string()]
        );

        // The close path keys off the lifetime of the notification in the
        // slot. A timed one must never arm it: closing one early would cut
        // short a message the user is still reading.
        remember_notification_id(b"901", Lifetime::UntilClosed);
        assert!(PERSISTENT_ACTIVE.load(Ordering::Relaxed));
        remember_notification_id(b"902", Lifetime::Millis(2000));
        assert!(!PERSISTENT_ACTIVE.load(Ordering::Relaxed));

        LAST_NOTIFICATION_ID.store(0, Ordering::Relaxed);
    }

    #[test]
    fn common_args_carry_identity_and_gnome_hints() {
        let args = linux_common_args(Lifetime::Millis(2000), None);
        assert!(args.contains(&"--app-name=Voxtype".to_string()));
        assert!(args.contains(&"--expire-time=2000".to_string()));
        // The GNOME hint stays: it is what suppresses stacking there, while
        // --replace-id is what does it on KDE.
        assert!(args.contains(&"string:x-canonical-private-synchronous:voxtype".to_string()));
        // No urgency unless the caller asked for one, so the server keeps its
        // own default rather than being pinned to "normal".
        assert!(!args.iter().any(|a| a.starts_with("--urgency=")));
    }

    #[test]
    fn common_args_carry_sanitized_urgency() {
        let args = linux_common_args(Lifetime::Millis(2000), Some("critical"));
        assert!(args.contains(&"--urgency=critical".to_string()));

        let args = linux_common_args(Lifetime::Millis(2000), Some("nonsense"));
        assert!(args.contains(&"--urgency=normal".to_string()));
    }

    #[test]
    fn until_closed_never_expires() {
        // 0 is the freedesktop value for "no timeout".
        let args = linux_common_args(Lifetime::UntilClosed, None);
        assert!(args.contains(&"--expire-time=0".to_string()));
    }

    #[test]
    fn bounded_downgrades_only_the_unbounded_lifetime() {
        // Without --print-id we never learn the ID, so an unbounded
        // notification could never be closed again.
        assert_eq!(Lifetime::UntilClosed.bounded(2000), Lifetime::Millis(2000));
        assert_eq!(Lifetime::Millis(3000).bounded(2000), Lifetime::Millis(3000));
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
