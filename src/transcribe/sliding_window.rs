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
//! New text is extracted by aligning the fresh transcription against the
//! already-committed text and taking everything beyond where they diverge.
//! The daemon appends each emitted delta at the cursor, so **events must
//! always be deltas** — never cumulative transcripts (or the cursor receives
//! duplicates).
//!
//! ## Commit policy (`revision_mode` config, default true)
//!
//! - **Type-then-correct (default)** — the whole best-guess tail is typed
//!   immediately as `provisional_words` and reconciled every tick against
//!   the freshly re-transcribed one:
//!   - If the fresh tail still agrees with everything currently displayed,
//!     whatever's new beyond it is appended (a plain `Partial`/`Final`).
//!   - If the fresh tail disagrees partway through what's displayed, the
//!     wrong suffix is backspaced (character-exact, accounting for the
//!     inter-word space) and the corrected text retyped — a
//!     [`StreamingEvent::Replace`], the same mechanism Soniox already uses
//!     for punctuation-flip revisions. Divergence is punctuation-sensitive,
//!     so a word that only changed its trailing punctuation (e.g. "it." →
//!     "it?") is fixed with a minimal backspace from the stale punct char.
//!   - **Question-mark reconciliation** — when the tail's last word gains a
//!     "?", a stale "?" within `question_mark_lookback_words` behind it is
//!     dropped, so a still-open question keeps exactly one "?" and it lands
//!     on the true end: "what time is it? tomorrow afternoon?" → "what time
//!     is it tomorrow afternoon?".
//!   - A word only stays revisable for `revision_lag_words` words' worth of
//!     further growth behind it, and only once it has been stable for
//!     `stability_passes` ticks; beyond both it's promoted into
//!     permanently-confirmed text and never revised again, bounding how far
//!     back a correction can ever reach. A single flickering word never
//!     stalls the tail — it's corrected every tick instead.
//! - **Conservative gate (legacy opt-in, `revision_mode = false`)** — the
//!   tail is withheld until each word has been present for `stability_passes`
//!   ticks; only the stable prefix is typed, and it's committed immediately.
//!   No visible corrections, at the cost of ~`stability_passes` ticks of
//!   latency. Gated per-word, so one unstable word no longer blocks the
//!   whole tail.
//!
//! The engine wraps any batch [`Transcriber`] (whisper.cpp, OpenVINO GenAI,
//! …), so the same code powers every streaming backend.
//!
//! Type-then-correct trades the "occasionally pauses, never wrong" property
//! of the conservative gate for "more responsive, occasionally visibly
//! backspaces-and-retypes." That's a materially different (and riskier)
//! failure mode for the live-typing case: a wrong backspace count doesn't
//! just look bad, it can delete characters that were never ours to begin
//! with if bookkeeping ever drifts (moved focus, a backend with no backspace
//! primitive, ...) — see `replace_and_commit`'s defensive handling of that
//! in `output/streaming.rs`. File-output sessions have no such risk
//! (`replace_and_commit_silent` just edits an in-memory string), which is
//! the safer place to exercise corrections.
//!
//! ## Diffing: one unified alignment
//!
//! Growing mode (committed text is a prefix of the fresh transcription) and
//! sliding mode (only a suffix of the committed tail overlaps the fresh
//! window's start) used to be two separate code paths with different state,
//! which duplicated text at the growing→sliding transition. Both are now one
//! [`find_new_tail`]: anchor the committed tail (bounded to
//! `SLIDING_DIFF_LOOKBACK_WORDS` words) somewhere inside the fresh
//! transcription with text-tolerant, banded matching, and take everything
//! after it as new.
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

/// Type-then-correct default: how many trailing words of the provisional
/// tail stay eligible for correction. Once more than this many words of
/// fresh content have appeared behind a word, it's promoted to permanently
/// confirmed text and can never be revised again — this bounds how far
/// back (in words, and therefore in backspaced characters) any single
/// correction can ever reach. Overridable via `[streaming]
/// revision_lag_words`.
const REVISION_LAG_WORDS: usize = 4;

/// Question-mark reconciliation default: when a new "?" is committed, scan
/// back this many words for a stale earlier "?" to remove. Capped at the
/// revision lag so a stale "?" can only ever be found still within
/// revisable (provisional) text. Overridable via `[streaming]
/// question_mark_lookback_words`.
const QUESTION_MARK_LOOKBACK_WORDS: usize = 3;

/// Type-then-correct default: how many consecutive ticks a word must appear
/// (same text and punctuation) before it's eligible to be promoted out of
/// the revisable provisional tail. Overridable via `[streaming]
/// stability_passes`.
const STABILITY_PASSES: u32 = 2;

/// How far the unified alignment may skip ahead (in words) while anchoring
/// the committed tail inside the current transcription. Small enough that a
/// genuinely-new word can't be swallowed by the anchor, large enough to
/// absorb a reworded boundary word.
const ALIGN_BAND: usize = 2;

/// One committed change to the streamed output, produced by a tick or by
/// `final_flush`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Delta {
    /// Append `text` after whatever's already been typed. Maps to a plain
    /// `Partial`/`Final` event depending on `type_partials` — the only
    /// kind of delta the non-revision-mode gate ever produces.
    Append(String),
    /// Revision mode only: backspace `backspace` (Unicode scalar) chars,
    /// then type `text`. Maps to `StreamingEvent::Replace`.
    Replace { backspace: usize, text: String },
}

/// Map a [`Delta`] to the [`StreamingEvent`] the daemon expects. `Append`
/// becomes a `Partial` (typed live) or commit-only `Final` depending on
/// `type_partials`, matching the non-revision-mode behavior exactly.
/// `Replace` always maps to `StreamingEvent::Replace` regardless of
/// `type_partials` — it's a correction to already-displayed text, not a
/// progressive-typing choice, so there's no "non-partial" equivalent.
fn delta_to_event(delta: Delta, type_partials: bool) -> StreamingEvent {
    match delta {
        Delta::Append(text) => {
            if type_partials {
                StreamingEvent::Partial {
                    text,
                    segment_id: 0,
                }
            } else {
                StreamingEvent::Final {
                    text,
                    segment_id: 0,
                }
            }
        }
        Delta::Replace { backspace, text } => StreamingEvent::Replace {
            backspace,
            text,
            segment_id: 0,
        },
    }
}

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
    /// Type the current best-guess tail immediately and correct it later via
    /// backspace + retype (`StreamingEvent::Replace`) if a following tick
    /// disagrees, instead of withholding it until it's been stable for a few
    /// ticks. More responsive; can visibly flicker (type then backspace then
    /// retype) when Whisper changes its mind about a word. **Default true.**
    /// `false` opts back into the legacy conservative wait-for-stability
    /// gate. See the module doc's "Commit policy" section.
    pub revision_mode: bool,
    /// When a new "?" lands on the tail's last word, scan back this many
    /// words for a stale earlier "?" and remove it, so a question ends with
    /// exactly one "?". See `reconcile_question_marks`.
    pub question_mark_lookback_words: usize,
    /// Consecutive ticks a word must be present (text + punctuation) before
    /// it's promoted out of the revisable provisional tail (type-then-correct
    /// mode) or typed at all (conservative mode).
    pub stability_passes: u32,
    /// How many trailing words of the provisional tail stay revisable in
    /// type-then-correct mode (replaces the `REVISION_LAG_WORDS` constant).
    pub revision_lag_words: usize,
}

