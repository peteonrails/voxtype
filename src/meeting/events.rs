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
//! {"event":"hello","status":"recording","meeting_id":"...","title":"...","started_at":"...","chunk_count":7,"diarization":{"enabled":true,"backend":"simple"},"segments":[...],"recording_notice":false}
//! {"event":"segments","chunk_id":8,"segments":[...]}
//! {"event":"status","status":"paused","recording_notice":false}
//! {"event":"title_changed","meeting_id":"...","title":"Pete <> Chad"}
//! {"event":"error","message":"..."}
//! {"event":"error","message":"Your microphone has sound, but nothing you say is transcribed: ...","code":"track_not_transcribed"}
//! ```
//!
//! After a meeting stops, auto-summary reports on the meeting it just
//! finished, naming it, without changing the `hello` snapshot:
//!
//! ```text
//! {"event":"status","status":"summarizing","meeting_id":"...","recording_notice":false}
//! {"event":"summary_ready","meeting_id":"...","agent":"claude","agent_name":"Claude","path":".../summary.md","summary":{...}}
//! {"event":"error","message":"Summary failed: ...","meeting_id":"...","code":"consent_required"}
//! ```
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
use std::sync::atomic::{AtomicUsize, Ordering};
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
    /// The meeting's echo handling: `webrtc`, `gtcrn` or `none`
    pub echo_cancel: Option<String>,
    pub segments: Vec<TranscriptSegment>,
    /// The recording-consent notice has not been acknowledged yet
    /// (`voxtype meeting notice --ack`), so views should show it.
    pub recording_notice: bool,
    /// The recording announcement's outcome, when this meeting announces
    pub announcement: Option<crate::meeting::announce::Status>,
    /// This meeting announces the recording, so views word the consent
    /// notice accordingly from the start, before the announcement plays
    pub announces: bool,
}

impl MeetingHudState {
    /// The reset state: no meeting, nothing to show.
    pub fn idle() -> Self {
        Self {
            status: "idle".to_string(),
            ..Default::default()
        }
    }

    /// Add a chunk's segments to the backlog, kept in order of start time
    /// like the saved transcript, so the `hello` snapshot interleaves the
    /// two tracks instead of listing them a chunk at a time.
    pub fn push_segments(&mut self, segments: &[TranscriptSegment]) {
        for segment in segments {
            let at = self
                .segments
                .partition_point(|s| s.start_ms <= segment.start_ms);
            self.segments.insert(at, segment.clone());
        }
    }

    /// Move the backlog's segments from speaker `from` to `to`.
    pub fn rename_speaker(&mut self, from: &str, to: &str) {
        for segment in &mut self.segments {
            if segment.speaker_id.as_deref() == Some(from) {
                segment.speaker_id = Some(to.to_string());
            }
            if segment.original_speaker.as_deref() == Some(from) {
                segment.original_speaker = Some(to.to_string());
            }
        }
    }

    /// Replace the backlog's copy of a segment that changed after it was
    /// sent, or put back one that was deleted and is now restored.
    pub fn update_segment(&mut self, segment: &TranscriptSegment) {
        match self.segments.iter_mut().find(|s| s.id == segment.id) {
            Some(existing) => *existing = segment.clone(),
            None => self.push_segments(std::slice::from_ref(segment)),
        }
    }

