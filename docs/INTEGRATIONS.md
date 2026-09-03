# Integrating with Voxtype

This document is the contract for programs that drive Voxtype: desktop shells,
bar widgets, and AI agent harnesses. Anything described here is intended to stay
stable; anything not described here is an implementation detail, including the
sentinel files under `$XDG_RUNTIME_DIR/voxtype/`.

## Checking that Voxtype is available

```sh
voxtype status --format json
```

Exit code `0` means a daemon is running and reachable. The JSON is shaped for
status bars (`text`, `alt`, `class`, `tooltip`) and reports one of `idle`,
`recording`, `streaming`, `transcribing`, or `stopped` in `alt`/`class`.

## Dictating into your own program

Voxtype normally types or pastes into whatever window has focus. A program that
wants the text for itself asks for file output instead, and waits for it.

```sh
transcript="${XDG_RUNTIME_DIR:-/tmp}/myapp/dictation.txt"
rm -f "$transcript"

voxtype record start --file="$transcript" --no-auto-submit --no-smart-auto-submit

# … the user speaks; your UI shows its own indicator …

voxtype record stop --wait --json
```

`record stop --wait` blocks until the transcription is final and prints one JSON
object:

```json
{"status": "ok", "text": "the quick brown fox", "chars": 19, "message": null}
```

| `status`  | Exit | Meaning |
|-----------|------|---------|
| `ok`      | 0    | `text` holds the transcript |
| `empty`   | 3    | Nothing to transcribe: no speech detected, or the recording was too short |
| `timeout` | 4    | No outcome within `--timeout` seconds (default 120) |
| `error`   | 1    | Transcription or the file write failed; see `message` |

Without `--json`, an `ok` transcript is printed on stdout and any other outcome
is reported on stderr. The exit code is the same either way.

`--wait` needs to know which transcript to wait on. It uses the `--file` path
the recording was started with, so you normally pass nothing; use
`--wait-file <PATH>` to name it explicitly, and `--timeout <SECS>` to bound the
wait.

### Why not poll the file yourself

Because two cases have no file to poll for. If voice-activity detection finds no
speech, Voxtype deliberately transcribes nothing and writes nothing; if the write
fails, likewise. A poller cannot tell either apart from "still working", so it
waits out its own deadline and reports a timeout that never happened. `--wait`
returns `empty` or `error` in milliseconds instead.

Programs that must poll should watch for `<transcript>.done`, a JSON completion
record written after the transcript itself is complete, and only then. It is
written exactly once per file-mode recording and is consumed by `--wait`.

### Cancelling

```sh
voxtype record cancel
```

Discards the recording. No transcript and no completion record are written, so a
concurrent `--wait` ends at its timeout; cancel your own wait alongside it.

## Per-recording overrides

These apply to a single recording and are consumed by it. They are the supported
way to keep your integration from disturbing the user's own dictation settings.

| Flag | Effect |
|------|--------|
| `--file[=PATH]` | Write the transcript to a file instead of typing it |
| `--no-auto-submit` | Do not press Enter after the transcript |
| `--no-smart-auto-submit` | Ignore a spoken "submit" |
| `--profile NAME` | Apply a `[profiles.NAME]` post-processing command |
| `--model NAME` | Use a specific model for this recording |

Agent harnesses generally want the first three: the agent composes a message and
decides for itself when to send it.

A `[profiles.agent]` block with a `post_process_command` is a good place to
normalise dictation before an agent sees it, stripping filler words for instance.

## A menu for your dictation, before it lands

Dictation is instant, but speech is not text: no punctuation, filler words,
half-finished thoughts. `post_process` can open a small menu over the
transcript — edit it, run actions on it (tidy, translate, reformat — any
shell pipeline), steer a local model with typed or spoken instructions — and
only then commit what you see.

![The voxtype-review menu](https://raw.githubusercontent.com/DimitriGeelen/voxtype-review/master/docs/screenshot.png)

One maintained implementation is
[`voxtype-review`](https://github.com/DimitriGeelen/voxtype-review): the
transcript in an editable box, an extensible action list on number
shortcuts, results shown before they are committed, `Alt+arrow` through
every round, `Esc` aborts: the hook prints nothing and exits 0, and with
`fallback_on_empty = false` nothing lands at all — no paste, not even the
original. (Keep that flag in mind when you build your own menu: it is the
difference between "cancel to the original" and "cancel to nothing".)

The pattern needs nothing from Voxtype but the hook it already has:
`post_process` is a gate, not just a filter — whatever the hook prints on
stdout is what lands, and a hook that is killed at `timeout_ms` — or exits
non-zero — falls back to the raw transcript.

The contract a review hook must honour:

- read the transcript on stdin
- print the final text on stdout, nothing else
- exit `0`; any other exit, or the `timeout_ms` kill, means "use the raw
  transcript" — a hook must never block past the timeout or the dictation
  is silently lost
- the text lands wherever the user was; the hook never needs to know

A 30-second human will routinely exceed the default timeout, so raise it:

```toml
[output.post_process]
command = "/usr/local/bin/my-review-gui"
timeout_ms = 600000     # a human reading their own words, not a model answering
trim = false            # the hook owns whitespace
```

Terminal targets paste with `Ctrl+Shift+V`, not `Ctrl+V`; point
`output.paste_keys` at the chord your targets actually bind, or the paste
silently does nothing in some of them.

One maintained implementation of this pattern is
[`voxtype-review`](https://github.com/DimitriGeelen/voxtype-review): a small
Rust/egui popup that shows the transcript for editing, runs configurable
actions (tidy, translate, bullets — local LLM or HTTP), keeps a per-round
instruction box that accepts spoken input, and steps back through every
round with `Alt+arrow`. `Esc` always emits the raw transcript; `Enter`
commits what the box holds.

## What Voxtype will not do for you

Voxtype does not edit your configuration, and asks that you not edit its own:
`~/.config/voxtype/config.toml` belongs to the user. If you need behaviour that
is only reachable by editing it, that is a missing flag. Please open an issue.

## Note for existing integrations

Not every Voxtype has `record stop --wait`. Probe for it rather than pinning a
version, since the release lines do not carry it at the same time:

```sh
voxtype record stop --help | grep -q -- --wait
```

Older versions need the polling fallback described above.
