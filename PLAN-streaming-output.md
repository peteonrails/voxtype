# Plan: streaming output, daemon wiring, and live OSD partial display

Branch: `feature/streaming-transcription` (rebased on `rc/0.7.0` 2026-05-01).
Status before this plan: scaffolding-only.

- `State::Streaming` exists in `src/state.rs` but is never entered.
- `StreamingTranscriber` trait + `StreamHandle` exist in `src/transcribe/streaming.rs`. Zero implementations.
- `StreamingSession` exists in `src/output/streaming.rs` with `observe_partial`, `commit_segment`, `rewind`. Tested but never called from the daemon.
- Default `Transcriber::as_streaming()` returns `None` for every backend.

## Goals

1. Make incremental output work end-to-end with a local Parakeet backend.
2. Show in-progress partial text on the existing OSD.
3. Land a cloud streaming backend (Gemini Live or similar) on top of the same plumbing without redesign.

Order matters: each phase below is independently shippable. Phases A+B together form a usable v0.

## Engine decision: Parakeet-only for v1

Streaming requires an engine whose architecture supports it. After surveying the
backends voxtype ships:

- **Parakeet (RNN-T, via `parakeet-rs` 0.3.5):** real cache-aware streaming via
  `ParakeetUnified::transcribe_chunk` and `UnifiedStreamingConfig`. Sub-second
  first-token latency by design. **This is what we use.**
- **Whisper:** encoder-decoder, no native streaming. A VAD-chunking shim was
  considered and declined (see Out of Scope) because the latency ceiling is ~1s
  and the OSD partial display becomes a progress bar instead of a live readout.
- **Moonshine, SenseVoice, Paraformer, Dolphin, Omnilingual:** attention-based,
  no streaming exposed by their respective Rust wrappers.
- **Cohere, remote/HTTP:** API-batch by definition.

Users on a non-streaming engine who set `streaming = true` get a clear error:
"Streaming requires Parakeet. Run `voxtype setup model parakeet-streaming` to
install, or unset `streaming` in your config." The runtime check lives in the
transcriber factory.

This commits voxtype to Parakeet as the primary streaming engine and, by
implication, makes Parakeet the recommended default for users who want the
best experience. English-only is the v1 reality. Multilingual streaming is a
follow-up question (see Out of Scope).

---

## Phase A — Parakeet streaming adapter

**Why first:** validates the trait shape against a real streaming backend with
zero daemon changes. Pure backend code wrapping `parakeet-rs`'s existing
streaming API.

**File:** `src/transcribe/parakeet.rs` gets a `StreamingTranscriber` impl
alongside its existing `Transcriber` impl. Override `as_streaming()` to
return `Some(self)` when the engine config selects Parakeet. No new file
needed; the streaming code lives in the same module.

**Wire shape:**

```rust
impl StreamingTranscriber for ParakeetTranscriber {
    fn start_stream(
        &self,
        mut samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        let (events_tx, events_rx) = mpsc::channel(64);
        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        let unified = self.unified.clone(); // ParakeetUnified handle

        let task = tokio::spawn(async move {
            let mut segment_id: u64 = 0;
            let mut last_text = String::new();
            loop {
                tokio::select! {
                    _ = &mut cancel_rx => break,
                    chunk = samples_rx.recv() => {
                        let Some(chunk) = chunk else { break }; // EOF
                        let text = unified.transcribe_chunk(&chunk)?;
                        if text != last_text {
                            // parakeet-rs returns growing partials; treat
                            // as Partial until we see end-of-segment.
                            let _ = events_tx.send(StreamingEvent::Partial {
                                text: text.clone(),
                                segment_id,
                            }).await;
                            last_text = text;
                        }
                        // Final emission rule: TBD during implementation —
                        // either on end-of-utterance signal from the model
                        // (if exposed) or on `samples_rx` close.
                    }
                }
            }
            // Flush any pending text as a Final on close.
            if !last_text.is_empty() {
                let _ = events_tx.send(StreamingEvent::Final {
                    text: last_text,
                    segment_id,
                }).await;
            }
            let _ = events_tx.send(StreamingEvent::Ended).await;
            Ok(())
        });

        Ok(StreamHandle { events: events_rx, cancel: cancel_tx, task })
    }
}
```

The exact partial-vs-final emission rule depends on what end-of-utterance
signals `parakeet-rs` exposes. If the unified streaming API gives us segment
boundaries (e.g., via the EOU model — `parakeet-rs/src/model_eou.rs`), use
those. Otherwise emit `Final` only on stream close and treat all interim
events as `Partial`. This is the **first thing to verify in Phase A**.

