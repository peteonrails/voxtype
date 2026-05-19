//! Audio level emitter for the on-screen visualizer
//!
//! During recording, the daemon buckets the live audio sample stream into
//! 10 ms windows (100 Hz) and emits a small binary frame for each window
//! over a Unix socket at `$XDG_RUNTIME_DIR/voxtype/audio.sock`.
//!
//! Per-frame payload (16 bytes, native byte order):
//!
//! ```text
//! struct AudioFrame { seq: u32, min: f32, max: f32, peak_dbfs: f32 }
//! ```
//!
//! This is a lossy, best-effort broadcast: subscribers that fall behind get
//! disconnected. The daemon never blocks on slow consumers.
//!
//! The emitter is *additive*: it taps the existing
//! `mpsc::Receiver<Vec<f32>>` returned by `AudioCapture::start()` (which the
//! daemon used to discard). When recording stops, the input channel closes
//! and the emitter task exits, which in turn causes the bucketing loop to
//! end. The hub keeps running across recordings and accepts subscribers in
//! between, but only emits frames while a recording session has provided a
//! sample stream.
//!
//! ## Self-healing listener (issue #391)
//!
//! `LevelHub::start()` spawns two long-lived tasks (accept loop + broadcast
//! loop) that under normal operation never exit. If one of them does exit
//! (panic, runtime quirk, or an unforeseen bug), the listener gets dropped
//! silently: the socket file stays on disk but new `connect()` calls fail
//! with `ECONNREFUSED`. That orphaned-listener state was observed in the
//! wild on a long-running daemon and broke fresh OSD / audio-bridge
//! connections without any visible error in the daemon process.
//!
//! To make the failure both loud and survivable, the hub now:
//!
//! 1. Captures the `JoinHandle` for both tasks instead of letting them
//!    detach as fire-and-forget.
//! 2. Spawns a watchdog task that `await`s both handles via `select!`. If
//!    either resolves (which should be impossible through normal code
//!    paths) the watchdog logs at ERROR level and respawns the listener
//!    plus both internal tasks from scratch.
//! 3. Stores the live broadcast `Sender` behind a `Arc<RwLock<...>>` so
//!    `FrameSink::publish` always picks up the current sender after a
//!    respawn without recording sessions needing to be torn down and
//!    rebuilt. The lock is read-only on the hot path (one acquire per
//!    100 Hz frame), so the overhead is negligible.
//!
//! Pre-existing accepted connections do not migrate across a respawn;
//! their per-client writer tasks will see their queue receiver close and
//! exit cleanly. New clients connecting to the freshly-bound listener
//! work as if the daemon had just started. That's an acceptable trade-off
//! since the alternative is a permanently broken socket.

use crate::config::Config;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

/// Native sample rate of audio fed to the emitter (matches the daemon's
/// resampled mono stream).
pub const SAMPLE_RATE: u32 = 16_000;

/// Frame emit rate. 100 Hz = one frame every 10 ms = 160 samples at 16 kHz.
pub const FRAME_HZ: u32 = 100;

/// Samples per emitted frame.
pub const SAMPLES_PER_FRAME: usize = (SAMPLE_RATE / FRAME_HZ) as usize;

/// Wire size of an `AudioFrame` in bytes.
pub const FRAME_BYTES: usize = 16;

/// One audio level frame.
///
/// `repr(C)` so the layout is stable for the wire format. We serialise
/// fields explicitly via `to_bytes()` rather than reinterpret-casting,
/// to avoid relying on padding rules.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioFrame {
    /// Monotonic frame counter (wraps at u32::MAX).
    pub seq: u32,
    /// Minimum sample in the 10 ms window (range -1.0..=1.0).
    pub min: f32,
    /// Maximum sample in the 10 ms window (range -1.0..=1.0).
    pub max: f32,
    /// Peak amplitude in dBFS. -inf for silence; clamped to -120.0 below.
    pub peak_dbfs: f32,
}

