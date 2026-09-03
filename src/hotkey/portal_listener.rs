//! XDG GlobalShortcuts portal listener.

use super::evdev_listener::parse_key_name;
use super::{HotkeyEvent, HotkeyListener};
use crate::config::{ActivationMode, HotkeyConfig};
use crate::error::HotkeyError;
use async_trait::async_trait;
use evdev::KeyCode;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str};
use zbus::{Connection, Proxy};

const APP_ID: &str = "io.voxtype.Voxtype";
const PORTAL_SERVICE: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REGISTRY_INTERFACE: &str = "org.freedesktop.host.portal.Registry";
const SHORTCUTS_INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
const SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";

const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(500);
const MAXIMUM_RETRY_DELAY: Duration = Duration::from_secs(60);

const MINIMUM_STABLE_SESSION: Duration = Duration::from_secs(30);

/// How soon after an activation another activation of the same shortcut counts
/// as keyboard auto-repeat rather than a new press, when no `Deactivated`
/// arrived in between.
///
/// GNOME's backend emits `Activated` for every keyboard auto-repeat, so
/// holding the key produces a stream of activations. GNOME repeats after a
/// 500 ms delay and then every 30 ms, so the window has to clear the delay as
/// well as the interval. A real second press is preceded by a release, and the
/// release resets the filter, so on a backend that reports releases the window
/// never drops a deliberate press. It still drops the activations a backend
/// that omits `Deactivated` would deliver, where repeats and new presses are
/// indistinguishable.
const ACTIVATION_REPEAT_WINDOW: Duration = Duration::from_millis(600);

const ID_DIGEST_LENGTH: usize = 8;

type VariantMap = HashMap<String, OwnedValue>;
type ShortcutList = Vec<(String, VariantMap)>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PortalAction {
    id: String,
    description: String,
    preferred_trigger: Option<String>,
    event: HotkeyEvent,
    emits_release: bool,
}

impl PortalAction {
    fn shortcut(&self) -> (String, VariantMap) {
        let mut properties = VariantMap::new();
        properties.insert(
            "description".to_string(),
            OwnedValue::from(Str::from(self.description.clone())),
        );
        if let Some(trigger) = &self.preferred_trigger {
            properties.insert(
                "preferred_trigger".to_string(),
                OwnedValue::from(Str::from(trigger.clone())),
            );
        }

        (self.id.clone(), properties)
    }
}

/// What the listener does when it stops for a reason that reconnecting cannot
/// fix, such as the user dismissing the desktop's binding dialog.
pub(crate) enum OnPermanentFailure {
    /// Log the error and send a desktop notification.
    NotifyUser,
    /// Send the error to the auto listener, which decides whether to start
    /// evdev instead.
    Delegate(oneshot::Sender<HotkeyError>),
}

impl OnPermanentFailure {
    async fn report(self, error: HotkeyError) {
        match self {
            Self::NotifyUser => notify_permanent_failure(&error).await,
            Self::Delegate(sender) => {
                let _ = sender.send(error);
            }
        }
    }
}

/// Reports to the user that global shortcuts have stopped working.
pub(crate) async fn notify_permanent_failure(error: &HotkeyError) {
    tracing::error!("Global shortcuts are no longer available: {}", error);
    crate::notification::send("Voxtype: global shortcuts unavailable", &error.to_string()).await;
}

/// An established portal session and the streams the listener reads from.
/// Dropping it drops the D-Bus connection, which ends the session.
struct OpenSession {
    connection: Connection,
    session_path: OwnedObjectPath,
    /// `Activated`, `Deactivated` and `ShortcutsChanged` on one stream, so a
    /// release is always processed before the press that follows it. Separate
    /// per-signal streams would let `select!` take a queued activation ahead
    /// of the earlier release that should reset the repeat filter.
    shortcut_signals: zbus::proxy::SignalStream<'static>,
    closed: zbus::proxy::SignalStream<'static>,
    owner_changed: zbus::fdo::NameOwnerChangedStream<'static>,
    /// The trigger last logged for each shortcut. Desktops repeat their whole
    /// shortcut list in `ShortcutsChanged`, including at startup, so without
    /// this every shortcut is logged again each time.
    logged_triggers: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionEnd {
    Stop,
    Reconnect,
}

/// Separates the activations a user made from the ones keyboard auto-repeat
/// produced, by the gap between consecutive activations of the same shortcut.
///
/// The gap is measured from the last activation seen, including ones this
/// filter dropped, so a key held down stays quiet however long it is held.
/// A `Deactivated` resets the filter through [`Self::release`], so a press
/// after a reported release is accepted however soon it follows.
#[derive(Debug, Default)]
struct ActivationFilter<'a> {
    last_seen: HashMap<&'a str, Instant>,
}

impl<'a> ActivationFilter<'a> {
    /// Whether to act on this activation of `action_id`.
    fn accept(&mut self, action_id: &'a str, now: Instant) -> bool {
        let Some(previous) = self.last_seen.insert(action_id, now) else {
            return true;
        };
        let gap = now.duration_since(previous);
        if gap >= ACTIVATION_REPEAT_WINDOW {
            return true;
        }

        tracing::debug!(
            "Ignoring portal shortcut '{}' activated {} ms after the last activation",
            action_id,
            gap.as_millis()
        );
        false
    }

    /// Forget the shortcut's last activation. The desktop reported the key
    /// released, so the next activation cannot be auto-repeat.
    fn release(&mut self, action_id: &str) {
        self.last_seen.remove(action_id);
    }
}

/// Delay before the next attempt to reach the portal.
///
/// A session that ends sooner than [`MINIMUM_STABLE_SESSION`] counts as a
/// failure even though binding succeeded, because a desktop that closes each
/// session as soon as it is bound would otherwise be sent a fresh
/// `BindShortcuts` every 500 ms for ever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RetryDelay(Duration);

impl RetryDelay {
    fn new() -> Self {
        Self(INITIAL_RETRY_DELAY)
    }

    fn delay(self) -> Duration {
        self.0
    }

    fn record_failure(&mut self) {
        self.0 = (self.0 * 2).min(MAXIMUM_RETRY_DELAY);
    }

    fn record_session(&mut self, lifetime: Duration) {
        if lifetime >= MINIMUM_STABLE_SESSION {
            self.0 = INITIAL_RETRY_DELAY;
            return;
        }

        self.record_failure();
    }
}

/// Receives global shortcut events from XDG Desktop Portal.
pub(crate) struct PortalListener {
    actions: Vec<PortalAction>,
    on_permanent_failure: Option<OnPermanentFailure>,
    stop_signal: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl PortalListener {
    /// Creates a portal listener for the configured actions.
    pub(crate) fn new(
        config: &HotkeyConfig,
        secondary_model: Option<String>,
        profiles: &HashSet<String>,
        on_permanent_failure: OnPermanentFailure,
    ) -> Self {
        Self {
            actions: build_actions(config, secondary_model, profiles),
            on_permanent_failure: Some(on_permanent_failure),
            stop_signal: None,
            task: None,
        }
    }
}

#[async_trait]
impl HotkeyListener for PortalListener {
    async fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>, HotkeyError> {
        let on_permanent_failure = self
            .on_permanent_failure
            .take()
            .unwrap_or(OnPermanentFailure::NotifyUser);
        let (event_tx, event_rx) = mpsc::channel(32);
        let (stop_tx, stop_rx) = oneshot::channel();
        let actions = self.actions.clone();

        // Registering and binding run in the task rather than here. The desktop
        // usually answers BindShortcuts by asking the user, and until the
        // daemon reaches its event loop it handles neither SIGTERM nor the
        // record signals.
        self.stop_signal = Some(stop_tx);
        self.task = Some(tokio::spawn(async move {
            run_listener(actions, event_tx, stop_rx, on_permanent_failure).await;
        }));

        Ok(event_rx)
    }

    async fn stop(&mut self) -> Result<(), HotkeyError> {
        if let Some(stop) = self.stop_signal.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }

        Ok(())
    }
}

