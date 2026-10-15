//! Portal-based text output via a persistent XDG RemoteDesktop session.
//!
//! Like `eitype`, this types through the XDG RemoteDesktop portal, so it works
//! on compositors that lack the virtual-keyboard protocol (KDE Plasma 6,
//! GNOME) and needs no uinput / `input` group. Unlike `eitype`, it opens ONE
//! portal session and holds it for the whole process instead of registering a
//! fresh session on every call. On KDE that removes the per-call
//! re-registration that flickers the tray indicator and adds latency during
//! streaming dictation.
//!
//! We speak the portal protocol directly over zbus (the same D-Bus stack the
//! MPRIS code uses) rather than pulling a wrapper crate. Injection uses
//! `NotifyKeyboardKeysym` with Unicode keysyms, so it is layout-independent:
//! the codepoint is sent directly and no XKB layout/variant handling is needed
//! (unlike eitype/dotool).
//!
//! First use shows a one-time compositor consent dialog. A persisted restore
//! token keeps subsequent daemon starts silent until the grant is revoked.
//!
//! Requires the `portal` cargo feature.

use super::TextOutput;
use crate::error::OutputError;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::Mutex;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy};

const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const RD_IFACE: &str = "org.freedesktop.portal.RemoteDesktop";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

/// RemoteDesktop DeviceType bitmask (per the portal spec): 1 = KEYBOARD.
const DEVICE_KEYBOARD: u32 = 1;
/// SelectDevices persist_mode: keep the grant until explicitly revoked.
const PERSIST_UNTIL_REVOKED: u32 = 2;

const KEY_RELEASED: u32 = 0;
const KEY_PRESSED: u32 = 1;

/// X11/xkbcommon keysyms for the named keys we emit directly.
const KEYSYM_RETURN: i32 = 0xFF0D;
const KEYSYM_SHIFT_L: i32 = 0xFFE1;

/// A consented, live RemoteDesktop session: the D-Bus connection it was created
/// on (the portal binds the session to this connection) plus the session's
/// object path.
struct PortalSession {
    conn: Connection,
    session: OwnedObjectPath,
}

/// Process-global cache. The batch output path in `daemon.rs` rebuilds the
/// driver chain per transcription, so a session stored in the driver struct
/// would be re-handshaked every utterance. Caching here keeps one warm session
/// for the whole process, opened lazily on first use.
fn session_cell() -> &'static Mutex<Option<PortalSession>> {
    static CELL: OnceLock<Mutex<Option<PortalSession>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// Unique per-process handle tokens without needing a randomness crate.
fn next_token(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}{}_{}", std::process::id(), n)
}

fn restore_token_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into())).join(".cache")
        });
    base.join("voxtype").join("portal-restore.token")
}

fn read_restore_token() -> Option<String> {
    let token = std::fs::read_to_string(restore_token_path()).ok()?;
    let token = token.trim().to_string();
    (!token.is_empty()).then_some(token)
}

