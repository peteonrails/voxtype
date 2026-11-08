//! Meeting transcript/status broadcast for the meeting HUD.
//!
//! While a meeting is active, the daemon publishes meeting lifecycle and
//! transcript events as newline-delimited JSON (NDJSON) over a Unix socket
//! at `$XDG_RUNTIME_DIR/voxtype/meeting.sock`. Modeled on
//! `crate::audio::levels` (self-healing accept/broadcast tasks, drop-slow-
//! subscribers respawn watchdog — see issue #391), with two differences
//! suited to this consumer:
//!
//! - **NDJSON, not packed binary.** Meeting events are low-rate (about one
//!   per `chunk_duration_secs`, default 30s) and structured; JSON keeps the
//!   wire format legible to QML and shell scripts (`socat | jq`) without a
//!   binary schema to maintain.
//! - **Snapshot on connect.** A HUD that attaches mid-meeting must be
//!   correct immediately, so every new subscriber's first line is a `hello`
//!   event carrying the full transcript backlog — the daemon is the only
//!   source of truth, so a subscriber never needs to fall back to a DB read.
//!
//! ## Wire format
//!
//! One JSON object per line (`\n`-terminated), server to client only.
//! Segment shapes reuse [`TranscriptSegment`]'s existing `Serialize` derive
//! as-is: snake_case, matching `transcript.json` on disk (*not* the
//! camelCase that `meeting export -f json` emits — two schemas already
//! exist and a third would be worse).
//!
//! ```text
//! {"event":"hello","status":"recording","meeting_id":"...","title":"...","started_at":"...","chunk_count":7,"diarization":{"enabled":true,"backend":"simple"},"segments":[...]}
//! {"event":"segments","chunk_id":8,"segments":[...]}
//! {"event":"status","status":"paused"}
//! {"event":"error","message":"..."}
//! ```
//!
//! `speakers` and `summary` events belong to later phases (live speaker
//! naming and auto-summarization) and are not emitted yet.
//!
//! Delivery is lossy and best-effort, like `audio::levels`: a subscriber
//! whose queue fills up gets dropped rather than allowed to back-pressure
//! the daemon. A newly accepted connection is registered with the
//! broadcaster *before* its `hello` line is read and sent, so it can never
//! miss an event to a race — at worst it sees a slightly stale `hello`
//! immediately followed by an event that was already reflected in it.

use crate::config::Config;
use crate::meeting::data::TranscriptSegment;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Default path for the meeting-events socket.
pub fn default_socket_path() -> PathBuf {
    Config::runtime_dir().join("meeting.sock")
}

/// Per-subscriber bounded queue. Events are low-rate (about one per
/// `chunk_duration_secs`, default 30s); 64 gives ample headroom before a
/// genuinely stuck consumer gets disconnected.
const SUBSCRIBER_QUEUE_DEPTH: usize = 64;

/// Diarization info surfaced in the `hello` snapshot.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiarizationInfo {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
}

/// The daemon's current view of the active (or most recently ended) meeting,
/// used to build the `hello` snapshot sent to each new subscriber.
///
/// Owned by `Daemon` in `src/daemon.rs`, which is the sole writer; this
/// struct just carries the fields, it doesn't fetch them.
#[derive(Debug, Clone, Default)]
pub struct MeetingHudState {
    /// `idle`, `recording`, `paused`, or `finalizing`.
    pub status: String,
    pub meeting_id: Option<String>,
    pub title: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub chunk_count: u32,
    pub diarization: DiarizationInfo,
    pub segments: Vec<TranscriptSegment>,
}

impl MeetingHudState {
    /// The reset state: no meeting, nothing to show.
    pub fn idle() -> Self {
        Self {
            status: "idle".to_string(),
            ..Default::default()
        }
    }
}

/// One line on the wire. Internally tagged on `event` so consumers can
/// `match` on a single field without a second parse pass.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum WireEvent {
    Hello {
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        meeting_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        started_at: Option<DateTime<Utc>>,
        chunk_count: u32,
        diarization: DiarizationInfo,
        segments: Vec<TranscriptSegment>,
    },
    Segments {
        chunk_id: u32,
        segments: Vec<TranscriptSegment>,
    },
    Status {
        status: String,
    },
    Error {
        message: String,
    },
}

impl WireEvent {
    fn hello(state: &MeetingHudState) -> Self {
        WireEvent::Hello {
            status: state.status.clone(),
            meeting_id: state.meeting_id.clone(),
            title: state.title.clone(),
            started_at: state.started_at,
            chunk_count: state.chunk_count,
            diarization: state.diarization.clone(),
            segments: state.segments.clone(),
        }
    }