**Configuration:**

```toml
[transcribe.parakeet]
streaming = false                    # opt-in
streaming_left_context_secs = 1.5    # UnifiedStreamingConfig defaults from
streaming_chunk_secs = 0.5           # parakeet-rs; expose for tuning
streaming_right_context_secs = 0.5
```

**Acceptance:**
- Unit test: feed a recorded utterance WAV into the adapter, assert at
  least one `Partial`, exactly one `Final`, exactly one `Ended`.
- Manual: `cargo run --features parakeet-cuda` with `streaming = true`,
  speak into the mic, watch debug logs for `Partial` events at sub-second
  cadence.
- No daemon changes yet (Phase B). The adapter is constructed but not
  invoked from `daemon.rs` — verified standalone.

---

## Phase B — daemon wiring

**Why:** `State::Streaming` is the missing seam. Without this, no backend matters.

### B.1 — fan out audio chunks to two consumers

rc/0.7.0 introduced `audio::levels::LevelHub`, which already taps the
`mpsc::Receiver<Vec<f32>>` returned by `AudioCapture::start()`. We can't
double-consume the receiver, so the level emitter task becomes the fan-out point:

- Today: `spawn_emitter(samples_rx, hub)` forwards every chunk into the level
  bucketer.
- New: `spawn_emitter(samples_rx, hub, Some(streaming_tx))` *also* clones each
  chunk into `streaming_tx: mpsc::Sender<Vec<f32>>` when present.

The clone cost is one `Vec<f32>::clone()` per 10 ms window — bounded, cheap
(160 f32 = 640 bytes per chunk in the level bucketer's case; whatever capture's
chunk size is). When streaming is off, `streaming_tx` is `None` and the path is
zero overhead.

**Alternative considered:** a `tokio::sync::broadcast` channel directly from
capture. Rejected because broadcast requires `Clone` on the message type and
loses the bounded-mpsc backpressure shape capture relies on. The tee in the
existing emitter task keeps the existing flow intact.

### B.2 — `start_streaming_capture` helper

Parallel to `start_recording_capture` (introduced in rc/0.7.0). Returns
`(Box<dyn AudioCapture>, mpsc::Receiver<Vec<f32>>)` where the second is the tee
for the streaming backend. Daemon hands the receiver to
`StreamingTranscriber::start_stream` and gets a `StreamHandle` back.

### B.3 — hotkey-release / SIGUSR2 routing

In `src/daemon.rs`, where today the released-PTT path checks
`state.is_recording()` and `state.is_eager_recording()`, add a third branch:

```rust
if let Some(streaming) = self.transcriber.as_streaming() {
    if self.config.transcribe.streaming {
        let (capture, samples_rx) = self.start_streaming_capture().await?;
        let handle = streaming.start_stream(samples_rx)?;
        state = State::Streaming { /* ... */ };
        // Event pump runs in the existing tokio::select! via a new arm.
    }
}
```

### B.4 — event pump arm in `tokio::select!`

A new arm polls `stream_handle.events.recv()` while `state.is_streaming()`:

| Event | Action |
|---|---|
| `Partial { text, .. }` | `session.observe_partial(text)`, broadcast to `PartialHub` (Phase C) |
| `Final { text, .. }` | `session.commit_segment(...)` — types via existing fallback chain |
| `Ended` | drop `samples_tx`, await `task`, call `reset_to_idle` |
| `Error(e)` | log + notify, then same teardown as `Ended` |

### B.5 — cancel rewind

The cancel paths fixed by `cancel_to_idle` (commit `9bf6222` on
`fix/resume-media-on-cancel-and-error`) need a streaming-aware sibling:

```rust
async fn cancel_streaming_to_idle(&mut self, state: &mut State, body: &str) {
    if let State::Streaming { typed_chars, .. } = state {
        let _ = self.streaming_session.rewind().await;  // best-effort
    }
    self.cancel_to_idle(state, body).await;
}
```

Wire into the same 4 cancel sites.

### B.6 — `auto_submit` and `append_text` semantics

**Decision:** apply `append_text` and `auto_submit` only on the *terminal* event
(`Ended` or final-segment marker), not per-segment. Reasoning:

- `append_text = " "` per segment would insert a leading space into every
  finalized segment, doubling spaces around punctuation.
