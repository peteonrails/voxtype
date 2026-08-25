//! Sliding-window streaming transcription.
//!
//! Ported from nova-npu's `SlidingWindowTranscriber` (MIT) and adapted to
//! voxtype's [`StreamingTranscriber`] trait. Instead of segmenting audio
//! into small VAD-delimited chunks (which degrades Whisper accuracy), this
//! keeps a rolling buffer of the full recording and re-transcribes the
//! whole window every `interval_s` seconds. Because whisper.cpp / OpenVINO
//! inference is fast relative to speech, this gives the model full acoustic
//! context on every pass.
//!
//! New text is extracted by diffing successive transcriptions against the
//! already-committed text, and only the stable tail delta is emitted. The
//! daemon appends each emitted delta at the cursor, so **events must always
//! be deltas** — never cumulative transcripts (or the cursor receives
//! duplicates). The stable-prefix commit policy below guarantees this.
//!
//! ## Commit policy
//!
//! - **Growing mode** (buffer shorter than `max_buffer_seconds`): commit
//!   the common-prefix words between the previous and current Whisper
//!   outputs, gated by `partial_min_words`, advancing `confirmed_words` so
//!   each emission is strictly-new stable text.
//! - **Sliding mode** (buffer trimmed once it wraps): diff the whole
//!   Whisper output against the already-emitted text and commit only the
//!   common prefix of the current delta vs. the previous delta — a tail
//!   word is only committed once stable across two consecutive passes.
//!
//! The engine wraps any batch [`Transcriber`] (whisper.cpp, OpenVINO GenAI,
//! …), so the same code powers every streaming backend.
//!
//! ## Event mapping
//!
//! Each committed delta is emitted as a [`StreamingEvent::Partial`] (typed
//! live at the cursor) when `type_partials` is true, or a
//! [`StreamingEvent::Final`] (commit-only) when false. On graceful EOF the
//! remaining tail delta is emitted as a `Final`, then [`StreamingEvent::Ended`].
//! Cancellation ends the stream with `Ended` and no flush.

use super::streaming::{StreamHandle, StreamingEvent, StreamingTranscriber};
use super::Transcriber;
use crate::error::TranscribeError;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, MissedTickBehavior};

/// Whisper silence-hallucination phrases. When Whisper is fed silence or
/// noise it frequently emits one of these; drop the result entirely.
const HALLUCINATION_PATTERNS: &[&str] = &[
    "you",
    "the",
    "i",
    "a",
    "it",
    "is",
    "and",
    "to",
    "thank you",
    "thanks",
    "bye",
    "okay",
    "thank you for watching",
    "thanks for watching",
    "please subscribe",
    "subscribe",
    "thank you very much",
    "you're welcome",
    "good night",
    "good bye",
    "see you next time",
    "subtitles by the amara.org community",
];

/// Minimum RMS energy for the whole buffer to be treated as speech.
/// Below this we skip transcription entirely (prevents hallucination).
const MIN_SPEECH_RMS: f32 = 0.005;

/// Tuning knobs for the sliding-window engine. Defaults mirror nova-npu.
#[derive(Debug, Clone, Copy)]
pub struct SlidingWindowConfig {
    /// Re-transcribe the whole buffer every `interval_s` seconds.
    pub interval_s: f64,
    /// Maximum buffered audio before the window starts sliding (drops old
    /// samples). Hard-coded to 29.0 s in nova (Whisper context limit).
    pub max_buffer_s: f32,
    /// Assumed sample rate (16 kHz mono for whisper).
    pub sample_rate: u32,
    /// Skip transcription while whole-buffer RMS is below this.
    pub min_speech_rms: f32,
    /// Minimum buffered audio (seconds) before first transcription.
    pub min_audio_s: f32,
    /// Minimum number of new stable words before committing a delta.
    pub partial_min_words: usize,
    /// Emit committed deltas as `Partial` (typed live at the cursor) when
    /// true; emit them as commit-only `Final` segments when false.
    pub type_partials: bool,
}

impl Default for SlidingWindowConfig {
    fn default() -> Self {
        Self {
            interval_s: 0.8,
            max_buffer_s: 29.0,
            sample_rate: 16_000,
            min_speech_rms: MIN_SPEECH_RMS,
            min_audio_s: 1.0,
            partial_min_words: 2,
            type_partials: true,
        }
    }
}