impl Default for SlidingWindowConfig {
    fn default() -> Self {
        Self {
            interval_s: 0.8,
            max_buffer_s: 29.0,
            sample_rate: 16_000,
            min_speech_rms: MIN_SPEECH_RMS,
            min_audio_s: 1.0,
            // Matches the production default in config::StreamingConfig
            // (both whisper and openvino configs default this to 1). This
            // Default impl isn't constructed anywhere in production code
            // today (every real caller builds the struct field-by-field
            // from resolved config), but a stray `2` here was a trap for
            // any future caller who assumes it matches config defaults.
            partial_min_words: 1,
            type_partials: true,
            revision_mode: true,
            question_mark_lookback_words: QUESTION_MARK_LOOKBACK_WORDS,
            stability_passes: STABILITY_PASSES,
            revision_lag_words: REVISION_LAG_WORDS,
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
                            let event = delta_to_event(tail, config.type_partials);
                            let _ = runtime.block_on(events_tx.send(event));
                        }
                        let _ = runtime.block_on(events_tx.send(StreamingEvent::Ended));
                        return Ok(());
                    }
                    Outcome::Tick => match session.on_tick() {
                        Ok(deltas) => {
                            for delta in deltas {
                                let event = delta_to_event(delta, config.type_partials);
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
    /// True once the buffer wrapped (audio was dropped). Only affects the
    /// audio buffer — the diff logic is identical growing or sliding (the
    /// unified alignment anchors the committed tail wherever it lands in
    /// the fresh transcription).
    sliding: bool,

    // Diff state — one source of truth, shared by both commit policies.
    /// Permanently-confirmed words (already typed, never revised). Both
    /// mode's `full_text_parts` + `confirmed_words` + `last_words` +
    /// `last_delta_words` collapsed into a single word list.
    committed_words: Vec<Word>,
    /// Type-then-correct only: the tail currently typed at the cursor but
    /// not yet permanently confirmed — may still be corrected via a
    /// `Replace` on a later tick. Empty and unused when
    /// `config.revision_mode` is false.
    provisional_words: Vec<Word>,
    /// Stability tally for `provisional_words` (contiguous, aligned with
    /// it after promotion drains both fronts equally).
    provisional_stability: Vec<u32>,
    /// Previous tick's full new tail (pre-promotion), for the per-word
    /// stability comparison next tick.
    prev_tail: Vec<Word>,
    /// Previous tick's stability tally, aligned with `prev_tail`.
    prev_stability: Vec<u32>,
    /// How many front words of `prev_tail` were promoted (or, in
    /// conservative mode, typed) last tick — the offset that re-aligns this
    /// tick's new tail against `prev_tail`.
    promoted_last_tick: usize,
}

impl Session {
    fn new(base: Arc<dyn Transcriber>, config: SlidingWindowConfig) -> Self {
        Self {
            base,
            config,
            buffer: Vec::new(),
            sliding: false,
            committed_words: Vec::new(),
            provisional_words: Vec::new(),
            provisional_stability: Vec::new(),
            prev_tail: Vec::new(),
            prev_stability: Vec::new(),
            promoted_last_tick: 0,
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
        let infer_start = std::time::Instant::now();
        let text = self.base.transcribe(&self.buffer)?;
        let infer_secs = infer_start.elapsed().as_secs_f32();
        let text = text.trim().to_string();
        tracing::trace!(
            "[sliding] tick transcribe -> {text:?} ({} samples, {:.3}s infer, sliding={})",
            self.buffer.len(),
            infer_secs,
            self.sliding,
        );
        // Inference taking longer than the tick interval means every
        // subsequent tick falls further behind, and incoming audio
        // chunks queued on the bounded channel feeding this task start
        // getting silently dropped (see `try_send` at the capture side)
        // instead of backing up — the recording keeps running but the
        // transcript stalls. Surface this loudly since it's otherwise
        // invisible without trace logging.
        if infer_secs > self.config.interval_s as f32 {
            tracing::warn!(
                "[sliding] inference ({:.2}s) exceeded the tick interval ({:.2}s) — \
                 audio chunks may be getting dropped; buffer at {:.1}s",
                infer_secs,
                self.config.interval_s,
                self.buffer.len() as f32 / self.config.sample_rate as f32,
            );
        }
        if text.is_empty() || is_hallucination(&text) {
            return Ok(None);
        }
        Ok(Some(text))
    }

    /// One interval tick: re-transcribe, align, and return the deltas to
    /// emit (at most one per tick for either commit policy).
    fn on_tick(&mut self) -> Result<Vec<Delta>, TranscribeError> {
        let Some(curr) = self.transcribe_buffer()? else {
            return Ok(Vec::new());
        };
        let curr_words = parse_words(&curr);
        if curr_words.is_empty() {
            return Ok(Vec::new());
        }

        let deltas = if self.config.revision_mode {
            // Type-then-correct (default): type the whole new tail now and
            // correct it via `Replace` on any later tick that disagrees.
            match find_new_tail(&self.committed_tail(), &curr_words, ALIGN_BAND) {
                Some(mut new_tail) => {
                    self.reconcile_question_marks(&mut new_tail);
                    let delta = self.reconcile(&new_tail);
                    self.provisional_stability = self.update_stability(&new_tail);
                    self.promoted_last_tick = self.promote();
                    delta.into_iter().collect()
                }
                None => {
                    // No confident anchor this pass — type nothing, try again
                    // next tick (same contract as the old confident-only diff).
                    Vec::new()
                }
            }
        } else {
            // Legacy conservative gate (opt-in): type only the stable prefix
            // of the new tail; withhold the unstable remainder until it has
            // held for `stability_passes`. Per-word, so a single flickering
            // word no longer stalls the whole tail.
            match find_new_tail(&self.committed_tail(), &curr_words, ALIGN_BAND) {
                Some(new_tail) => {
                    let stability = self.update_stability(&new_tail);
                    let passes = self.config.stability_passes;
                    let mut stable_n = 0;
                    while stable_n < new_tail.len() && stability[stable_n] >= passes {
                        stable_n += 1;
                    }
                    if stable_n < self.config.partial_min_words {
                        return Ok(Vec::new());
                    }
                    // `new_tail` starts beyond committed text, so the whole
                    // stable prefix is genuinely new.
                    let typed = if self.committed_words.is_empty() {
                        render_words(&new_tail[..stable_n])
                    } else {
                        format!(" {}", render_words(&new_tail[..stable_n]))
                    };
                    tracing::debug!("[sliding] COMMIT (stable prefix): {typed:?}");
                    self.committed_words.extend_from_slice(&new_tail[..stable_n]);
                    self.promoted_last_tick = stable_n;
                    vec![Delta::Append(typed)]
                }
                None => Vec::new(),
            }
        };

        Ok(deltas)
    }

    /// The last `SLIDING_DIFF_LOOKBACK_WORDS` permanently-confirmed words —
    /// what [`find_new_tail`] anchors `curr` against (see that function's doc
    /// comment for why the anchor is bounded and not the whole session).
    fn committed_tail(&self) -> Vec<Word> {
        let n = self.committed_words.len();
        let start = n.saturating_sub(SLIDING_DIFF_LOOKBACK_WORDS);
        self.committed_words[start..].to_vec()
    }

    /// Type-then-correct: reconcile this pass's full best-guess tail
    /// (`new_tail`, already question-mark-cleaned) against what's currently
    /// displayed (`self.provisional_words`). Returns the single `Delta` that
    /// brings the cursor's visible text in line with `new_tail` — a pure
    /// append, a backspace+retype correction, or `None` if nothing changed.
    /// Divergence is punctuation-sensitive: a word that changed only its
    /// trailing punctuation is corrected with a minimal backspace from the
    /// stale punct char instead of re-typing the whole word.
    fn reconcile(&mut self, new_tail: &[Word]) -> Option<Delta> {
        let agree_n = punct_prefix_len(&self.provisional_words, new_tail);
        // Whether there's confirmed text before the whole provisional block —
        // only matters when agree_n == 0; typed_len_from/format_typed already
        // account for agree_n > 0 themselves (an earlier surviving word is
        // always immediately before the point of correction/extension).
        let has_prefix = !self.committed_words.is_empty();

        let delta = if agree_n < self.provisional_words.len() {
            if agree_n < new_tail.len()
                && word_text_eq(&self.provisional_words[agree_n], &new_tail[agree_n])
            {
                // Punctuation-only change: same word, different trailing
                // punct. Backspace from the stale punct char onward and
                // retype the new punct + following tail (fixes "it." → "it?"
                // with a 1-char backspace instead of re-typing the word).
                let backspace = tail_from_punct(&self.provisional_words, agree_n);
                let text = typed_from_punct(new_tail, agree_n);
                tracing::debug!(
                    "[sliding] REVISE punct: backspace {backspace} chars, retype {text:?}"
                );
                Some(Delta::Replace { backspace, text })
            } else {
                // Word-level divergence: backspace the wrong tail (char-exact,
                // including its leading separator) and retype the corrected
                // + extended tail.
                let backspace = typed_len_from(&self.provisional_words, agree_n, has_prefix);
                let text = format_typed(new_tail, agree_n, has_prefix);
                tracing::debug!(
                    "[sliding] REVISE: backspace {backspace} chars, retype {text:?} (was {:?})",
                    self.provisional_words
                );
                Some(Delta::Replace { backspace, text })
            }
        } else if agree_n < new_tail.len() {
            // Pure extension: everything already displayed still agrees;
            // append whatever's new beyond it.
            let text = format_typed(new_tail, agree_n, has_prefix);
            if text.is_empty() {
                None
            } else {
                tracing::debug!("[sliding] REVISE append: {text:?}");
                Some(Delta::Append(text))
            }
        } else {
            None
        };

        self.provisional_words = new_tail.to_vec();
        delta
    }

    /// Revision logic that keeps exactly one "?" where a question actually
    /// ends. When `new_tail`'s last word ends in "?", Whisper may already
    /// have emitted a premature "?" a few words earlier (re-processing a
    /// still-open question) and then extended the utterance — turning "what
    /// time is it?" into "what time is it? tomorrow afternoon?" with two
    /// question marks, the first one wrong. When the new tail's final word
    /// is "?", scan back up to `question_mark_lookback_words` for an earlier
    /// "?" in the combined committed + new text and drop it, leaving only
    /// the true end. Only words in the still-revisable `new_tail` are ever
    /// edited; the lookback is capped at the revision lag so a stale "?"
    /// can never have already been promoted into permanently-confirmed text.
    fn reconcile_question_marks(&mut self, new_tail: &mut [Word]) {
        if new_tail.last().and_then(|w| w.punct) != Some('?') {
            return;
        }
        let lookback = self.config.question_mark_lookback_words;
        let committed_len = self.committed_words.len();
        let combined_len = committed_len + new_tail.len();
        // Skip the final "?" itself: scan the `lookback` words behind it.
        for back in 1..=lookback {
            let Some(idx) = combined_len.checked_sub(1 + back) else {
                break;
            };
            let has_q = if idx < committed_len {
                self.committed_words[idx].punct == Some('?')
            } else {
                new_tail[idx - committed_len].punct == Some('?')
            };
            if has_q {
                if idx >= committed_len {
                    new_tail[idx - committed_len].punct = None;
                    tracing::debug!(
                        "[sliding] question-mark reconcile: dropped premature '?' {back} word(s) back"
                    );
                }
                return;
            }
        }
    }

    /// Compute per-word stability for `new_tail` against the previous tick's
    /// tail, snapshot the full pre-promotion state for next tick's
    /// comparison, and return the tally. A word is stable if its text AND
    /// punctuation were present at the corresponding position
    /// (`i + promoted_last_tick`) last tick; if so it carries its previous
    /// tally forward incremented, otherwise it resets to one.
    fn update_stability(&mut self, new_tail: &[Word]) -> Vec<u32> {
        let mut tally = Vec::with_capacity(new_tail.len());
        for (i, w) in new_tail.iter().enumerate() {
            let prev_i = i + self.promoted_last_tick;
            let stable = prev_i < self.prev_tail.len() && word_eq(w, &self.prev_tail[prev_i]);
            let base = self.prev_stability.get(prev_i).copied().unwrap_or(1);
            tally.push(if stable { base + 1 } else { 1 });
        }
        // Snapshot the FULL tail and tally (pre-promotion) so next tick's
        // comparison can still see the words promoted out of the front.
        self.prev_tail = new_tail.to_vec();
        self.prev_stability = tally.clone();
        self.promoted_last_tick = 0;
        tally
    }

    /// Type-then-correct bookkeeping: promote a contiguous prefix of the
    /// provisional tail that is both stable for `stability_passes` and sits
    /// beyond `revision_lag_words` from the tail's end into
    /// permanently-confirmed text. This content is already on screen (earlier
    /// deltas typed it), so promotion emits nothing — it only shrinks what a
    /// future correction is allowed to touch. Halts at the first unstable
    /// word, so a flickering word stays revisable. Returns how many words
    /// were promoted.
    fn promote(&mut self) -> usize {
        let lag = self.config.revision_lag_words;
        let passes = self.config.stability_passes;
        let mut promoted = 0;
        while self.provisional_words.len() - promoted > lag
            && self.provisional_stability.get(promoted).copied().unwrap_or(0) >= passes
        {
            promoted += 1;
        }
        if promoted > 0 {
            let words: Vec<Word> = self.provisional_words.drain(..promoted).collect();
            drop(self.provisional_stability.drain(..promoted));
            tracing::debug!("[sliding] promote {promoted} word(s): {words:?}");
            self.committed_words.extend(words);
        }
        promoted
    }

    /// One last transcription at end-of-recording. Returns the remaining tail
    /// delta (never cumulative text — the daemon already typed the partials).
    /// Unlike `on_tick` there is no next tick to wait for, so a pass that
    /// can't find a confident anchor falls back to a single best-effort
    /// index-based guess rather than leaving the last text uncorrected
    /// forever (same rationale as the old `extract_new_text` fallback).
    fn final_flush(&mut self) -> Option<Delta> {
        let final_text = match self.transcribe_buffer() {
            Ok(Some(t)) => t,
            _ => return None,
        };
        let curr = parse_words(&final_text);

        let committed_tail = self.committed_tail();
        let mut new_tail = match find_new_tail(&committed_tail, &curr, ALIGN_BAND) {
            Some(t) => t,
            None => {
                if curr.len() > self.committed_words.len() {
                    curr[self.committed_words.len()..].to_vec()
                } else {
                    return None;
                }
            }
        };

        if self.config.revision_mode {
            self.reconcile_question_marks(&mut new_tail);
            self.reconcile(&new_tail)
        } else {
            // Conservative: type the whole remaining tail as one final delta.
            // No stability gating (the recording is over) and no Replace is
            // ever needed (nothing is typed before it's stable).
            if new_tail.is_empty() {
                None
            } else {
                let typed = if self.committed_words.is_empty() {
                    render_words(&new_tail)
                } else {
                    format!(" {}", render_words(&new_tail))
                };
                self.committed_words.extend(new_tail);
                Some(Delta::Append(typed))
            }
        }
    }
}

// ── Word model and unified alignment ────────────────────────────────────

/// A single token of a transcription, with trailing punctuation split off
/// so the engine can (a) match words without noise, (b) preserve the model's
/// punctuation verbatim, and (c) correct punctuation changes — including the
/// premature "?" reconciliation — with exact backspace arithmetic.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Word {
    text: String,
    /// Trailing punctuation char, if any ("it?" → Some('?')). Repeated
    /// punctuation collapses to the last char ("it??" → "it?").
    punct: Option<char>,
}

/// Punctuation stripped to the last trailing char of a token.
fn trailing_punct(chars: &[char]) -> Option<char> {
    chars
        .iter()
        .rev()
        .find(|c| matches!(c, '.' | ',' | '!' | '?' | ';' | ':'))
        .copied()
}

/// Parse a transcription into `Word`s. "It's 3:30?" → `[It's, 3:30?]`. A
/// standalone punctuation token ("it ?") attaches to the previous word.
fn parse_words(text: &str) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    for tok in text.split_whitespace() {
        let chars: Vec<char> = tok.chars().collect();
        let mut punct = None;
        let mut text_len = chars.len();
        while text_len > 0 && matches!(chars[text_len - 1], '.' | ',' | '!' | '?' | ';' | ':') {
            punct = Some(chars[text_len - 1]);
            text_len -= 1;
        }
        let token_text: String = chars[..text_len].iter().collect();
        if token_text.is_empty() {
            // Entirely punctuation ("?" as its own token): merge into the
            // previous word so "it ?" renders as "it?".
            if let Some(prev) = out.last_mut() {
                if prev.punct.is_none() {
                    prev.punct = punct;
                }
            } else {
                out.push(Word {
                    text: String::new(),
                    punct: trailing_punct(&chars),
                });
            }
        } else {
            out.push(Word { text: token_text, punct });
        }
    }
    out
}

/// Render `Word`s back to text ("it?" + "tomorrow" → "it? tomorrow").
fn render_words(words: &[Word]) -> String {
    let mut s = String::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&w.text);
        if let Some(p) = w.punct {
            s.push(p);
        }
    }
    s
}

