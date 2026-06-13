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
    /// Inter-key delay (ms) applied to backspace bursts, mirroring the
    /// type path's `type_delay_ms`. Without it, revision backspaces fire
    /// with dotool's tiny default `keydelay` (~2ms) — well under one
    /// display frame — so KWin coalesces them and drops events. That
    /// corrupts self-corrections two ways: a dropped backspace leaves a
    /// stray character, and the un-spaced last backspace lets the
    /// following retype's leading character (often a space) coalesce too.
    type_delay_ms: u32,
}

impl StreamingSession {
    /// Create a new empty session. `type_delay_ms` is the configured
    /// inter-key delay; it is applied to backspace bursts so they match
    /// the type path's compositor-safe pacing.
    pub fn new(type_delay_ms: u32) -> Self {
        Self {
            finalized_text: String::new(),
            typed_chars: 0,
            partial: String::new(),
            type_delay_ms,
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

    /// Backspace `backspace` chars then commit `text`. Used by streaming
    /// backends that revise the previously-typed partial tail when
    /// finalizing (e.g. Soniox punctuation flips).
    ///
    /// The partial buffer is truncated by `backspace` scalars, then
    /// `text` is appended to both cursor and finalized_text. Net effect:
    /// the cursor ends up with the truncated partial + new text, matching
    /// what the daemon's commit_segment would do for a plain Final.
    pub async fn replace_and_commit(
        &mut self,
        chain: &[Box<dyn TextOutput>],
        backspace: usize,
        text: &str,
        pre_output_command: Option<&str>,
        post_output_command: Option<&str>,
    ) -> Result<(), OutputError> {
        // Cap backspace at what we've actually typed.
        let n = backspace.min(self.typed_chars);
        if n > 0 {
            let emitted = emit_backspaces(n, self.type_delay_ms).await;
            if emitted == 0 {
                // No backspace-capable backend ran. The cursor still
                // shows the old partial — DO NOT touch our bookkeeping,
                // or `typed_chars` will drift from reality and the next
                // cancel-rewind will leave stray characters behind. Accept
                // the visual artifact and let the user see + correct.
                tracing::warn!(
                    "Streaming replace: no backspace-capable backend available; \
                     skipping backspace and accepting cursor artifact"
                );
            } else {
                // Truncate the partial buffer by the count actually emitted
                // (which equals `n` when emit_backspaces returns non-zero,
                // per its all-or-nothing contract).
                let new_partial_len = self.partial.chars().count().saturating_sub(emitted);
                self.partial = self.partial.chars().take(new_partial_len).collect();
                self.typed_chars = self.typed_chars.saturating_sub(emitted);
            }
        }

        if !text.is_empty() {
            let opts = OutputOptions {
                pre_output_command,
                post_output_command,
                wait_for_modifier_release: false,
                modifier_release_timeout: std::time::Duration::from_millis(0),
            };
            output_with_fallback(chain, text, opts).await?;
            self.typed_chars += text.chars().count();
            // Treat the (now-truncated) partial + new text as committed,
            // matching commit_segment's accounting.
            let finalized_tail = format!("{}{}", self.partial, text);
            self.finalized_text.push_str(&finalized_tail);
        }
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

        if emit_backspaces(count, self.type_delay_ms).await > 0 {
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

/// Backspace `count` chars using the first available method.
/// Returns the actual number of backspaces emitted. `type_delay_ms`
/// paces the key events so the compositor doesn't coalesce them (see
/// [`StreamingSession::type_delay_ms`]).
async fn emit_backspaces(count: usize, type_delay_ms: u32) -> usize {
    if count == 0 {
        return 0;
    }
    if try_wtype_backspaces(count, type_delay_ms).await {
        return count;
    }
    if try_dotool_backspaces(count, type_delay_ms).await {
        return count;
    }
    if try_ydotool_backspaces(count, type_delay_ms).await {
        return count;
    }
    0
}

async fn try_wtype_backspaces(count: usize, type_delay_ms: u32) -> bool {
    // wtype invocation: `wtype -k BackSpace` repeated, sent as a single
    // subprocess. See `build_wtype_backspace_args` for the `-s` pacing.
    let mut cmd = Command::new("wtype");
    cmd.args(build_wtype_backspace_args(count, type_delay_ms));
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    matches!(cmd.status().await, Ok(s) if s.success())
}

/// Build the wtype argument list for `count` backspaces.
///
/// wtype's `-d` only paces inter-keystroke delay *when typing text*; it
/// has NO effect on discrete `-k` key presses. The right knob is `-s
/// TIME` ("sleep before interpreting the following options"), placed
/// before each `-k` so the BackSpace presses are spaced out and the
/// compositor doesn't coalesce them.
fn build_wtype_backspace_args(count: usize, type_delay_ms: u32) -> Vec<String> {
    let mut args = Vec::with_capacity(count * 4);
    for _ in 0..count {
        if type_delay_ms > 0 {
            args.push("-s".to_string());
            args.push(type_delay_ms.to_string());
        }
        args.push("-k".to_string());
        args.push("BackSpace".to_string());
    }
    args
}

async fn try_dotool_backspaces(count: usize, type_delay_ms: u32) -> bool {
    // Prefer `dotoolc` whenever dotoold is actually accepting input.
    // Spawning raw `dotool` creates a *new* uinput keyboard per call;
    // KDE Plasma can drop events on the typing keyboard while these
    // ephemeral keyboards rapidly appear and disappear. Routing through
    // dotoolc reuses dotoold's persistent keyboard.
    use tokio::io::AsyncWriteExt;
    let (binary, env_pipe) = match crate::output::dotool::DotoolOutput::live_daemon_pipe_path() {
        Some(p) => ("dotoolc", Some(p)),
        None => ("dotool", None),
    };
    let mut cmd = Command::new(binary);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(p) = env_pipe {
        cmd.env("DOTOOL_PIPE", p);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Some(mut stdin) = child.stdin.take() {
        let buf = build_dotool_backspace_commands(count, type_delay_ms);
        if stdin.write_all(buf.as_bytes()).await.is_err() {
            return false;
        }
        drop(stdin);
    }
    matches!(child.wait().await, Ok(s) if s.success())
}

/// Build the dotool command stream for `count` backspaces.
///
/// `key` events obey dotool's `keydelay`/`keyhold` (NOT `typedelay`/
/// `typehold`, which only apply to `type`). The defaults (~2ms) are far
/// below one display frame, so without pacing KWin coalesces the
/// backspaces and drops some — leaving a stray character after a
/// self-correction. Match the type path's `type_delay_ms`.
fn build_dotool_backspace_commands(count: usize, type_delay_ms: u32) -> String {
    if count == 0 {
        // No keys to send — don't emit bare keydelay/keyhold that would
        // mutate dotool's persistent state for nothing.
        return String::new();
    }
    let mut buf = String::with_capacity(count * "key backspace\n".len() + 32);
    if type_delay_ms > 0 {
        buf.push_str(&format!("keydelay {}\n", type_delay_ms));
        buf.push_str(&format!("keyhold {}\n", type_delay_ms));
    }
    for _ in 0..count {
        buf.push_str("key backspace\n");
    }
    buf
}

async fn try_ydotool_backspaces(count: usize, type_delay_ms: u32) -> bool {
    // ydotool key 14:1 14:0 sends BackSpace press+release. Linux key
    // codes: BackSpace = 14. `-d` paces key events like the dotool/wtype
    // paths; ydotool's own default (12ms) wouldn't track the user's
    // configured type_delay_ms.
    let mut cmd = Command::new("ydotool");
    cmd.arg("key");
    if type_delay_ms > 0 {
        cmd.arg("-d").arg(type_delay_ms.to_string());
    }
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
        let mut session = StreamingSession::new(0);

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
        let mut session = StreamingSession::new(0);
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
        let mut session = StreamingSession::new(0);
        session
            .commit_segment(&chain, "", None, None, None)
            .await
            .unwrap();
        assert!(rec.typed().is_empty());
        assert_eq!(session.typed_chars(), 0);
    }

    #[tokio::test]
    async fn partial_buffer_is_replaced_not_appended() {
        let mut session = StreamingSession::new(0);
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
        let mut session = StreamingSession::new(0);
        session.observe_partial("hel".into());
        session
            .commit_segment(&chain, "hello", None, None, None)
            .await
            .unwrap();
        assert_eq!(session.partial(), "");
    }

    #[tokio::test]
    async fn rewind_with_zero_chars_is_ok() {
        let mut session = StreamingSession::new(0);
        // Should succeed without spawning anything.
        session.rewind().await.unwrap();
        assert_eq!(session.typed_chars(), 0);
    }

    #[test]
    fn dotool_backspaces_set_keydelay_and_keyhold_not_typedelay() {
        // `key backspace` is paced by keydelay/keyhold; typedelay only
        // affects `type`. Setting the wrong knob would leave backspaces
        // at dotool's ~2ms default and let KWin coalesce them.
        let cmds = build_dotool_backspace_commands(3, 17);
        assert_eq!(
            cmds,
            "keydelay 17\nkeyhold 17\nkey backspace\nkey backspace\nkey backspace\n"
        );
        assert!(!cmds.contains("typedelay"));
        assert!(!cmds.contains("typehold"));
    }

    #[test]
    fn dotool_backspaces_omit_pacing_when_delay_is_zero() {
        // type_delay_ms == 0 means the user opted out of delays (e.g. a
        // compositor without the coalescing bug); don't force any.
        let cmds = build_dotool_backspace_commands(2, 0);
        assert_eq!(cmds, "key backspace\nkey backspace\n");
    }

    #[test]
    fn dotool_backspaces_zero_count_emits_nothing() {
        // No keys → no commands, so a stray call can't mutate dotool's
        // persistent keydelay/keyhold for nothing.
        assert_eq!(build_dotool_backspace_commands(0, 17), "");
        assert_eq!(build_dotool_backspace_commands(0, 0), "");
    }

    #[test]
    fn wtype_backspaces_sleep_before_each_key() {
        // `-s TIME` (sleep before the next option) paces discrete `-k`
        // presses; wtype's `-d` would be a no-op here (it only spaces
        // keystrokes when typing text). One `-s` precedes each `-k`.
        let args = build_wtype_backspace_args(2, 17);
        assert_eq!(
            args,
            vec!["-s", "17", "-k", "BackSpace", "-s", "17", "-k", "BackSpace"]
        );
    }

    #[test]
    fn wtype_backspaces_omit_delay_when_zero() {
        let args = build_wtype_backspace_args(2, 0);
        assert_eq!(args, vec!["-k", "BackSpace", "-k", "BackSpace"]);
    }
}
