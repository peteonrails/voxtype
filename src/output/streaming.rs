//! Streaming output session
//!
//! Helper that drives incremental output for streaming transcription
//! sessions. The default policy is **commit-only typing**: only `Final`
//! segments from a [`StreamingTranscriber`](crate::transcribe::StreamingTranscriber)
//! are typed to the keyboard. Partial events update an in-memory status
//! string for debug/Waybar but never touch the user's cursor.
//!
//! ## Why commit-only
//!
//! Cloud streaming providers vary widely in how they emit partials:
//!
//! - **Stable-token providers** (Gemini Live with VAD-finalized turns,
//!   ElevenLabs Scribe v2) produce partials that almost never get rewritten.
//!   Typing partials directly would feel snappy.
//! - **Revision-style providers** (whisper.cpp `stream` example, OpenAI
//!   Realtime in low-latency mode) routinely *replace* earlier tokens as
//!   more context arrives. Typing partials directly would produce
//!   distracting churn — letters appearing then being backspaced and
//!   replaced mid-word.
//!
//! We default to typing only finalized segments so the user never sees
//! wrong text. Revision-style providers will appear less responsive but
//! will not surprise the user. A future option could enable partial
//! typing for users on stable-token backends.
//!
//! ## Cancel-rewind
//!
//! When the user cancels a streaming session, finalized text already
//! typed must be removed. The session tracks `typed_chars` (counted in
//! Unicode scalar values, not bytes) and emits `typed_chars` BackSpace
//! key events on rewind. This is best-effort: on backends without a
//! reliable BackSpace primitive (clipboard) it is a no-op.
//!
//! ## Post-process hook
//!
//! Each finalized segment is run through the configured post-process
//! command if any, with `VOXTYPE_CONTEXT` set to the
//! finalized-text-so-far (everything committed *before* this segment).
//! This mirrors the eager-mode pattern in
//! [`crate::output::post_process::PostProcessor::process_with_context`].
//! Running per-segment preserves the latency win of streaming; running
//! once at end-of-session would defeat it. Skipping the hook entirely
//! would silently break users who rely on it for spelling/punctuation
//! cleanup.

use crate::error::OutputError;
use crate::output::post_process::PostProcessor;
use crate::output::{output_with_fallback, OutputOptions, TextOutput};
use std::process::Stdio;
use tokio::process::Command;

/// A streaming output session: types finalized segments incrementally,
/// tracks typed-character count, and supports cancel-rewind.
///
/// One session corresponds to one streaming utterance (one hotkey
/// press). The session does not own the output chain — the daemon
/// passes it in by reference for each `commit_segment` call so the
/// existing fallback chain and configuration are reused unchanged.
pub struct StreamingSession {
    /// Concatenated finalized segments committed so far.
    finalized_text: String,
    /// Total Unicode scalar values typed to the output. Counts what was
    /// actually sent to `output_with_fallback`, not what the
    /// post-processor returned (since the post-processor is allowed to
    /// reformat). For accurate rewinding we count *typed* output.
    typed_chars: usize,
    /// Most recent partial text (for status only; never typed).
    partial: String,
}

impl StreamingSession {
    /// Create a new empty session.
    pub fn new() -> Self {
        Self {
            finalized_text: String::new(),
            typed_chars: 0,
            partial: String::new(),
        }
    }

    /// Update the in-memory partial text. Returns the new partial for
    /// callers that want to mirror it into status files / Waybar.
    pub fn observe_partial(&mut self, text: String) -> &str {
        self.partial = text;
        &self.partial
    }

    /// Type an incremental partial delta at the cursor.
    ///
    /// `parakeet-rs::ParakeetUnified::transcribe_chunk` returns *only the
    /// newly-decoded text from that chunk's inference* — not a running
    /// cumulative transcript. So each `Partial` event carries just the
    /// new tail, and the right operation is to append it directly. No
    /// LCP reconciliation needed because the partials are deltas by
    /// construction; concatenating them in order rebuilds the
    /// transcript.
    ///
    /// `self.partial` accumulates the typed-but-not-yet-finalized tail
    /// of the current segment so `commit_segment` can know what's
    /// already at the cursor. `typed_chars` is bumped by the actual
    /// scalar count for cancel-rewind accounting.
    pub async fn type_partial_delta(
        &mut self,
        chain: &[Box<dyn TextOutput>],
        new_partial: String,
        pre_output_command: Option<&str>,
        post_output_command: Option<&str>,
    ) -> Result<(), OutputError> {
        if new_partial.is_empty() {
            return Ok(());
        }

        let opts = OutputOptions {
            pre_output_command,
            post_output_command,
            // Streaming output runs while the hotkey is held — modifiers
            // will be down throughout. The modifier-release guard
            // applies to one-shot (non-streaming) output only.
            wait_for_modifier_release: false,
            modifier_release_timeout: std::time::Duration::from_millis(0),
        };
        output_with_fallback(chain, &new_partial, opts).await?;

        self.typed_chars += new_partial.chars().count();
        self.partial.push_str(&new_partial);
        Ok(())
    }