/// Sliding-window streaming transcriber that wraps any batch [`Transcriber`].
///
/// Implements both [`Transcriber`] (delegating to the wrapped backend) and
/// [`StreamingTranscriber`] (the live-delta engine). The factory constructs
/// this wrapper when the engine's `streaming` config flag is set.
pub struct SlidingWindowStreamingTranscriber {
    base: Arc<dyn Transcriber>,
    config: SlidingWindowConfig,
}

impl SlidingWindowStreamingTranscriber {
    /// Wrap `base` in the sliding-window streaming engine.
    pub fn new(base: Arc<dyn Transcriber>, config: SlidingWindowConfig) -> Self {
        Self { base, config }
    }
}

impl Transcriber for SlidingWindowStreamingTranscriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        self.base.transcribe(samples)
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        Some(self)
    }
}

impl StreamingTranscriber for SlidingWindowStreamingTranscriber {
    fn start_stream(
        &self,
        mut samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        let (events_tx, events_rx) = mpsc::channel::<StreamingEvent>(64);
        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();

        let base = Arc::clone(&self.base);
        let config = self.config;

        let task = tokio::task::spawn_blocking(move || -> Result<(), TranscribeError> {
            let runtime = tokio::runtime::Handle::current();
            let mut session = Session::new(base, config);

            // Skip the interval's immediate first tick so we don't
            // re-transcribe an empty buffer.
            let mut ticker = interval(Duration::from_secs_f64(config.interval_s));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            runtime.block_on(ticker.tick());

            enum Outcome {
                Chunk(Vec<f32>),
                Eof,
                Tick,
                Cancelled,
            }

            loop {
                let outcome = runtime.block_on(async {
                    tokio::select! {
                        chunk = samples_rx.recv() => match chunk {
                            Some(c) => Outcome::Chunk(c),
                            None => Outcome::Eof,
                        },
                        _ = ticker.tick() => Outcome::Tick,
                        _ = &mut cancel_rx => Outcome::Cancelled,
                    }
                });

                match outcome {
                    Outcome::Chunk(chunk) => session.feed(&chunk),
                    Outcome::Cancelled => {
                        // Abort promptly; contract allows ending without flush.
                        let _ = runtime.block_on(events_tx.send(StreamingEvent::Ended));
                        return Ok(());
                    }
                    Outcome::Eof => {
                        // Graceful end: one last flush so no audio is lost.
                        if let Some(tail) = session.final_flush() {
                            let _ = runtime.block_on(events_tx.send(StreamingEvent::Final {
                                text: tail,
                                segment_id: 0,
                            }));
                        }
                        let _ = runtime.block_on(events_tx.send(StreamingEvent::Ended));
                        return Ok(());
                    }
                    Outcome::Tick => match session.on_tick() {
                        Ok(deltas) => {
                            for delta in deltas {
                                let event = if config.type_partials {
                                    StreamingEvent::Partial {
                                        text: delta,
                                        segment_id: 0,
                                    }
                                } else {
                                    StreamingEvent::Final {
                                        text: delta,
                                        segment_id: 0,
                                    }
                                };
                                let _ = runtime.block_on(events_tx.send(event));
                            }
                        }
                        Err(err) => {
                            let _ = runtime.block_on(events_tx.send(StreamingEvent::Error(err)));
                            let _ = runtime.block_on(events_tx.send(StreamingEvent::Ended));
                            return Ok(());
                        }
                    },
                }
            }
        });

        // Map the spawn_blocking JoinHandle to the trait's expected shape.
        let task = tokio::spawn(async move {
            match task.await {
                Ok(r) => r,
                Err(join_err) => Err(TranscribeError::InferenceFailed(format!(
                    "Sliding-window streaming task panicked: {}",
                    join_err
                ))),
            }
        });

        Ok(StreamHandle {
            events: events_rx,
            cancel: cancel_tx,
            task,
        })
    }
}

/// Mutable per-session state for one `start_stream` call.
struct Session {
    base: Arc<dyn Transcriber>,
    config: SlidingWindowConfig,

    // Audio accumulator.
    buffer: Vec<f32>,
    /// True once the buffer wrapped (audio was dropped). Switches diffing
    /// from prefix-stable (growing) to tail-delta (sliding).
    sliding: bool,