impl AudioFrame {
    /// Serialise to 16 bytes in native byte order.
    ///
    /// We use native order because the OSD runs on the same machine as the
    /// daemon. There's no portability concern.
    #[inline]
    pub fn to_bytes(self) -> [u8; FRAME_BYTES] {
        let mut buf = [0u8; FRAME_BYTES];
        buf[0..4].copy_from_slice(&self.seq.to_ne_bytes());
        buf[4..8].copy_from_slice(&self.min.to_ne_bytes());
        buf[8..12].copy_from_slice(&self.max.to_ne_bytes());
        buf[12..16].copy_from_slice(&self.peak_dbfs.to_ne_bytes());
        buf
    }

    /// Parse a frame from 16 bytes in native byte order.
    pub fn from_bytes(bytes: &[u8; FRAME_BYTES]) -> Self {
        let seq = u32::from_ne_bytes(bytes[0..4].try_into().unwrap());
        let min = f32::from_ne_bytes(bytes[4..8].try_into().unwrap());
        let max = f32::from_ne_bytes(bytes[8..12].try_into().unwrap());
        let peak_dbfs = f32::from_ne_bytes(bytes[12..16].try_into().unwrap());
        Self {
            seq,
            min,
            max,
            peak_dbfs,
        }
    }
}

/// Default path for the audio-frames socket.
pub fn default_socket_path() -> PathBuf {
    Config::runtime_dir().join("audio.sock")
}

/// Per-subscriber bounded queue. 30 frames = 300 ms at 100 Hz; if a client
/// can't keep up over that window, drop it.
const SUBSCRIBER_QUEUE_DEPTH: usize = 30;

/// Hub for distributing audio frames to subscribers.
///
/// The hub owns the Unix listener and a list of currently-connected
/// subscribers. Recording sessions feed frames into the hub via
/// [`LevelHub::frame_sink`]; the hub fans them out non-blockingly.
///
/// If the internal accept or broadcast task ever exits unexpectedly, the
/// hub's watchdog respawns both, rebinding the socket. See the module
/// doc comment and issue #391 for the rationale.
#[derive(Clone)]
pub struct LevelHub {
    inner: Arc<HubInner>,
}

/// The currently-live broadcast endpoint plus telemetry counters.
///
/// This is the mutable state of the hub. Replaced wholesale by a respawn.
/// Wrapped in an `Arc` so `FrameSink` can hold a cheap snapshot reference
/// without copying the channel internals.
struct HubState {
    /// Bounded mpsc channel: recording-session producers send frames here,
    /// the broadcaster task drains it and fans out to clients.
    broadcast_tx: mpsc::Sender<AudioFrame>,
    /// Running count of attached subscribers, for telemetry/logging.
    subscriber_count: Mutex<usize>,
}

struct HubInner {
    /// Live state; replaced atomically by the watchdog on respawn.
    state: RwLock<Arc<HubState>>,
    /// Immutable: the path used both for `bind()` and `cleanup()`.
    socket_path: PathBuf,
}

impl HubInner {
    fn current(&self) -> Arc<HubState> {
        // Panic-on-poison is fine here: a poisoned lock means a writer
        // task panicked while replacing the state, which is already
        // unrecoverable. The watchdog logs and we crash visibly.
        self.state.read().expect("levels hub state lock").clone()
    }

    fn replace(&self, new_state: Arc<HubState>) {
        let mut guard = self.state.write().expect("levels hub state lock");
        *guard = new_state;
    }
}

impl LevelHub {
    /// Bind a Unix socket and start the broadcaster task.
    ///
    /// Returns the hub plus the socket path that was bound. If a stale
    /// socket file exists (left by a prior daemon crash), it is removed
    /// before binding.
    pub async fn start(socket_path: PathBuf) -> io::Result<Self> {
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let (state, accept_h, broadcast_h) = bind_and_spawn(&socket_path)?;
        tracing::info!("Audio level socket listening at {:?}", socket_path);

        let inner = Arc::new(HubInner {
            state: RwLock::new(state),
            socket_path: socket_path.clone(),
        });

        spawn_watchdog(inner.clone(), accept_h, broadcast_h);

        Ok(Self { inner })
    }

