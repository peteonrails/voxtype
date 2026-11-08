//! Platform-specific desktop notifications
//!
//! Provides a unified interface for sending desktop notifications on
//! different platforms:
//! - Linux: notify-send (libnotify), falling back to the freedesktop
//!   `org.freedesktop.Notifications` D-Bus interface when notify-send is
//!   missing or fails
//! - macOS: Uses terminal-notifier with engine-specific icons

#[cfg(target_os = "macos")]
use std::process::Stdio;

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
        linux::send(title, body, Lifetime::Millis(2000), None).await;
    }

    #[cfg(target_os = "macos")]
    send_macos_native(title, body, engine);

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        tracing::debug!("Notifications not supported on this platform");
        let _ = (title, body, engine); // Suppress unused warnings
    }
}

/// Linux delivery: notify-send first, the notification server over D-Bus
/// second.
///
/// notify-send stays first so nothing changes for anyone whose notifications
/// already work, including users who put their own `notify-send` wrapper on
/// PATH. D-Bus is what notify-send itself speaks, so the fallback produces the
/// same notification (app name, urgency, hints, expiry, replace-id) for users
/// without libnotify installed. Only when both fail is the notification lost,
/// and then the daemon log says so once, at warn level, with the fix.
#[cfg(target_os = "linux")]
mod linux {
    use super::Lifetime;
    use std::collections::HashMap;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::Duration;

    const BUS_NAME: &str = "org.freedesktop.Notifications";
    const OBJECT_PATH: &str = "/org/freedesktop/Notifications";
    const INTERFACE: &str = "org.freedesktop.Notifications";
    const APP_NAME: &str = "Voxtype";

    /// Upper bound on a D-Bus call to the notification server. A healthy
    /// server answers in milliseconds; this only stops a wedged one from
    /// stalling the daemon's event loop.
    const DBUS_TIMEOUT: Duration = Duration::from_secs(5);

    /// What we know about delivering notifications in this process.
    pub(super) struct Delivery {
        /// The freedesktop notification ID of the last notification we
        /// posted, so the next one can replace it instead of stacking beside
        /// it. 0 means "none yet".
        ///
        /// The `x-canonical-private-synchronous` hint only suppresses stacking
        /// on GNOME and Canonical's notify-osd. KDE's notification daemon
        /// ignores it entirely, which is why notifications kept stacking on
        /// Plasma long after #345 (see #532). The replace ID is the mechanism
        /// in the freedesktop specification, and every conforming server
        /// honours it.
        last_id: AtomicU32,
        /// Whether the notification currently occupying the slot was posted
        /// with `Lifetime::UntilClosed`. Only such a notification is closed by
        /// `close_persistent`; a timed one is already on its way out, and
        /// closing it early would cut short a message the user is still
        /// reading.
        persistent_active: AtomicBool,
        /// Set once `notify-send` has told us it does not understand
        /// `--print-id`, so we stop paying for a failed invocation on every
        /// notification. libnotify gained `--print-id` in 0.7.9; older builds
        /// still get notifications, they just stack.
        replace_unsupported: AtomicBool,
        /// Set once spawning `notify-send` failed with "not found", so every
        /// later notification goes straight to D-Bus.
        notify_send_missing: AtomicBool,
        /// Set once we have logged that notifications cannot be delivered, so
        /// a missing notification server costs one warning, not one per alert.
        warned_undeliverable: AtomicBool,
    }

    impl Delivery {
        pub(super) const fn new() -> Self {
            Self {
                last_id: AtomicU32::new(0),
                persistent_active: AtomicBool::new(false),
                replace_unsupported: AtomicBool::new(false),
                notify_send_missing: AtomicBool::new(false),
                warned_undeliverable: AtomicBool::new(false),
            }
        }

        /// Remember the ID the server assigned so the next notification can
        /// replace this one, and whether that notification needs closing to
        /// go away.
        fn remember(&self, id: u32, lifetime: Lifetime) {
            self.last_id.store(id, Ordering::Relaxed);
            self.persistent_active
                .store(lifetime == Lifetime::UntilClosed, Ordering::Relaxed);
        }