    // Diff state.
    /// Committed deltas (text already emitted).
    full_text_parts: Vec<String>,
    /// Last raw Whisper output.
    #[allow(dead_code)]
    prev_whisper: String,
    /// Words of the previous transcription (growing mode).
    last_words: Vec<String>,
    /// Words already confirmed & emitted (growing mode).
    confirmed_words: Vec<String>,
    /// Delta words from the previous pass (sliding mode).
    last_delta_words: Vec<String>,
}

impl Session {
    fn new(base: Arc<dyn Transcriber>, config: SlidingWindowConfig) -> Self {
        Self {
            base,
            config,
            buffer: Vec::new(),
            sliding: false,
            full_text_parts: Vec::new(),
            prev_whisper: String::new(),
            last_words: Vec::new(),
            confirmed_words: Vec::new(),
            last_delta_words: Vec::new(),
        }
    }

    /// Append audio and trim once past `max_buffer_s` (enters sliding mode).
    fn feed(&mut self, samples: &[f32]) {
        self.buffer.extend_from_slice(samples);

        let max_samples = (self.config.max_buffer_s * self.config.sample_rate as f32) as usize;
        let mut dropped = 0usize;
        if self.buffer.len() > max_samples {
            dropped = self.buffer.len() - max_samples;
            self.buffer.drain(..dropped);
        }
        if dropped > 0 && !self.sliding {
            tracing::debug!("[sliding] Buffer full — entering sliding mode");
            self.sliding = true;
        }
    }

    /// Transcribe the whole buffer, applying the silence + hallucination
    /// gates. Returns `None` when there is nothing worth committing.
    fn transcribe_buffer(&self) -> Result<Option<String>, TranscribeError> {
        if (self.buffer.len() as f32) < self.config.min_audio_s * self.config.sample_rate as f32 {
            return Ok(None);
        }
        if rms(&self.buffer) < self.config.min_speech_rms {
            return Ok(None);
        }
        let text = self.base.transcribe(&self.buffer)?;
        let text = text.trim().to_string();
        tracing::trace!(
            "[sliding] tick transcribe -> {text:?} ({} samples)",
            self.buffer.len()
        );
        if text.is_empty() || is_hallucination(&text) {
            return Ok(None);
        }
        Ok(Some(text))
    }

    /// One interval tick: re-transcribe, diff, and return the newly-committed
    /// stable deltas (at most one per tick).
    fn on_tick(&mut self) -> Result<Vec<String>, TranscribeError> {
        let Some(curr) = self.transcribe_buffer()? else {
            return Ok(Vec::new());
        };
        let curr_words: Vec<String> = curr.split_whitespace().map(str::to_owned).collect();
        if curr_words.is_empty() {
            return Ok(Vec::new());
        }

        let mut deltas = Vec::new();

        if self.sliding {
            // Sliding mode: diff the whole output against what's already
            // been emitted, then only commit the portion of the delta that
            // was also present in the previous delta (stable across two
            // consecutive passes).
            let already_emitted = self.full_text_parts.join(" ");
            let mut delta = extract_new_text(&already_emitted, &curr);
            let mut delta_words: Vec<String> =
                delta.split_whitespace().map(str::to_owned).collect();

            if !self.last_delta_words.is_empty() && !delta_words.is_empty() {
                let stable_n = common_prefix_len(&self.last_delta_words, &delta_words);
                if stable_n >= self.config.partial_min_words {
                    let new = delta_words[..stable_n].join(" ");
                    let last_emitted = self.full_text_parts.last();
                    let not_repeat = last_emitted.map(|s| s != &new).unwrap_or(true);
                    if !new.is_empty() && not_repeat {
                        // `new` is a bare word-join with no leading space.
                        // The daemon types deltas verbatim at the cursor, so
                        // separate it from whatever was already committed.
                        let typed = if self.full_text_parts.is_empty() {
                            new.clone()
                        } else {
                            format!(" {new}")
                        };
                        self.full_text_parts.push(new);
                        deltas.push(typed);
                        // Re-diff so last_delta_words reflects the remaining
                        // unconfirmed words only.
                        let already_emitted = self.full_text_parts.join(" ");
                        delta = extract_new_text(&already_emitted, &curr);
                        delta_words = delta.split_whitespace().map(str::to_owned).collect();
                    }
                }
            }
            self.last_delta_words = delta_words;
        } else {
            // Growing buffer: commit the common-prefix words between the
            // previous and current transcriptions, advancing confirmed_words.
            if !self.last_words.is_empty() {
                let stable_n = common_prefix_len(&self.last_words, &curr_words);
                if stable_n > self.confirmed_words.len() {
                    let confirmed_new = &curr_words[self.confirmed_words.len()..stable_n];
                    if confirmed_new.len() >= self.config.partial_min_words {
                        let new = confirmed_new.join(" ");
                        let last_emitted = self.full_text_parts.last();
                        let not_repeat = last_emitted.map(|s| s != &new).unwrap_or(true);
                        if !new.is_empty() && not_repeat {
                            // `new` is a bare word-join with no leading space.
                            // The daemon types deltas verbatim at the cursor,
                            // so separate it from whatever was already committed.
                            let typed = if self.full_text_parts.is_empty() {
                                new.clone()
                            } else {
                                format!(" {new}")
                            };
                            self.full_text_parts.push(new);
                            self.confirmed_words.extend(confirmed_new.iter().cloned());
                            deltas.push(typed);
                        }
                    }
                }
            }
        }

        self.prev_whisper = curr;
        self.last_words = curr_words;
        Ok(deltas)
    }