/// Word equality for **anchoring**: case-insensitive, ignoring trailing
/// punctuation. A reworded or re-punctuated word at the committed boundary
/// must not lose the anchor (this is what lets the alignment find the
/// committed text even when Whisper changed a word's casing or punctuation).
fn word_text_eq(a: &Word, b: &Word) -> bool {
    a.text.eq_ignore_ascii_case(&b.text)
}

/// Word equality for **divergence detection and stability**: the trailing
/// punctuation must agree exactly, while the text is compared
/// case-insensitively. A punctuation-only change is visible to the reconcile
/// step and the stability tally, but a casing flip ("world" → "WORLD") is
/// not — Whisper capitalizes inconsistently and backspacing to fix casing
/// would just flicker.
fn word_eq(a: &Word, b: &Word) -> bool {
    a.punct == b.punct && a.text.eq_ignore_ascii_case(&b.text)
}

/// Number of leading words that agree by text AND punctuation.
fn punct_prefix_len(a: &[Word], b: &[Word]) -> usize {
    let n = a.len().min(b.len());
    let mut i = 0;
    while i < n && word_eq(&a[i], &b[i]) {
        i += 1;
    }
    i
}

/// How many trailing words of already-committed text to anchor `curr`
/// against — deliberately NOT the whole session. Once a long-running
/// session's committed text exceeds one window's worth of words (a couple
/// of minutes of continuous dictation), the fresh transcription `curr` is
/// never longer than the whole committed history, so an unbounded anchor
/// would silently stop matching (found live: a 60s recording stopped
/// committing anything ~15s into sliding mode).
const SLIDING_DIFF_LOOKBACK_WORDS: usize = 20;