    /// Returns a sender that recording sessions can use to publish frames.
    ///
    /// The handle survives a hub respawn: it reads the current broadcast
    /// channel on each publish, so after the watchdog rebuilds the hub
    /// the same `FrameSink` keeps working without the caller noticing.
    pub fn frame_sink(&self) -> FrameSink {
        FrameSink {
            inner: self.inner.clone(),
        }
    }

    /// Path of the bound Unix socket.
    pub fn socket_path(&self) -> &std::path::Path {
        &self.inner.socket_path
    }

    /// Best-effort cleanup of the socket file. Called on shutdown.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.inner.socket_path);
    }
}

/// Bind the listener and spawn the accept + broadcast tasks. Returns the
/// fresh `HubState` plus both `JoinHandle`s so the watchdog can supervise
/// them. Called once from [`LevelHub::start`] and again from the
/// watchdog's respawn path after a task exit.
fn bind_and_spawn(
    socket_path: &std::path::Path,
) -> io::Result<(Arc<HubState>, JoinHandle<()>, JoinHandle<()>)> {
    // Remove any stale socket from a prior run (or a prior incarnation
    // of this hub in the respawn case).
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }

    let listener = UnixListener::bind(socket_path)?;

    // Frame fan-in: any number of recording-session producers can send.
    // 200 frames = 2 seconds of buffered headroom at 100 Hz, plenty.
    let (broadcast_tx, broadcast_rx) = mpsc::channel::<AudioFrame>(200);

    let state = Arc::new(HubState {
        broadcast_tx,
        subscriber_count: Mutex::new(0),
    });

    let (sub_tx, sub_rx) = mpsc::unbounded_channel::<SubscriberSlot>();

    let state_for_accept = state.clone();
    let accept_handle = tokio::spawn(async move {
        run_accept_loop(listener, sub_tx, state_for_accept).await;
    });

    let broadcast_handle = tokio::spawn(async move {
        run_broadcast_loop(broadcast_rx, sub_rx).await;
    });

    Ok((state, accept_handle, broadcast_handle))
}

/// Watchdog task. Lives for the lifetime of the daemon.
///
/// Awaits both internal task handles via `select!`. The first one to
/// resolve triggers a loud ERROR log and a respawn of the entire hub.
/// After respawn, the watchdog loops back to supervise the new handles.
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
                    "Audio level {} loop exited unexpectedly (no panic). \
                     Respawning the LevelHub. See issue #391.",
                    which
                ),
                Err(join_err) => tracing::error!(
                    task = which,
                    err = %join_err,
                    "Audio level {} loop panicked or was cancelled. \
                     Respawning the LevelHub. See issue #391.",
                    which
                ),
            }

            // Drop the other handle so its task is not orphaned in the
            // join set; the underlying task will be aborted when we
            // overwrite the broadcast Sender (which closes its
            // receiver) and the listener (which closes the accept
            // loop's UnixListener).
            accept_handle.abort();
            broadcast_handle.abort();

            // Rebuild. If bind fails (e.g., XDG_RUNTIME_DIR vanished),
            // back off and try again. We never give up: a dead listener
            // is the bug we're paid to fix.
            loop {
                match bind_and_spawn(&inner.socket_path) {
                    Ok((new_state, ah, bh)) => {
                        inner.replace(new_state);
                        tracing::info!("Audio level socket rebound at {:?}", inner.socket_path);
                        accept_handle = ah;
                        broadcast_handle = bh;
                        break;
                    }
                    Err(e) => {
                        tracing::error!(
                            err = %e,
                            "Failed to rebind audio level socket; will retry"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        }
    });
}

/// Sender handle handed out by [`LevelHub::frame_sink`].
///
/// Holds a reference to `HubInner` rather than the channel directly, so a
/// respawn of the hub is transparent: the next `publish` call picks up
/// the new channel automatically.
#[derive(Clone)]
pub struct FrameSink {
    inner: Arc<HubInner>,
}

impl FrameSink {
    /// Try to publish a frame. Drops the frame if the broadcaster is
    /// backed up. Never blocks and never allocates.
    ///
    /// The `RwLock` read here is the only synchronisation overhead added
    /// by the self-healing design. At 100 Hz on a single producer it is
    /// well under the noise floor of the audio pipeline.
    #[inline]
    pub fn publish(&self, frame: AudioFrame) {
        let state = self.inner.current();
        let _ = state.broadcast_tx.try_send(frame);
    }
}

