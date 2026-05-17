# Streaming transcription — implementation log

Branch: `feature/streaming-transcription`. Tracking commits, design decisions,
and what is left.

## Commits landed

### Commit 1 — state machine + streaming trait scaffolding (in progress)

Adds the trait, event types, and `State::Streaming` variant. No backends yet,
no daemon.rs wiring yet. All existing `Transcriber` impls untouched (default
`as_streaming()` returns `None`). All 544 existing lib tests still pass; 7 new
tests added (5 in `state::tests`, 2 in `transcribe::streaming::tests`).

Changed files:
- `src/state.rs` — `State::Streaming { started_at, model_override, partial_buffer, finalized_text, typed_chars }`. `is_recording()` now also returns `true` for `Streaming`. New `is_streaming()` helper. `Display` impl extended.
- `src/transcribe/mod.rs` — `pub mod streaming;` and a default `as_streaming(&self) -> Option<&dyn StreamingTranscriber>` method on `Transcriber`.
- `src/transcribe/streaming.rs` — new file. `StreamingEvent { Partial, Final, Ended, Error }`, `StreamHandle { events: mpsc::Receiver, cancel: oneshot::Sender, task: JoinHandle }`, `StreamingTranscriber` trait taking `mpsc::Receiver<Vec<f32>>` (matches `AudioCapture`'s output type).

Design notes / divergences from the v2 proposal in the prior STATUS:
- The v2 proposal had `cancel: Box<dyn FnOnce() + Send>`. Replaced with `oneshot::Sender<()>` — `FnOnce` requires consuming `self` to call, awkward to share alongside the events Receiver. A oneshot is cleaner and idiomatic in tokio.
- Added `task: JoinHandle<Result<(), TranscribeError>>` so the daemon can `await` the backend's drive task on shutdown / error reporting.
- `StreamingEvent` is **not** `Clone` (because `TranscribeError` isn't). Documented in code; events are consumed once from the channel which is the only realistic path.

### Commit 2 — output-layer streaming session

`src/output/streaming.rs` adds `StreamingSession`:

- `commit_segment()` types a finalized segment via the existing
  `output_with_fallback` chain. Tracks `typed_chars` in **Unicode scalars**
  (not bytes) so cancel-rewind sends one BackSpace per visible character.
- `observe_partial()` updates an in-memory partial buffer. *Never* typed.
- `rewind()` emits N BackSpace events via wtype → dotool → ydotool fallback.
  Best-effort: returns `Err(AllMethodsFailed)` if no backend works, daemon
  may surface as a soft warning.
- **Post-process decision: per-segment, with `VOXTYPE_CONTEXT` set to
  finalized-text-so-far.** Matches the existing eager-mode pattern in
  `output/post_process.rs::process_with_context`. Skipping it would silently
  break users who rely on the hook for cleanup. End-of-session would defeat
  the latency win. Per-segment is the only choice that keeps both.
- pre/post output hooks fire once per finalized segment (not once per session).
  Compositor submap toggles need to wrap each typing burst.

550 lib tests pass (6 new in `output::streaming::tests`). Mock `RecordingOutput`
covers commit, unicode counting, empty-segment no-op, partial replacement,
finalize-clears-partial, and zero-char rewind.

## What's next

- **Commit 3** — Gemini Live backend via `tokio-tungstenite`. Add dependency, implement `StreamingTranscriber`. Daemon wiring — only when `[transcribe] streaming = true`. Largest commit; will likely need its own session.
- **Commit 4** — one local streaming backend. First investigate `whisper-rs` streaming; fall back to chunked-VAD over the existing `WhisperTranscriber` if it doesn't expose stream APIs.
- **Commit 5** — docs (USER_MANUAL.md, CONFIGURATION.md, TROUBLESHOOTING.md).

## Permission status

Resolved. `git -C <wt> ...` and `cd <wt> && ...` forms both work. `cargo check`, `cargo test`, and explicit-file `rustfmt` invocations work. Note: `cargo fmt -- path/to/file.rs` in workspace mode reformats the whole crate, not just the named files — pre-existing fmt drift in unmodified files leaks in. Workaround: format the new files in isolation, then `git checkout` any unrelated file the workspace fmt touched.