    /// One last transcription at end-of-recording. Returns the remaining tail
    /// delta (never cumulative text — the daemon already typed the partials).
    fn final_flush(&mut self) -> Option<String> {
        let final_text = match self.transcribe_buffer() {
            Ok(Some(t)) => t,
            _ => return None,
        };
        let already_emitted = self.full_text_parts.join(" ");
        let delta = extract_new_text(&already_emitted, &final_text);
        let delta = delta.trim().to_string();
        if delta.is_empty() {
            None
        } else {
            // `delta` has no leading space (see `on_tick`); separate it from
            // whatever was already committed before typing it at the cursor.
            let typed = if self.full_text_parts.is_empty() {
                delta.clone()
            } else {
                format!(" {delta}")
            };
            self.full_text_parts.push(delta);
            Some(typed)
        }
    }
}

// ── Diff helpers (ported 1:1 from nova's sliding_window.py) ──────────────

/// Whisper-tolerant word equality: case-insensitive, ignoring trailing
/// punctuation.
fn word_eq(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim_end_matches(['.', ',', '!', '?', ';', ':'])
            .to_lowercase()
    };
    norm(a) == norm(b)
}

fn words_match<T: AsRef<str>>(a: &[T], b: &[T]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| word_eq(x.as_ref(), y.as_ref()))
}

fn common_prefix_len<T: AsRef<str>>(a: &[T], b: &[T]) -> usize {
    let n = a.len().min(b.len());
    let mut i = 0;
    while i < n && word_eq(a[i].as_ref(), b[i].as_ref()) {
        i += 1;
    }
    i
}

/// Return the portion of `curr` that is new compared to `prev`.
///
/// Strategies, in order:
/// 1. Longest suffix→prefix word overlap (the common case).
/// 2. Verbatim string-prefix check.
/// 3. Greedy forward scan — how far into `curr` the `prev` words reach,
///    trusting the position when ≥50% matched (handles Whisper punctuation
///    / casing rewrites).
/// 4. Safety: if `curr` is not substantially longer, Whisper is refining —
///    no new text; otherwise emit only the tail beyond `prev`'s length.
fn extract_new_text(prev: &str, curr: &str) -> String {
    if prev.is_empty() {
        return curr.to_string();
    }
    if curr.is_empty() {
        return String::new();
    }

    let prev_words: Vec<&str> = prev.split_whitespace().collect();
    let curr_words: Vec<&str> = curr.split_whitespace().collect();

    if prev_words.is_empty() || curr_words.is_empty() {
        return curr.to_string();
    }

    // 1. Exact suffix→prefix overlap (longest wins).
    let mut best_overlap = 0;
    let max_check = prev_words.len().min(curr_words.len());
    for length in 1..=max_check {
        let suffix = &prev_words[prev_words.len() - length..];
        let prefix = &curr_words[..length];
        if words_match(suffix, prefix) {
            best_overlap = length;
        }
    }
    if best_overlap > 0 {
        return curr_words[best_overlap..].join(" ");
    }

    // 2. Verbatim prefix check.
    if let Some(rest) = curr.strip_prefix(prev) {
        return rest.trim().to_string();
    }

    // 3. Greedy forward scan.
    let mut pi = 0;
    let mut best_ci = 0;
    for (ci, cw) in curr_words.iter().enumerate() {
        if pi >= prev_words.len() {
            break;
        }
        if word_eq(prev_words[pi], cw) {
            pi += 1;
            best_ci = ci + 1;
        }
        // else: skip — Whisper rewrote this word, keep scanning.
    }
    if pi as f32 >= prev_words.len() as f32 * 0.5 {
        return curr_words[best_ci..].join(" ");
    }

    // 4. Safety: Whisper may have fully rewritten the window.
    if curr_words.len() <= prev_words.len() + 1 {
        return String::new();
    }
    curr_words[prev_words.len()..].join(" ")
}