        /// `--replace-id <n>` for the notification we last posted, or nothing
        /// if we have not posted one yet. A stale ID is harmless: the
        /// specification has the server create a new notification when the ID
        /// is unknown.
        fn replace_args(&self) -> Vec<String> {
            match self.last_id.load(Ordering::Relaxed) {
                0 => Vec::new(),
                id => vec!["--replace-id".to_string(), id.to_string()],
            }
        }

        /// Log, once per process, that a notification could not be delivered
        /// by any route. Returns whether this call logged.
        fn report_undeliverable(&self, notify_send: &str, dbus: &str) -> bool {
            if self.warned_undeliverable.swap(true, Ordering::Relaxed) {
                tracing::debug!(
                    "Notification not delivered (notify-send: {}; D-Bus: {})",
                    notify_send,
                    dbus
                );
                return false;
            }
            tracing::warn!("{}", undeliverable_message(notify_send, dbus));
            true
        }
    }

    pub(super) static DELIVERY: Delivery = Delivery::new();

    /// The warning logged when no route reaches a notification server.
    pub(super) fn undeliverable_message(notify_send: &str, dbus: &str) -> String {
        format!(
            "Desktop notifications cannot be delivered, so Voxtype alerts will only \
             appear in this log (notify-send: {}; D-Bus {}: {}). Install and start a \
             notification daemon such as mako, swaync or dunst (GNOME and KDE ship \
             their own), and make sure the session bus is reachable \
             (DBUS_SESSION_BUS_ADDRESS). For notify-send, install libnotify \
             (Arch: libnotify, Debian/Ubuntu: libnotify-bin, Fedora: libnotify). \
             Test with: notify-send Test Hello",
            notify_send, BUS_NAME, dbus
        )
    }