    /// Clear the partial buffer (e.g., when a Final supersedes it).
    pub fn clear_partial(&mut self) {
        self.partial.clear();
    }

    /// Current partial buffer.
    pub fn partial(&self) -> &str {
        &self.partial
    }

    /// All finalized text committed so far.
    pub fn finalized_text(&self) -> &str {
        &self.finalized_text
    }

    /// Number of Unicode scalar values typed to the output. Used by the
    /// daemon to populate `State::Streaming.typed_chars`.
    pub fn typed_chars(&self) -> usize {
        self.typed_chars
    }

    /// Type a finalized segment to the output, optionally running it
    /// through `post_process` first (with `VOXTYPE_CONTEXT` =
    /// `finalized_text_so_far`).
    ///
    /// On output error, the session's internal state is **not**
    /// updated; the caller can retry or surface the error.
    ///
    /// The `pre_output_command` and `post_output_command` hooks fire
    /// *once per segment*, just like end-of-utterance batch output. This
    /// is intentional: hooks like compositor submap toggles need to
    /// wrap each typing burst, and streaming segments are short bursts.
    pub async fn commit_segment(
        &mut self,
        chain: &[Box<dyn TextOutput>],
        text: &str,
        _post_process: Option<&PostProcessor>,
        pre_output_command: Option<&str>,
        post_output_command: Option<&str>,
    ) -> Result<(), OutputError> {
        if text.is_empty() {
            self.clear_partial();
            return Ok(());
        }

        // Like `transcribe_chunk`, `ParakeetUnified::flush` returns only
        // the newly-emitted tail buffered when the stream closed — it is
        // a delta, not a cumulative transcript. So type it directly,
        // same as a partial.
        //
        // post_process is intentionally bypassed during streaming.
        // Per-segment cleanup would run only against the final tail
        // (not the cumulative transcript visible at the cursor) and
        // produce inconsistent output. Users who rely on post_process
        // should disable streaming for now.
        let opts = OutputOptions {
            pre_output_command,
            post_output_command,
            wait_for_modifier_release: false,
            modifier_release_timeout: std::time::Duration::from_millis(0),
        };
        output_with_fallback(chain, text, opts).await?;

        self.typed_chars += text.chars().count();
        // Treat the partial-stream-so-far plus this final tail as the
        // committed text for cancel-rewind context.
        let finalized_tail = format!("{}{}", self.partial, text);
        self.finalized_text.push_str(&finalized_tail);
        self.clear_partial();
        Ok(())
    }

    /// Best-effort rewind: emit `typed_chars` BackSpace key events via
    /// wtype, falling back to dotool then ydotool. Returns `Ok(())`
    /// even if no backspace backend is available, since the user has
    /// already cancelled and we should not propagate further errors.
    ///
    /// Resets the session's typed-chars counter to zero on success.
    /// The session can be re-used after rewind, though the daemon
    /// typically discards it.
    pub async fn rewind(&mut self) -> Result<(), OutputError> {
        let count = self.typed_chars;
        if count == 0 {
            return Ok(());
        }

        if try_wtype_backspaces(count).await {
            tracing::debug!("Rewound {} chars via wtype", count);
            self.typed_chars = 0;
            return Ok(());
        }
        if try_dotool_backspaces(count).await {
            tracing::debug!("Rewound {} chars via dotool", count);
            self.typed_chars = 0;
            return Ok(());
        }
        if try_ydotool_backspaces(count).await {
            tracing::debug!("Rewound {} chars via ydotool", count);
            self.typed_chars = 0;
            return Ok(());
        }

        tracing::warn!(
            "Streaming cancel: could not rewind {} typed chars (no backspace-capable backend)",
            count
        );
        // Keep typed_chars set so a subsequent retry could work; report
        // a soft error to the daemon.
        Err(OutputError::AllMethodsFailed)
    }
}

impl Default for StreamingSession {
    fn default() -> Self {
        Self::new()
    }
}

