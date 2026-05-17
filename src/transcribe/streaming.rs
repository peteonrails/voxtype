//! Streaming transcription support
//!
//! Defines the [`StreamingTranscriber`] trait and supporting event types
//! for backends that emit transcribed text incrementally as audio arrives,
//! rather than after the user releases the hotkey.
//!
//! ## Why a separate trait
//!
//! The classic [`Transcriber`](super::Transcriber) trait is a one-shot
//! operation: take the recorded buffer, return the final text. Streaming
//! providers (Gemini Live, OpenAI Realtime, ElevenLabs Scribe v2, the
//! whisper.cpp `stream` example, etc.) instead push partial and final
//! segments over a long-lived connection.
//!
//! Rather than retrofit every existing backend, streaming-capable backends
//! opt in by implementing this trait *in addition* to `Transcriber`. The
//! daemon discovers streaming capability via
//! [`Transcriber::as_streaming`](super::Transcriber::as_streaming).
//!
//! ## Lifecycle
//!
//! ```text
//!     daemon                              backend
//!       │                                    │
//!       │  start_stream(samples_rx)          │
//!       │ ─────────────────────────────────▶ │
//!       │                                    │ (open WS / start worker)
//!       │  StreamHandle{events, cancel}      │
//!       │ ◀───────────────────────────────── │
//!       │                                    │
//!       │  push samples on samples_rx        │
//!       │ ─────────────────────────────────▶ │
//!       │                                    │
//!       │            Partial / Final         │
//!       │ ◀───────────────────────────────── │
//!       │            ...                     │
//!       │                                    │
//!       │  drop samples_rx (graceful end)    │
//!       │ ─────────────────────────────────▶ │
//!       │                                    │ (flush, close)
//!       │              Ended                 │
//!       │ ◀───────────────────────────────── │
//! ```
//!
//! The daemon may also send on `cancel` to abort early. After cancel,
//! backends should stop emitting events as soon as practical.

use crate::error::TranscribeError;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Identifier for a logical segment within a streaming session.
///
/// Backends are free to define what constitutes a segment — for VAD-driven
/// providers it usually corresponds to one utterance between pauses; for
/// fixed-window providers it may be one chunk. The daemon treats segments
/// as opaque grouping for *Partial* → *Final* progression.
pub type SegmentId = u64;

/// Events emitted by a streaming transcription backend.
///
/// Not `Clone`: the `Error` variant carries [`TranscribeError`], which is
/// not clonable. Events flow one-to-one over an mpsc channel, so cloning
/// is not needed in normal use.
#[derive(Debug)]
pub enum StreamingEvent {
    /// In-progress text for a segment. May be revised by later partials
    /// or superseded by a `Final` event with the same `segment_id`.
    Partial { text: String, segment_id: SegmentId },

    /// Committed text for a segment. The daemon's default output policy
    /// is to type only `Final` segments, so revision-style providers do
    /// not produce visible churn.
    Final { text: String, segment_id: SegmentId },

    /// The backend has finished processing all audio sent so far and is
    /// closing the stream gracefully (e.g., the daemon dropped the
    /// samples sender and the backend has flushed).
    Ended,

    /// A non-recoverable error occurred. The daemon should treat this
    /// like `Ended` for state purposes and surface the error to the user.
    Error(TranscribeError),
}

/// Handle returned by [`StreamingTranscriber::start_stream`].
///
/// The daemon polls `events` and may signal early termination via
/// `cancel`. `task` is held so the daemon can `await` shutdown.
pub struct StreamHandle {
    /// Events from the backend (partials, finals, end, errors).
    pub events: mpsc::Receiver<StreamingEvent>,

    /// Send `()` to abort the stream early. Dropping this sender is a
    /// no-op; backends should also treat the samples sender being
    /// dropped as a graceful end-of-input.
    pub cancel: oneshot::Sender<()>,

    /// The background task driving the backend connection. Awaiting this
    /// after `events` produces `Ended` ensures resources are reclaimed.
    pub task: JoinHandle<Result<(), TranscribeError>>,
}

/// Trait for transcription backends that can stream partial results.
///
/// Implementors should:
///
/// 1. Accept 16 kHz mono `f32` samples on `samples_rx` (matching
///    [`crate::audio::AudioCapture`]'s output).
/// 2. Emit at least one `Final` per committed utterance, plus zero or
///    more `Partial`s leading up to it.
/// 3. Emit `Ended` exactly once when the stream closes cleanly. If the
///    `cancel` signal fires, backends may emit `Ended` immediately
///    without flushing pending partials.
/// 4. Treat the `samples_rx` sender being dropped as graceful EOF.
///
/// Errors during setup (auth, network, model load) should be returned
/// from `start_stream` directly. Errors during the session should arrive
/// as `StreamingEvent::Error` followed by `Ended`.
pub trait StreamingTranscriber: Send + Sync {
    /// Open a streaming session.
    ///
    /// `samples_rx` is the channel from which the backend will pull
    /// audio. The daemon will keep this sender alive for as long as the
    /// user is recording, then drop it to signal end-of-input.
    fn start_stream(
        &self,
        samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_event_partial_fields() {
        let ev = StreamingEvent::Partial {
            text: "hello".into(),
            segment_id: 1,
        };
        match ev {
            StreamingEvent::Partial { text, segment_id } => {
                assert_eq!(text, "hello");
                assert_eq!(segment_id, 1);
            }
            _ => panic!("expected Partial"),
        }
    }

    #[test]
    fn streaming_event_final_distinct_from_partial() {
        let p = StreamingEvent::Partial {
            text: "x".into(),
            segment_id: 1,
        };
        let f = StreamingEvent::Final {
            text: "x".into(),
            segment_id: 1,
        };
        // Sanity: discriminant difference matters for the daemon's match arms.
        assert!(matches!(p, StreamingEvent::Partial { .. }));
        assert!(matches!(f, StreamingEvent::Final { .. }));
    }
}
