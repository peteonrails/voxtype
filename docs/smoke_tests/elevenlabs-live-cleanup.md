# ElevenLabs Live Cleanup

Verify ElevenLabs cleanup, manual commit, partial correction, correction drivers, and failure handling.

## Prerequisites

- Build with the `elevenlabs` feature.
- Load a restricted `ELEVENLABS_API_KEY` through the environment. Do not put it on the command line.
- Accept the ElevenLabs Scribe terms.
- Open a disposable text editor and keep its caret at the end of streamed text.
- Install at least one BackSpace-capable driver: `wtype`, `dotool` or `dotoold` with `dotoolc`, or `ydotool` with its daemon.

```bash
cargo build --features elevenlabs
```

Use a temporary config file or restore your normal config after each test.

## 1. Configuration layering

Start with:

```toml
engine = "elevenlabs"

[elevenlabs]
mode = "partials"
commit_strategy = "vad"
no_verbatim = false
```

Check environment overrides:

```bash
VOXTYPE_ELEVENLABS_COMMIT_STRATEGY=manual \
VOXTYPE_ELEVENLABS_NO_VERBATIM=true \
  target/debug/voxtype config | sed -n '/\[elevenlabs\]/,/^\[/p'
```

Check CLI precedence:

```bash
VOXTYPE_ELEVENLABS_NO_VERBATIM=true \
  target/debug/voxtype --elevenlabs-verbatim config \
  | sed -n '/\[elevenlabs\]/,/^\[/p'

target/debug/voxtype \
  --elevenlabs-commit-strategy manual \
  --elevenlabs-no-verbatim config \
  | sed -n '/\[elevenlabs\]/,/^\[/p'
```

Expected:

- The first output shows `commit_strategy = Manual` and `no_verbatim = true`.
- The inverse CLI flag shows `no_verbatim = false` despite the environment value.
- The final CLI example shows `commit_strategy = Manual` and `no_verbatim = true`.
- `--elevenlabs-no-verbatim --elevenlabs-verbatim` is rejected.
- An unknown commit strategy is rejected by CLI and ignored with a warning when supplied through the environment.

## 2. Backward-compatible VAD behavior

```toml
[elevenlabs]
mode = "partials"
commit_strategy = "vad"
no_verbatim = false
vad_silence_threshold_secs = 0.8
```

Dictate fillers, a false start, and a pause longer than 0.8 seconds.

Expected:

- Filler words and false starts remain when the provider recognizes them.
- The pause can commit a segment.
- Speech after the commit starts a new segment.

## 3. Provider cleanup with VAD

Change only:

```toml
[elevenlabs]
no_verbatim = true
```

Dictate: `um, I, I think we should, uh, continue with the original plan`.

Expected:

- ElevenLabs removes at least one detected filler, false start, or disfluency.
- Cleanup does not depend on `[text] filter_filler_words`.
- The VAD pause behavior from the previous test remains.

Provider decisions vary by audio and language. Repeat with another clear false start if the first phrase is unchanged.

## 4. Manual partial correction across a pause

```toml
[elevenlabs]
mode = "partials"
commit_strategy = "manual"
no_verbatim = true
```

Dictate a clause, pause longer than the earlier VAD threshold, then continue the same sentence.

Expected:

- The short pause does not trigger a VAD commit.
- Provisional text remains visible during the pause.
- A later provider snapshot may retract punctuation or revise the tail.
- When a revision occurs, Voxtype sends BackSpaces and types the corrected suffix without finalizing the segment.
- The model may retain its first punctuation choice. Do not treat the absence of a particular correction as a cursor failure.

## 5. Stop and final drain

Release the hotkey after a visible partial revision.

Expected:

- Voxtype sends one explicit commit and waits for the bounded final response.
- An identical final commits the corrected partial without typing it twice.
- The daemon returns to idle after the final drain.

Repeat with:

```toml
[elevenlabs]
mode = "realtime"
commit_strategy = "manual"
no_verbatim = true
```

Expected: normal cursor output waits until stop because committed-only mode ignores partial snapshots.

## 6. Cleanup independence

Use manual partial mode with:

```toml
[elevenlabs]
no_verbatim = false
```

Expected: provider tail revisions can still occur. Cleanup and commit strategy are independent.

## 7. Correction drivers

Where available, repeat a revision with each setup:

1. `wtype`
2. `dotoold` through `dotoolc`, then direct `dotool`
3. `ydotool` with `ydotoold` running

Expected:

- Each driver removes exactly the revised tail and types the corrected suffix.
- `eitype` and clipboard-only output cannot supply correction BackSpaces.
- With clipboard-only output in `partials` mode, the first partial ends the session with an error that points to `realtime` mode.

## 8. Long-session automatic commit

Keep one manual recording open for more than 36 seconds, then continue speaking.

Expected:

- An ElevenLabs automatic commit behaves like an ordinary segment final.
- The next segment starts with empty provisional state.
- Releasing the hotkey still commits and drains the remaining segment.

## 9. Missing BackSpace driver

Use an isolated test environment where `wtype`, `dotool`, `dotoolc`, and `ydotool` are unavailable, then provoke a partial revision.

Expected:

- Voxtype shows one `Streaming Error` notification saying streaming output stopped.
- The backend is cancelled and the daemon returns to idle.
- Visible text stays in the editor without rewind.
- Later provider events do not type more text.

Restore `PATH` after the test. Do not alter system packages for this check.

## 10. Network loss

During a provisional manual segment, disconnect the test process from the network or briefly disable the test connection.

Expected:

- Voxtype shows one streaming error and returns to idle.
- Visible provisional text remains.
- Voxtype does not claim that the provisional text was finalized.

Restore the network before continuing.

## 11. Focus and caret limitation

During a disposable partial session, move focus or move the caret before a revision.

Expected: correction follows the current focus and caret. This is an unsupported operating condition; Voxtype does not infer or restore the original cursor position.

## 12. Secret and payload inspection

After the tests, inspect the relevant logs without printing the key:

```bash
journalctl --user -u voxtype --since "30 min ago" > /tmp/voxtype-elevenlabs.log
python3 - <<'PY'
import os
from pathlib import Path
log = Path('/tmp/voxtype-elevenlabs.log').read_text(errors='replace')
key = os.environ.get('ELEVENLABS_API_KEY', '')
print('api_key_present=', bool(key and key in log))
print('audio_payload_marker_present=', 'audio_base_64' in log)
PY
rm -f /tmp/voxtype-elevenlabs.log
```

Expected: both checks print `False`. Review logs for captured transcript text before sharing them.