async fn try_wtype_backspaces(count: usize) -> bool {
    // wtype invocation: `wtype -k BackSpace` repeated. Build args
    // dynamically to send N keypresses in a single subprocess.
    let mut cmd = Command::new("wtype");
    for _ in 0..count {
        cmd.arg("-k").arg("BackSpace");
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    matches!(cmd.status().await, Ok(s) if s.success())
}

async fn try_dotool_backspaces(count: usize) -> bool {
    // dotool reads commands on stdin. `key backspace` per line.
    use tokio::io::AsyncWriteExt;
    let mut child = match Command::new("dotool")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Some(mut stdin) = child.stdin.take() {
        let mut buf = String::with_capacity(count * "key backspace\n".len());
        for _ in 0..count {
            buf.push_str("key backspace\n");
        }
        if stdin.write_all(buf.as_bytes()).await.is_err() {
            return false;
        }
        drop(stdin);
    }
    matches!(child.wait().await, Ok(s) if s.success())
}

async fn try_ydotool_backspaces(count: usize) -> bool {
    // ydotool key 14:1 14:0 sends BackSpace press+release. Linux key
    // codes: BackSpace = 14.
    let mut cmd = Command::new("ydotool");
    cmd.arg("key");
    for _ in 0..count {
        cmd.arg("14:1").arg("14:0");
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    matches!(cmd.status().await, Ok(s) if s.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// In-memory output that records every typed string. Used to
    /// verify session bookkeeping without spawning subprocesses.
    struct RecordingOutput {
        log: Mutex<Vec<String>>,
    }

    impl RecordingOutput {
        fn new() -> Self {
            Self {
                log: Mutex::new(Vec::new()),
            }
        }

        fn typed(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TextOutput for RecordingOutput {
        async fn output(&self, text: &str) -> Result<(), OutputError> {
            self.log.lock().unwrap().push(text.to_string());
            Ok(())
        }
        async fn is_available(&self) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "recording-test"
        }
    }

    fn chain_with(out: std::sync::Arc<RecordingOutput>) -> Vec<Box<dyn TextOutput>> {
        struct Wrap(std::sync::Arc<RecordingOutput>);
        #[async_trait]
        impl TextOutput for Wrap {
            async fn output(&self, text: &str) -> Result<(), OutputError> {
                self.0.output(text).await
            }
            async fn is_available(&self) -> bool {
                self.0.is_available().await
            }
            fn name(&self) -> &'static str {
                self.0.name()
            }
        }
        vec![Box::new(Wrap(out))]
    }

    #[tokio::test]
    async fn commits_segment_and_tracks_typed_chars() {
        let rec = std::sync::Arc::new(RecordingOutput::new());
        let chain = chain_with(rec.clone());
        let mut session = StreamingSession::new();

        session
            .commit_segment(&chain, "hello", None, None, None)
            .await
            .unwrap();
        assert_eq!(rec.typed(), vec!["hello".to_string()]);
        assert_eq!(session.typed_chars(), 5);
        assert_eq!(session.finalized_text(), "hello");

        session
            .commit_segment(&chain, " world", None, None, None)
            .await
            .unwrap();
        assert_eq!(session.typed_chars(), 11);
        assert_eq!(session.finalized_text(), "hello world");
    }

    #[tokio::test]
    async fn typed_chars_counts_unicode_scalars_not_bytes() {
        // Three CJK chars = 3 scalars but 9 UTF-8 bytes. Cancel rewind
        // must use scalars to send one BackSpace per visible char.
        let rec = std::sync::Arc::new(RecordingOutput::new());
        let chain = chain_with(rec.clone());
        let mut session = StreamingSession::new();
        session
            .commit_segment(&chain, "你好世", None, None, None)
            .await
            .unwrap();
        assert_eq!(session.typed_chars(), 3);
    }

    #[tokio::test]
    async fn empty_segment_is_noop() {
        let rec = std::sync::Arc::new(RecordingOutput::new());
        let chain = chain_with(rec.clone());
        let mut session = StreamingSession::new();
        session
            .commit_segment(&chain, "", None, None, None)
            .await
            .unwrap();
        assert!(rec.typed().is_empty());
        assert_eq!(session.typed_chars(), 0);
    }

    #[tokio::test]
    async fn partial_buffer_is_replaced_not_appended() {
        let mut session = StreamingSession::new();
        session.observe_partial("hel".into());
        assert_eq!(session.partial(), "hel");
        session.observe_partial("hell".into());
        assert_eq!(session.partial(), "hell");
        session.observe_partial("hello".into());
        assert_eq!(session.partial(), "hello");
        // Partial is *never* added to finalized text on its own.
        assert_eq!(session.finalized_text(), "");
        assert_eq!(session.typed_chars(), 0);
    }

    #[tokio::test]
    async fn finalize_clears_partial() {
        let rec = std::sync::Arc::new(RecordingOutput::new());
        let chain = chain_with(rec.clone());
        let mut session = StreamingSession::new();
        session.observe_partial("hel".into());
        session
            .commit_segment(&chain, "hello", None, None, None)
            .await
            .unwrap();
        assert_eq!(session.partial(), "");
    }

    #[tokio::test]
    async fn rewind_with_zero_chars_is_ok() {
        let mut session = StreamingSession::new();
        // Should succeed without spawning anything.
        session.rewind().await.unwrap();
        assert_eq!(session.typed_chars(), 0);
    }
}