async fn run_listener(
    actions: Vec<PortalAction>,
    event_tx: mpsc::Sender<HotkeyEvent>,
    mut stop_rx: oneshot::Receiver<()>,
    on_permanent_failure: OnPermanentFailure,
) {
    let mut retry = RetryDelay::new();
    let mut first_attempt = true;

    loop {
        // Binding can block on a dialog the user never answers, so awaiting it
        // bare would make shutdown wait for the desktop too.
        let connected = tokio::select! {
            biased;
            _ = &mut stop_rx => return,
            connected = connect_and_bind(&actions) => connected,
        };

        let session = match connected {
            Ok(session) => session,
            Err(error) if is_permanent(&error, first_attempt, &on_permanent_failure) => {
                return on_permanent_failure.report(error).await;
            }
            Err(error) => {
                tracing::warn!("Could not reach XDG GlobalShortcuts: {}", error);
                retry.record_failure();
                if wait_before_retry(retry, &mut stop_rx).await {
                    return;
                }
                continue;
            }
        };

        first_attempt = false;
        let started = Instant::now();
        match run_session(&actions, session, &event_tx, &mut stop_rx).await {
            Ok(SessionEnd::Stop) => return,
            Ok(SessionEnd::Reconnect) => {}
            Err(error) => tracing::warn!("XDG GlobalShortcuts session ended: {}", error),
        }

        retry.record_session(started.elapsed());
        if wait_before_retry(retry, &mut stop_rx).await {
            return;
        }
    }
}

/// Whether an error that ended a connection attempt stops the listener.
///
/// An error the desktop answered is always permanent. An unreachable portal is
/// retried, except on the first attempt of a listener that delegates its
/// failures: the auto listener starts evdev from there, and retrying would
/// leave the daemon with no hotkeys while it waited. A portal-only listener
/// has nothing else to start, so it keeps trying, which covers a daemon that
/// starts before the portal at login.
fn is_permanent(
    error: &HotkeyError,
    first_attempt: bool,
    on_permanent_failure: &OnPermanentFailure,
) -> bool {
    if !error.allows_portal_retry() {
        return true;
    }

    first_attempt && matches!(on_permanent_failure, OnPermanentFailure::Delegate(_))
}

/// Waits out the reconnection delay. Returns `true` if the listener was asked
/// to stop while waiting.
async fn wait_before_retry(retry: RetryDelay, stop_rx: &mut oneshot::Receiver<()>) -> bool {
    let delay = retry.delay();
    tracing::info!(
        "Reconnecting to XDG GlobalShortcuts in {} ms",
        delay.as_millis()
    );

    tokio::select! {
        _ = &mut *stop_rx => true,
        _ = tokio::time::sleep(delay) => false,
    }
}

async fn run_session(
    actions: &[PortalAction],
    mut session: OpenSession,
    event_tx: &mpsc::Sender<HotkeyEvent>,
    stop_rx: &mut oneshot::Receiver<()>,
) -> Result<SessionEnd, HotkeyError> {
    let session_proxy = Proxy::new_owned(
        session.connection.clone(),
        PORTAL_SERVICE,
        session.session_path.clone(),
        SESSION_INTERFACE,
    )
    .await
    .map_err(HotkeyError::PortalUnavailable)?;
    let action_map: HashMap<&str, &PortalAction> = actions
        .iter()
        .map(|action| (action.id.as_str(), action))
        .collect();
    let mut active_action: Option<&str> = None;

    // Both cleanup steps run on every exit from the signal loop: the daemon
    // needs a release for a shortcut it still believes is held, and the desktop
    // needs this session closed before the next one asks for the same triggers.
    let outcome = dispatch_signals(
        &action_map,
        &mut session,
        &mut active_action,
        event_tx,
        stop_rx,
    )
    .await;
    release_active(&mut active_action, event_tx).await;
    close_session(&session_proxy).await;

    outcome
}

async fn dispatch_signals<'a>(
    action_map: &HashMap<&'a str, &'a PortalAction>,
    session: &mut OpenSession,
    active_action: &mut Option<&'a str>,
    event_tx: &mpsc::Sender<HotkeyEvent>,
    stop_rx: &mut oneshot::Receiver<()>,
) -> Result<SessionEnd, HotkeyError> {
    let expected_session_path = session.session_path.clone();
    let OpenSession {
        shortcut_signals,
        closed,
        owner_changed,
        logged_triggers,
        ..
    } = session;
    let mut activations = ActivationFilter::default();

    loop {
        tokio::select! {
            biased;
            _ = &mut *stop_rx => return Ok(SessionEnd::Stop),
            message = shortcut_signals.next() => {
                let Some(message) = message else {
                    return Ok(SessionEnd::Reconnect);
                };
                let member = message.header().member().map(|member| member.to_string());
                match member.as_deref() {
                    Some("Activated") => {
                        let (session_path, shortcut_id) = shortcut_signal(&message)?;
                        if session_path != expected_session_path {
                            tracing::debug!("Ignoring GlobalShortcuts activation for another session");
                            continue;
                        }
                        let Some(action) = action_map.get(shortcut_id.as_str()).copied() else {
                            tracing::warn!("Ignoring unknown portal shortcut '{}'", shortcut_id);
                            continue;
                        };
                        if !activations.accept(action.id.as_str(), Instant::now()) {
                            continue;
                        }
                        if matches!(action.event, HotkeyEvent::Cancel) {
                            *active_action = None;
                            if event_tx.send(HotkeyEvent::Cancel).await.is_err() {
                                return Ok(SessionEnd::Stop);
                            }
                            continue;
                        }
                        if *active_action == Some(action.id.as_str()) {
                            // A repeat means the Deactivated was lost. Releasing
                            // here keeps the shortcut usable instead of latched
                            // for the life of the daemon.
                            tracing::warn!(
                                "Portal shortcut '{}' was activated while still held; releasing it first",
                                action.id
                            );
                            *active_action = None;
                            if event_tx.send(HotkeyEvent::Released).await.is_err() {
                                return Ok(SessionEnd::Stop);
                            }
                        } else if let Some(active) = *active_action {
                            tracing::warn!(
                                "Ignoring portal shortcut '{}' while '{}' is active",
                                action.id,
                                active
                            );
                            continue;
                        }
                        if event_tx.send(action.event.clone()).await.is_err() {
                            return Ok(SessionEnd::Stop);
                        }
                        if action.emits_release {
                            *active_action = Some(action.id.as_str());
                        }
                    }
                    Some("Deactivated") => {
                        let (session_path, shortcut_id) = shortcut_signal(&message)?;
                        if session_path != expected_session_path {
                            continue;
                        }
                        if let Some(action) = action_map.get(shortcut_id.as_str()) {
                            activations.release(action.id.as_str());
                        }
                        if *active_action == Some(shortcut_id.as_str()) {
                            *active_action = None;
                            if event_tx.send(HotkeyEvent::Released).await.is_err() {
                                return Ok(SessionEnd::Stop);
                            }
                        }
                    }
                    Some("ShortcutsChanged") => {
                        let (session_path, shortcuts): (OwnedObjectPath, ShortcutList) = message
                            .body()
                            .deserialize()
                            .map_err(|error| HotkeyError::PortalProtocol(error.to_string()))?;
                        if session_path == expected_session_path {
                            log_bound_shortcuts(logged_triggers, &shortcuts);
                        }
                    }
                    _ => {}
                }
            }
            _ = closed.next() => return Ok(SessionEnd::Reconnect),
            _ = owner_changed.next() => return Ok(SessionEnd::Reconnect),
        }
    }
}

/// Reads the session path and shortcut id from an `Activated` or `Deactivated`
/// signal. Both carry `(o, s, t, a{sv})`, and the timestamp and options are
/// unused.
fn shortcut_signal(message: &zbus::Message) -> Result<(OwnedObjectPath, String), HotkeyError> {
    let (session_path, shortcut_id, _timestamp, _options): (
        OwnedObjectPath,
        String,
        u64,
        VariantMap,
    ) = message
        .body()
        .deserialize()
        .map_err(|error| HotkeyError::PortalProtocol(error.to_string()))?;

    Ok((session_path, shortcut_id))
}

async fn release_active(active_action: &mut Option<&str>, event_tx: &mpsc::Sender<HotkeyEvent>) {
    if active_action.take().is_some() {
        let _ = event_tx.send(HotkeyEvent::Released).await;
    }
}

async fn close_session(session: &Proxy<'_>) {
    if let Err(error) = session.call::<_, _, ()>("Close", &()).await {
        tracing::debug!("Could not close XDG GlobalShortcuts session: {}", error);
    }
}

async fn connect_and_bind(actions: &[PortalAction]) -> Result<OpenSession, HotkeyError> {
    let connection = Connection::session()
        .await
        .map_err(HotkeyError::PortalUnavailable)?;
    let sender = connection.unique_name().ok_or_else(|| {
        HotkeyError::PortalProtocol("session bus did not assign a unique name".to_string())
    })?;
    let sender = sender.as_str().trim_start_matches(':').replace('.', "_");

    connect_and_bind_on(connection, &sender, actions).await
}

