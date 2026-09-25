//! Daemon IPC for the on-screen visualizer.
//!
//! The daemon emits 16-byte [`AudioFrame`]s at 100 Hz over a Unix socket
//! (default `$XDG_RUNTIME_DIR/voxtype/audio.sock`). This module encapsulates:
//!
//! - The connect / read / reconnect loop, abstracted over a per-frame
//!   callback so each frontend can plug in its own state.
//! - A fixed-capacity ring buffer of decoded frames, used by the renderer
//!   to draw the scrolling waveform.
//!
//! The design goal is that the two frontends (`voxtype-osd-native` and
//! `voxtype-osd-gtk4`) can share an identical IPC surface and only differ
//! in their rendering stack.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tokio::time::sleep;

use crate::audio::levels::{default_socket_path, AudioFrame, FRAME_BYTES, FRAME_HZ};

/// Default ring buffer depth: 3 seconds at 100 Hz.
pub const DEFAULT_RING_DEPTH: usize = 300;

// Hold back 50 ms of audio so ordinary 10–40 ms capture batches can be
// interpolated at display refresh rate without stopping between arrivals.
const PLAYOUT_DELAY_FRAMES: f64 = FRAME_HZ as f64 * 0.05;

/// Fixed-capacity ring buffer of audio frames.
///
/// New frames overwrite the oldest. The renderer iterates in oldest-first
/// order via [`FrameRing::iter`] to draw the scrolling waveform.
pub struct FrameRing {
    buf: Vec<Option<AudioFrame>>,
    head: usize,
    len: usize,
    clock_origin: Option<Instant>,
    received: u64,
}

impl FrameRing {
    /// Keep the visible window plus interpolation history for a capture batch
    /// on either side of the delayed right edge.
    pub fn for_window(window_secs: f32) -> Self {
        let visible = (window_secs.max(0.01) as f64 * FRAME_HZ as f64).ceil() as usize;
        Self::new(visible + (PLAYOUT_DELAY_FRAMES * 2.0) as usize + 2)
    }

    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "FrameRing capacity must be > 0");
        Self {
            buf: vec![None; capacity],
            head: 0,
            len: 0,
            clock_origin: None,
            received: 0,
        }
    }

    pub fn push(&mut self, frame: AudioFrame) {
        self.push_at(frame, Instant::now());
    }

    fn push_at(&mut self, frame: AudioFrame, now: Instant) {
        // A new recording (or a discontinuity after reconnecting) must not
        // animate the previous recording's history. wrapping_add also handles
        // the wire counter rolling over during a long capture.
        if self
            .latest()
            .is_some_and(|last| frame.seq != last.seq.wrapping_add(1))
        {
            self.clear();
        }
        let origin = self.clock_origin.get_or_insert(now);
        // Re-anchor after an underrun, rather than jumping through a late
        // batch at socket-read speed. Normal batch arrivals never move this
        // clock: scrolling follows elapsed time, not delivery timing.
        let elapsed = now.duration_since(*origin).as_secs_f64() * FRAME_HZ as f64;
        if elapsed > self.received as f64 + PLAYOUT_DELAY_FRAMES {
            // Resume where the display stopped, without rewinding history to
            // refill the delay buffer or jumping straight to the late data.
            let stopped_at = self.received.saturating_sub(1) as f64;
            *origin = now
                - Duration::from_secs_f64((stopped_at + PLAYOUT_DELAY_FRAMES) / FRAME_HZ as f64);
        }
        self.received += 1;
        let cap = self.buf.len();
        self.buf[self.head] = Some(frame);
        self.head = (self.head + 1) % cap;
        if self.len < cap {
            self.len += 1;
        }
    }

    pub fn latest(&self) -> Option<AudioFrame> {
        if self.len == 0 {
            return None;
        }
        let cap = self.buf.len();
        let idx = (self.head + cap - 1) % cap;
        self.buf[idx]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Fractional position of the scrolling right edge relative to the newest
    /// frame. Negative values keep a small interpolation buffer; zero means
    /// the display caught up and waits for audio rather than inventing samples.
    pub fn scroll_offset(&self, now: Instant) -> f64 {
        let Some(origin) = self.clock_origin else {
            return 0.0;
        };
        (now.saturating_duration_since(origin).as_secs_f64() * FRAME_HZ as f64
            - self.received.saturating_sub(1) as f64
            - PLAYOUT_DELAY_FRAMES)
            .min(0.0)
    }

    /// Iterate over the buffered frames in oldest-first order.
    pub fn iter(&self) -> impl Iterator<Item = AudioFrame> + '_ {
        let cap = self.buf.len();
        let start = if self.len < cap {
            0
        } else {
            self.head // oldest is the position about to be overwritten
        };
        (0..self.len).filter_map(move |i| self.buf[(start + i) % cap])
    }

    /// Drop all buffered frames.
    pub fn clear(&mut self) {
        for slot in self.buf.iter_mut() {
            *slot = None;
        }
        self.head = 0;
        self.len = 0;
        self.clock_origin = None;
        self.received = 0;
    }
}