    /// Serialize to one NDJSON line, trailing newline included.
    fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_else(|e| {
            format!(
                r#"{{"event":"error","message":"meeting event serialization failure: {}"}}"#,
                e
            )
        });
        s.push('\n');
        s
    }
}

/// Hub for distributing meeting events to subscribers.
///
/// Mirrors `audio::levels::LevelHub`'s self-healing design: if the internal
/// accept or broadcast task ever exits unexpectedly, a watchdog respawns
/// both and rebinds the socket. See that module's doc comment and issue
/// #391 for the rationale; it isn't repeated here.
#[derive(Clone)]
pub struct MeetingEventHub {
    inner: Arc<HubInner>,
}

struct HubState {
    broadcast_tx: mpsc::Sender<String>,
}

struct HubInner {
    state: RwLock<Arc<HubState>>,
    socket_path: PathBuf,
    /// Current `hello` line, shared with the accept loop so every newly
    /// accepted connection reads the live snapshot without round-tripping
    /// through the daemon. Independent of `state` so it survives a
    /// watchdog respawn untouched.
    hello: Arc<RwLock<String>>,
}

impl HubInner {
    fn current(&self) -> Arc<HubState> {
        self.state.read().expect("meeting hub state lock").clone()
    }

    fn replace(&self, new_state: Arc<HubState>) {
        let mut guard = self.state.write().expect("meeting hub state lock");
        *guard = new_state;
    }
}

impl MeetingEventHub {
    /// Bind a Unix socket and start the broadcaster task. The initial
    /// `hello` snapshot is the idle state; call [`MeetingEventHub::update_snapshot`]
    /// once real meeting state exists.
    pub async fn start(socket_path: PathBuf) -> io::Result<Self> {
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let hello = Arc::new(RwLock::new(
            WireEvent::hello(&MeetingHudState::idle()).to_line(),
        ));
        let (state, accept_h, broadcast_h) = bind_and_spawn(&socket_path, hello.clone())?;
        tracing::info!("Meeting event socket listening at {:?}", socket_path);

        let inner = Arc::new(HubInner {
            state: RwLock::new(state),
            socket_path: socket_path.clone(),
            hello,
        });

        spawn_watchdog(inner.clone(), accept_h, broadcast_h);

        Ok(Self { inner })
    }

    /// Path of the bound Unix socket.
    pub fn socket_path(&self) -> &std::path::Path {
        &self.inner.socket_path
    }

    /// Best-effort cleanup of the socket file. Called on shutdown.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.inner.socket_path);
    }

    /// Replace the `hello` snapshot sent to every new subscriber from now
    /// on. Does not itself notify already-connected subscribers — pair with
    /// [`MeetingEventHub::publish_status`] or
    /// [`MeetingEventHub::publish_segments`] when live subscribers also need
    /// to see the change.
    pub fn update_snapshot(&self, state: &MeetingHudState) {
        let line = WireEvent::hello(state).to_line();
        *self.inner.hello.write().expect("meeting hub hello lock") = line;
    }

    /// Broadcast a `segments` event to currently-connected subscribers.
    pub fn publish_segments(&self, chunk_id: u32, segments: &[TranscriptSegment]) {
        self.publish(WireEvent::Segments {
            chunk_id,
            segments: segments.to_vec(),
        });
    }

    /// Broadcast a `status` event.
    pub fn publish_status(&self, status: &str) {
        self.publish(WireEvent::Status {
            status: status.to_string(),
        });
    }

    /// Broadcast an `error` event.
    pub fn publish_error(&self, message: &str) {
        self.publish(WireEvent::Error {
            message: message.to_string(),
        });
    }

    fn publish(&self, event: WireEvent) {
        let line = event.to_line();
        let state = self.inner.current();
        let _ = state.broadcast_tx.try_send(line);
    }
}