async fn connect_and_bind_on(
    connection: Connection,
    sender: &str,
    actions: &[PortalAction],
) -> Result<OpenSession, HotkeyError> {
    register_host(&connection).await?;

    let portal = portal_proxy(connection.clone()).await?;
    let version: u32 = portal
        .get_property("version")
        .await
        .map_err(HotkeyError::PortalUnavailable)?;
    if version < 1 {
        return Err(HotkeyError::PortalProtocol(format!(
            "unsupported GlobalShortcuts interface version {version}"
        )));
    }

    // Subscribe before CreateSession, so a portal that exits during setup
    // still ends the session. The session path comes from a token we choose,
    // which is what lets Closed be watched this early too.
    let owner_changed = portal_owner_changed(&connection).await?;
    let shortcut_signals = portal
        .receive_all_signals()
        .await
        .map_err(HotkeyError::PortalUnavailable)?;

    let session_token = token("session");
    let expected_session = session_path(sender, &session_token)?;
    let session_proxy = Proxy::new_owned(
        connection.clone(),
        PORTAL_SERVICE,
        expected_session.clone(),
        SESSION_INTERFACE,
    )
    .await
    .map_err(HotkeyError::PortalUnavailable)?;
    let closed = session_proxy
        .receive_signal("Closed")
        .await
        .map_err(HotkeyError::PortalUnavailable)?;

    let session_path = create_session(&connection, &portal, sender, session_token).await?;
    if session_path != expected_session {
        return Err(HotkeyError::PortalProtocol(format!(
            "CreateSession returned session path {session_path}, expected {expected_session}"
        )));
    }
    let mut logged_triggers = HashMap::new();
    bind_missing_shortcuts(
        &connection,
        &portal,
        &session_path,
        sender,
        actions,
        &mut logged_triggers,
    )
    .await?;

    Ok(OpenSession {
        connection,
        session_path,
        shortcut_signals,
        closed,
        owner_changed,
        logged_triggers,
    })
}

async fn portal_owner_changed(
    connection: &Connection,
) -> Result<zbus::fdo::NameOwnerChangedStream<'static>, HotkeyError> {
    let bus = zbus::fdo::DBusProxy::builder(connection)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(HotkeyError::PortalUnavailable)?;

    bus.receive_name_owner_changed_with_args(&[(0, PORTAL_SERVICE)])
        .await
        .map_err(HotkeyError::PortalUnavailable)
}

async fn register_host(connection: &Connection) -> Result<(), HotkeyError> {
    let registry = Proxy::new(connection, PORTAL_SERVICE, PORTAL_PATH, REGISTRY_INTERFACE)
        .await
        .map_err(HotkeyError::PortalRegistration)?;
    let options = VariantMap::new();
    registry
        .call::<_, _, ()>("Register", &(APP_ID, options))
        .await
        .map_err(HotkeyError::PortalRegistration)
}

async fn portal_proxy(connection: Connection) -> Result<Proxy<'static>, HotkeyError> {
    Proxy::new_owned(connection, PORTAL_SERVICE, PORTAL_PATH, SHORTCUTS_INTERFACE)
        .await
        .map_err(HotkeyError::PortalUnavailable)
}

async fn create_session(
    connection: &Connection,
    portal: &Proxy<'_>,
    sender: &str,
    session_token: String,
) -> Result<OwnedObjectPath, HotkeyError> {
    let handle_token = token("create");
    let expected_request = request_path(sender, &handle_token)?;
    let request = request_proxy(connection.clone(), expected_request.clone()).await?;
    let mut responses = request
        .receive_signal("Response")
        .await
        .map_err(HotkeyError::PortalUnavailable)?;
    let mut options = VariantMap::new();
    insert_string(&mut options, "handle_token", handle_token);
    insert_string(&mut options, "session_handle_token", session_token);

    let returned_request: OwnedObjectPath = portal
        .call("CreateSession", &(options,))
        .await
        .map_err(HotkeyError::PortalUnavailable)?;
    if returned_request != expected_request {
        return Err(HotkeyError::PortalProtocol(format!(
            "CreateSession returned request path {returned_request}, expected {expected_request}"
        )));
    }

    let results = response_results(&mut responses).await?;
    let returned_session = string_result(&results, "session_handle")?;

    OwnedObjectPath::try_from(returned_session.as_str())
        .map_err(|error| HotkeyError::PortalProtocol(error.to_string()))
}

/// Binds the actions unless the desktop already holds every one of them.
///
/// `BindShortcuts` opens a configuration dialog on some desktops, and it
/// resends every preferred trigger, which replaces the triggers the user chose
/// in the desktop's own settings.
async fn bind_missing_shortcuts(
    connection: &Connection,
    portal: &Proxy<'_>,
    session_path: &OwnedObjectPath,
    sender: &str,
    actions: &[PortalAction],
    logged_triggers: &mut HashMap<String, String>,
) -> Result<(), HotkeyError> {
    let existing =
        existing_shortcuts(connection, portal, session_path, sender, logged_triggers).await;
    if let Some(existing) = existing {
        if actions.iter().all(|action| existing.contains(&action.id)) {
            tracing::info!("The desktop already holds every Voxtype shortcut");
            return Ok(());
        }
    }

    bind_shortcuts(
        connection,
        portal,
        session_path,
        sender,
        actions,
        logged_triggers,
    )
    .await
}

/// The shortcut ids the desktop already holds for Voxtype, or `None` when
/// `ListShortcuts` returned an error. A desktop that does not implement the
/// call therefore still gets a `BindShortcuts`.
async fn existing_shortcuts(
    connection: &Connection,
    portal: &Proxy<'_>,
    session_path: &OwnedObjectPath,
    sender: &str,
    logged_triggers: &mut HashMap<String, String>,
) -> Option<HashSet<String>> {
    match list_shortcuts(connection, portal, session_path, sender).await {
        Ok(shortcuts) => {
            log_bound_shortcuts(logged_triggers, &shortcuts);
            Some(shortcuts.into_iter().map(|(id, _)| id).collect())
        }
        Err(error) => {
            tracing::debug!("Could not list existing global shortcuts: {}", error);
            None
        }
    }
}

async fn list_shortcuts(
    connection: &Connection,
    portal: &Proxy<'_>,
    session_path: &OwnedObjectPath,
    sender: &str,
) -> Result<ShortcutList, HotkeyError> {
    let handle_token = token("list");
    let expected_request = request_path(sender, &handle_token)?;
    let request = request_proxy(connection.clone(), expected_request.clone()).await?;
    let mut responses = request
        .receive_signal("Response")
        .await
        .map_err(HotkeyError::PortalUnavailable)?;
    let mut options = VariantMap::new();
    insert_string(&mut options, "handle_token", handle_token);

    let returned_request: OwnedObjectPath = portal
        .call("ListShortcuts", &(session_path, options))
        .await
        .map_err(HotkeyError::PortalUnavailable)?;
    if returned_request != expected_request {
        return Err(HotkeyError::PortalProtocol(format!(
            "ListShortcuts returned request path {returned_request}, expected {expected_request}"
        )));
    }

    let results = response_results(&mut responses).await?;

    shortcut_list_result(&results, "ListShortcuts")
}

async fn bind_shortcuts(
    connection: &Connection,
    portal: &Proxy<'_>,
    session_path: &OwnedObjectPath,
    sender: &str,
    actions: &[PortalAction],
    logged_triggers: &mut HashMap<String, String>,
) -> Result<(), HotkeyError> {
    let handle_token = token("bind");
    let expected_request = request_path(sender, &handle_token)?;
    let request = request_proxy(connection.clone(), expected_request.clone()).await?;
    let mut responses = request
        .receive_signal("Response")
        .await
        .map_err(HotkeyError::PortalUnavailable)?;
    let shortcuts: ShortcutList = actions.iter().map(PortalAction::shortcut).collect();
    let mut options = VariantMap::new();
    insert_string(&mut options, "handle_token", handle_token);

    let returned_request: OwnedObjectPath = portal
        .call("BindShortcuts", &(session_path, shortcuts, "", options))
        .await
        .map_err(HotkeyError::PortalBinding)?;
    if returned_request != expected_request {
        return Err(HotkeyError::PortalProtocol(format!(
            "BindShortcuts returned request path {returned_request}, expected {expected_request}"
        )));
    }

    let results = response_results(&mut responses).await?;
    let bound = shortcut_list_result(&results, "BindShortcuts")?;
    let bound_ids: HashSet<&str> = bound.iter().map(|(id, _)| id.as_str()).collect();
    if !bound_ids.contains("dictate") {
        return Err(HotkeyError::PortalMissingRequired("dictate".to_string()));
    }
    for action in actions {
        if !bound_ids.contains(action.id.as_str()) {
            tracing::warn!("The desktop did not bind optional shortcut '{}'", action.id);
        }
    }
    log_bound_shortcuts(logged_triggers, &bound);

    Ok(())
}