/// Trailing `n` words of `text` (or the whole thing if shorter). Kept for
/// the test suite; production anchoring goes through `Session::committed_tail`,
/// which bounds the same way over `Word`s.
#[cfg(test)]
fn last_n_words(text: &str, n: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let start = words.len().saturating_sub(n);
    words[start..].join(" ")
}

/// Find the new tail: everything in `curr` beyond the committed text.
///
/// Unified across growing mode (the committed text is a prefix of `curr`)
/// and sliding mode (only a suffix of the committed tail overlaps `curr`'s
/// start). Every start position in `curr` is tried against every suffix
/// start of `prev` (the committed tail), greedily matching forward with each
/// `prev` word allowed to land up to `BAND` positions ahead. The anchor
/// that consumes the most committed words wins (ties go to the one that
/// consumes the most of `curr`), and its remainder — `curr[ci..]` — is the
/// new tail.
///
/// Searching every `curr` start position (not just the prefix) matters once
/// the committed text exceeds `SLIDING_DIFF_LOOKBACK_WORDS`: the committed
/// tail is then no longer a prefix of `curr` (it starts mid-sentence), and a
/// prefix-only search anchors on a single coincidental word and re-types the
/// whole committed sentence as new — found live as literal duplication.
///
/// Returns `None` when no suffix aligns with even one `curr` word; the
/// caller treats that as "nothing safe to type this pass".
fn find_new_tail(prev: &[Word], curr: &[Word], band: usize) -> Option<Vec<Word>> {
    if prev.is_empty() {
        return Some(curr.to_vec());
    }
    if curr.is_empty() {
        return Some(Vec::new());
    }

    let n = prev.len();
    let m = curr.len();
    // Best anchor: (committed words matched, curr position consumed).
    let mut best: Option<(usize, usize)> = None;

    for j0 in 0..m {
        for s in 0..n {
            let mut ci = j0;
            let mut matched = 0usize;
            for committed_word in &prev[s..] {
                let lookahead_end = (ci + band + 1).min(m);
                let found = curr[ci..lookahead_end]
                    .iter()
                    .position(|c| word_text_eq(committed_word, c))
                    .map(|k| ci + k);
                match found {
                    Some(j) => {
                        matched += 1;
                        ci = j + 1;
                    }
                    None => break,
                }
            }
            if matched > 0 {
                let better = match best {
                    None => true,
                    Some((b_matched, b_ci)) => {
                        matched > b_matched || (matched == b_matched && ci > b_ci)
                    }
                };
                if better {
                    best = Some((matched, ci));
                }
            }
        }
    }

    best.map(|(_, ci)| curr[ci..].to_vec())
}