/// Resolve the socket path: explicit override, else the daemon's default.
pub fn resolve_socket_path(override_path: Option<PathBuf>) -> PathBuf {
    override_path.unwrap_or_else(default_socket_path)
}

/// Outcome of one connection attempt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionOutcome {
    /// The daemon closed the socket (recording ended, daemon shut down).
    Eof,
    /// We failed to connect (daemon not running yet).
    ConnectFailed,
    /// We were reading frames and hit a non-EOF error.
    ReadError,
}

/// Run one connect/read cycle, calling `on_frame` for each decoded frame.
///
/// Returns when the connection ends. The caller is expected to sleep and
/// retry per [`run_ipc_loop`], which composes this with a reconnect delay.
pub async fn run_one_connection<F>(socket_path: &Path, mut on_frame: F) -> ConnectionOutcome
where
    F: FnMut(AudioFrame),
{
    let mut stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("Cannot connect to {:?}: {}", socket_path, e);
            return ConnectionOutcome::ConnectFailed;
        }
    };
    tracing::info!("Connected to daemon at {:?}", socket_path);

    let mut buf = [0u8; FRAME_BYTES];
    loop {
        match stream.read_exact(&mut buf).await {
            Ok(_) => {
                let frame = AudioFrame::from_bytes(&buf);
                on_frame(frame);
            }
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                tracing::info!("Daemon closed the socket (EOF)");
                return ConnectionOutcome::Eof;
            }
            Err(e) => {
                tracing::warn!("Read error on audio socket: {}", e);
                return ConnectionOutcome::ReadError;
            }
        }
    }
}