    /// Drop a deleted segment from the backlog.
    pub fn remove_segment(&mut self, segment_id: u32) {
        self.segments.retain(|s| s.id != segment_id);
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
        #[serde(skip_serializing_if = "Option::is_none")]
        echo_cancel: Option<String>,
        segments: Vec<TranscriptSegment>,
        recording_notice: bool,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        announces: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        announcement: Option<serde_json::Value>,
    },
    /// The stored summary changed without being regenerated (action-item
    /// owners followed a speaker rename): views showing it read it again.
    SummaryUpdated {
        meeting_id: String,
    },
    /// The recording announcement changed: waiting for the call, played
    /// into it, played locally only, or failed.
    Announcement {
        #[serde(skip_serializing_if = "Option::is_none")]
        meeting_id: Option<String>,
        state: crate::meeting::announce::State,
        #[serde(skip_serializing_if = "Option::is_none")]
        app: Option<String>,
        detail: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        text: String,
    },
    Segments {
        chunk_id: u32,
        segments: Vec<TranscriptSegment>,
    },
    SpeakerRenamed {
        from: String,
        to: String,
    },
    SegmentUpdated {
        segment: TranscriptSegment,
    },
    SegmentDeleted {
        meeting_id: String,
        segment_id: u32,
    },
    TitleChanged {
        meeting_id: String,
        title: String,
    },
    NotesUpdated {
        meeting_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        updated_at: Option<DateTime<Utc>>,
    },
    Status {
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        meeting_id: Option<String>,
        recording_notice: bool,
    },
    ImportProgress(crate::meeting::import::Progress),
    Playback {
        meeting_id: String,
        track: String,
        state: String,
        from_ms: u64,
    },
    Levels(crate::meeting::levels::Levels),
    Soundcheck(crate::meeting::soundcheck::SoundcheckEvent),
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        meeting_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        /// The assistant backend that failed ("Claude", "Local (llama3)").
        #[serde(skip_serializing_if = "Option::is_none")]
        backend: Option<String>,
    },
    /// Meetings the daemon never finished (it crashed or was killed while
    /// they recorded), for a recovery card. Empty once none are left.
    Recoverable {
        meetings: Vec<crate::meeting::recover::RecoverableMeeting>,
    },
    SummaryReady {
        meeting_id: String,
        agent: String,
        agent_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
        summary: crate::meeting::data::MeetingSummary,
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
            echo_cancel: state.echo_cancel.clone(),
            segments: state.segments.clone(),
            recording_notice: state.recording_notice,
            announces: state.announces,
            announcement: state
                .announcement
                .as_ref()
                .map(crate::meeting::announce::Status::to_json),
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
    /// watchdog respawn untouched. It is the greeting's lines together.
    hello: Arc<RwLock<String>>,
    /// Connected subscribers, kept by the broadcast loop. A client that has
    /// gone counts until the next event fails to reach it.
    subscribers: Arc<AtomicUsize>,
    greeting: std::sync::Mutex<Greeting>,
    /// The snapshot's `recording_notice`, repeated on every `status` event.
    recording_notice: std::sync::atomic::AtomicBool,
}

/// What a new subscriber is sent first: the `hello` snapshot, then the
/// `recoverable` meetings while there are any, so a panel that connects
/// late still shows its recovery card.
#[derive(Default)]
struct Greeting {
    hello: String,
    recoverable: String,
}

impl HubInner {
    fn set_greeting(&self, change: impl FnOnce(&mut Greeting)) {
        let mut greeting = self.greeting.lock().expect("meeting hub greeting lock");
        change(&mut greeting);
        *self.hello.write().expect("meeting hub hello lock") =
            format!("{}{}", greeting.hello, greeting.recoverable);
    }
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

        let idle = WireEvent::hello(&MeetingHudState::idle()).to_line();
        let hello = Arc::new(RwLock::new(idle.clone()));
        let subscribers = Arc::new(AtomicUsize::new(0));
        let (state, accept_h, broadcast_h) =
            bind_and_spawn(&socket_path, hello.clone(), subscribers.clone())?;
        tracing::info!("Meeting event socket listening at {:?}", socket_path);