/// Exact character length that would be BACKSPACED to remove
/// `words[from_idx..]` from the cursor. `has_prefix` is whether there's
/// content immediately before `words[from_idx]` — either prior confirmed
/// text or an earlier surviving provisional word — which means a separating
/// space was typed right before it that also needs removing.
fn typed_len_from(words: &[Word], from_idx: usize, has_prefix: bool) -> usize {
    if from_idx >= words.len() {
        return 0;
    }
    let tail = render_words(&words[from_idx..]);
    // A leading separating space was typed before `words[from_idx]`
    // whenever there's anything before it — either an earlier word in this
    // same array (`from_idx > 0`) or context the caller says exists before
    // the whole array (`has_prefix`, meaningful only at `from_idx == 0`).
    let leading_space = from_idx > 0 || has_prefix;
    tail.chars().count() + usize::from(leading_space)
}

/// The exact string that would be TYPED for `words[from_idx..]`, given
/// `has_prefix` (see [`typed_len_from`]) — a leading separating space is
/// prepended whenever there's content before `words[from_idx]`, by the same
/// rule `typed_len_from` uses.
fn format_typed(words: &[Word], from_idx: usize, has_prefix: bool) -> String {
    if from_idx >= words.len() {
        return String::new();
    }
    let tail = render_words(&words[from_idx..]);
    if from_idx > 0 || has_prefix {
        format!(" {tail}")
    } else {
        tail
    }
}

/// Char count of everything from `words[k]`'s trailing punctuation (inclusive)
/// to the end of the rendered tail — what backspace removes to fix a
/// punctuation-only change at word `k` (e.g. "it? tomorrow" → backspace 9 to
/// pull the stale "?" out of the middle of the typed text).
fn tail_from_punct(words: &[Word], k: usize) -> usize {
    let mut s = String::new();
    if let Some(p) = words[k].punct {
        s.push(p);
    }
    if k + 1 < words.len() {
        s.push(' ');
        s.push_str(&render_words(&words[k + 1..]));
    }
    s.chars().count()
}