async fn response_results(
    responses: &mut zbus::proxy::SignalStream<'_>,
) -> Result<VariantMap, HotkeyError> {
    let message = responses.next().await.ok_or_else(|| {
        HotkeyError::PortalProtocol("portal request ended without a response".to_string())
    })?;
    let (response, results): (u32, VariantMap) = message
        .body()
        .deserialize()
        .map_err(|error| HotkeyError::PortalProtocol(error.to_string()))?;

    match response {
        0 => Ok(results),
        1 => Err(HotkeyError::PortalCancelled),
        code => Err(HotkeyError::PortalResponse(code)),
    }
}

async fn request_proxy(
    connection: Connection,
    path: OwnedObjectPath,
) -> Result<Proxy<'static>, HotkeyError> {
    Proxy::new_owned(connection, PORTAL_SERVICE, path, REQUEST_INTERFACE)
        .await
        .map_err(HotkeyError::PortalUnavailable)
}

fn request_path(sender: &str, token: &str) -> Result<OwnedObjectPath, HotkeyError> {
    portal_object_path(sender, "request", token)
}

fn session_path(sender: &str, token: &str) -> Result<OwnedObjectPath, HotkeyError> {
    portal_object_path(sender, "session", token)
}

fn portal_object_path(
    sender: &str,
    kind: &str,
    token: &str,
) -> Result<OwnedObjectPath, HotkeyError> {
    OwnedObjectPath::try_from(format!(
        "/org/freedesktop/portal/desktop/{kind}/{sender}/{token}"
    ))
    .map_err(|error| HotkeyError::PortalProtocol(error.to_string()))
}

fn token(prefix: &str) -> String {
    format!("voxtype_{prefix}_{}", uuid::Uuid::new_v4().simple())
}

fn insert_string(map: &mut VariantMap, key: &str, value: String) {
    map.insert(key.to_string(), OwnedValue::from(Str::from(value)));
}

fn string_result(results: &VariantMap, key: &str) -> Result<String, HotkeyError> {
    let value = results
        .get(key)
        .ok_or_else(|| HotkeyError::PortalProtocol(format!("portal response omitted '{key}'")))?;
    <&str>::try_from(value)
        .map(str::to_string)
        .map_err(|error| HotkeyError::PortalProtocol(error.to_string()))
}

fn shortcut_list_result(results: &VariantMap, method: &str) -> Result<ShortcutList, HotkeyError> {
    let value = results.get("shortcuts").ok_or_else(|| {
        HotkeyError::PortalProtocol(format!("{method} omitted its shortcuts result"))
    })?;

    value
        .try_clone()
        .and_then(ShortcutList::try_from)
        .map_err(|error| HotkeyError::PortalProtocol(error.to_string()))
}

fn log_bound_shortcuts(logged_triggers: &mut HashMap<String, String>, shortcuts: &ShortcutList) {
    for (id, properties) in shortcuts {
        let description = properties
            .get("trigger_description")
            .and_then(|value| <&str>::try_from(value).ok())
            .unwrap_or("assigned by the desktop");
        if record_trigger(logged_triggers, id, description) {
            tracing::info!("Portal shortcut '{}': {}", id, description);
        }
    }
}

/// Records the trigger the desktop reports for a shortcut, and reports whether
/// it differs from the one recorded before.
fn record_trigger(
    logged_triggers: &mut HashMap<String, String>,
    id: &str,
    description: &str,
) -> bool {
    let previous = logged_triggers.insert(id.to_string(), description.to_string());

    previous.as_deref() != Some(description)
}

fn build_actions(
    config: &HotkeyConfig,
    secondary_model: Option<String>,
    profiles: &HashSet<String>,
) -> Vec<PortalAction> {
    // Toggle mode ignores releases, and a desktop that never sends Deactivated
    // would leave the shortcut latched for the life of the daemon.
    let emits_release = config.mode == ActivationMode::PushToTalk;
    let mut actions = Vec::new();
    let base_trigger = preferred_trigger(&config.modifiers, &config.key);
    actions.push(PortalAction {
        id: "dictate".to_string(),
        description: "Dictate with the default model".to_string(),
        preferred_trigger: base_trigger,
        event: HotkeyEvent::Pressed {
            model_override: None,
            profile_override: None,
        },
        emits_release,
    });

    if let (Some(model), Some(modifier)) = (&secondary_model, &config.model_modifier) {
        actions.push(PortalAction {
            id: "dictate-secondary".to_string(),
            description: "Dictate with the secondary model".to_string(),
            preferred_trigger: action_trigger(config, [modifier.as_str()]),
            event: HotkeyEvent::Pressed {
                model_override: Some(model.clone()),
                profile_override: None,
            },
            emits_release,
        });
    }

    let mut profile_modifiers: Vec<_> = config.profile_modifiers.iter().collect();
    profile_modifiers.sort_by(
        |(left_modifier, left_profile), (right_modifier, right_profile)| {
            (left_profile, left_modifier).cmp(&(right_profile, right_modifier))
        },
    );
    let mut bound_profiles = HashSet::new();
    for (modifier, profile) in profile_modifiers {
        if !profiles.contains(profile) {
            continue;
        }
        if !bound_profiles.insert(profile) {
            tracing::warn!(
                "Profile '{}' already has a portal shortcut; ignoring its '{}' modifier",
                profile,
                modifier
            );
            continue;
        }
        let suffix = profile_action_suffix(profile);
        actions.push(PortalAction {
            id: format!("dictate-profile-{suffix}"),
            description: format!("Dictate with the '{profile}' profile"),
            preferred_trigger: action_trigger(config, [modifier.as_str()]),
            event: HotkeyEvent::Pressed {
                model_override: None,
                profile_override: Some(profile.clone()),
            },
            emits_release,
        });

        if let (Some(model), Some(model_modifier)) = (&secondary_model, &config.model_modifier) {
            actions.push(PortalAction {
                id: format!("dictate-secondary-profile-{suffix}"),
                description: format!(
                    "Dictate with the secondary model and the '{profile}' profile"
                ),
                preferred_trigger: action_trigger(
                    config,
                    [model_modifier.as_str(), modifier.as_str()],
                ),
                event: HotkeyEvent::Pressed {
                    model_override: Some(model.clone()),
                    profile_override: Some(profile.clone()),
                },
                emits_release,
            });
        }
    }

    if config.cancel_key.is_some() {
        // The desktop grabs a requested trigger exclusively, so asking for the
        // configured cancel key would take a bare Escape or Backspace away from
        // every other application.
        actions.push(PortalAction {
            id: "cancel".to_string(),
            description: "Cancel dictation".to_string(),
            preferred_trigger: None,
            event: HotkeyEvent::Cancel,
            emits_release: false,
        });
    }

    remove_duplicate_triggers(&mut actions);
    actions
}

/// The part of a profile action's id that names the profile.
///
/// The desktop stores the user's binding against this id, so the id must not
/// change while the action still selects the same profile. The modifier is left
/// out because it only supplies the preferred trigger. A name that cannot be
/// used literally gains a digest of the original, because two names can
/// sanitise to the same string.
fn profile_action_suffix(profile: &str) -> String {
    let sanitised: String = profile
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect();
    if sanitised == profile {
        return sanitised;
    }

    let digest = Sha256::digest(profile.as_bytes());
    let digest: String = format!("{digest:x}")
        .chars()
        .take(ID_DIGEST_LENGTH)
        .collect();

    format!("{sanitised}-{digest}")
}

fn action_trigger<'a>(
    config: &HotkeyConfig,
    action_modifiers: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let mut modifiers = config.modifiers.clone();
    modifiers.extend(action_modifiers.into_iter().map(str::to_string));
    preferred_trigger(&modifiers, &config.key)
}

fn remove_duplicate_triggers(actions: &mut [PortalAction]) {
    let mut triggers = HashSet::new();
    for action in actions {
        let Some(trigger) = action.preferred_trigger.as_ref() else {
            continue;
        };
        if !triggers.insert(trigger.clone()) {
            tracing::warn!(
                "Portal shortcut '{}' has duplicate preferred trigger '{}'; the desktop will assign it",
                action.id,
                trigger
            );
            action.preferred_trigger = None;
        }
    }
}

fn preferred_trigger(modifiers: &[String], key: &str) -> Option<String> {
    let mut mapped = HashSet::new();
    for modifier in modifiers {
        mapped.insert(trigger_modifier(modifier)?);
    }
    let mut ordered = Vec::new();
    for modifier in ["CTRL", "ALT", "SHIFT", "NUM", "LOGO"] {
        if mapped.contains(modifier) {
            ordered.push(modifier);
        }
    }
    ordered.push(trigger_keysym(key)?);

    Some(ordered.join("+"))
}