        let inner = Arc::new(HubInner {
            state: RwLock::new(state),
            socket_path: socket_path.clone(),
            hello,
            greeting: std::sync::Mutex::new(Greeting {
                hello: idle,
                recoverable: String::new(),
            }),
            subscribers,
            recording_notice: std::sync::atomic::AtomicBool::new(false),
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
        self.inner
            .recording_notice
            .store(state.recording_notice, std::sync::atomic::Ordering::Relaxed);
        let line = WireEvent::hello(state).to_line();
        self.inner.set_greeting(|g| g.hello = line);
    }

    /// Announce the meetings the daemon never finished. New subscribers get
    /// the list right after `hello` while it is not empty; an empty list is
    /// sent to those connected, so a recovery card can go away.
    pub fn publish_recoverable(&self, meetings: &[crate::meeting::recover::RecoverableMeeting]) {
        let event = WireEvent::Recoverable {
            meetings: meetings.to_vec(),
        };
        let line = event.to_line();
        self.inner.set_greeting(|g| {
            g.recoverable = if meetings.is_empty() {
                String::new()
            } else {
                line
            }
        });
        self.publish(event);
    }

    /// Whether the recording-consent notice is still unacknowledged, as the
    /// last snapshot said.
    fn recording_notice(&self) -> bool {
        self.inner
            .recording_notice
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Broadcast a `segments` event to currently-connected subscribers.
    pub fn publish_segments(&self, chunk_id: u32, segments: &[TranscriptSegment]) {
        self.publish(WireEvent::Segments {
            chunk_id,
            segments: segments.to_vec(),
        });
    }

    /// Broadcast a `speaker_renamed` event: segments already sent under
    /// speaker `from` now belong to `to`.
    pub fn publish_speaker_renamed(&self, from: &str, to: &str) {
        self.publish(WireEvent::SpeakerRenamed {
            from: from.to_string(),
            to: to.to_string(),
        });
    }

    /// Broadcast a `segment_updated` event: one segment already sent now
    /// reads differently, as after `meeting reassign`.
    pub fn publish_segment_updated(&self, segment: &TranscriptSegment) {
        self.publish(WireEvent::SegmentUpdated {
            segment: segment.clone(),
        });
    }

    /// Broadcast a `segment_deleted` event: a segment already sent was
    /// deleted with `meeting delete-line`.
    pub fn publish_segment_deleted(&self, meeting_id: &str, segment_id: u32) {
        self.publish(WireEvent::SegmentDeleted {
            meeting_id: meeting_id.to_string(),
            segment_id,
        });
    }

    /// Broadcast a `notes_updated` event: the meeting's notes.md changed.
    pub fn publish_notes_updated(&self, notes: &crate::meeting::notes::MeetingNotes) {
        self.publish(WireEvent::NotesUpdated {
            meeting_id: notes.meeting_id.clone(),
            updated_at: notes.updated_at,
        });
    }

    /// Broadcast a `title_changed` event: the meeting was retitled, by hand
    /// or from the calendar.
    pub fn publish_title_changed(&self, meeting_id: &str, title: &str) {
        self.publish(WireEvent::TitleChanged {
            meeting_id: meeting_id.to_string(),
            title: title.to_string(),
        });
    }

    /// Broadcast a `summary_updated` event.
    pub fn publish_summary_updated(&self, meeting_id: &str) {
        self.publish(WireEvent::SummaryUpdated {
            meeting_id: meeting_id.to_string(),
        });
    }

    /// Broadcast an `announcement` event.
    pub fn publish_announcement(
        &self,
        meeting_id: Option<&str>,
        status: &crate::meeting::announce::Status,
    ) {
        self.publish(WireEvent::Announcement {
            meeting_id: meeting_id.map(str::to_string),
            state: status.state,
            app: status.app.clone(),
            detail: status.detail.clone(),
            note: status.note.clone(),
            text: status.describe(),
        });
    }

    /// Broadcast a `status` event.
    pub fn publish_status(&self, status: &str) {
        self.publish(WireEvent::Status {
            status: status.to_string(),
            meeting_id: None,
            recording_notice: self.recording_notice(),
        });
    }

    /// Broadcast an `import_progress` event: how far a `meeting import` or
    /// `meeting retranscribe` has got.
    pub fn publish_import_progress(&self, progress: &crate::meeting::import::Progress) {
        self.publish(WireEvent::ImportProgress(progress.clone()));
    }

    /// Broadcast a `playback` event: `meeting play` started or stopped
    /// playing a meeting's audio. Nothing is sent before the first playback.
    pub fn publish_playback(&self, playback: &crate::meeting::playback::PlaybackStatus) {
        let (Some(meeting_id), Some(track), Some(from_ms)) =
            (&playback.meeting_id, playback.track, playback.from_ms)
        else {
            return;
        };
        self.publish(WireEvent::Playback {
            meeting_id: meeting_id.clone(),
            track: track.to_string(),
            state: playback.state.clone(),
            from_ms,
        });
    }

    /// Broadcast a `levels` event: the meeting's per-track levels.
    pub fn publish_levels(&self, levels: crate::meeting::levels::Levels) {
        self.publish(WireEvent::Levels(levels));
    }

    /// Broadcast a `soundcheck` event: a sound check started or stopped.
    pub fn publish_soundcheck(&self, event: crate::meeting::soundcheck::SoundcheckEvent) {
        self.publish(WireEvent::Soundcheck(event));
    }

    /// Is any client connected? Lets frequent events like `levels` skip
    /// the work when nobody listens.
    pub fn has_subscribers(&self) -> bool {
        self.inner.subscribers.load(Ordering::Relaxed) > 0
    }

    /// Broadcast an `error` event.
    pub fn publish_error(&self, message: &str) {
        self.publish(WireEvent::Error {
            message: message.to_string(),
            meeting_id: None,
            code: None,
            backend: None,
        });
    }

    /// Broadcast an `error` event for a track of the running meeting that
    /// has sound and no lines, with `code`
    /// [`EVENT_CODE`](crate::meeting::silent_track::EVENT_CODE). It names
    /// no meeting, so the views show it in their banner like any other
    /// error of the meeting being recorded.
    pub fn publish_track_notice(&self, track: &crate::meeting::silent_track::SilentTrack) {
        self.publish(WireEvent::Error {
            message: track.summary(),
            meeting_id: None,
            code: Some(crate::meeting::silent_track::EVENT_CODE.to_string()),
            backend: None,
        });
    }

    /// Broadcast what the auto-summary of a finished meeting is doing.
    pub fn publish_summary_event(&self, event: crate::meeting::assistant::summarize::SummaryEvent) {
        use crate::meeting::assistant::summarize::SummaryEvent;
        self.publish(match event {
            SummaryEvent::Summarizing { meeting_id } => WireEvent::Status {
                status: "summarizing".to_string(),
                meeting_id: Some(meeting_id),
                recording_notice: self.recording_notice(),
            },
            SummaryEvent::Ready(outcome) => WireEvent::SummaryReady {
                meeting_id: outcome.meeting_id,
                agent: outcome.agent,
                agent_name: outcome.agent_name,
                path: outcome.path,
                summary: outcome.summary,
            },
            SummaryEvent::Failed {
                meeting_id,
                code,
                message,
                backend,
            } => WireEvent::Error {
                message,
                meeting_id: Some(meeting_id),
                code: Some(code.to_string()),
                backend,
            },
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
    subscribers: Arc<AtomicUsize>,
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
        run_broadcast_loop(broadcast_rx, sub_rx, subscribers).await;
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
                match bind_and_spawn(
                    &inner.socket_path,
                    inner.hello.clone(),
                    inner.subscribers.clone(),
                ) {
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
    count: Arc<AtomicUsize>,
) {
    let mut subscribers: Vec<mpsc::Sender<String>> = Vec::new();
    count.store(0, Ordering::Relaxed);

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
                        if !subscribers.is_empty() {
                            subscribers.retain(|tx| tx.try_send(line.clone()).is_ok());
                        }
                    }
                    None => break,
                }
            }
        }
        // A writer that has gone (client disconnected) closes its queue.
        subscribers.retain(|tx| !tx.is_closed());
        count.store(subscribers.len(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Level events reach a client in their documented shape, and the hub
    /// knows when nobody is listening, so the publisher can skip the work.
    #[tokio::test]
    async fn levels_reach_subscribers_and_the_hub_counts_them() {
        use tokio::io::AsyncBufReadExt;

        let dir = tempfile::tempdir().unwrap();
        let hub = MeetingEventHub::start(dir.path().join("meeting.sock"))
            .await
            .unwrap();
        assert!(!hub.has_subscribers());

        let stream = UnixStream::connect(hub.socket_path()).await.unwrap();
        let mut lines = tokio::io::BufReader::new(stream).lines();
        lines.next_line().await.unwrap().unwrap(); // hello
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !hub.has_subscribers() {
            assert!(std::time::Instant::now() < deadline, "never counted");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        hub.publish_levels(crate::meeting::levels::Levels {
            meeting_id: Some("m1".into()),
            soundcheck: false,
            t_ms: 1200,
            mic: Some(-23.4),
            mic_peak: Some(-15.1),
            loopback: None,
            loopback_peak: None,
        });
        let line = lines.next_line().await.unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"event": "levels", "meeting_id": "m1", "t_ms": 1200,
                               "mic": -23.4, "mic_peak": -15.1,
                               "loopback": null, "loopback_peak": null})
        );

        // The client leaves: the next event finds it gone and the count
        // drops to zero.
        drop(lines);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while hub.has_subscribers() {
            assert!(std::time::Instant::now() < deadline, "still counted");
            hub.publish_status("recording");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        hub.cleanup();
    }

    #[tokio::test]
    async fn summary_events_reach_subscribers() {
        use crate::meeting::assistant::summarize::{SummaryEvent, SummaryOutcome};
        use tokio::io::AsyncBufReadExt;

        let dir = tempfile::tempdir().unwrap();
        let hub = MeetingEventHub::start(dir.path().join("meeting.sock"))
            .await
            .unwrap();
        let stream = UnixStream::connect(hub.socket_path()).await.unwrap();
        let mut lines = tokio::io::BufReader::new(stream).lines();
        let hello = lines.next_line().await.unwrap().unwrap();
        assert!(hello.contains(r#""event":"hello""#));

        hub.publish_summary_event(SummaryEvent::Summarizing {
            meeting_id: "m1".into(),
        });
        hub.publish_summary_event(SummaryEvent::Ready(Box::new(SummaryOutcome {
            meeting_id: "m1".into(),
            agent: "claude".into(),
            agent_name: "Claude".into(),
            path: Some(PathBuf::from("/x/summary.md")),
            fell_back_from: None,
            summary: crate::meeting::data::MeetingSummary {
                summary: "ok".into(),
                key_points: vec![],
                action_items: vec![],
                decisions: vec![],
                chapters: vec![],
                generated_at: Utc::now(),
                model: Some("claude".into()),
            },
        })));
        hub.publish_summary_event(SummaryEvent::Failed {
            meeting_id: "m1".into(),
            code: "agent_failed",
            message: "Summary failed: x".into(),
            backend: Some("Claude".into()),
        });

        async fn next(
            lines: &mut tokio::io::Lines<tokio::io::BufReader<UnixStream>>,
        ) -> serde_json::Value {
            let line = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            serde_json::from_str(&line).unwrap()
        }
        let v = next(&mut lines).await;
        assert_eq!(
            v,
            serde_json::json!({"event": "status", "status": "summarizing", "meeting_id": "m1", "recording_notice": false})
        );
        let v = next(&mut lines).await;
        assert_eq!(v["event"], "summary_ready");
        assert_eq!(v["agent_name"], "Claude");
        assert_eq!(v["path"], "/x/summary.md");
        assert_eq!(v["summary"]["summary"], "ok");
        let v = next(&mut lines).await;
        assert_eq!(
            v,
            serde_json::json!({"event": "error", "message": "Summary failed: x",
                               "meeting_id": "m1", "code": "agent_failed", "backend": "Claude"})
        );
        hub.cleanup();
    }

    #[test]
    fn hud_backlog_follows_a_speaker_rename() {
        let mut seg = TranscriptSegment::new(0, 0, 1_000, "hi".into(), 0);
        seg.speaker_id = Some("Remote".to_string());
        let mut you = TranscriptSegment::new(1, 2_000, 3_000, "hey".into(), 0);
        you.speaker_id = Some("You".to_string());
        let mut hud = MeetingHudState::idle();
        hud.push_segments(&[seg, you]);

        hud.rename_speaker("Remote", "Remote 1");

        let ids: Vec<_> = hud.segments.iter().map(|s| s.speaker_key()).collect();
        assert_eq!(ids, vec!["Remote 1", "You"]);
    }

    #[test]
    fn hud_backlog_interleaves_tracks_by_start_time() {
        use crate::meeting::data::AudioSource;
        let seg = |id: u32, start: u64, source: AudioSource| {
            let mut s = TranscriptSegment::new(id, start, start + 1000, String::new(), 0);
            s.source = source;
            s
        };
        let mut hud = MeetingHudState::idle();
        hud.push_segments(&[
            seg(0, 1_000, AudioSource::Microphone),
            seg(1, 9_000, AudioSource::Microphone),
        ]);
        hud.push_segments(&[seg(2, 4_000, AudioSource::Loopback)]);
        let order: Vec<u32> = hud.segments.iter().map(|s| s.id).collect();
        assert_eq!(order, vec![0, 2, 1]);
    }
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
    async fn hello_and_status_carry_the_recording_notice() {
        let (_tmp, path) = temp_socket_dir();
        let hub = MeetingEventHub::start(path.clone())
            .await
            .expect("start hub");

        let mut client = UnixStream::connect(&path).await.expect("connect");
        let hello = read_line(&mut client).await;
        assert_eq!(hello["recording_notice"], false, "idle default: {hello}");

        let mut state = MeetingHudState::idle();
        state.status = "recording".to_string();
        state.recording_notice = true;
        hub.update_snapshot(&state);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        hub.publish_status("recording");
        let status = read_line(&mut client).await;
        assert_eq!(status["event"], "status");
        assert_eq!(status["recording_notice"], true, "{status}");

        let mut fresh = UnixStream::connect(&path).await.expect("connect");
        let hello = read_line(&mut fresh).await;
        assert_eq!(hello["recording_notice"], true, "{hello}");

        // Acknowledged: the next status says so to everyone connected.
        state.recording_notice = false;
        hub.update_snapshot(&state);
        hub.publish_status("recording");
        let status = read_line(&mut client).await;
        assert_eq!(status["recording_notice"], false, "{status}");

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
    async fn a_track_with_sound_and_no_lines_is_an_error_event_naming_no_meeting() {
        let (_tmp, path) = temp_socket_dir();
        let hub = MeetingEventHub::start(path.clone())
            .await
            .expect("start hub");
        let mut client = UnixStream::connect(&path).await.expect("connect");
        let _hello = read_line(&mut client).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let track = crate::meeting::silent_track::SilentTrack {
            track: crate::meeting::AudioSource::Microphone,
            level: 0.052,
            threshold: 0.11,
        };
        hub.publish_track_notice(&track);

        let value = read_line(&mut client).await;
        assert_eq!(value["event"], "error");
        assert_eq!(value["code"], "track_not_transcribed");
        assert_eq!(value["message"], track.summary());
        // The views put an error naming no meeting in their banner; one
        // naming a meeting goes to that meeting's summary instead.
        assert!(value.get("meeting_id").is_none(), "{value}");

        hub.cleanup();
    }

    #[tokio::test]
    async fn import_progress_reaches_subscribers() {
        let (_tmp, path) = temp_socket_dir();
        let hub = MeetingEventHub::start(path.clone())
            .await
            .expect("start hub");
        let mut client = UnixStream::connect(&path).await.expect("connect");
        let _hello = read_line(&mut client).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        hub.publish_import_progress(&crate::meeting::import::Progress {
            meeting_id: "abc-123".to_string(),
            operation: "import".to_string(),
            state: "transcribing".to_string(),
            chunks_done: 2,
            chunks_total: 5,
            position_ms: 60_000,
            duration_ms: 150_000,
        });
        assert_eq!(
            read_line(&mut client).await,
            serde_json::json!({"event": "import_progress", "meeting_id": "abc-123",
                "operation": "import", "state": "transcribing", "chunks_done": 2,
                "chunks_total": 5, "position_ms": 60_000, "duration_ms": 150_000})
        );
        hub.cleanup();
    }

    #[tokio::test]
    async fn playback_changes_reach_subscribers() {
        use crate::meeting::playback::{PlaybackStatus, Track};
        let (_tmp, path) = temp_socket_dir();
        let hub = MeetingEventHub::start(path.clone())
            .await
            .expect("start hub");

        let mut client = UnixStream::connect(&path).await.expect("connect");
        let _hello = read_line(&mut client).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut playback = PlaybackStatus {
            state: "stopped".to_string(),
            meeting_id: None,
            track: None,
            from_ms: None,
            pid: None,
            started_at: None,
            position_ms: None,
        };
        // Nothing has played yet: nothing to say.
        hub.publish_playback(&playback);
        playback.state = "playing".to_string();
        playback.meeting_id = Some("abc-123".to_string());
        playback.track = Some(Track::Mixed);
        playback.from_ms = Some(61_250);
        hub.publish_playback(&playback);

        let value = read_line(&mut client).await;
        assert_eq!(
            value,
            serde_json::json!({"event": "playback", "meeting_id": "abc-123",
                "track": "mixed", "state": "playing", "from_ms": 61_250})
        );

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

    #[tokio::test]
    async fn segment_updated_and_notes_updated_reach_subscribers() {
        let (_tmp, path) = temp_socket_dir();
        let hub = MeetingEventHub::start(path.clone())
            .await
            .expect("start hub");
        let mut client = UnixStream::connect(&path).await.expect("connect");
        let _hello = read_line(&mut client).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut segment = TranscriptSegment::new(28, 127_000, 130_000, "hm".to_string(), 4);
        segment.speaker_id = Some("Person 1".to_string());
        segment.original_speaker = Some("Remote 1".to_string());
        segment.annotation = Some(crate::meeting::SegmentAnnotation {
            text: "check this".to_string(),
            highlight: true,
        });
        hub.publish_segment_updated(&segment);
        let value = read_line(&mut client).await;
        assert_eq!(value["event"], "segment_updated");
        assert_eq!(value["segment"]["id"], 28);
        assert_eq!(value["segment"]["original_speaker"], "Remote 1");
        assert_eq!(value["segment"]["annotation"]["text"], "check this");
        assert_eq!(value["segment"]["annotation"]["highlight"], true);

        hub.publish_segment_deleted("abc", 28);
        let value = read_line(&mut client).await;
        assert_eq!(
            value,
            serde_json::json!({"event": "segment_deleted", "meeting_id": "abc", "segment_id": 28})
        );

        let updated_at = chrono::DateTime::parse_from_rfc3339("2026-11-11T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        hub.publish_notes_updated(&crate::meeting::notes::MeetingNotes {
            meeting_id: "abc".to_string(),
            notes: "secret".to_string(),
            updated_at: Some(updated_at),
        });
        let value = read_line(&mut client).await;
        assert_eq!(
            value,
            serde_json::json!({"event": "notes_updated", "meeting_id": "abc", "updated_at": "2026-11-11T20:00:00Z"})
        );

        hub.publish_title_changed("abc", "Pete <> Chad");
        let value = read_line(&mut client).await;
        assert_eq!(
            value,
            serde_json::json!({"event": "title_changed", "meeting_id": "abc", "title": "Pete <> Chad"})
        );

        hub.cleanup();
    }

    #[test]
    fn hud_backlog_drops_a_deleted_line_and_takes_it_back_in_order() {
        let seg =
            |id: u32, start: u64| TranscriptSegment::new(id, start, start + 500, String::new(), 0);
        let mut hud = MeetingHudState::idle();
        hud.push_segments(&[seg(0, 0), seg(1, 1_000), seg(2, 2_000)]);
        hud.remove_segment(1);
        assert_eq!(
            hud.segments.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![0, 2]
        );
        hud.update_segment(&seg(1, 1_000));
        assert_eq!(
            hud.segments.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
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