fn save_restore_token(token: &str) {
    let path = restore_token_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&path, token).is_ok() {
        // Bearer credential: keep other users out. This does nothing against
        // other processes running as the same user.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}

/// Run a portal method that answers asynchronously via a `Response` signal.
///
/// Portal calls (CreateSession/SelectDevices/Start) return immediately with a
/// request handle and deliver their real result later on the
/// `org.freedesktop.portal.Request.Response` signal. We predict the request
/// object path from our unique bus name and the `handle_token`, subscribe
/// BEFORE issuing the call (so we cannot miss the signal), then await it.
///
/// `call_fut` is the (not-yet-awaited) method-call future; futures are lazy, so
/// creating it in the caller does not send the message until we await it here.
async fn await_response(
    conn: &Connection,
    handle_token: &str,
    call_fut: impl Future<Output = zbus::Result<OwnedObjectPath>>,
) -> Result<HashMap<String, OwnedValue>, OutputError> {
    let unique = conn
        .unique_name()
        .ok_or_else(|| OutputError::PortalFailed("no unique bus name".to_string()))?;
    let sender = unique.as_str().trim_start_matches(':').replace('.', "_");
    let request_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{handle_token}");

    let request = Proxy::new(conn, PORTAL_DEST, request_path, REQUEST_IFACE)
        .await
        .map_err(|e| OutputError::PortalFailed(format!("request proxy: {e}")))?;
    let mut responses = request
        .receive_signal("Response")
        .await
        .map_err(|e| OutputError::PortalFailed(format!("subscribe Response: {e}")))?;

    call_fut
        .await
        .map_err(|e| OutputError::PortalFailed(format!("portal call: {e}")))?;

    let message = responses
        .next()
        .await
        .ok_or_else(|| OutputError::PortalFailed("no Response signal".to_string()))?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = message
        .body()
        .deserialize()
        .map_err(|e| OutputError::PortalFailed(format!("decode Response: {e}")))?;
    if code != 0 {
        return Err(OutputError::PortalFailed(format!(
            "portal request denied (code {code})"
        )));
    }
    Ok(results)
}

fn get_string(results: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    results
        .get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
}

fn get_u32(results: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    results.get(key).and_then(|v| u32::try_from(v.clone()).ok())
}

/// Open a fresh RemoteDesktop keyboard session: create -> select keyboard
/// (persist, presenting a cached restore token if any) -> start. Shows the
/// compositor consent dialog on first use; the persisted restore token keeps
/// later runs silent.
async fn open_session() -> Result<PortalSession, OutputError> {
    let conn = Connection::session()
        .await
        .map_err(|e| OutputError::PortalFailed(format!("connect to session bus: {e}")))?;
    let rd = Proxy::new(&conn, PORTAL_DEST, PORTAL_PATH, RD_IFACE)
        .await
        .map_err(|e| OutputError::PortalFailed(format!("remote desktop proxy: {e}")))?;

    // CreateSession
    let token = next_token("wtc");
    let session_token = next_token("wts");
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("session_handle_token", Value::from(session_token.as_str()));
    let results = await_response(
        &conn,
        &token,
        rd.call::<_, _, OwnedObjectPath>("CreateSession", &(options,)),
    )
    .await?;
    let session_str = get_string(&results, "session_handle").ok_or_else(|| {
        OutputError::PortalFailed("CreateSession returned no session_handle".into())
    })?;
    let session = OwnedObjectPath::try_from(session_str)
        .map_err(|e| OutputError::PortalFailed(format!("bad session path: {e}")))?;

    // SelectDevices (keyboard, persist, present cached restore token if any)
    let token = next_token("wtd");
    let cached = read_restore_token();
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("types", Value::from(DEVICE_KEYBOARD));
    options.insert("persist_mode", Value::from(PERSIST_UNTIL_REVOKED));
    if let Some(ref restore) = cached {
        options.insert("restore_token", Value::from(restore.as_str()));
    }
    await_response(
        &conn,
        &token,
        rd.call::<_, _, OwnedObjectPath>("SelectDevices", &(session.as_ref(), options)),
    )
    .await?;

    // Start
    let token = next_token("wtst");
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    let results = await_response(
        &conn,
        &token,
        rd.call::<_, _, OwnedObjectPath>("Start", &(session.as_ref(), "", options)),
    )
    .await?;

    // The portal echoes the granted device bitmask as "devices". If the
    // keyboard bit is absent, NotifyKeyboardKeysym would be refused.
    let granted = get_u32(&results, "devices").unwrap_or(0);
    if granted & DEVICE_KEYBOARD == 0 {
        return Err(OutputError::PortalFailed(
            "compositor granted a keyboard-less session".to_string(),
        ));
    }
    if let Some(restore) = get_string(&results, "restore_token") {
        save_restore_token(&restore);
    }

    tracing::info!("portal: RemoteDesktop keyboard session started");
    Ok(PortalSession { conn, session })
}

/// Map a Unicode scalar to an X11/xkbcommon keysym. ASCII and Latin-1 printable
/// ranges map directly; everything else uses the 0x01000000 Unicode-keysym
/// convention. Layout-independent, so no XKB handling is needed.
fn keysym_for_char(ch: char) -> i32 {
    let cp = ch as u32;
    let sym = if (0x20..=0x7E).contains(&cp) || (0xA0..=0xFF).contains(&cp) {
        cp
    } else {
        0x0100_0000 + cp
    };
    sym as i32
}

/// Send one key state transition via NotifyKeyboardKeysym.
async fn notify(
    rd: &Proxy<'_>,
    session: &ObjectPath<'_>,
    keysym: i32,
    state: u32,
) -> Result<(), OutputError> {
    let options: HashMap<&str, Value> = HashMap::new();
    rd.call::<_, _, ()>("NotifyKeyboardKeysym", &(session, options, keysym, state))
        .await
        .map_err(|e| OutputError::PortalFailed(e.to_string()))
}

/// Press then release a single keysym.
async fn tap(rd: &Proxy<'_>, session: &ObjectPath<'_>, keysym: i32) -> Result<(), OutputError> {
    notify(rd, session, keysym, KEY_PRESSED).await?;
    notify(rd, session, keysym, KEY_RELEASED).await
}

/// Probe whether the RemoteDesktop portal is present and offers keyboard
/// input, WITHOUT creating a session (no consent dialog). Used by the driver's
/// availability check and by `voxtype setup` status.
pub async fn probe_available() -> bool {
    let Ok(conn) = Connection::session().await else {
        return false;
    };
    let Ok(rd) = Proxy::new(&conn, PORTAL_DEST, PORTAL_PATH, RD_IFACE).await else {
        return false;
    };
    matches!(
        rd.get_property::<u32>("AvailableDeviceTypes").await,
        Ok(types) if types & DEVICE_KEYBOARD != 0
    )
}

/// Portal-based text output holding one persistent RemoteDesktop session.
pub struct PortalOutput {
    auto_submit: bool,
    append_text: Option<String>,
    type_delay_ms: u32,
    pre_type_delay_ms: u32,
    shift_enter_newlines: bool,
}

impl PortalOutput {
    pub fn new(
        auto_submit: bool,
        append_text: Option<String>,
        type_delay_ms: u32,
        pre_type_delay_ms: u32,
        shift_enter_newlines: bool,
    ) -> Self {
        Self {
            auto_submit,
            append_text,
            type_delay_ms,
            pre_type_delay_ms,
            shift_enter_newlines,
        }
    }

    async fn type_str(
        &self,
        rd: &Proxy<'_>,
        session: &ObjectPath<'_>,
        text: &str,
    ) -> Result<(), OutputError> {
        for ch in text.chars() {
            let keysym = if ch == '\n' {
                KEYSYM_RETURN
            } else {
                keysym_for_char(ch)
            };
            tap(rd, session, keysym).await?;
            if self.type_delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.type_delay_ms as u64)).await;
            }
        }
        Ok(())
    }

    async fn shift_return(
        &self,
        rd: &Proxy<'_>,
        session: &ObjectPath<'_>,
    ) -> Result<(), OutputError> {
        notify(rd, session, KEYSYM_SHIFT_L, KEY_PRESSED).await?;
        tap(rd, session, KEYSYM_RETURN).await?;
        notify(rd, session, KEYSYM_SHIFT_L, KEY_RELEASED).await
    }

    /// Emit the full payload (text, optional appended text, optional Enter) on
    /// an already-open session.
    async fn emit(
        &self,
        rd: &Proxy<'_>,
        session: &ObjectPath<'_>,
        text: &str,
    ) -> Result<(), OutputError> {
        if self.shift_enter_newlines && text.contains('\n') {
            let segments: Vec<&str> = text.split('\n').collect();
            let last = segments.len() - 1;
            for (i, segment) in segments.iter().enumerate() {
                if !segment.is_empty() {
                    self.type_str(rd, session, segment).await?;
                }
                if i < last {
                    self.shift_return(rd, session).await?;
                }
            }
        } else {
            self.type_str(rd, session, text).await?;
        }

        if let Some(ref append) = self.append_text {
            self.type_str(rd, session, append).await?;
        }
        if self.auto_submit {
            tap(rd, session, KEYSYM_RETURN).await?;
        }
        Ok(())
    }

    /// Emit on the given session, building a RemoteDesktop proxy on its
    /// connection.
    async fn emit_on(&self, sess: &PortalSession, text: &str) -> Result<(), OutputError> {
        let rd = Proxy::new(&sess.conn, PORTAL_DEST, PORTAL_PATH, RD_IFACE)
            .await
            .map_err(|e| OutputError::PortalFailed(format!("remote desktop proxy: {e}")))?;
        self.emit(&rd, &sess.session.as_ref(), text).await
    }
}