    /// One notification, as every route receives it.
    pub(super) struct Message<'a> {
        pub title: &'a str,
        pub body: &'a str,
        pub lifetime: Lifetime,
        pub urgency: Option<&'a str>,
    }

    /// The result of one `notify-send` invocation.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) enum Attempt {
        /// Exited successfully; carries the ID if `--print-id` printed one.
        Delivered(Option<u32>),
        /// Ran but exited with an error: an unknown flag, or no server.
        Rejected,
        /// Not installed.
        Missing,
        /// Could not be started for another reason.
        Failed(String),
    }

    /// Which route delivered a notification.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Route {
        NotifySend,
        NotifySendWithoutId,
        DBus,
    }

    /// The two ways to reach a notification server. A trait so the fallback
    /// order can be tested without a server or a notify-send binary.
    pub(super) trait Transport {
        async fn notify_send(&self, args: Vec<String>) -> Attempt;
        async fn dbus_notify(&self, msg: &Message<'_>, replaces_id: u32) -> Result<u32, String>;
    }

    /// Arguments shared by both notify-send invocations: identity, lifetime,
    /// urgency, and the GNOME hints, which stay because they are still what
    /// suppresses stacking there.
    pub(super) fn common_args(lifetime: Lifetime, urgency: Option<&str>) -> Vec<String> {
        let mut args = vec![
            format!("--app-name={}", APP_NAME),
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

    /// The D-Bus counterpart of `common_args`' hints, typed the way
    /// notify-send sends them: `int:` is an `i32`, urgency is a byte.
    pub(super) fn dbus_hints(
        urgency: Option<&str>,
    ) -> HashMap<&'static str, zbus::zvariant::Value<'static>> {
        use zbus::zvariant::Value;
        let mut hints = HashMap::new();
        hints.insert(
            "x-canonical-private-synchronous",
            Value::from("voxtype".to_string()),
        );
        hints.insert("transient", Value::I32(1));
        if let Some(urgency) = urgency {
            let level: u8 = match crate::output::sanitize_urgency(urgency) {
                "low" => 0,
                "critical" => 2,
                _ => 1,
            };
            hints.insert("urgency", Value::U8(level));
        }
        hints
    }

    fn parse_id(stdout: &[u8]) -> Option<u32> {
        String::from_utf8_lossy(stdout).trim().parse::<u32>().ok()
    }

    /// Deliver one notification, trying notify-send (with `--print-id`, then
    /// without) and then D-Bus. Returns the route that worked, or `None` after
    /// logging that nothing did.
    pub(super) async fn deliver<T: Transport>(
        transport: &T,
        state: &Delivery,
        msg: &Message<'_>,
    ) -> Option<Route> {
        let notify_send_error = if state.notify_send_missing.load(Ordering::Relaxed) {
            "not installed".to_string()
        } else {
            match try_notify_send(transport, state, msg).await {
                Ok(route) => return Some(route),
                Err(e) => e,
            }
        };

        match transport
            .dbus_notify(msg, state.last_id.load(Ordering::Relaxed))
            .await
        {
            Ok(id) => {
                state.remember(id, msg.lifetime);
                Some(Route::DBus)
            }
            Err(dbus_error) => {
                state.report_undeliverable(&notify_send_error, &dbus_error);
                None
            }
        }
    }

    /// The notify-send half of `deliver`. On failure, returns a description
    /// of what went wrong for the warning.
    async fn try_notify_send<T: Transport>(
        transport: &T,
        state: &Delivery,
        msg: &Message<'_>,
    ) -> Result<Route, String> {
        let tried_print_id = !state.replace_unsupported.load(Ordering::Relaxed);

        if tried_print_id {
            let mut args = common_args(msg.lifetime, msg.urgency);
            args.push("--print-id".to_string());
            args.extend(state.replace_args());
            args.push(msg.title.to_string());
            args.push(msg.body.to_string());

            match transport.notify_send(args).await {
                Attempt::Delivered(id) => {
                    // Garbage on stdout must not clobber a good ID: better to
                    // replace the previous notification than to start
                    // stacking again.
                    if let Some(id) = id {
                        state.remember(id, msg.lifetime);
                    }
                    return Ok(Route::NotifySend);
                }
                Attempt::Missing => return Err(mark_missing(state)),
                // Either a libnotify too old for --print-id, or no server.
                // The plain invocation below tells the two apart.
                Attempt::Rejected | Attempt::Failed(_) => {}
            }
        }

        // Without --print-id we never learn the ID, so nothing posted here
        // may outlive its timeout.
        let mut args = common_args(msg.lifetime.bounded(2000), msg.urgency);
        args.push(msg.title.to_string());
        args.push(msg.body.to_string());

        match transport.notify_send(args).await {
            Attempt::Delivered(_) => {
                if tried_print_id {
                    // The plain call worked where --print-id did not, so it
                    // was the flag. Stop trying it.
                    state.replace_unsupported.store(true, Ordering::Relaxed);
                }
                Ok(Route::NotifySendWithoutId)
            }
            Attempt::Missing => Err(mark_missing(state)),
            Attempt::Rejected => Err("exited with an error".to_string()),
            Attempt::Failed(e) => Err(e),
        }
    }

    fn mark_missing(state: &Delivery) -> String {
        if !state.notify_send_missing.swap(true, Ordering::Relaxed) {
            tracing::debug!("notify-send not found; delivering notifications over D-Bus instead");
        }
        "not installed".to_string()
    }

    /// The real transports: the `notify-send` binary and the session bus.
    pub(super) struct System;

    impl Transport for System {
        async fn notify_send(&self, args: Vec<String>) -> Attempt {
            match tokio::process::Command::new("notify-send")
                .args(&args)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .await
            {
                Ok(out) if out.status.success() => Attempt::Delivered(parse_id(&out.stdout)),
                Ok(_) => Attempt::Rejected,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Attempt::Missing,
                Err(e) => Attempt::Failed(e.to_string()),
            }
        }

        async fn dbus_notify(&self, msg: &Message<'_>, replaces_id: u32) -> Result<u32, String> {
            match tokio::time::timeout(DBUS_TIMEOUT, dbus_notify(msg, replaces_id)).await {
                Ok(result) => result.map_err(|e| e.to_string()),
                Err(_) => Err("timed out".to_string()),
            }
        }
    }

    /// `Notify` on the session bus, exactly as notify-send would send it.
    async fn dbus_notify(msg: &Message<'_>, replaces_id: u32) -> zbus::Result<u32> {
        let conn = zbus::Connection::session().await?;
        let expire = i32::try_from(msg.lifetime.expire_ms()).unwrap_or(i32::MAX);
        let actions: Vec<&str> = Vec::new();
        let reply = conn
            .call_method(
                Some(BUS_NAME),
                OBJECT_PATH,
                Some(INTERFACE),
                "Notify",
                &(
                    APP_NAME,
                    replaces_id,
                    "",
                    msg.title,
                    msg.body,
                    actions,
                    dbus_hints(msg.urgency),
                    expire,
                ),
            )
            .await?;
        reply.body().deserialize::<u32>()
    }

    /// Deliver through the real transports with the process-wide state.
    pub(super) async fn send(title: &str, body: &str, lifetime: Lifetime, urgency: Option<&str>) {
        let msg = Message {
            title,
            body,
            lifetime,
            urgency,
        };
        deliver(&System, &DELIVERY, &msg).await;
    }

    /// Blocking counterpart of `send`, for callers without a runtime (or
    /// inside one but in synchronous code, such as daemon startup checks).
    ///
    /// Runs the same async delivery on a private current-thread runtime in a
    /// scoped thread: blocking on a runtime from inside another one panics,
    /// and the thread keeps both cases safe. notify-send returns as soon as
    /// the server accepts the notification, so this waits milliseconds.
    pub(super) fn send_blocking(
        title: &str,
        body: &str,
        lifetime: Lifetime,
        urgency: Option<&str>,
    ) {
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(send(title, body, lifetime, urgency)),
                    Err(e) => tracing::debug!("No runtime to send notification: {}", e),
                }
            });
            let _ = worker.join();
        });
    }

    /// Close the notification in the slot if it was posted until closed.
    pub(super) async fn close_persistent() {
        if !DELIVERY.persistent_active.swap(false, Ordering::Relaxed) {
            return;
        }
        let id = DELIVERY.last_id.swap(0, Ordering::Relaxed);
        if id == 0 {
            return;
        }

        let close = async {
            let conn = zbus::Connection::session().await?;
            conn.call_method(
                Some(BUS_NAME),
                OBJECT_PATH,
                Some(INTERFACE),
                "CloseNotification",
                &(id,),
            )
            .await
            .map(|_| ())
        };
        match tokio::time::timeout(DBUS_TIMEOUT, close).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::debug!("Failed to close notification {}: {}", id, e),
            Err(_) => tracing::debug!("Timed out closing notification {}", id),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::Mutex;

        /// A scripted transport: each notify-send call pops the next result,
        /// and the D-Bus result is fixed. Records what was asked of it.
        struct Fake {
            notify_send: Mutex<Vec<Attempt>>,
            dbus: Result<u32, String>,
            calls: Mutex<Vec<String>>,
        }

        impl Fake {
            fn new(notify_send: Vec<Attempt>, dbus: Result<u32, String>) -> Self {
                Self {
                    notify_send: Mutex::new(notify_send),
                    dbus,
                    calls: Mutex::new(Vec::new()),
                }
            }

            fn calls(&self) -> Vec<String> {
                self.calls.lock().unwrap().clone()
            }
        }

        impl Transport for Fake {
            async fn notify_send(&self, args: Vec<String>) -> Attempt {
                let label = if args.contains(&"--print-id".to_string()) {
                    "notify-send -p"
                } else {
                    "notify-send"
                };
                self.calls.lock().unwrap().push(label.to_string());
                let mut queue = self.notify_send.lock().unwrap();
                assert!(!queue.is_empty(), "unexpected notify-send call");
                queue.remove(0)
            }

            async fn dbus_notify(
                &self,
                _msg: &Message<'_>,
                replaces_id: u32,
            ) -> Result<u32, String> {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("dbus replaces={}", replaces_id));
                self.dbus.clone()
            }
        }

        fn msg(lifetime: Lifetime) -> Message<'static> {
            Message {
                title: "Voxtype",
                body: "body",
                lifetime,
                urgency: Some("normal"),
            }
        }

        #[tokio::test]
        async fn working_notify_send_is_the_only_route_used() {
            let state = Delivery::new();
            let fake = Fake::new(vec![Attempt::Delivered(Some(42))], Ok(7));
            let route = deliver(&fake, &state, &msg(Lifetime::Millis(2000))).await;
            assert_eq!(route, Some(Route::NotifySend));
            assert_eq!(fake.calls(), vec!["notify-send -p"]);
            assert_eq!(state.last_id.load(Ordering::Relaxed), 42);
        }

        #[tokio::test]
        async fn missing_notify_send_falls_back_to_dbus_and_is_remembered() {
            let state = Delivery::new();
            let fake = Fake::new(vec![Attempt::Missing], Ok(9));
            let route = deliver(&fake, &state, &msg(Lifetime::UntilClosed)).await;
            assert_eq!(route, Some(Route::DBus));
            assert_eq!(fake.calls(), vec!["notify-send -p", "dbus replaces=0"]);
            // The D-Bus ID feeds replace-id and the close path just like
            // notify-send's does.
            assert_eq!(state.last_id.load(Ordering::Relaxed), 9);
            assert!(state.persistent_active.load(Ordering::Relaxed));

            // The next notification skips the spawn and replaces the first.
            let fake = Fake::new(vec![], Ok(10));
            let route = deliver(&fake, &state, &msg(Lifetime::Millis(2000))).await;
            assert_eq!(route, Some(Route::DBus));
            assert_eq!(fake.calls(), vec!["dbus replaces=9"]);
            assert!(!state.persistent_active.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn old_libnotify_drops_print_id_once() {
            let state = Delivery::new();
            let fake = Fake::new(vec![Attempt::Rejected, Attempt::Delivered(None)], Ok(1));
            let route = deliver(&fake, &state, &msg(Lifetime::Millis(2000))).await;
            assert_eq!(route, Some(Route::NotifySendWithoutId));
            assert_eq!(fake.calls(), vec!["notify-send -p", "notify-send"]);
            assert!(state.replace_unsupported.load(Ordering::Relaxed));

            // From then on, one plain invocation per notification.
            let fake = Fake::new(vec![Attempt::Delivered(None)], Ok(1));
            deliver(&fake, &state, &msg(Lifetime::Millis(2000))).await;
            assert_eq!(fake.calls(), vec!["notify-send"]);
        }

        #[tokio::test]
        async fn failing_notify_send_falls_back_to_dbus_without_blaming_the_flag() {
            // No server for notify-send to reach, but the bus call works (for
            // instance a notify-send wrapper that is broken). Both
            // invocations failing must not disable --print-id: that would
            // lose replace-id for good once the server comes back.
            let state = Delivery::new();
            let fake = Fake::new(vec![Attempt::Rejected, Attempt::Rejected], Ok(5));
            let route = deliver(&fake, &state, &msg(Lifetime::Millis(2000))).await;
            assert_eq!(route, Some(Route::DBus));
            assert_eq!(
                fake.calls(),
                vec!["notify-send -p", "notify-send", "dbus replaces=0"]
            );
            assert!(!state.replace_unsupported.load(Ordering::Relaxed));
            assert!(!state.notify_send_missing.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn nothing_reachable_warns_once() {
            let state = Delivery::new();
            let fake = Fake::new(vec![Attempt::Missing], Err("ServiceUnknown".to_string()));
            assert_eq!(
                deliver(&fake, &state, &msg(Lifetime::Millis(2000))).await,
                None
            );
            assert!(state.warned_undeliverable.load(Ordering::Relaxed));
            // Already warned: a second failure only logs at debug.
            assert!(!state.report_undeliverable("not installed", "ServiceUnknown"));
        }

        #[test]
        fn undeliverable_message_names_the_fix() {
            let text = undeliverable_message("not installed", "ServiceUnknown");
            assert!(text.contains("not installed"));
            assert!(text.contains("ServiceUnknown"));
            assert!(text.contains("mako"));
            assert!(text.contains("libnotify-bin"));
        }

        #[test]
        fn replace_args_follow_the_last_id() {
            let state = Delivery::new();
            // Nothing posted yet: no --replace-id, so the server assigns a
            // fresh notification rather than being handed an ID it never
            // issued.
            assert!(state.replace_args().is_empty());
            state.remember(778, Lifetime::Millis(2000));
            assert_eq!(
                state.replace_args(),
                vec!["--replace-id".to_string(), "778".to_string()]
            );
            assert_eq!(parse_id(b"779\n"), Some(779));
            assert_eq!(parse_id(b"not an id"), None);
        }

        #[test]
        fn common_args_carry_identity_and_gnome_hints() {
            let args = common_args(Lifetime::Millis(2000), None);
            assert!(args.contains(&"--app-name=Voxtype".to_string()));
            assert!(args.contains(&"--expire-time=2000".to_string()));
            // The GNOME hint stays: it is what suppresses stacking there,
            // while --replace-id is what does it on KDE.
            assert!(args.contains(&"string:x-canonical-private-synchronous:voxtype".to_string()));
            assert!(!args.iter().any(|a| a.starts_with("--urgency=")));
        }

        #[test]
        fn common_args_carry_sanitized_urgency() {
            let args = common_args(Lifetime::Millis(2000), Some("critical"));
            assert!(args.contains(&"--urgency=critical".to_string()));
            let args = common_args(Lifetime::Millis(2000), Some("nonsense"));
            assert!(args.contains(&"--urgency=normal".to_string()));
        }

        #[test]
        fn dbus_hints_match_notify_send() {
            use zbus::zvariant::Value;
            let hints = dbus_hints(Some("critical"));
            assert_eq!(hints["urgency"], Value::U8(2));
            assert_eq!(hints["transient"], Value::I32(1));
            assert_eq!(
                hints["x-canonical-private-synchronous"],
                Value::from("voxtype".to_string())
            );
            assert_eq!(dbus_hints(Some("low"))["urgency"], Value::U8(0));
            assert_eq!(dbus_hints(Some("nonsense"))["urgency"], Value::U8(1));
            assert!(!dbus_hints(None).contains_key("urgency"));
        }

        #[test]
        fn until_closed_never_expires() {
            // 0 is the freedesktop value for "no timeout".
            let args = common_args(Lifetime::UntilClosed, None);
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
    linux::send(title, body, lifetime, Some(urgency)).await;
}

/// No-op off Linux: the macOS output drivers post their own completion
/// notifications through terminal-notifier (see `output/cgevent.rs`), and
/// there is no freedesktop server to talk to.
#[cfg(not(target_os = "linux"))]
pub async fn send_status(title: &str, body: &str, urgency: &str, lifetime: Lifetime) {
    let _ = (title, body, urgency, lifetime);
}

/// Synchronous counterpart to `send_status`, for callers without a tokio
/// runtime (the TUI's detached model-download worker thread). Same single
/// notification slot, same urgency handling, always a timed lifetime: a
/// caller with no runtime has no daemon event loop that would ever close a
/// persistent banner.
pub fn send_status_sync(title: &str, body: &str, urgency: &str) {
    #[cfg(target_os = "linux")]
    linux::send_blocking(title, body, Lifetime::Millis(5000), Some(urgency));

    #[cfg(not(target_os = "linux"))]
    let _ = (title, body, urgency);
}

/// Close the notification posted with `Lifetime::UntilClosed`, if one is still
/// up. Does nothing when the slot holds a timed notification.
///
/// `notify-send` cannot close a notification, so this goes to the server
/// directly. Best effort: if the call fails the banner stays until the next
/// notification replaces it.
#[cfg(target_os = "linux")]
pub async fn close_persistent() {
    linux::close_persistent().await;
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
        linux::send_blocking(title, body, Lifetime::Millis(5000), None);
    }

    #[cfg(target_os = "macos")]
    send_macos_native(title, body, engine);

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (title, body, engine); // Suppress unused warnings
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