- `auto_submit = true` per segment would press Enter after each utterance,
  which most users don't want — they want one Enter at end-of-recording.

If a user genuinely wants per-segment auto-submit (utterance-as-line), expose it
later as `[transcribe] streaming_auto_submit_per_segment = false` (default
preserves today's behavior). Don't add the option until someone asks.

### B.7 — config + CLI

```toml
[transcribe]
streaming = false                 # default off; opt-in
# engine selection stays with [transcribe] engine (parakeet, whisper, ...).
# streaming = true is only honored when the selected engine implements
# StreamingTranscriber. Anything else fails fast with an actionable error.
```

CLI:
- `--streaming` boolean flag
- `VOXTYPE_STREAMING=true`

No separate `streaming_engine` selector — the existing engine choice + the
`as_streaming()` method on `Transcriber` already discriminates. Per voxtype
principle 5 (every option configurable everywhere).

### B.8 — notification urgency

rc/0.7.0's `send_notification` now takes an urgency arg. Streaming session start
and end use `"normal"`; streaming errors use `"critical"`. Trivial plumbing.

**Acceptance for Phase B:**
- With `streaming = true` and the Phase A shim selected, recording produces
  incrementally typed words during the recording window.
- Cancel hotkey rewinds typed characters.
- Existing non-streaming flow (default) is unchanged. Regression test: all
  653 currently-passing tests still pass (the 1 pre-existing `parakeet`
  test failure is independent).

---

## Phase C — partial-text broadcast (`PartialHub`)

**Why:** the OSD work in Phase D needs an IPC to subscribe to. Build the
broadcaster on the daemon side first, with no UI consumer, so we can verify
it independently.

**File:** `src/transcribe/partial_hub.rs` (new). Mirrors the design of
`audio::levels::LevelHub` deliberately — same Unix socket pattern, same
bounded-queue fan-out, same drop-slow-consumers policy.

```text
$XDG_RUNTIME_DIR/voxtype/partials.sock

Wire format per frame (variable length):
  u32  seq            — monotonic frame counter
  u32  segment_id     — matches StreamingEvent.segment_id
  u8   kind           — 0 = partial, 1 = final, 2 = ended
  u32  text_len       — bytes of UTF-8 text
  []u8 text           — UTF-8, no NUL termination
```

`StreamingSession::observe_partial` and `commit_segment` gain a
`broadcast: Option<&PartialHub>` parameter and emit a frame on each call. When
no subscribers are connected, frames are discarded with a `try_send` no-op.
Idle cost: zero, like `LevelHub`.

**Acceptance:**
- Unit test: subscribe to the socket via a test harness, drive a streaming
  session, assert the expected sequence of frames is observed.
- No OSD changes yet. The hub is purely additive; existing OSD-less builds
  still work.

---

## Phase D — OSD live partial display

**Why:** the user wants to see what's coming. The OSD already has a 100 Hz
audio-level visualizer; the natural extension is a text caption underneath
that updates as partials arrive and freezes briefly when a final commits.

**Files:**
- `src/osd/ipc.rs` — add `PartialFrame` decoder, mirror of `AudioFrame`.
- `src/bin/voxtype_osd_native/app.rs` — add a `partial_text: String` field,
  subscribe to `partials.sock` alongside the existing `audio.sock` subscription,
  re-render text on each frame.
- `src/osd/visual.rs` — text rendering primitive (the OSD doesn't render text
  today; this is the largest single piece of work in Phase D).

**Render policy:**
- Partials: rendered in a slightly dimmer color than finals.
- Finals: appended to a "committed" line (or the whole text), rendered in the
  primary color. After 800 ms of no new events, the committed line fades.
- `Ended` event: hold final text on screen for 1.5 s, then dismiss.

**OSD text widget options to investigate:**
- The native OSD uses GTK4 (`voxtype_osd_gtk4`) and a separate native
  (non-GTK) variant. GTK4 has a `Label` widget — straightforward.
- The native variant currently does only level visualization. We'd add a
  `cosmic-text` or `pangocairo` text layer. Decide during implementation;
  keep both variants in sync.

**Acceptance:**
- Run daemon with `streaming = true` and the OSD; speak; partial text
  appears on the OSD as it streams.
- OSD without daemon (or daemon without streaming) shows nothing new and
  doesn't crash. No regressions in the existing level visualizer.

---

## Phase E — Gemini Live backend

**Why last:** proves the trait against a true streaming provider with
sub-second latency, but adds zero value before A–D are working. Also has the
biggest external surface (auth, network errors, Google's API churn).

**File:** `src/transcribe/gemini_live.rs` (new). Direct WebSocket via
`tokio-tungstenite` (avoid pulling a Google SDK). Implements
`StreamingTranscriber`. Server-side VAD, native 16 kHz audio.

Config:
```toml
[transcribe]
streaming = true
streaming_engine = "gemini"

[transcribe.gemini]
api_key_env = "GEMINI_API_KEY"
model = "gemini-2.0-flash-realtime"  # or whatever ships
```

**Acceptance:**
- End-to-end: recording → partial text appears on OSD within ~300 ms of
  speech → final segments type into the cursor.
- Auth failure produces a clear error notification, not a silent hang.
- Network failure mid-stream: emit `Error`, daemon falls back to non-streaming
  for the remainder of the session (or terminates cleanly with a notification).

---

## Phasing summary

| Phase | What ships | Touches | Dependent on |
|---|---|---|---|
| A — Parakeet adapter | `StreamingTranscriber for ParakeetTranscriber` | `src/transcribe/parakeet.rs`, `src/config.rs` | nothing |
| B — daemon wiring | `State::Streaming` entered, words type incrementally | `src/daemon.rs`, `src/audio/levels.rs` (tee), `src/config.rs`, `src/cli.rs` | A |
| C — partial broadcast | IPC for partials, no UI | `src/transcribe/partial_hub.rs`, `StreamingSession` | B |
| D — OSD text | Live partial display on OSD | `src/osd/`, `src/bin/voxtype_osd_native/`, `src/bin/voxtype_osd_gtk4.rs` | C |
| E — second backend | Cloud (Gemini Live) or other streaming engine | `src/transcribe/gemini_live.rs`, `Cargo.toml` | B (not D) |

A+B is the MVP for "Parakeet users see incremental typing." A+B+C+D is the
MVP for "Parakeet users see incremental text *and* a live OSD readout." E
proves the architecture against a second streaming backend.

## Out of scope for this branch

- **Whisper streaming via VAD-chunking shim.** Considered and declined: the
  latency ceiling is ~1s, the OSD partial display becomes a progress bar, and
  the shim is ~400 lines of plumbing for an inferior product. Whisper users
  who want streaming switch to Parakeet. Revisit if (a) `whisper-rs` exposes
  real streaming upstream, or (b) we discover meaningful demand from users
  who can't switch off Whisper for accuracy or language reasons.
- **Multilingual streaming.** Parakeet is English-first. The multilingual
  variants in NVIDIA NeMo are weaker than Whisper for non-English. Revisit
  when (a) a streaming-capable multilingual ONNX engine ships, or (b) users
  ask for it loudly enough to justify either training a streaming
  multilingual model or paying the whisper-shim cost.
- OpenAI Realtime, ElevenLabs Scribe v2, Cohere streaming — adapters,
  follow-up branches once the architecture is proven on Parakeet + one
  cloud provider in Phase E.
- Continuous mode / always-on listening — separate feature, separate state
  machine work.
- Per-segment auto_submit toggle — wait for a user to ask.
- TUI streaming section in `src/tui/` — polish, do after E lands.

## Open questions to resolve during Phase A

1. **End-of-utterance signal from `parakeet-rs`.** Does the unified streaming
   API expose a segment-boundary event we can use to emit `Final`, or do we
   only emit `Final` on stream close? Check `parakeet-rs/src/model_eou.rs`
   and `model_multitalker.rs` — the multitalker pipeline mentions
   end-of-utterance probabilities. Decision affects whether mid-recording
   users see segments commit (good UX) or just see partials grow until they
   release the hotkey (acceptable but worse).

2. **`UnifiedStreamingConfig` defaults.** What chunk/context lengths give the
   best latency-vs-accuracy tradeoff for typing? `parakeet-rs` ships
   defaults; verify they're sensible for our use case before exposing as
   config.

3. **What happens on `Ended` if `partial` is non-empty?** Drop it
   (committed-only policy) or commit it as a final? Default to *commit it*
   if no end-of-utterance signal is available — otherwise users would lose
   their last few words on hotkey release. Document the rationale.

4. **GPU dispatch for streaming.** Parakeet-CUDA / parakeet-MIGraphX / CPU —
   do they all expose the same streaming API surface, or does CPU-only
   streaming have caveats? Check before promising "streaming works on all
   Parakeet builds."