/// One subscriber's per-connection mailbox.
struct SubscriberSlot {
    /// Sender feeding the per-client writer task.
    tx: mpsc::Sender<AudioFrame>,
}

async fn run_accept_loop(
    listener: UnixListener,
    sub_tx: mpsc::UnboundedSender<SubscriberSlot>,
    state: Arc<HubState>,
) {
    loop {
        // Test-only fault injection. The flag lives on `HubInner`, but
        // we don't have a direct reference here; instead, the test arms
        // the panic before calling `LevelHub::arm_accept_panic`, which
        // toggles a global thread-local. We avoid threading that through
        // by checking via a separate hook.
        #[cfg(test)]
        {
            if tests::should_panic_now() {
                panic!("levels::run_accept_loop test-injected panic");
            }
        }

        match listener.accept().await {
            Ok((stream, _addr)) => {
                let (tx, rx) = mpsc::channel::<AudioFrame>(SUBSCRIBER_QUEUE_DEPTH);
                let slot = SubscriberSlot { tx };
                if sub_tx.send(slot).is_err() {
                    // Broadcaster has shut down; close the new connection.
                    drop(stream);
                    break;
                }

                let state_for_writer = state.clone();
                tokio::spawn(async move {
                    {
                        let mut count = state_for_writer.subscriber_count.lock().await;
                        *count += 1;
                        tracing::debug!("Audio subscriber connected (count={})", *count);
                    }
                    run_subscriber_writer(stream, rx).await;
                    {
                        let mut count = state_for_writer.subscriber_count.lock().await;
                        *count = count.saturating_sub(1);
                        tracing::debug!("Audio subscriber disconnected (count={})", *count);
                    }
                });
            }
            Err(e) => {
                tracing::warn!("Audio socket accept error: {}", e);
                // Brief pause to avoid tight error loops.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

async fn run_subscriber_writer(mut stream: UnixStream, mut rx: mpsc::Receiver<AudioFrame>) {
    while let Some(frame) = rx.recv().await {
        let bytes = frame.to_bytes();
        if let Err(e) = stream.write_all(&bytes).await {
            tracing::trace!("Audio subscriber write error: {}", e);
            break;
        }
    }
    let _ = stream.shutdown().await;
}

async fn run_broadcast_loop(
    mut frame_rx: mpsc::Receiver<AudioFrame>,
    mut new_subs: mpsc::UnboundedReceiver<SubscriberSlot>,
) {
    let mut subscribers: Vec<mpsc::Sender<AudioFrame>> = Vec::new();

    loop {
        tokio::select! {
            // New subscriber connected; add to fan-out list.
            slot = new_subs.recv() => {
                match slot {
                    Some(slot) => subscribers.push(slot.tx),
                    None => {
                        // Listener gone; nothing more to do.
                        break;
                    }
                }
            }
            // New frame from a recording session; fan out.
            frame = frame_rx.recv() => {
                match frame {
                    Some(frame) => {
                        // Drain any pending new-subscriber notifications first
                        // so a fast reconnect after recording doesn't wait a
                        // whole frame to start receiving.
                        while let Ok(slot) = new_subs.try_recv() {
                            subscribers.push(slot.tx);
                        }
                        if subscribers.is_empty() {
                            continue;
                        }
                        // Fan out, dropping any subscriber whose queue is full.
                        subscribers.retain(|tx| tx.try_send(frame).is_ok());
                    }
                    None => {
                        // Hub shutdown.
                        break;
                    }
                }
            }
        }
    }
}

/// Bucketing helper: groups f32 samples into fixed-size 10 ms windows and
/// emits an [`AudioFrame`] per completed window.
///
/// Holds a small running accumulator across calls; a final partial bucket
/// at end-of-stream is discarded (10 ms of "lost" tail audio is well below
/// perceptual threshold).
pub struct LevelBucketer {
    samples_per_frame: usize,
    accumulated: usize,
    min: f32,
    max: f32,
    peak_abs: f32,
    seq: u32,
}

impl LevelBucketer {
    pub fn new() -> Self {
        Self {
            samples_per_frame: SAMPLES_PER_FRAME,
            accumulated: 0,
            min: f32::INFINITY,
            max: f32::NEG_INFINITY,
            peak_abs: 0.0,
            seq: 0,
        }
    }

    /// Push samples into the bucketer. For each completed 10 ms window an
    /// [`AudioFrame`] is appended to `out`. No allocation when `out` has
    /// sufficient capacity reserved by the caller.
    pub fn push(&mut self, samples: &[f32], out: &mut Vec<AudioFrame>) {
        for &s in samples {
            if s < self.min {
                self.min = s;
            }
            if s > self.max {
                self.max = s;
            }
            let a = s.abs();
            if a > self.peak_abs {
                self.peak_abs = a;
            }
            self.accumulated += 1;

            if self.accumulated >= self.samples_per_frame {
                let peak_dbfs = if self.peak_abs <= 1e-6 {
                    -120.0
                } else {
                    20.0 * self.peak_abs.log10()
                };
                let frame = AudioFrame {
                    seq: self.seq,
                    min: if self.min.is_finite() { self.min } else { 0.0 },
                    max: if self.max.is_finite() { self.max } else { 0.0 },
                    peak_dbfs,
                };
                out.push(frame);
                self.seq = self.seq.wrapping_add(1);
                self.accumulated = 0;
                self.min = f32::INFINITY;
                self.max = f32::NEG_INFINITY;
                self.peak_abs = 0.0;
            }
        }
    }
}

impl Default for LevelBucketer {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn a forwarder task that reads from an `mpsc::Receiver<Vec<f32>>`
/// (the chunk stream from `AudioCapture::start()`), buckets the samples
/// into 100 Hz frames, and publishes them to the supplied `FrameSink`.
///
/// The task ends when the input receiver is closed. This is the correct
/// signal for "recording stopped".
pub fn spawn_emitter(
    chunk_rx: mpsc::Receiver<Vec<f32>>,
    sink: FrameSink,
) -> tokio::task::JoinHandle<()> {
    spawn_emitter_with_streaming_tap(chunk_rx, sink, None)
}

/// Like [`spawn_emitter`] but also forwards every chunk to an optional
/// `streaming_tx`, used by the streaming transcription pipeline to feed
/// audio into a backend without disturbing the OSD level emitter.
///
/// When `streaming_tx` is `Some`, each chunk is cloned and `try_send`'d to
/// it. Failure to send (closed receiver, full bounded channel) is logged at
/// trace and never blocks the level emitter. When `streaming_tx` is `None`,
/// behavior is identical to [`spawn_emitter`].
pub fn spawn_emitter_with_streaming_tap(
    mut chunk_rx: mpsc::Receiver<Vec<f32>>,
    sink: FrameSink,
    streaming_tx: Option<mpsc::Sender<Vec<f32>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut bucketer = LevelBucketer::new();
        // Reusable scratch buffer for emitted frames per chunk.
        // 4 frames is plenty for typical 10–40 ms chunks; we'll grow if needed.
        let mut out: Vec<AudioFrame> = Vec::with_capacity(8);

        while let Some(chunk) = chunk_rx.recv().await {
            out.clear();
            bucketer.push(&chunk, &mut out);
            for frame in out.drain(..) {
                sink.publish(frame);
            }

            if let Some(ref tx) = streaming_tx {
                if let Err(e) = tx.try_send(chunk) {
                    tracing::trace!("streaming sample tap try_send failed: {}", e);
                }
            }
        }
        tracing::trace!("Audio level emitter task ended");
    })
}

/// Publish silent frames at 30 Hz to keep the OSD visible while a
/// streaming session is draining server-side after the mic has been
/// stopped. 30 Hz matches typical OSD redraw rates; pumping faster just
/// burns timer wakeups. Cancelling the returned `JoinHandle` ends the pump.
pub fn spawn_silence_pump(sink: FrameSink) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut seq: u32 = 0;
        let mut tick = tokio::time::interval(Duration::from_millis(33));
        loop {
            tick.tick().await;
            sink.publish(AudioFrame {
                seq,
                min: 0.0,
                max: 0.0,
                peak_dbfs: -120.0,
            });
            seq = seq.wrapping_add(1);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::AsyncReadExt;

    /// Global one-shot panic flag for the accept-loop fault injection
    /// test. We use a global because the panic check is inside
    /// `run_accept_loop`, which doesn't have access to the test
    /// scaffolding directly. Tests serialise on `panic_test_lock`.
    static ACCEPT_PANIC_NEXT: AtomicBool = AtomicBool::new(false);
    /// Serialise tests that touch the global panic flag.
    static PANIC_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(super) fn should_panic_now() -> bool {
        ACCEPT_PANIC_NEXT.swap(false, Ordering::SeqCst)
    }

    fn arm_global_panic() {
        ACCEPT_PANIC_NEXT.store(true, Ordering::SeqCst);
    }

    #[test]
    fn frame_roundtrip() {
        let f = AudioFrame {
            seq: 42,
            min: -0.5,
            max: 0.75,
            peak_dbfs: -3.0,
        };
        let bytes = f.to_bytes();
        let parsed = AudioFrame::from_bytes(&bytes);
        assert_eq!(parsed, f);
    }

    #[test]
    fn frame_size_is_16_bytes() {
        assert_eq!(FRAME_BYTES, 16);
        assert_eq!(std::mem::size_of::<AudioFrame>(), 16);
    }

    #[test]
    fn bucketer_emits_at_100hz() {
        let mut b = LevelBucketer::new();
        let mut out = Vec::new();
        // 1600 samples = exactly 10 frames at 16 kHz / 100 Hz.
        let samples = vec![0.5_f32; 1600];
        b.push(&samples, &mut out);
        assert_eq!(out.len(), 10);
        assert_eq!(out[0].seq, 0);
        assert_eq!(out[9].seq, 9);
        assert!((out[0].max - 0.5).abs() < 1e-6);
        assert!((out[0].min - 0.5).abs() < 1e-6);
    }

    #[test]
    fn bucketer_partial_window_holds_state() {
        let mut b = LevelBucketer::new();
        let mut out = Vec::new();
        b.push(&vec![0.1_f32; 100], &mut out);
        assert!(out.is_empty(), "incomplete bucket should not emit");
        b.push(&vec![0.2_f32; 60], &mut out);
        assert_eq!(out.len(), 1, "completing the bucket should emit one frame");
    }

    #[test]
    fn bucketer_silence_yields_minus_120_dbfs() {
        let mut b = LevelBucketer::new();
        let mut out = Vec::new();
        b.push(&vec![0.0_f32; SAMPLES_PER_FRAME], &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].peak_dbfs, -120.0);
    }

    #[test]
    fn bucketer_full_scale_yields_zero_dbfs() {
        let mut b = LevelBucketer::new();
        let mut out = Vec::new();
        let mut samples = vec![0.0_f32; SAMPLES_PER_FRAME];
        samples[42] = 1.0;
        b.push(&samples, &mut out);
        assert_eq!(out.len(), 1);
        assert!(out[0].peak_dbfs.abs() < 1e-3);
    }

    #[test]
    fn bucketer_min_max_track_polarity() {
        let mut b = LevelBucketer::new();
        let mut out = Vec::new();
        let mut samples = vec![0.0_f32; SAMPLES_PER_FRAME];
        samples[0] = -0.8;
        samples[1] = 0.4;
        b.push(&samples, &mut out);
        assert_eq!(out.len(), 1);
        assert!((out[0].min - -0.8).abs() < 1e-6);
        assert!((out[0].max - 0.4).abs() < 1e-6);
    }

    fn temp_socket_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!(
            "voxtype-levels-test-{}-{}-{}.sock",
            name, pid, nanos
        ));
        p
    }

    /// Smoke: starting the hub binds the socket and a client can connect.
    #[tokio::test]
    async fn hub_start_accepts_a_connection() {
        let path = temp_socket_path("accept");
        let hub = LevelHub::start(path.clone()).await.expect("start hub");

        // Connect a client.
        let _client = UnixStream::connect(&path).await.expect("client connect");

        // Drive a frame through; the client should see 16 bytes.
        hub.frame_sink().publish(AudioFrame {
            seq: 7,
            min: -0.1,
            max: 0.2,
            peak_dbfs: -10.0,
        });

        let mut buf = [0u8; FRAME_BYTES];
        let mut client = _client;
        let read_fut = client.read_exact(&mut buf);
        let read_res = tokio::time::timeout(std::time::Duration::from_secs(2), read_fut).await;
        assert!(read_res.is_ok(), "timed out reading frame");
        read_res.unwrap().expect("read frame bytes");
        let got = AudioFrame::from_bytes(&buf);
        assert_eq!(got.seq, 7);

        // Cleanup.
        hub.cleanup();
    }

    /// Inject a panic in the accept loop and verify the watchdog rebinds
    /// the socket so that a fresh client can connect afterwards.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn watchdog_respawns_after_accept_panic() {
        // Holding this std::sync::Mutex across `.await` is intentional:
        // the lock serialises tests that touch the global panic flag,
        // and the critical section never contends with itself.
        let _guard = PANIC_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let path = temp_socket_path("respawn");
        let hub = LevelHub::start(path.clone()).await.expect("start hub");

        // Arm the panic, then poke the accept loop so it advances past
        // its `accept().await` and hits the panic check at the top of
        // the next iteration. The connect itself may succeed or fail
        // depending on whether the panic interleaves before or after the
        // kernel completes the SYN handshake; we don't care, we only
        // care that the watchdog respawns afterward.
        arm_global_panic();
        let _ = UnixStream::connect(&path).await;

        // Give the watchdog up to 5s to notice and rebind.
        let mut last_err = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let new_client = loop {
            match UnixStream::connect(&path).await {
                Ok(c) => break Some(c),
                Err(e) => {
                    last_err = Some(e);
                    if std::time::Instant::now() > deadline {
                        break None;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        };
        assert!(
            new_client.is_some(),
            "watchdog did not respawn the listener; last connect error: {:?}",
            last_err
        );

        // Frames continue to flow through the same FrameSink because it
        // dereferences the live HubState via Arc<HubInner>. The accept
        // loop may not yet have registered the subscriber with the
        // broadcast loop at the moment we publish, so emit a small burst
        // and let the reader pick up the first one that lands.
        let sink = hub.frame_sink();
        let mut client = new_client.unwrap();
        let mut buf = [0u8; FRAME_BYTES];

        let read_res = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                sink.publish(AudioFrame {
                    seq: 99,
                    min: 0.0,
                    max: 0.0,
                    peak_dbfs: -120.0,
                });
                let try_read = tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    client.read_exact(&mut buf),
                )
                .await;
                if let Ok(Ok(_)) = try_read {
                    return;
                }
            }
        })
        .await;
        assert!(read_res.is_ok(), "post-respawn read timed out");
        let got = AudioFrame::from_bytes(&buf);
        assert_eq!(got.seq, 99);

        hub.cleanup();
    }

    /// FrameSink should keep working after a respawn without being
    /// recreated.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn frame_sink_survives_respawn() {
        // See note in `watchdog_respawns_after_accept_panic`: holding
        // this lock across awaits is intentional test serialisation.
        let _guard = PANIC_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let path = temp_socket_path("sink-survives");
        let hub = LevelHub::start(path.clone()).await.expect("start hub");
        let sink = hub.frame_sink();

        // Snapshot the pre-respawn HubState pointer for a sanity check.
        let pre = Arc::as_ptr(&hub.inner.current()) as usize;

        arm_global_panic();
        {
            let _ = UnixStream::connect(&path).await;
        }

        // Wait until HubState pointer changes (i.e., respawn replaced it).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let now = Arc::as_ptr(&hub.inner.current()) as usize;
            if now != pre {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!("HubState was never replaced; respawn did not occur");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // The sink still works: publish does not panic, channel is live.
        sink.publish(AudioFrame {
            seq: 1,
            min: 0.0,
            max: 0.0,
            peak_dbfs: -120.0,
        });

        hub.cleanup();
    }
}