/// The corrected text to type for a punctuation-only change at word `k`: the
/// replacement punctuation (if any) followed by the rest of the tail, exactly
/// as it would render at the cursor.
fn typed_from_punct(words: &[Word], k: usize) -> String {
    let mut s = String::new();
    if let Some(p) = words[k].punct {
        s.push(p);
    }
    if k + 1 < words.len() {
        s.push(' ');
        s.push_str(&render_words(&words[k + 1..]));
    }
    s
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

    // ── Word model ───────────────────────────────────────────────────

    #[test]
    fn parse_words_splits_trailing_punctuation() {
        let ws = parse_words("It's 3:30?");
        assert_eq!(
            ws,
            vec![
                Word { text: "It's".into(), punct: None },
                Word { text: "3:30".into(), punct: Some('?') },
            ]
        );
    }

    #[test]
    fn parse_words_merges_standalone_punctuation_token() {
        // "it ?" (space-separated question mark) must render back as "it?".
        let ws = parse_words("what time is it ?");
        assert_eq!(
            ws.last(),
            Some(&Word { text: "it".into(), punct: Some('?') })
        );
        assert_eq!(render_words(&ws), "what time is it?");
    }

    #[test]
    fn parse_words_collapses_repeated_punctuation() {
        let ws = parse_words("right??");
        assert_eq!(
            ws.last(),
            Some(&Word { text: "right".into(), punct: Some('?') })
        );
        assert_eq!(render_words(&ws), "right?");
    }

    #[test]
    fn parse_words_round_trips_through_render() {
        for s in [
            "turn on the light.",
            "what time is it?",
            "hello world",
            "it's 3:30, isn't it?",
            "a single word",
        ] {
            assert_eq!(render_words(&parse_words(s)), s);
        }
    }

    #[test]
    fn word_text_eq_ignores_case_and_punct() {
        let comma = parse_words("hello,");
        let upper = parse_words("HELLO");
        assert!(word_text_eq(&comma[0], &upper[0]));
        assert!(!word_text_eq(&comma[0], &parse_words("there")[0]));
    }

    #[test]
    fn word_eq_is_punct_sensitive() {
        let q = parse_words("it?")[0].clone();
        let dot = parse_words("it.")[0].clone();
        let bare = Word { text: "it".into(), punct: None };
        // Divergence detection and stability must SEE a punctuation flip
        // (the old `word_eq` stripped trailing punctuation and could not).
        assert_ne!(q, dot);
        assert_ne!(q, bare);
        assert!(!word_eq(&q, &dot));
    }

    #[test]
    fn punct_prefix_len_stops_at_text_or_punct_divergence() {
        let t = |s: &str| parse_words(s);
        assert_eq!(punct_prefix_len(&t("hello world foo"), &t("hello WORLD bar")), 2);
        // A punctuation-only change stops the punct-sensitive prefix...
        assert_eq!(punct_prefix_len(&t("it."), &t("it?")), 0);
        // ...where the text-only comparison would keep going.
        assert_eq!(punct_prefix_len(&t("it"), &t("it")), 1);
    }

    // ── Unified alignment ────────────────────────────────────────────

    #[test]
    fn find_new_tail_empty_prev_returns_all_curr() {
        assert_eq!(
            find_new_tail(&[], &parse_words("hello world"), 2).unwrap(),
            parse_words("hello world")
        );
    }

    #[test]
    fn find_new_tail_verbatim_prefix() {
        assert_eq!(
            find_new_tail(&parse_words("hello"), &parse_words("hello world foo"), 2).unwrap(),
            parse_words("world foo"),
        );
    }

    #[test]
    fn find_new_tail_suffix_prefix_overlap() {
        // A suffix of the committed tail overlaps curr's start (sliding mode).
        assert_eq!(
            find_new_tail(&parse_words("hello world"), &parse_words("world foo bar"), 2).unwrap(),
            parse_words("foo bar"),
        );
        // Longest match wins.
        assert_eq!(
            find_new_tail(&parse_words("we should deploy now"), &parse_words("deploy now please"), 2)
                .unwrap(),
            parse_words("please"),
        );
    }

    #[test]
    fn find_new_tail_tolerates_a_reworded_boundary_word() {
        // Whisper re-punctuated/cased "hello, world" into "hello World foo" —
        // the anchor must survive so only "foo" is treated as new.
        assert_eq!(
            find_new_tail(&parse_words("hello, world"), &parse_words("hello World foo"), 2).unwrap(),
            parse_words("foo"),
        );
    }

    #[test]
    fn find_new_tail_refining_emits_no_tail() {
        assert_eq!(
            find_new_tail(&parse_words("hello world foo"), &parse_words("hello world"), 2),
            Some(vec![])
        );
        assert_eq!(find_new_tail(&parse_words("a b c"), &parse_words("a b"), 2), Some(vec![]));
    }

    #[test]
    fn find_new_tail_declines_when_no_anchor_exists() {
        // No committed word appears in curr: nothing safe to type this pass
        // (the old confident-only diff's exact contract, now unified).
        assert_eq!(find_new_tail(&parse_words("x y"), &parse_words("a b c d"), 2), None);
    }

    #[test]
    fn find_new_tail_skips_restated_filler_at_the_window_start() {
        // A leading fragment Whisper restated differently ("noise") must be
        // swallowed by the anchor, not re-typed as if it were new.
        assert_eq!(
            find_new_tail(
                &parse_words("cat sat the mat"),
                &parse_words("noise cat sat the mat and then"),
                2,
            )
            .unwrap(),
            parse_words("and then"),
        );
    }

    #[test]
    fn find_new_tail_anchors_committed_tail_anywhere_in_curr_not_just_the_prefix() {
        // Regression for the live duplication: once committed text exceeds
        // SLIDING_DIFF_LOOKBACK_WORDS, the committed tail (last 20 words) is
        // NOT a prefix of curr — curr starts with the full committed text.
        // A prefix-only alignment anchored on a single coincidental word
        // ("This") and re-typed the whole committed sentence as new.
        let committed = parse_words(
            "This definitely looks better out of the gate. I really like that we fixed \
             the punctuation thing. This is really cool. I just noticed",
        );
        assert!(committed.len() > SLIDING_DIFF_LOOKBACK_WORDS);
        let tail = committed[committed.len() - SLIDING_DIFF_LOOKBACK_WORDS..].to_vec();
        let curr = parse_words(
            "This definitely looks better out of the gate. I really like that we fixed \
             the punctuation thing. This is really cool. I just noticed that after a \
             while it does get",
        );
        let new_tail = find_new_tail(&tail, &curr, ALIGN_BAND).unwrap();
        assert_eq!(
            render_words(&new_tail),
            "that after a while it does get",
            "must not re-include any already-committed word"
        );
    }

    #[test]
    fn find_new_tail_is_bounded_by_lookback_not_session_history() {
        // Regression for the silent-freeze bug: once committed text exceeds
        // one window's worth of words, the anchor must only consider the
        // recent SLIDING_DIFF_LOOKBACK_WORDS tail — not the whole session —
        // so a fresh window with genuinely new content still anchors.
        let mut committed = Vec::new();
        for i in 0..80 {
            committed.push(Word {
                text: if i % 2 == 0 { "word".into() } else { "other".into() },
                punct: None,
            });
        }
        let tail = committed[committed.len() - SLIDING_DIFF_LOOKBACK_WORDS..].to_vec();
        let found =
            find_new_tail(&tail, &parse_words("word other word other word other brand new content"), 2)
                .unwrap();
        assert_eq!(render_words(&found), "brand new content");
    }

    fn words(s: &str) -> Vec<Word> {
        parse_words(s)
    }

    fn revision_session() -> Session {
        let mut cfg = streaming_config();
        cfg.revision_mode = true;
        Session::new(Arc::new(FakeTranscriber), cfg)
    }

    #[test]
    fn final_flush_falls_back_to_a_best_effort_guess_when_no_confident_anchor_exists() {
        // Reproduces a real gap found live: a recording hit its hard timeout
        // mid-sentence, and the very last pass reworded the trailing content
        // so thoroughly that no anchor could re-attach it. In the tick loop,
        // declining and waiting for the next tick is correct; at
        // `final_flush` there is no next tick, so a best-effort guess beats
        // leaving the previous text uncorrected forever.
        let mut session = revision_session();
        session.committed_words = words("x y");
        session.feed(&loud_samples(1.5));
        session.base = Arc::new(RevisingTranscriber::new(vec!["a b c d"]));

        let delta = session
            .final_flush()
            .expect("must fall back to a guess instead of silently giving up");

        // Committed "x y" has no overlap with the fresh "a b c d", so the
        // fallback slices the tail beyond committed; it carries its leading
        // separator, like every delta that follows existing text.
        assert_eq!(
            delta,
            Delta::Append(" c d".to_string()),
            "should type the safety-guess tail rather than leave nothing corrected"
        );
    }

    #[test]
    fn find_new_tail_growing_mode_is_immune_to_reworded_prefix_drift() {
        // Regression: growing mode used to slice curr by raw word-count
        // index, which re-grabbed "better." after Whisper heard one extra
        // word ("much") and shifted the boundary. The unified alignment
        // anchors on text, never a raw index, so the reworded "much" is
        // swallowed and only genuinely-new text remains the new tail.
        let mut session = revision_session();
        session.committed_words = words("This works way better.");
        let tail = find_new_tail(
            &session.committed_tail(),
            &words("This works way much better. No hang ups."),
            ALIGN_BAND,
        )
        .unwrap();
        assert_eq!(
            render_words(&tail),
            "No hang ups.",
            "must not re-include any part of the reworded confirmed sentence"
        );
    }

    #[test]
    fn typed_len_from_counts_leading_space_only_when_prefixed() {
        let w = words("alpha beta gamma");
        // Removing everything, with something before it (leading space
        // counts): " alpha beta gamma" = 1 + 16 = 17 chars.
        assert_eq!(typed_len_from(&w, 0, true), 17);
        // Removing everything, nothing before it (no leading space to
        // remove): "alpha beta gamma" = 16 chars.
        assert_eq!(typed_len_from(&w, 0, false), 16);
        // Removing from index 1 onward: " beta gamma" (leading space
        // before "beta" always counts, regardless of has_prefix, since
        // "alpha" is right before it either way).
        assert_eq!(typed_len_from(&w, 1, true), 11);
        assert_eq!(typed_len_from(&w, 1, false), 11);
        // Nothing left to remove.
        assert_eq!(typed_len_from(&w, 3, true), 0);
    }

    #[test]
    fn format_typed_matches_typed_len_from() {
        let w = words("alpha beta gamma");
        assert_eq!(format_typed(&w, 0, true), " alpha beta gamma");
        assert_eq!(format_typed(&w, 0, false), "alpha beta gamma");
        assert_eq!(format_typed(&w, 1, true), " beta gamma");
        // Symmetry: the backspace count always matches the removed
        // string's exact character length.
        for (from_idx, has_prefix) in [(0, true), (0, false), (1, true), (2, false)] {
            let removed = format_typed(&w, from_idx, has_prefix);
            assert_eq!(
                typed_len_from(&w, from_idx, has_prefix),
                removed.chars().count()
            );
        }
    }

    /// Drive the real per-tick flow the engine runs for type-then-correct:
    /// reconcile (type/correct), tally stability, promote.
    fn feed_tick(session: &mut Session, tail: &[Word]) -> Option<Delta> {
        let delta = session.reconcile(tail);
        session.provisional_stability = session.update_stability(tail);
        session.promoted_last_tick = session.promote();
        delta
    }

    #[test]
    fn reconcile_pure_extension_has_zero_backspace() {
        let mut session = revision_session();
        // First tick: nothing displayed yet, tail is "hello".
        let delta = feed_tick(&mut session, &words("hello"));
        assert_eq!(delta, Some(Delta::Append("hello".to_string())));
        assert_eq!(session.provisional_words, words("hello"));

        // Second tick: tail grew to "hello world" — pure extension.
        let delta = feed_tick(&mut session, &words("hello world"));
        assert_eq!(delta, Some(Delta::Append(" world".to_string())));
        assert_eq!(session.provisional_words, words("hello world"));
    }

    #[test]
    fn reconcile_no_change_emits_nothing() {
        let mut session = revision_session();
        session.reconcile(&words("hello world"));
        let delta = session.reconcile(&words("hello world"));
        assert_eq!(delta, None);
    }

    #[test]
    fn reconcile_word_divergence_backspaces_only_the_wrong_suffix() {
        let mut session = revision_session();
        session.reconcile(&words("turn on the lamp"));

        // Whisper changes its mind: "lamp" should have been "light".
        let delta = session.reconcile(&words("turn on the light"));
        match delta {
            Some(Delta::Replace { backspace, text }) => {
                // Only "lamp" (4 chars) plus its leading space needs
                // removing — "turn on the" survived unchanged.
                assert_eq!(backspace, " lamp".chars().count());
                assert_eq!(text, " light");
            }
            other => panic!("expected a Replace correction, got {other:?}"),
        }
        assert_eq!(session.provisional_words, words("turn on the light"));
    }

    #[test]
    fn reconcile_punctuation_flip_backspaces_only_the_stale_punct_char() {
        // Whisper heard "it." first, then "it?" — the type-then-correct
        // engine must fix the punctuation with a 1-char backspace, not
        // re-type the word.
        let mut session = revision_session();
        session.reconcile(&words("what time is it."));
        let delta = session.reconcile(&words("what time is it?"));
        match delta {
            Some(Delta::Replace { backspace, text }) => {
                assert_eq!(backspace, 1, "only the stale '.' needs removing");
                assert_eq!(text, "?");
            }
            other => panic!("expected a punct Replace, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_punct_flip_mid_tail_rewrites_only_the_following_tail() {
        // A stale punctuation char sits mid-text behind already-typed words
        // ("it? tomorrow"): removing it must backspace the stale "?" plus
        // everything after it, then retype the corrected tail.
        let mut session = revision_session();
        feed_tick(&mut session, &words("what time is it? tomorrow"));
        let mut new_tail = words("what time is it tomorrow afternoon?");
        session.reconcile_question_marks(&mut new_tail);
        let delta = session.reconcile(&new_tail);
        match delta {
            Some(Delta::Replace { backspace, text }) => {
                assert_eq!(backspace, "? tomorrow".chars().count());
                assert_eq!(text, " tomorrow afternoon?");
            }
            other => panic!("expected a punct Replace, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_divergence_mid_tail_only_touches_wrong_suffix() {
        let mut session = revision_session();
        session.config.stability_passes = 1; // promote eagerly for this test
        // 6 words with lag 4 immediately confirms the first 2 ("the",
        // "cat") — provisional_words is left as ["sat", "on", "the", "mat"].
        feed_tick(&mut session, &words("the cat sat on the mat"));
        assert_eq!(session.provisional_words, words("sat on the mat"));

        // Next tail (as `find_new_tail` would return it) excludes the
        // now-confirmed "the cat". Only "sat on the" still agrees;
        // "mat" -> "rug" is the actual correction.
        let delta = session.reconcile(&words("sat on the rug"));
        match delta {
            Some(Delta::Replace { backspace, text }) => {
                assert_eq!(backspace, " mat".chars().count());
                assert_eq!(text, " rug");
            }
            other => panic!("expected a Replace correction, got {other:?}"),
        }
    }

    #[test]
    fn promotion_confirms_stable_words_behind_the_lag_and_never_revisits_them() {
        let mut session = revision_session();
        // Feed the tail one word at a time through the real per-tick flow.
        let all_words = words("one two three four five six seven");
        for n in 1..=all_words.len() {
            let committed_len = session.committed_words.len();
            // Same tail the engine would compute: everything beyond what's
            // already confirmed.
            let new_tail = all_words[committed_len..n].to_vec();
            feed_tick(&mut session, &new_tail);
        }
        // The lag window keeps the last `revision_lag_words` provisional...
        assert!(
            session.provisional_words.len() <= session.config.revision_lag_words
        );
        // ...but committed + provisional still reconstruct the whole tail,
        // with no word duplicated or lost.
        let mut combined = session.committed_words.clone();
        combined.extend(session.provisional_words.iter().cloned());
        assert_eq!(combined, all_words);
    }

    #[test]
    fn question_mark_reconcile_drops_a_stale_question_mark() {
        // The priority fix: "what time is it? tomorrow afternoon?" must end
        // with exactly one "?" — the premature one after "it" is removed.
        let mut session = revision_session();
        session.committed_words = words("what time is");
        let mut new_tail = words("it? tomorrow afternoon?");
        session.reconcile_question_marks(&mut new_tail);
        assert_eq!(render_words(&new_tail), "it tomorrow afternoon?");
    }

    #[test]
    fn question_mark_reconcile_leaves_a_single_question_alone() {
        let mut session = revision_session();
        session.committed_words = words("what time is");
        let mut new_tail = words("it tomorrow afternoon?");
        session.reconcile_question_marks(&mut new_tail);
        assert_eq!(render_words(&new_tail), "it tomorrow afternoon?");
    }

    #[test]
    fn question_mark_reconcile_never_touches_committed_text() {
        // The stale "?" is within `question_mark_lookback_words`, which is
        // capped at the revision lag, so it is always still provisional and
        // committed words are never edited. Here the only "?" is inside
        // committed text (defensive): the reconcile must leave new_tail alone.
        let mut session = revision_session();
        session.config.question_mark_lookback_words = 0;
        session.committed_words = words("really?");
        let mut new_tail = words("no thanks");
        session.reconcile_question_marks(&mut new_tail);
        assert_eq!(render_words(&new_tail), "no thanks");
    }

    #[test]
    fn last_n_words_returns_tail_or_whole_string() {
        assert_eq!(last_n_words("a b c d e", 3), "c d e");
        assert_eq!(last_n_words("a b", 3), "a b");
        assert_eq!(last_n_words("", 3), "");
        assert_eq!(last_n_words("a b c", 0), "");
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
            revision_mode: true,
            question_mark_lookback_words: 3,
            stability_passes: 2,
            revision_lag_words: 4,
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

    /// Simulate what actually ends up on screen after applying every
    /// event in order — `Partial`/`Final` append, `Replace` backspaces
    /// `backspace` chars off the end first. This is what a revision-mode
    /// test needs to assert on (unlike `emitted_text`, which only looks at
    /// what was sent, not what survives after corrections).
    fn reconstruct_typed_text(events: &[StreamingEvent]) -> String {
        let mut screen = String::new();
        for ev in events {
            match ev {
                StreamingEvent::Partial { text, .. } | StreamingEvent::Final { text, .. } => {
                    screen.push_str(text);
                }
                StreamingEvent::Replace {
                    backspace, text, ..
                } => {
                    let keep = screen.chars().count().saturating_sub(*backspace);
                    screen = screen.chars().take(keep).collect();
                    screen.push_str(text);
                }
                _ => {}
            }
        }
        screen
    }

    /// Fake backend that replays a fixed sequence of transcriptions, one
    /// per call, holding the last one for any further calls — for testing
    /// revision mode's correction path (`FakeTranscriber`'s monotonic
    /// growth never disagrees with itself, so it can't exercise Replace).
    struct RevisingTranscriber {
        calls: std::sync::Mutex<usize>,
        sequence: Vec<&'static str>,
    }

    impl RevisingTranscriber {
        fn new(sequence: Vec<&'static str>) -> Self {
            Self {
                calls: std::sync::Mutex::new(0),
                sequence,
            }
        }
    }

    impl Transcriber for RevisingTranscriber {
        fn transcribe(&self, _samples: &[f32]) -> Result<String, TranscribeError> {
            let mut calls = self.calls.lock().unwrap();
            let idx = (*calls).min(self.sequence.len() - 1);
            *calls += 1;
            Ok(self.sequence[idx].to_string())
        }
    }

    async fn run_session_with(
        transcriber: Arc<dyn Transcriber>,
        config: SlidingWindowConfig,
        ticks: usize,
    ) -> Vec<StreamingEvent> {
        let engine = SlidingWindowStreamingTranscriber::new(transcriber, config);
        let (tx, rx) = mpsc::channel::<Vec<f32>>(32);
        let mut handle = engine.start_stream(rx).expect("start stream");

        for _ in 0..ticks {
            tx.send(loud_samples(0.5)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
        drop(tx);

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
        // Conservative gate + Final events: the legacy non-typing-tail mode.
        let mut cfg = streaming_config();
        cfg.revision_mode = false;
        cfg.type_partials = false;
        let events = run_session(cfg).await;

        let parts = emitted_text(&events);
        assert_eq!(parts.concat(), "alpha beta gamma delta");
        assert!(events
            .iter()
            .all(|ev| !matches!(ev, StreamingEvent::Partial { .. })));
    }

    #[tokio::test]
    async fn revision_mode_end_to_end_reconstructs_final_transcript_via_growth_only() {
        // FakeTranscriber never disagrees with itself, so this exercises
        // the plain-Append path end-to-end through real events (no
        // Replace expected) — a sanity check that revision mode doesn't
        // regress the ordinary case.
        let mut cfg = streaming_config();
        cfg.revision_mode = true;
        let events = run_session(cfg).await;

        assert!(matches!(events.last(), Some(StreamingEvent::Ended)));
        assert_eq!(reconstruct_typed_text(&events), "alpha beta gamma delta");
    }

    #[tokio::test]
    async fn revision_mode_end_to_end_corrects_a_misheard_word() {
        // Whisper's growing-buffer transcription first hears "lamp", then
        // on a later pass corrects itself to "light" for the same word —
        // exactly the live scenario revision mode exists for. Sequence
        // indexed by call count (one call per ~0.5s chunk fed below); the
        // buffer only has enough audio for a non-empty transcription from
        // the 3rd chunk onward (min_audio_s = 1.0 in streaming_config()).
        let transcriber: Arc<dyn Transcriber> = Arc::new(RevisingTranscriber::new(vec![
            "",
            "",
            "turn on the lamp",
            "turn on the lamp",
            "turn on the light",
            "turn on the light",
        ]));
        let mut cfg = streaming_config();
        cfg.revision_mode = true;
        let events = run_session_with(transcriber, cfg, 6).await;

        assert!(matches!(events.last(), Some(StreamingEvent::Ended)));
        assert!(
            events
                .iter()
                .any(|ev| matches!(ev, StreamingEvent::Replace { .. })),
            "expected a correction event, got {events:?}"
        );
        // The end result must be the corrected text, not a mix of both —
        // this is the whole point: the wrong guess actually gets undone,
        // not just papered over by later appends.
        assert_eq!(reconstruct_typed_text(&events), "turn on the light");
    }

    #[tokio::test]
    async fn question_mark_reconcile_end_to_end_puts_one_question_mark_in_the_right_place() {
        // The priority fix, end-to-end: Whisper first hears "what time is
        // it?" (the question isn't over), then hears the full question
        // "what time is it? tomorrow afternoon?" with a second "?". The stale
        // "?" after "it" must be dropped so the utterance ends with exactly
        // one "?" — in the right place (the true end).
        let transcriber: Arc<dyn Transcriber> = Arc::new(RevisingTranscriber::new(vec![
            "",
            "",
            "what time is it?",
            "what time is it?",
            "what time is it? tomorrow afternoon?",
            "what time is it? tomorrow afternoon?",
        ]));
        let mut cfg = streaming_config();
        cfg.revision_mode = true;
        let events = run_session_with(transcriber, cfg, 6).await;

        assert!(matches!(events.last(), Some(StreamingEvent::Ended)));
        assert!(
            events
                .iter()
                .any(|ev| matches!(ev, StreamingEvent::Replace { .. })),
            "expected a revision event that removes the premature '?', got {events:?}"
        );
        assert_eq!(
            reconstruct_typed_text(&events),
            "what time is it tomorrow afternoon?",
            "exactly one question mark, at the end"
        );
    }

    #[tokio::test]
    async fn flickering_word_is_corrected_every_tick_without_stalling_the_tail() {
        // The live stall scenario: one word flickers between passes ("said" /
        // "doesn't" / "so") while the rest of the tail stays well-formed.
        // Type-then-correct must correct that one word on every tick instead
        // of withholding the whole tail until two passes agree — no
        // multi-tick stall.
        let transcriber: Arc<dyn Transcriber> = Arc::new(RevisingTranscriber::new(vec![
            "",
            "",
            "still said okay.",
            "still doesn't okay.",
            "still so okay.",
            "still so okay.",
        ]));
        let mut cfg = streaming_config();
        cfg.revision_mode = true;
        let events = run_session_with(transcriber, cfg, 6).await;

        assert!(matches!(events.last(), Some(StreamingEvent::Ended)));
        assert!(
            events
                .iter()
                .filter(|ev| matches!(ev, StreamingEvent::Replace { .. }))
                .count() >= 2,
            "expected the flickering word to be revised on each disagreeing tick, got {events:?}"
        );
        // Ends on the last stable form — a stall would leave an earlier form
        // stuck on screen uncorrected.
        assert_eq!(reconstruct_typed_text(&events), "still so okay.");
    }

    #[tokio::test]
    async fn punct_flip_end_to_end_fixes_punctuation() {
        // Whisper re-punctuates the same word ("it." → "it?") on a later
        // pass. Type-then-correct revises it (a Replace), so the final
        // transcript carries the corrected punctuation.
        let transcriber: Arc<dyn Transcriber> = Arc::new(RevisingTranscriber::new(vec![
            "",
            "",
            "what time is it.",
            "what time is it.",
            "what time is it?",
            "what time is it?",
        ]));
        let mut cfg = streaming_config();
        cfg.revision_mode = true;
        let events = run_session_with(transcriber, cfg, 6).await;

        assert!(matches!(events.last(), Some(StreamingEvent::Ended)));
        assert!(
            events
                .iter()
                .any(|ev| matches!(ev, StreamingEvent::Replace { .. })),
            "expected a punctuation revision, got {events:?}"
        );
        assert_eq!(reconstruct_typed_text(&events), "what time is it?");
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