/// Whole-buffer RMS (sqrt of mean of squares), float32.
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// Whisper hallucination check: lowercase, strip trailing `.,!?`, membership.
fn is_hallucination(text: &str) -> bool {
    let norm = text.trim().to_lowercase();
    let norm = norm.trim_end_matches(['.', ',', '!', '?']);
    HALLUCINATION_PATTERNS.contains(&norm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    // ── Pure diff helpers ────────────────────────────────────────────

    #[test]
    fn word_eq_ignores_case_and_trailing_punct() {
        assert!(word_eq("Hello,", "hello"));
        assert!(word_eq("WORLD.", "world"));
        assert!(word_eq("you?", "you"));
        assert!(!word_eq("there", "their"));
    }

    #[test]
    fn common_prefix_len_stops_at_first_divergence() {
        let a = ["hello", "world", "foo"];
        let b = ["hello", "WORLD", "bar"];
        assert_eq!(common_prefix_len(&a, &b), 2);
        let c = ["hello"];
        assert_eq!(common_prefix_len(&a, &c), 1);
    }

    #[test]
    fn extract_new_text_empty_prev_returns_curr() {
        assert_eq!(extract_new_text("", "hello world"), "hello world");
    }

    #[test]
    fn extract_new_text_empty_curr_returns_empty() {
        assert_eq!(extract_new_text("hello", ""), "");
    }

    #[test]
    fn extract_new_text_suffix_prefix_overlap() {
        // "you" overlaps: suffix of prev == prefix of curr.
        assert_eq!(extract_new_text("hello world", "world foo bar"), "foo bar");
        // Longest overlap wins: 2 words overlap.
        assert_eq!(
            extract_new_text("we should deploy now", "deploy now please"),
            "please"
        );
    }

    #[test]
    fn extract_new_text_verbatim_prefix() {
        assert_eq!(extract_new_text("hello", "hello world foo"), "world foo");
    }

    #[test]
    fn extract_new_text_fuzzy_scan() {
        // Whisper rewrote punctuation: prev matches 2/3 words fuzzily.
        assert_eq!(extract_new_text("hello, world", "hello world foo"), "foo");
    }

    #[test]
    fn extract_new_text_refining_emits_nothing() {
        // curr no longer than prev → Whisper is refining; no new text.
        assert_eq!(extract_new_text("hello world foo", "hello world"), "");
        assert_eq!(extract_new_text("a b c", "a b"), "");
    }

    #[test]
    fn extract_new_text_safety_tail() {
        // No overlap, prev unreachable via fuzzy scan, curr much longer.
        assert_eq!(extract_new_text("x y", "a b c d"), "c d");
    }

    #[test]
    fn is_hallucination_matches_and_strips_punct() {
        assert!(is_hallucination("thank you."));
        assert!(is_hallucination("  YOU  "));
        assert!(!is_hallucination("hello world"));
        assert!(!is_hallucination("the quick brown fox"));
    }

    #[test]
    fn rms_measures_energy() {
        assert_eq!(rms(&[]), 0.0);
        let silence = vec![0.0; 100];
        assert_eq!(rms(&silence), 0.0);
        let loud = vec![0.5; 100];
        assert!((rms(&loud) - 0.5).abs() < 1e-6);
    }

    /// Deterministic fake backend: transcript grows with buffer length.
    struct FakeTranscriber;

    impl Transcriber for FakeTranscriber {
        fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
            let secs = samples.len() as f32 / 16_000.0;
            let text = if secs < 1.0 {
                ""
            } else if secs < 2.0 {
                "alpha beta"
            } else if secs < 3.0 {
                "alpha beta gamma"
            } else {
                "alpha beta gamma delta"
            };
            Ok(text.to_string())
        }
    }

    fn loud_samples(secs: f32) -> Vec<f32> {
        let n = (secs * 16_000.0) as usize;
        // 0.05 amplitude tone-ish samples — above MIN_SPEECH_RMS.
        (0..n)
            .map(|i| if i % 2 == 0 { 0.05 } else { -0.05 })
            .collect()
    }

    fn streaming_config() -> SlidingWindowConfig {
        SlidingWindowConfig {
            interval_s: 0.02,
            max_buffer_s: 29.0,
            sample_rate: 16_000,
            min_speech_rms: 0.004,
            min_audio_s: 1.0,
            partial_min_words: 1,
            type_partials: true,
        }
    }

    /// Drive a session: feed ~3s of audio, then EOF, and collect events.
    async fn run_session(config: SlidingWindowConfig) -> Vec<StreamingEvent> {
        let transcriber: Arc<dyn Transcriber> = Arc::new(FakeTranscriber);
        let engine = SlidingWindowStreamingTranscriber::new(transcriber, config);

        let (tx, rx) = mpsc::channel::<Vec<f32>>(32);
        let mut handle = engine.start_stream(rx).expect("start stream");

        // Feed three seconds in 0.5s chunks with small gaps between ticks.
        for _ in 0..6 {
            tx.send(loud_samples(0.5)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
        drop(tx); // graceful EOF

        let mut events = Vec::new();
        while let Some(ev) = handle.events.recv().await {
            events.push(ev);
            if matches!(events.last(), Some(StreamingEvent::Ended)) {
                break;
            }
        }
        handle.task.await.unwrap().expect("task ok");
        events
    }

    fn emitted_text(events: &[StreamingEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|ev| match ev {
                StreamingEvent::Partial { text, .. } | StreamingEvent::Final { text, .. } => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn session_emits_stable_deltas_then_final_flush_and_ended() {
        let events = run_session(streaming_config()).await;

        // Must end cleanly.
        assert!(matches!(events.last(), Some(StreamingEvent::Ended)));
        assert!(events.len() >= 3, "expected partials + final + ended");

        let parts = emitted_text(&events);
        // Deltas are typed verbatim at the cursor (see `on_tick`), each
        // carrying its own separating leading space — so plain
        // concatenation, not `join(" ")`, is what the daemon actually
        // produces and what must reproduce the full transcript.
        assert_eq!(parts.concat(), "alpha beta gamma delta");
        // Each delta is strictly new text — no duplicates at boundaries.
        for i in 1..parts.len() {
            let tail = parts[..i].concat();
            assert!(!tail.contains(parts[i].trim()), "repeated text: {parts:?}");
        }
    }

    #[tokio::test]
    async fn commit_only_mode_emits_final_deltas() {
        let mut cfg = streaming_config();
        cfg.type_partials = false;
        let events = run_session(cfg).await;

        let parts = emitted_text(&events);
        assert_eq!(parts.concat(), "alpha beta gamma delta");
        assert!(events
            .iter()
            .all(|ev| !matches!(ev, StreamingEvent::Partial { .. })));
    }

    #[tokio::test]
    async fn silence_only_buffer_emits_no_events() {
        let (tx, rx) = mpsc::channel::<Vec<f32>>(32);
        let engine =
            SlidingWindowStreamingTranscriber::new(Arc::new(FakeTranscriber), streaming_config());
        let mut handle = engine.start_stream(rx).expect("start stream");

        // 3s of near-silence — below the RMS gate.
        tx.send(vec![0.0; 16000]).await.unwrap();
        tx.send(vec![0.0; 16000]).await.unwrap();
        tx.send(vec![0.0; 16000]).await.unwrap();
        drop(tx);

        let mut events = Vec::new();
        while let Some(ev) = handle.events.recv().await {
            events.push(ev);
            if matches!(events.last(), Some(StreamingEvent::Ended)) {
                break;
            }
        }
        assert!(matches!(events.last(), Some(StreamingEvent::Ended)));
        assert_eq!(
            emitted_text(&events).len(),
            0,
            "silence should not transcribe"
        );
        handle.task.await.unwrap().unwrap();
    }
}
