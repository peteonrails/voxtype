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
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tokio::time::sleep;

use crate::audio::levels::{default_socket_path, AudioFrame, FRAME_BYTES};

/// Default ring buffer depth: 3 seconds at 100 Hz.
pub const DEFAULT_RING_DEPTH: usize = 300;

/// Fixed-capacity ring buffer of audio frames.
///
/// New frames overwrite the oldest. The renderer iterates in oldest-first
/// order via [`FrameRing::iter`] to draw the scrolling waveform.
pub struct FrameRing {
    buf: Vec<Option<AudioFrame>>,
    head: usize,
    len: usize,
}

impl FrameRing {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "FrameRing capacity must be > 0");
        Self {
            buf: vec![None; capacity],
            head: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, frame: AudioFrame) {
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
}