/// Bind the listener and spawn the accept + broadcast tasks. Returns the
/// fresh `HubState` plus both `JoinHandle`s so the watchdog can supervise
/// them. Called once from [`MeetingEventHub::start`] and again from the
/// watchdog's respawn path after a task exit.
fn bind_and_spawn(
    socket_path: &std::path::Path,
    hello: Arc<RwLock<String>>,
) -> io::Result<(Arc<HubState>, JoinHandle<()>, JoinHandle<()>)> {
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }

    let listener = UnixListener::bind(socket_path)?;

    // Fan-in from the daemon; 256 lines of buffered headroom is generous
    // for an event that fires roughly once per chunk_duration_secs.
    let (broadcast_tx, broadcast_rx) = mpsc::channel::<String>(256);
    let state = Arc::new(HubState { broadcast_tx });
    let (sub_tx, sub_rx) = mpsc::unbounded_channel::<SubscriberSlot>();

    let accept_handle = tokio::spawn(async move {
        run_accept_loop(listener, sub_tx, hello).await;
    });
    let broadcast_handle = tokio::spawn(async move {
        run_broadcast_loop(broadcast_rx, sub_rx).await;
    });

    Ok((state, accept_handle, broadcast_handle))
}

/// Watchdog task. Lives for the lifetime of the daemon.
///
/// Awaits both internal task handles via `select!`. The first one to
/// resolve triggers a loud ERROR log and a respawn of the entire hub. See
/// `audio::levels::spawn_watchdog` for the full rationale (issue #391) —
/// this is the same pattern applied to the meeting socket.
fn spawn_watchdog(
    inner: Arc<HubInner>,
    mut accept_handle: JoinHandle<()>,
    mut broadcast_handle: JoinHandle<()>,
) {
    tokio::spawn(async move {
        loop {
            let exit = tokio::select! {
                res = &mut accept_handle => ("accept", res),
                res = &mut broadcast_handle => ("broadcast", res),
            };

            let (which, res) = exit;
            match res {
                Ok(()) => tracing::error!(
                    task = which,
                    "Meeting event {} loop exited unexpectedly (no panic). \
                     Respawning the MeetingEventHub. See issue #391.",
                    which
                ),
                Err(join_err) => tracing::error!(
                    task = which,
                    err = %join_err,
                    "Meeting event {} loop panicked or was cancelled. \
                     Respawning the MeetingEventHub. See issue #391.",
                    which
                ),
            }

            accept_handle.abort();
            broadcast_handle.abort();

            loop {
                match bind_and_spawn(&inner.socket_path, inner.hello.clone()) {
                    Ok((new_state, ah, bh)) => {
                        inner.replace(new_state);
                        tracing::info!("Meeting event socket rebound at {:?}", inner.socket_path);
                        accept_handle = ah;
                        broadcast_handle = bh;
                        break;
                    }
                    Err(e) => {
                        tracing::error!(
                            err = %e,
                            "Failed to rebind meeting event socket; will retry"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        }
    });
}

/// One subscriber's per-connection mailbox.
struct SubscriberSlot {
    tx: mpsc::Sender<String>,
}

async fn run_accept_loop(
    listener: UnixListener,
    sub_tx: mpsc::UnboundedSender<SubscriberSlot>,
    hello: Arc<RwLock<String>>,
) {
    loop {
        #[cfg(test)]
        {
            if tests::should_panic_now() {
                panic!("events::run_accept_loop test-injected panic");
            }
        }

        match listener.accept().await {
            Ok((stream, _addr)) => {
                let (tx, rx) = mpsc::channel::<String>(SUBSCRIBER_QUEUE_DEPTH);
                let slot = SubscriberSlot { tx };
                // Register before reading the hello snapshot: a slightly
                // stale hello is fine, a missed event is not.
                if sub_tx.send(slot).is_err() {
                    drop(stream);
                    break;
                }

                let hello_line = hello.read().expect("meeting hub hello lock").clone();
                tokio::spawn(async move {
                    run_subscriber_writer(stream, hello_line, rx).await;
                });
            }
            Err(e) => {
                tracing::warn!("Meeting event socket accept error: {}", e);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

async fn run_subscriber_writer(
    mut stream: UnixStream,
    hello_line: String,
    mut rx: mpsc::Receiver<String>,
) {
    if let Err(e) = stream.write_all(hello_line.as_bytes()).await {
        tracing::trace!("Meeting event subscriber write error (hello): {}", e);
        return;
    }
    while let Some(line) = rx.recv().await {
        if let Err(e) = stream.write_all(line.as_bytes()).await {
            tracing::trace!("Meeting event subscriber write error: {}", e);
            break;
        }
    }
    let _ = stream.shutdown().await;
}

async fn run_broadcast_loop(
    mut line_rx: mpsc::Receiver<String>,
    mut new_subs: mpsc::UnboundedReceiver<SubscriberSlot>,
) {
    let mut subscribers: Vec<mpsc::Sender<String>> = Vec::new();

    loop {
        tokio::select! {
            slot = new_subs.recv() => {
                match slot {
                    Some(slot) => subscribers.push(slot.tx),
                    None => break,
                }
            }
            line = line_rx.recv() => {
                match line {
                    Some(line) => {
                        while let Ok(slot) = new_subs.try_recv() {
                            subscribers.push(slot.tx);
                        }
                        if subscribers.is_empty() {
                            continue;
                        }
                        subscribers.retain(|tx| tx.try_send(line.clone()).is_ok());
                    }
                    None => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tempfile::TempDir;
    use tokio::io::{AsyncBufReadExt, BufReader};

    static ACCEPT_PANIC_NEXT: AtomicBool = AtomicBool::new(false);

    pub(super) fn should_panic_now() -> bool {
        ACCEPT_PANIC_NEXT.swap(false, Ordering::SeqCst)
    }

    fn temp_socket_dir() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("create tempdir");
        let path = dir.path().join("meeting.sock");
        (dir, path)
    }

    async fn read_line(stream: &mut UnixStream) -> serde_json::Value {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            reader.read_line(&mut line),
        )
        .await
        .expect("timed out reading line")
        .expect("read line");
        serde_json::from_str(&line).expect("valid JSON line")
    }

    #[tokio::test]
    async fn hello_sent_on_connect() {
        let (_tmp, path) = temp_socket_dir();
        let hub = MeetingEventHub::start(path.clone())
            .await
            .expect("start hub");

        let mut client = UnixStream::connect(&path).await.expect("connect");
        let value = read_line(&mut client).await;

        assert_eq!(value["event"], "hello");
        assert_eq!(value["status"], "idle");
        assert_eq!(value["chunk_count"], 0);

        hub.cleanup();
    }

    #[tokio::test]
    async fn update_snapshot_changes_hello_for_new_subscribers() {
        let (_tmp, path) = temp_socket_dir();
        let hub = MeetingEventHub::start(path.clone())
            .await
            .expect("start hub");

        let mut state = MeetingHudState::idle();
        state.status = "recording".to_string();
        state.meeting_id = Some("abc-123".to_string());
        state.title = Some("Standup".to_string());
        state.chunk_count = 3;
        state.diarization = DiarizationInfo {
            enabled: true,
            backend: Some("simple".to_string()),
        };
        hub.update_snapshot(&state);

        let mut client = UnixStream::connect(&path).await.expect("connect");
        let value = read_line(&mut client).await;

        assert_eq!(value["event"], "hello");
        assert_eq!(value["status"], "recording");
        assert_eq!(value["meeting_id"], "abc-123");
        assert_eq!(value["title"], "Standup");
        assert_eq!(value["chunk_count"], 3);
        assert_eq!(value["diarization"]["enabled"], true);
        assert_eq!(value["diarization"]["backend"], "simple");

        hub.cleanup();
    }

    #[tokio::test]
    async fn publish_status_reaches_connected_subscriber() {
        let (_tmp, path) = temp_socket_dir();
        let hub = MeetingEventHub::start(path.clone())
            .await
            .expect("start hub");

        let mut client = UnixStream::connect(&path).await.expect("connect");
        let _hello = read_line(&mut client).await;

        // Give the accept loop a moment to register the subscriber with the
        // broadcaster before we publish.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        hub.publish_status("paused");

        let value = read_line(&mut client).await;
        assert_eq!(value["event"], "status");
        assert_eq!(value["status"], "paused");

        hub.cleanup();
    }

    #[tokio::test]
    async fn publish_segments_reaches_connected_subscriber() {
        let (_tmp, path) = temp_socket_dir();
        let hub = MeetingEventHub::start(path.clone())
            .await
            .expect("start hub");

        let mut client = UnixStream::connect(&path).await.expect("connect");
        let _hello = read_line(&mut client).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let segment = TranscriptSegment::new(0, 0, 1000, "hello there".to_string(), 0);
        hub.publish_segments(0, &[segment]);

        let value = read_line(&mut client).await;
        assert_eq!(value["event"], "segments");
        assert_eq!(value["chunk_id"], 0);
        assert_eq!(value["segments"][0]["text"], "hello there");

        hub.cleanup();
    }

    #[test]
    fn meeting_hud_state_idle_defaults() {
        let state = MeetingHudState::idle();
        assert_eq!(state.status, "idle");
        assert!(state.meeting_id.is_none());
        assert!(state.segments.is_empty());
        assert_eq!(state.chunk_count, 0);
    }
}