/// Translates a configured modifier name to an XDG trigger modifier, warning
/// when the name cannot be used so a typo does not look like a binding the user
/// deliberately left for the desktop.
fn trigger_modifier(name: &str) -> Option<&'static str> {
    let key = configured_key(name)?;
    let modifier = xdg_modifier(key);
    if modifier.is_none() {
        tracing::warn!(
            "'{}' is not a modifier the XDG shortcuts portal understands; the desktop will assign this binding",
            name
        );
    }

    modifier
}

/// Translates a configured key name to an XDG trigger keysym.
fn trigger_keysym(name: &str) -> Option<&'static str> {
    let key = configured_key(name)?;
    let keysym = xdg_keysym(key);
    if keysym.is_none() {
        tracing::warn!(
            "Key '{}' has no XDG shortcut keysym; the desktop will assign this binding",
            name
        );
    }

    keysym
}

fn configured_key(name: &str) -> Option<KeyCode> {
    match parse_key_name(name) {
        Ok(key) => Some(key),
        Err(error) => {
            tracing::warn!(
                "Cannot use '{}' as a portal shortcut trigger: {}",
                name,
                error
            );
            None
        }
    }
}

fn xdg_modifier(key: KeyCode) -> Option<&'static str> {
    match key {
        KeyCode::KEY_LEFTCTRL | KeyCode::KEY_RIGHTCTRL => Some("CTRL"),
        KeyCode::KEY_LEFTALT | KeyCode::KEY_RIGHTALT => Some("ALT"),
        KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT => Some("SHIFT"),
        KeyCode::KEY_LEFTMETA | KeyCode::KEY_RIGHTMETA => Some("LOGO"),
        KeyCode::KEY_NUMLOCK => Some("NUM"),
        _ => None,
    }
}

/// Maps a kernel key to the xkbcommon keysym name the shortcuts portal
/// expects. Names and aliases a user may type belong in `parse_key_name`, which
/// every caller goes through first.
fn xdg_keysym(key: KeyCode) -> Option<&'static str> {
    let keysym = match key {
        KeyCode::KEY_A => "a",
        KeyCode::KEY_B => "b",
        KeyCode::KEY_C => "c",
        KeyCode::KEY_D => "d",
        KeyCode::KEY_E => "e",
        KeyCode::KEY_F => "f",
        KeyCode::KEY_G => "g",
        KeyCode::KEY_H => "h",
        KeyCode::KEY_I => "i",
        KeyCode::KEY_J => "j",
        KeyCode::KEY_K => "k",
        KeyCode::KEY_L => "l",
        KeyCode::KEY_M => "m",
        KeyCode::KEY_N => "n",
        KeyCode::KEY_O => "o",
        KeyCode::KEY_P => "p",
        KeyCode::KEY_Q => "q",
        KeyCode::KEY_R => "r",
        KeyCode::KEY_S => "s",
        KeyCode::KEY_T => "t",
        KeyCode::KEY_U => "u",
        KeyCode::KEY_V => "v",
        KeyCode::KEY_W => "w",
        KeyCode::KEY_X => "x",
        KeyCode::KEY_Y => "y",
        KeyCode::KEY_Z => "z",
        KeyCode::KEY_0 => "0",
        KeyCode::KEY_1 => "1",
        KeyCode::KEY_2 => "2",
        KeyCode::KEY_3 => "3",
        KeyCode::KEY_4 => "4",
        KeyCode::KEY_5 => "5",
        KeyCode::KEY_6 => "6",
        KeyCode::KEY_7 => "7",
        KeyCode::KEY_8 => "8",
        KeyCode::KEY_9 => "9",
        KeyCode::KEY_F1 => "F1",
        KeyCode::KEY_F2 => "F2",
        KeyCode::KEY_F3 => "F3",
        KeyCode::KEY_F4 => "F4",
        KeyCode::KEY_F5 => "F5",
        KeyCode::KEY_F6 => "F6",
        KeyCode::KEY_F7 => "F7",
        KeyCode::KEY_F8 => "F8",
        KeyCode::KEY_F9 => "F9",
        KeyCode::KEY_F10 => "F10",
        KeyCode::KEY_F11 => "F11",
        KeyCode::KEY_F12 => "F12",
        KeyCode::KEY_F13 => "F13",
        KeyCode::KEY_F14 => "F14",
        KeyCode::KEY_F15 => "F15",
        KeyCode::KEY_F16 => "F16",
        KeyCode::KEY_F17 => "F17",
        KeyCode::KEY_F18 => "F18",
        KeyCode::KEY_F19 => "F19",
        KeyCode::KEY_F20 => "F20",
        KeyCode::KEY_F21 => "F21",
        KeyCode::KEY_F22 => "F22",
        KeyCode::KEY_F23 => "F23",
        KeyCode::KEY_F24 => "F24",
        KeyCode::KEY_HOME => "Home",
        KeyCode::KEY_END => "End",
        KeyCode::KEY_PAGEUP => "Page_Up",
        KeyCode::KEY_PAGEDOWN => "Page_Down",
        KeyCode::KEY_UP => "Up",
        KeyCode::KEY_DOWN => "Down",
        KeyCode::KEY_LEFT => "Left",
        KeyCode::KEY_RIGHT => "Right",
        KeyCode::KEY_INSERT => "Insert",
        KeyCode::KEY_DELETE => "Delete",
        KeyCode::KEY_ENTER => "Return",
        KeyCode::KEY_ESC => "Escape",
        KeyCode::KEY_SPACE => "space",
        KeyCode::KEY_TAB => "Tab",
        KeyCode::KEY_BACKSPACE => "BackSpace",
        KeyCode::KEY_GRAVE => "grave",
        KeyCode::KEY_PAUSE => "Pause",
        KeyCode::KEY_SYSRQ => "Print",
        KeyCode::KEY_SCROLLLOCK => "Scroll_Lock",
        KeyCode::KEY_CAPSLOCK => "Caps_Lock",
        KeyCode::KEY_NUMLOCK => "Num_Lock",
        KeyCode::KEY_MENU => "Menu",
        KeyCode::KEY_LEFTCTRL => "Control_L",
        KeyCode::KEY_RIGHTCTRL => "Control_R",
        KeyCode::KEY_LEFTALT => "Alt_L",
        KeyCode::KEY_RIGHTALT => "Alt_R",
        KeyCode::KEY_LEFTSHIFT => "Shift_L",
        KeyCode::KEY_RIGHTSHIFT => "Shift_R",
        KeyCode::KEY_LEFTMETA => "Super_L",
        KeyCode::KEY_RIGHTMETA => "Super_R",
        KeyCode::KEY_MUTE => "XF86AudioMute",
        KeyCode::KEY_VOLUMEDOWN => "XF86AudioLowerVolume",
        KeyCode::KEY_VOLUMEUP => "XF86AudioRaiseVolume",
        KeyCode::KEY_PLAYPAUSE => "XF86AudioPlay",
        KeyCode::KEY_NEXTSONG => "XF86AudioNext",
        KeyCode::KEY_PREVIOUSSONG => "XF86AudioPrev",
        KeyCode::KEY_RECORD => "XF86AudioRecord",
        KeyCode::KEY_REWIND => "XF86AudioRewind",
        KeyCode::KEY_FASTFORWARD => "XF86AudioForward",
        KeyCode::KEY_MEDIA => "XF86AudioMedia",
        _ => return None,
    };

    Some(keysym)
}

pub(crate) fn desktop_entry_path() -> Option<PathBuf> {
    data_directories().find_map(|directory| {
        let path = directory
            .join("applications")
            .join(format!("{APP_ID}.desktop"));
        path.is_file().then_some(path)
    })
}

fn data_directories() -> impl Iterator<Item = PathBuf> {
    let user = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let system =
        std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut directories = user.into_iter().collect::<Vec<_>>();
    directories.extend(
        std::env::split_paths(&system).filter(|directory| !directory.as_os_str().is_empty()),
    );
    directories.into_iter()
}