/// Run the connect / read / reconnect loop forever.
///
/// `reconnect_secs` controls the gap between retry attempts when the
/// daemon is unavailable or the socket closes.
///
/// This function never returns under normal operation; it is intended to
/// be spawned on a Tokio runtime by each frontend.
pub async fn run_ipc_loop<F>(socket_path: PathBuf, reconnect_secs: f32, mut on_frame: F) -> !
where
    F: FnMut(AudioFrame) + Send,
{
    let delay = Duration::from_secs_f32(reconnect_secs.max(0.05));
    loop {
        let _ = run_one_connection(&socket_path, &mut on_frame).await;
        sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: u32) -> AudioFrame {
        AudioFrame {
            seq,
            min: -0.1,
            max: 0.1,
            peak_dbfs: -20.0,
        }
    }

    #[test]
    fn ring_keeps_latest_within_capacity() {
        let mut r = FrameRing::new(4);
        for i in 0..10 {
            r.push(frame(i));
        }
        assert_eq!(r.len(), 4);
        let latest = r.latest().unwrap();
        assert_eq!(latest.seq, 9);
    }

    #[test]
    fn ring_latest_none_when_empty() {
        let r = FrameRing::new(8);
        assert!(r.latest().is_none());
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
    }

    #[test]
    fn ring_grows_until_capacity() {
        let mut r = FrameRing::new(8);
        for i in 0..3 {
            r.push(frame(i));
        }
        assert_eq!(r.len(), 3);
        assert_eq!(r.latest().unwrap().seq, 2);
    }

    #[test]
    fn ring_iter_oldest_first_when_full() {
        let mut r = FrameRing::new(4);
        for i in 0..6 {
            r.push(frame(i));
        }
        let seqs: Vec<u32> = r.iter().map(|f| f.seq).collect();
        // After 6 pushes into a 4-deep ring: contents are 2,3,4,5 oldest-first.
        assert_eq!(seqs, vec![2, 3, 4, 5]);
    }

    #[test]
    fn ring_iter_oldest_first_partial() {
        let mut r = FrameRing::new(4);
        r.push(frame(7));
        r.push(frame(8));
        let seqs: Vec<u32> = r.iter().map(|f| f.seq).collect();
        assert_eq!(seqs, vec![7, 8]);
    }

    #[test]
    fn ring_clear_resets_state() {
        let mut r = FrameRing::new(4);
        for i in 0..3 {
            r.push(frame(i));
        }
        r.clear();
        assert_eq!(r.len(), 0);
        assert!(r.latest().is_none());
        r.push(frame(99));
        assert_eq!(r.latest().unwrap().seq, 99);
    }

    #[test]
    fn scroll_clock_is_continuous_across_capture_batches_and_ring_wrap() {
        // At 60, 144 and 240 Hz, each displayed frame must advance by elapsed
        // time even though four audio frames arrive together every 40 ms.
        for refresh_hz in [60, 144, 240] {
            let mut ring = FrameRing::new(20);
            let start = Instant::now();
            let mut received = 0;
            for tick in 0..refresh_hz * 2 {
                let seconds = tick as f64 / refresh_hz as f64;
                let now = start + Duration::from_secs_f64(seconds);
                let available = (seconds / 0.04).floor() as u32 * 4 + 4;
                while received < available {
                    ring.push_at(frame(received), now);
                    received += 1;
                }
                let position = received as f64 - 1.0 + ring.scroll_offset(now);
                let expected = seconds * FRAME_HZ as f64 - PLAYOUT_DELAY_FRAMES;
                assert!(
                    (position - expected).abs() < 1e-5,
                    "{refresh_hz} Hz, tick {tick}: {position} != {expected}"
                );
            }
        }
    }

    #[test]
    fn recording_restart_clears_history_and_clock() {
        let start = Instant::now();
        let mut ring = FrameRing::new(10);
        ring.push_at(frame(4), start);
        ring.push_at(frame(5), start + Duration::from_millis(10));
        let restart = start + Duration::from_secs(1);
        ring.push_at(frame(0), restart);
        assert_eq!(ring.len(), 1);
        assert_eq!(ring.scroll_offset(restart), -PLAYOUT_DELAY_FRAMES);
    }

    #[test]
    fn wire_sequence_wrap_keeps_history() {
        let start = Instant::now();
        let mut ring = FrameRing::new(10);
        ring.push_at(frame(u32::MAX), start);
        ring.push_at(frame(0), start + Duration::from_millis(10));
        assert_eq!(ring.len(), 2);
    }

    #[test]
    fn stalled_input_stops_at_latest_sample_and_recovers() {
        let start = Instant::now();
        let mut ring = FrameRing::new(10);
        ring.push_at(frame(0), start);
        let later = start + Duration::from_secs(1);
        assert_eq!(ring.scroll_offset(later), 0.0);
        ring.push_at(frame(1), later);
        assert_eq!(ring.scroll_offset(later), -1.0);
    }
}
