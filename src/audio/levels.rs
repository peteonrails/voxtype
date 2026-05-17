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
//! ## Performance
//!
//! - No allocations in the hot path (per-sample). The input chunks are
//!   already allocated by cpal_capture; bucketing reuses a small fixed
//!   `[f32; 2]` accumulator.
//! - Subscriber writes use non-blocking `try_send` on a bounded queue per
//!   client; clients that can't keep up are dropped, not buffered.
//! - Idle: zero work. The hub only spins up a forwarder task per
//!   recording session and tears it down when the session ends.

use crate::config::Config;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Mutex};

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
#[derive(Clone)]
pub struct LevelHub {
    inner: Arc<HubInner>,
}

struct HubInner {
    /// Bounded mpsc channel: recording-session producers send frames here,
    /// the broadcaster task drains it and fans out to clients.
    broadcast_tx: mpsc::Sender<AudioFrame>,
    /// Running count of attached subscribers, for telemetry/logging.
    subscriber_count: Mutex<usize>,
    socket_path: PathBuf,
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
        // Remove any stale socket from a prior run.
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }

        let listener = UnixListener::bind(&socket_path)?;
        tracing::info!("Audio level socket listening at {:?}", socket_path);

        // Frame fan-in: any number of recording-session producers can send.
        // 200 frames = 2 seconds of buffered headroom at 100 Hz, plenty.
        let (broadcast_tx, broadcast_rx) = mpsc::channel::<AudioFrame>(200);

        let inner = Arc::new(HubInner {
            broadcast_tx,
            subscriber_count: Mutex::new(0),
            socket_path: socket_path.clone(),
        });

        // The list of active subscriber senders is owned by the
        // broadcaster task, not the hub, so we don't need a lock around
        // it on the hot path.
        let (sub_tx, sub_rx) = mpsc::unbounded_channel::<SubscriberSlot>();

        // Accept loop: per-connection senders are forwarded to the
        // broadcaster task via `sub_tx`.
        let inner_for_accept = inner.clone();
        tokio::spawn(async move {
            run_accept_loop(listener, sub_tx, inner_for_accept).await;
        });

        // Broadcast loop: drains incoming frames, fans out to all
        // connected subscribers, drops any whose queue is full.
        tokio::spawn(async move {
            run_broadcast_loop(broadcast_rx, sub_rx).await;
        });

        Ok(Self { inner })
    }

    /// Returns a sender that recording sessions can use to publish frames.
    ///
    /// Sending is bounded; if the broadcaster falls behind we drop frames
    /// rather than back-pressure the audio thread.
    pub fn frame_sink(&self) -> FrameSink {
        FrameSink {
            tx: self.inner.broadcast_tx.clone(),
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

/// Sender handle handed out by [`LevelHub::frame_sink`].
#[derive(Clone)]
pub struct FrameSink {
    tx: mpsc::Sender<AudioFrame>,
}

impl FrameSink {
    /// Try to publish a frame. Drops the frame if the broadcaster is
    /// backed up. Never blocks and never allocates.
    #[inline]
    pub fn publish(&self, frame: AudioFrame) {
        let _ = self.tx.try_send(frame);
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
    inner: Arc<HubInner>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let (tx, rx) = mpsc::channel::<AudioFrame>(SUBSCRIBER_QUEUE_DEPTH);
                let slot = SubscriberSlot { tx };
                if sub_tx.send(slot).is_err() {
                    // Broadcaster has shut down; close the new connection.
                    drop(stream);
                    break;
                }

                let inner_for_writer = inner.clone();
                tokio::spawn(async move {
                    {
                        let mut count = inner_for_writer.subscriber_count.lock().await;
                        *count += 1;
                        tracing::debug!("Audio subscriber connected (count={})", *count);
                    }
                    run_subscriber_writer(stream, rx).await;
                    {
                        let mut count = inner_for_writer.subscriber_count.lock().await;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