pub(crate) async fn portal_versions() -> Result<(u32, u32), HotkeyError> {
    let connection = Connection::session()
        .await
        .map_err(HotkeyError::PortalUnavailable)?;
    register_host(&connection).await?;
    let registry = Proxy::new(&connection, PORTAL_SERVICE, PORTAL_PATH, REGISTRY_INTERFACE)
        .await
        .map_err(HotkeyError::PortalRegistration)?;
    let registry_version = registry
        .get_property("version")
        .await
        .map_err(HotkeyError::PortalRegistration)?;
    let shortcuts_version = portal_proxy(connection)
        .await?
        .get_property("version")
        .await
        .map_err(HotkeyError::PortalUnavailable)?;

    Ok((registry_version, shortcuts_version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HotkeyConfig;
    use std::sync::{Arc, Mutex};
    use zbus::object_server::{ObjectServer, SignalContext};

    #[derive(Clone)]
    struct FakeRegistry {
        registrations: Arc<Mutex<Vec<String>>>,
    }

    #[zbus::interface(interface = "org.freedesktop.host.portal.Registry")]
    impl FakeRegistry {
        async fn register(&self, app_id: String, _options: VariantMap) {
            self.registrations
                .lock()
                .expect("registration state should not be poisoned")
                .push(app_id);
        }

        #[zbus(property, name = "version")]
        fn version(&self) -> u32 {
            1
        }
    }

    #[derive(Clone)]
    struct FakeShortcuts {
        sender: String,
        /// Ids returned by `ListShortcuts`.
        listed: Vec<String>,
        bound_ids: Arc<Mutex<Vec<String>>>,
    }

    #[zbus::interface(interface = "org.freedesktop.portal.GlobalShortcuts")]
    impl FakeShortcuts {
        async fn create_session(
            &self,
            #[zbus(object_server)] object_server: &ObjectServer,
            options: VariantMap,
        ) -> OwnedObjectPath {
            let handle_token = test_string_option(&options, "handle_token");
            let session_token = test_string_option(&options, "session_handle_token");
            let request_path =
                request_path(&self.sender, &handle_token).expect("request path should be valid");
            let session_path =
                session_path(&self.sender, &session_token).expect("session path should be valid");
            object_server
                .at(session_path.clone(), FakeSession)
                .await
                .expect("fake session should be registered");
            object_server
                .at(request_path.clone(), FakeRequest)
                .await
                .expect("fake request should be registered");

            let mut results = VariantMap::new();
            insert_string(&mut results, "session_handle", session_path.to_string());
            emit_response(object_server, &request_path, results).await;

            request_path
        }

        async fn list_shortcuts(
            &self,
            #[zbus(object_server)] object_server: &ObjectServer,
            _session_path: OwnedObjectPath,
            options: VariantMap,
        ) -> OwnedObjectPath {
            let handle_token = test_string_option(&options, "handle_token");
            let request_path =
                request_path(&self.sender, &handle_token).expect("request path should be valid");
            object_server
                .at(request_path.clone(), FakeRequest)
                .await
                .expect("fake request should be registered");
            let listed = self
                .listed
                .iter()
                .map(|id| (id.clone(), VariantMap::new()))
                .collect();
            emit_response(object_server, &request_path, shortcut_results(listed)).await;

            request_path
        }

        async fn bind_shortcuts(
            &self,
            #[zbus(object_server)] object_server: &ObjectServer,
            #[zbus(signal_context)] signal_context: SignalContext<'_>,
            session_path: OwnedObjectPath,
            shortcuts: ShortcutList,
            _parent_window: String,
            options: VariantMap,
        ) -> OwnedObjectPath {
            let handle_token = test_string_option(&options, "handle_token");
            let request_path =
                request_path(&self.sender, &handle_token).expect("request path should be valid");
            let bound_ids = shortcuts
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            *self
                .bound_ids
                .lock()
                .expect("binding state should not be poisoned") = bound_ids;
            object_server
                .at(request_path.clone(), FakeRequest)
                .await
                .expect("fake request should be registered");

            Self::activated(
                &signal_context,
                session_path,
                "dictate",
                1,
                VariantMap::new(),
            )
            .await
            .expect("activation should be emitted");

            emit_response(object_server, &request_path, shortcut_results(shortcuts)).await;

            request_path
        }

        #[zbus(property, name = "version")]
        fn version(&self) -> u32 {
            1
        }

        #[zbus(signal)]
        async fn activated(
            signal_context: &SignalContext<'_>,
            session_path: OwnedObjectPath,
            shortcut_id: &str,
            timestamp: u64,
            options: VariantMap,
        ) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn deactivated(
            signal_context: &SignalContext<'_>,
            session_path: OwnedObjectPath,
            shortcut_id: &str,
            timestamp: u64,
            options: VariantMap,
        ) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn shortcuts_changed(
            signal_context: &SignalContext<'_>,
            session_path: OwnedObjectPath,
            shortcuts: ShortcutList,
        ) -> zbus::Result<()>;
    }

    struct FakeRequest;

    #[zbus::interface(interface = "org.freedesktop.portal.Request")]
    impl FakeRequest {
        #[zbus(signal)]
        async fn response(
            signal_context: &SignalContext<'_>,
            response: u32,
            results: VariantMap,
        ) -> zbus::Result<()>;
    }

    struct FakeSession;

    #[zbus::interface(interface = "org.freedesktop.portal.Session")]
    impl FakeSession {
        async fn close(&self) {}

        #[zbus(signal)]
        async fn closed(signal_context: &SignalContext<'_>) -> zbus::Result<()>;
    }

    fn test_string_option(options: &VariantMap, key: &str) -> String {
        <&str>::try_from(options.get(key).expect("option should be present"))
            .expect("option should be a string")
            .to_string()
    }

    fn shortcut_results(shortcuts: ShortcutList) -> VariantMap {
        let mut results = VariantMap::new();
        let shortcuts = OwnedValue::try_from(zbus::zvariant::Value::new(shortcuts))
            .expect("shortcut list should convert to a variant");
        results.insert("shortcuts".to_string(), shortcuts);

        results
    }

    async fn emit_response(
        object_server: &ObjectServer,
        request_path: &OwnedObjectPath,
        results: VariantMap,
    ) {
        let request = object_server
            .interface::<_, FakeRequest>(request_path)
            .await
            .expect("fake request interface should exist");
        FakeRequest::response(request.signal_context(), 0, results)
            .await
            .expect("response should be emitted");
    }

    /// Runs `connect_and_bind_on` against a fake portal. Returns the session,
    /// the app ids the fake registered, and the shortcut ids it bound.
    async fn connect_to_fake_portal(
        listed: Vec<String>,
        actions: &[PortalAction],
    ) -> (Connection, OpenSession, Vec<String>, Vec<String>) {
        let registrations = Arc::new(Mutex::new(Vec::new()));
        let bound_ids = Arc::new(Mutex::new(Vec::new()));
        let sender = "test_sender";
        let (server_stream, client_stream) =
            tokio::net::UnixStream::pair().expect("Unix stream pair should be created");
        let guid = zbus::Guid::generate();
        let server_builder = zbus::connection::Builder::unix_stream(server_stream)
            .server(guid)
            .expect("server GUID should be accepted")
            .p2p()
            .serve_at(
                PORTAL_PATH,
                FakeRegistry {
                    registrations: Arc::clone(&registrations),
                },
            )
            .expect("registry interface should be served")
            .serve_at(
                PORTAL_PATH,
                FakeShortcuts {
                    sender: sender.to_string(),
                    listed,
                    bound_ids: Arc::clone(&bound_ids),
                },
            )
            .expect("shortcuts interface should be served");
        let client_builder = zbus::connection::Builder::unix_stream(client_stream).p2p();
        let (server, client) = tokio::try_join!(server_builder.build(), client_builder.build())
            .expect("peer connections should be established");

        let session = connect_and_bind_on(client, sender, actions)
            .await
            .expect("portal session should bind");
        let actual_registrations = registrations
            .lock()
            .expect("registration state should not be poisoned")
            .clone();
        let actual_bound_ids = bound_ids
            .lock()
            .expect("binding state should not be poisoned")
            .clone();

        (server, session, actual_registrations, actual_bound_ids)
    }

    fn dictate_action() -> PortalAction {
        PortalAction {
            id: "dictate".to_string(),
            description: "Dictate with the default model".to_string(),
            preferred_trigger: Some("F13".to_string()),
            event: HotkeyEvent::Pressed {
                model_override: None,
                profile_override: None,
            },
            emits_release: true,
        }
    }

    #[tokio::test]
    async fn registers_binds_and_subscribes_before_binding_returns() {
        let actions = vec![dictate_action()];

        let (server, mut session, registrations, bound_ids) =
            connect_to_fake_portal(Vec::new(), &actions).await;
        let message = tokio::time::timeout(Duration::from_secs(1), session.shortcut_signals.next())
            .await
            .expect("activation should already be queued")
            .expect("activation stream should remain open");
        let (session_path, shortcut_id, timestamp, options): (
            OwnedObjectPath,
            String,
            u64,
            VariantMap,
        ) = message
            .body()
            .deserialize()
            .expect("activation should have the portal wire shape");
        let actual_activation = (session_path, shortcut_id, timestamp, options.is_empty());

        assert_eq!(registrations, vec![APP_ID.to_string()]);
        assert_eq!(bound_ids, vec!["dictate".to_string()]);
        assert_eq!(
            actual_activation,
            (session.session_path.clone(), "dictate".to_string(), 1, true)
        );

        drop(server);
    }

    #[tokio::test]
    async fn shortcuts_the_desktop_already_holds_are_not_bound_again() {
        let actions = vec![dictate_action()];

        let (server, session, registrations, bound_ids) =
            connect_to_fake_portal(vec!["dictate".to_string()], &actions).await;

        assert_eq!(registrations, vec![APP_ID.to_string()]);
        assert_eq!(bound_ids, Vec::<String>::new());
        // Listing logs each shortcut, so the session starts with a record of
        // what it logged and the desktop's opening ShortcutsChanged is quiet.
        assert_eq!(
            session.logged_triggers,
            HashMap::from([("dictate".to_string(), "assigned by the desktop".to_string())])
        );

        drop(server);
    }

    #[tokio::test]
    async fn a_missing_shortcut_binds_the_whole_action_set() {
        let actions = vec![
            dictate_action(),
            PortalAction {
                id: "cancel".to_string(),
                description: "Cancel dictation".to_string(),
                preferred_trigger: None,
                event: HotkeyEvent::Cancel,
                emits_release: false,
            },
        ];

        let (server, _session, _registrations, bound_ids) =
            connect_to_fake_portal(vec!["dictate".to_string()], &actions).await;

        assert_eq!(bound_ids, vec!["dictate".to_string(), "cancel".to_string()]);

        drop(server);
    }

    /// Number of auto-repeat activations the burst tests send.
    const BURST_LENGTH: usize = 20;

    /// Emits `Activated` from the fake portal outside a method call.
    async fn emit_activated(
        server: &Connection,
        session_path: &OwnedObjectPath,
        shortcut_id: &str,
    ) {
        let shortcuts = server
            .object_server()
            .interface::<_, FakeShortcuts>(PORTAL_PATH)
            .await
            .expect("fake shortcuts interface should exist");
        FakeShortcuts::activated(
            shortcuts.signal_context(),
            session_path.clone(),
            shortcut_id,
            0,
            VariantMap::new(),
        )
        .await
        .expect("activation should be emitted");
    }

    /// Emits `Deactivated` from the fake portal outside a method call.
    async fn emit_deactivated(
        server: &Connection,
        session_path: &OwnedObjectPath,
        shortcut_id: &str,
    ) {
        let shortcuts = server
            .object_server()
            .interface::<_, FakeShortcuts>(PORTAL_PATH)
            .await
            .expect("fake shortcuts interface should exist");
        FakeShortcuts::deactivated(
            shortcuts.signal_context(),
            session_path.clone(),
            shortcut_id,
            0,
            VariantMap::new(),
        )
        .await
        .expect("deactivation should be emitted");
    }

    /// Runs a session against the fake portal, sends a burst of `Activated`
    /// signals for the dictate shortcut, and returns the events the daemon saw.
    ///
    /// The burst is followed by one activation of the cancel shortcut. Signals
    /// arrive in order on a single stream, so receiving the cancel event proves
    /// the whole burst has been handled.
    async fn events_from_repeat_burst(mode: ActivationMode) -> Vec<HotkeyEvent> {
        let config = HotkeyConfig {
            key: "F13".to_string(),
            cancel_key: Some("ESC".to_string()),
            mode,
            ..HotkeyConfig::default()
        };
        let actions = build_actions(&config, None, &HashSet::new());
        let (server, session, _, _) = connect_to_fake_portal(Vec::new(), &actions).await;
        let session_path = session.session_path.clone();
        // Wide enough to hold every event an unfiltered burst would produce,
        // so a regression fails on the assertion instead of blocking the
        // listener on a full channel.
        let (event_tx, mut event_rx) = mpsc::channel(BURST_LENGTH * 4);
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = run_session(&actions, session, &event_tx, &mut stop_rx).await;
        });

        // Binding emits the first activation, standing in for the key press.
        let mut events = vec![receive(&mut event_rx).await];
        for _ in 0..BURST_LENGTH {
            emit_activated(&server, &session_path, "dictate").await;
        }
        emit_activated(&server, &session_path, "cancel").await;
        events.push(receive(&mut event_rx).await);

        let _ = stop_tx.send(());
        let _ = task.await;
        while let Some(event) = event_rx.recv().await {
            events.push(event);
        }
        drop(server);

        events
    }

    async fn receive(event_rx: &mut mpsc::Receiver<HotkeyEvent>) -> HotkeyEvent {
        tokio::time::timeout(Duration::from_secs(5), event_rx.recv())
            .await
            .expect("an event should arrive")
            .expect("the listener should still be running")
    }

    fn dictate_pressed() -> HotkeyEvent {
        HotkeyEvent::Pressed {
            model_override: None,
            profile_override: None,
        }
    }

    #[tokio::test]
    async fn auto_repeat_does_not_chop_a_push_to_talk_recording() {
        let events = events_from_repeat_burst(ActivationMode::PushToTalk).await;

        assert_eq!(events, vec![dictate_pressed(), HotkeyEvent::Cancel]);
    }

    #[tokio::test]
    async fn auto_repeat_does_not_toggle_repeatedly() {
        let events = events_from_repeat_burst(ActivationMode::Toggle).await;

        assert_eq!(events, vec![dictate_pressed(), HotkeyEvent::Cancel]);
    }

    /// Runs a session in the given mode and replays a release of the dictate
    /// shortcut followed immediately by a second press. Returns the events the
    /// daemon saw, ending with the cancel event that proves the sequence was
    /// handled.
    async fn events_from_release_then_press(mode: ActivationMode) -> Vec<HotkeyEvent> {
        let config = HotkeyConfig {
            key: "F13".to_string(),
            cancel_key: Some("ESC".to_string()),
            mode,
            ..HotkeyConfig::default()
        };
        let actions = build_actions(&config, None, &HashSet::new());
        let (server, session, _, _) = connect_to_fake_portal(Vec::new(), &actions).await;
        let session_path = session.session_path.clone();
        let (event_tx, mut event_rx) = mpsc::channel(32);
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = run_session(&actions, session, &event_tx, &mut stop_rx).await;
        });

        // Binding emits the first activation, standing in for the first press.
        let mut events = vec![receive(&mut event_rx).await];
        emit_deactivated(&server, &session_path, "dictate").await;
        emit_activated(&server, &session_path, "dictate").await;
        emit_activated(&server, &session_path, "cancel").await;
        loop {
            let event = receive(&mut event_rx).await;
            let done = event == HotkeyEvent::Cancel;
            events.push(event);
            if done {
                break;
            }
        }

        let _ = stop_tx.send(());
        let _ = task.await;
        drop(server);

        events
    }

    #[tokio::test]
    async fn a_press_straight_after_a_release_is_not_repeat() {
        let events = events_from_release_then_press(ActivationMode::PushToTalk).await;

        assert_eq!(
            events,
            vec![
                dictate_pressed(),
                HotkeyEvent::Released,
                dictate_pressed(),
                HotkeyEvent::Cancel
            ]
        );
    }

    #[tokio::test]
    async fn a_quick_second_tap_after_a_release_toggles_again() {
        let events = events_from_release_then_press(ActivationMode::Toggle).await;

        assert_eq!(
            events,
            vec![dictate_pressed(), dictate_pressed(), HotkeyEvent::Cancel]
        );
    }

    #[test]
    fn a_held_key_stays_quiet_but_a_later_press_is_a_new_press() {
        let mut filter = ActivationFilter::default();
        let pressed = Instant::now();
        let mut accepted = vec![filter.accept("dictate", pressed)];

        // GNOME repeats after a 500 ms delay, then every 30 ms.
        let mut repeat = pressed + Duration::from_millis(500);
        for _ in 0..20 {
            accepted.push(filter.accept("dictate", repeat));
            repeat += Duration::from_millis(30);
        }
        accepted.push(filter.accept("dictate", repeat + ACTIVATION_REPEAT_WINDOW));

        let mut expected = vec![false; 22];
        expected[0] = true;
        expected[21] = true;
        assert_eq!(accepted, expected);
    }

    #[test]
    fn a_shortcut_is_logged_once_until_its_trigger_changes() {
        let mut logged = HashMap::new();
        let logged_now = [
            record_trigger(&mut logged, "dictate", "Press Scroll_Lock"),
            // The desktop repeats its whole list in ShortcutsChanged.
            record_trigger(&mut logged, "dictate", "Press Scroll_Lock"),
            record_trigger(&mut logged, "dictate", "Press F13"),
            record_trigger(&mut logged, "cancel", "Press Scroll_Lock"),
        ];

        assert_eq!(logged_now, [true, false, true, true]);
    }

    #[test]
    fn a_reported_release_resets_the_repeat_filter() {
        let mut filter = ActivationFilter::default();
        let pressed = Instant::now();
        let first = filter.accept("dictate", pressed);
        filter.release("dictate");
        let second = filter.accept("dictate", pressed + Duration::from_millis(50));

        assert_eq!([first, second], [true, true]);
    }

    #[test]
    fn each_shortcut_is_filtered_on_its_own() {
        let mut filter = ActivationFilter::default();
        let now = Instant::now();
        let accepted = [
            filter.accept("dictate", now),
            filter.accept("cancel", now),
            filter.accept("dictate", now),
        ];

        assert_eq!(accepted, [true, true, false]);
    }

    #[test]
    fn translates_xdg_triggers() {
        let cases = [
            (&["LEFTCTRL", "RIGHTSHIFT"][..], "A", Some("CTRL+SHIFT+a")),
            (&["LEFTMETA"][..], "SCROLLLOCK", Some("LOGO+Scroll_Lock")),
            (&[][..], "PLAYPAUSE", Some("XF86AudioPlay")),
            (&[][..], "RETURN", Some("Return")),
            (&[][..], "page-up", Some("Page_Up")),
            // A numeric keycode names a key like any other spelling, so it
            // translates whenever that key has a keysym.
            (&["EVTEST_42"][..], "WEV_234", Some("SHIFT+XF86AudioMedia")),
            // KEY_UNKNOWN has no keysym, and F13 is not a modifier.
            (&[][..], "EVTEST_240", None),
            (&["F13"][..], "A", None),
            (&[][..], "NOT_A_KEY", None),
        ];

        for (modifiers, key, expected) in cases {
            let modifiers = modifiers
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>();
            assert_eq!(preferred_trigger(&modifiers, key).as_deref(), expected);
        }
    }

    #[test]
    fn builds_all_configured_actions_in_stable_order() {
        let config = HotkeyConfig {
            key: "F13".to_string(),
            model_modifier: Some("LEFTSHIFT".to_string()),
            cancel_key: Some("ESC".to_string()),
            profile_modifiers: HashMap::from([
                ("RIGHTALT".to_string(), "translate".to_string()),
                ("LEFTCTRL".to_string(), "formal".to_string()),
            ]),
            ..HotkeyConfig::default()
        };
        let profiles = HashSet::from(["translate".to_string(), "formal".to_string()]);

        let actions = build_actions(&config, Some("large-v3".to_string()), &profiles);
        let actual = actions
            .iter()
            .map(|action| {
                (
                    action.id.clone(),
                    action.preferred_trigger.clone(),
                    action.event.clone(),
                )
            })
            .collect::<Vec<_>>();
        let expected = vec![
            (
                "dictate".to_string(),
                Some("F13".to_string()),
                HotkeyEvent::Pressed {
                    model_override: None,
                    profile_override: None,
                },
            ),
            (
                "dictate-secondary".to_string(),
                Some("SHIFT+F13".to_string()),
                HotkeyEvent::Pressed {
                    model_override: Some("large-v3".to_string()),
                    profile_override: None,
                },
            ),
            (
                "dictate-profile-formal".to_string(),
                Some("CTRL+F13".to_string()),
                HotkeyEvent::Pressed {
                    model_override: None,
                    profile_override: Some("formal".to_string()),
                },
            ),
            (
                "dictate-secondary-profile-formal".to_string(),
                Some("CTRL+SHIFT+F13".to_string()),
                HotkeyEvent::Pressed {
                    model_override: Some("large-v3".to_string()),
                    profile_override: Some("formal".to_string()),
                },
            ),
            (
                "dictate-profile-translate".to_string(),
                Some("ALT+F13".to_string()),
                HotkeyEvent::Pressed {
                    model_override: None,
                    profile_override: Some("translate".to_string()),
                },
            ),
            (
                "dictate-secondary-profile-translate".to_string(),
                Some("ALT+SHIFT+F13".to_string()),
                HotkeyEvent::Pressed {
                    model_override: Some("large-v3".to_string()),
                    profile_override: Some("translate".to_string()),
                },
            ),
            ("cancel".to_string(), None, HotkeyEvent::Cancel),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn profile_action_ids_name_the_profile() {
        assert_eq!(profile_action_suffix("translate"), "translate");
        assert_eq!(profile_action_suffix("formal_2"), "formal_2");

        // Two names that sanitise alike stay distinct.
        let spaced = profile_action_suffix("a b");
        let hyphenated = profile_action_suffix("a-b");

        assert!(spaced.starts_with("a-b-"), "unexpected id '{spaced}'");
        assert_eq!(spaced.len(), "a-b-".len() + ID_DIGEST_LENGTH);
        assert_eq!(hyphenated, "a-b");
    }

    #[test]
    fn toggle_mode_does_not_track_shortcut_releases() {
        let config = HotkeyConfig {
            key: "F13".to_string(),
            mode: ActivationMode::Toggle,
            ..HotkeyConfig::default()
        };

        let actions = build_actions(&config, None, &HashSet::new());
        let emits_release = actions
            .iter()
            .map(|action| action.emits_release)
            .collect::<Vec<_>>();

        assert_eq!(emits_release, vec![false]);
    }

    #[test]
    fn push_to_talk_tracks_shortcut_releases() {
        let config = HotkeyConfig {
            key: "F13".to_string(),
            mode: ActivationMode::PushToTalk,
            ..HotkeyConfig::default()
        };

        let actions = build_actions(&config, None, &HashSet::new());
        let emits_release = actions
            .iter()
            .map(|action| action.emits_release)
            .collect::<Vec<_>>();

        assert_eq!(emits_release, vec![true]);
    }

    #[test]
    fn one_profile_gets_one_action_however_many_modifiers_name_it() {
        let config = HotkeyConfig {
            key: "F13".to_string(),
            profile_modifiers: HashMap::from([
                ("LEFTCTRL".to_string(), "translate".to_string()),
                ("RIGHTALT".to_string(), "translate".to_string()),
            ]),
            ..HotkeyConfig::default()
        };
        let profiles = HashSet::from(["translate".to_string()]);

        let actions = build_actions(&config, None, &profiles);
        let ids = actions
            .iter()
            .map(|action| action.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["dictate", "dictate-profile-translate"]);
    }

    #[test]
    fn later_duplicate_trigger_is_left_for_the_desktop_to_assign() {
        let config = HotkeyConfig {
            key: "F13".to_string(),
            model_modifier: Some("LEFTSHIFT".to_string()),
            profile_modifiers: HashMap::from([("RIGHTSHIFT".to_string(), "translate".to_string())]),
            ..HotkeyConfig::default()
        };
        let profiles = HashSet::from(["translate".to_string()]);

        let actions = build_actions(&config, Some("large-v3".to_string()), &profiles);
        let triggers = actions
            .iter()
            .map(|action| action.preferred_trigger.as_deref())
            .collect::<Vec<_>>();

        assert_eq!(triggers, vec![Some("F13"), Some("SHIFT+F13"), None, None]);
    }

    #[test]
    fn only_a_delegating_listener_gives_up_on_an_unreachable_portal_at_the_start() {
        let (sender, _receiver) = oneshot::channel();
        let delegate = OnPermanentFailure::Delegate(sender);
        let notify = OnPermanentFailure::NotifyUser;
        let unreachable = HotkeyError::PortalUnavailable(zbus::Error::Unsupported);
        let cancelled = HotkeyError::PortalCancelled;

        let decisions = [
            is_permanent(&unreachable, true, &notify),
            is_permanent(&unreachable, false, &notify),
            is_permanent(&unreachable, true, &delegate),
            is_permanent(&unreachable, false, &delegate),
            is_permanent(&cancelled, true, &notify),
            is_permanent(&cancelled, false, &delegate),
        ];

        assert_eq!(decisions, [false, false, true, false, true, true]);
    }

    #[test]
    fn a_short_session_backs_off_and_a_long_one_does_not() {
        let mut retry = RetryDelay::new();
        let mut delays = vec![retry.delay()];

        retry.record_session(Duration::from_secs(1));
        delays.push(retry.delay());
        retry.record_session(Duration::from_secs(1));
        delays.push(retry.delay());
        retry.record_session(MINIMUM_STABLE_SESSION);
        delays.push(retry.delay());

        assert_eq!(
            delays,
            vec![
                INITIAL_RETRY_DELAY,
                INITIAL_RETRY_DELAY * 2,
                INITIAL_RETRY_DELAY * 4,
                INITIAL_RETRY_DELAY,
            ]
        );
    }

    #[test]
    fn repeated_failures_stop_growing_at_the_maximum_delay() {
        let mut retry = RetryDelay::new();

        for _ in 0..20 {
            retry.record_failure();
        }

        assert_eq!(retry.delay(), MAXIMUM_RETRY_DELAY);
    }
}