#[async_trait::async_trait]
impl TextOutput for PortalOutput {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() {
            return Ok(());
        }
        if self.pre_type_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.pre_type_delay_ms as u64)).await;
        }

        let mut guard = session_cell().lock().await;
        if guard.is_none() {
            *guard = Some(open_session().await?);
        }

        // First attempt on the warm session.
        let first = self.emit_on(guard.as_ref().unwrap(), text).await;
        if first.is_ok() {
            return Ok(());
        }

        // The portal may have closed the session out from under us (suspend,
        // revoke, portal restart). Drop it, reopen once, and retry.
        tracing::warn!(
            "portal: typing failed ({}); reopening session and retrying",
            first.unwrap_err()
        );
        *guard = None;
        let reopened = open_session().await?;
        let retry = self.emit_on(&reopened, text).await;
        *guard = Some(reopened);
        retry
    }

    async fn is_available(&self) -> bool {
        // A live session means it is available (and avoids a needless probe).
        if session_cell().lock().await.is_some() {
            return true;
        }
        probe_available().await
    }

    fn name(&self) -> &'static str {
        "portal"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysym_ascii_maps_directly() {
        assert_eq!(keysym_for_char('A'), 0x41);
        assert_eq!(keysym_for_char(' '), 0x20);
        assert_eq!(keysym_for_char('~'), 0x7E);
    }

    #[test]
    fn keysym_latin1_maps_directly() {
        assert_eq!(keysym_for_char('\u{E9}'), 0xE9); // é
        assert_eq!(keysym_for_char('\u{FF}'), 0xFF); // ÿ
    }

    #[test]
    fn keysym_beyond_latin1_uses_unicode_convention() {
        assert_eq!(keysym_for_char('\u{4F60}'), 0x0100_0000 + 0x4F60); // 你
        assert_eq!(keysym_for_char('\u{1F389}'), 0x0100_0000 + 0x1F389); // 🎉
    }

    #[test]
    fn newline_falls_through_keysym_helper() {
        // keysym_for_char has no special case for '\n': 0x0A is below the
        // printable ranges, so it takes the Unicode-keysym branch. The typing
        // path never relies on this - type_str maps '\n' to Return first.
        assert_eq!(keysym_for_char('\n'), 0x0100_0000 + 0x0A);
    }

    #[test]
    fn name_is_portal() {
        let out = PortalOutput::new(false, None, 0, 0, false);
        assert_eq!(out.name(), "portal");
    }
}
