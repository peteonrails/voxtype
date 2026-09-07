//! Live partial transcript, published for the OSD to draw.
//!
//! ## Why a file and not the audio socket
//!
//! The daemon's socket carries fixed 16-byte audio frames, and
//! [`StreamingEvent::Partial`](crate::transcribe::StreamingEvent::Partial)
//! is already spoken for: the daemon *types* partials at the cursor,
//! treating them as the newly-decoded delta that parakeet's chunked
//! pipeline produces. A sliding-window backend like GigaAM re-transcribes
//! the whole uncommitted tail instead, so its partials are cumulative
//! revisions — routing those through the same event would retype the tail
//! on every update.
//!
//! So this text takes its own path: the backend publishes it here, the OSD
//! reads it while it paints, and nothing it says ever reaches the cursor.
//!
//! ponytail: a file polled on redraw, not a push channel. It costs one
//! small read per painted frame, only while the OSD is visible. Give it a
//! second Unix socket if it ever needs to drive something that is not
//! already repainting at frame rate.

use std::path::PathBuf;

use crate::config::Config;

/// Where the current partial lives, beside the audio socket.
pub fn partial_path() -> PathBuf {
    Config::runtime_dir().join("partial.txt")
}

/// Publish the in-progress text. Best effort: the OSD is a nicety, and a
/// failed write must never disturb transcription.
///
/// Written to a temporary file and renamed so a reader mid-paint sees
/// either the previous text or the new one, never a half-written line.
pub fn publish(text: &str) {
    let path = partial_path();
    let tmp = path.with_extension("tmp");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&tmp, text.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Clear the partial — the text has been committed, or the session ended.
pub fn clear() {
    publish("");
}

/// Read the current partial, if any. `None` when empty or unreadable.
pub fn read() -> Option<String> {
    let text = std::fs::read_to_string(partial_path()).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}
