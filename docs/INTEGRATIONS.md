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

## Meeting mode

### Checking meeting status alongside dictation status

`voxtype status --format json --extended` appends `meeting_active`,
`meeting_status`, and (only when a meeting is active) `meeting_id` after the
existing `model`/`device`/`backend` fields:

```json
{"text": "R", "alt": "recording", "class": "recording", "tooltip": "...", "model": "base.en", "device": "default", "backend": "CPU (AVX2)", "meeting_active": true, "meeting_status": "recording", "meeting_id": "b6b2e5b2-..."}
```

`meeting_status` is one of `idle`, `recording`, `paused`, or `finalizing`.
`finalizing` covers the window between `meeting stop` and the meeting actually
disappearing: the daemon is still transcribing the last partial chunk, which
takes real wall-clock time. Treat it as "still running, about to end," not as
"stopped." These fields are omitted entirely without `--extended`, and
`meeting_active`/`meeting_status` never appear without a `meeting_id` when a
meeting is running.

### `--json` on read-only meeting commands

`meeting status`, `meeting show <id>`, and `meeting list` accept `--json`:

```sh
voxtype meeting status --json
# {"active": true, "status": "recording", "meeting_id": "b6b2e5b2-..."}

voxtype meeting show latest --json     # the full MeetingData (metadata + transcript)
voxtype meeting list --json            # an array of MeetingMetadata
```

`show`/`list` JSON is the same serde shape written to `metadata.json` and
`transcript.json` on disk (snake_case), not the camelCase `meeting export -f
json` emits — the export format predates this and is a separate, deliberately
different schema for downstream tooling that already consumes it.

### Live transcript and status: `meeting.sock`

While a meeting is active (and even when one isn't — the socket is always
bound), the daemon publishes meeting lifecycle and transcript events as
newline-delimited JSON (NDJSON) over a Unix socket at
`$XDG_RUNTIME_DIR/voxtype/meeting.sock`. This is how the meeting HUD gets a
live transcript without polling `voxtype meeting show`.

Connect and read lines; the first line is always a `hello` snapshot so a late
subscriber is correct immediately, with no separate read of `transcript.json`
needed:

```json
{"event": "hello", "status": "recording", "meeting_id": "...", "title": "Standup", "started_at": "2026-09-03T14:00:00Z", "chunk_count": 7, "diarization": {"enabled": true, "backend": "simple"}, "segments": [ /* TranscriptSegment[], same shape as transcript.json */ ]}
{"event": "segments", "chunk_id": 8, "segments": [ /* TranscriptSegment[] for this chunk only */ ]}
{"event": "speaker_renamed", "from": "Remote", "to": "Remote 1"}
{"event": "segment_updated", "segment": { /* one TranscriptSegment, same shape as in segments */ }}
{"event": "notes_updated", "meeting_id": "...", "updated_at": "2026-11-11T20:00:00Z"}
{"event": "status", "status": "paused"}
{"event": "error", "message": "..."}
```

Delivery is best-effort: a subscriber that falls behind is disconnected
rather than allowed to back-pressure the daemon, the same trade-off the audio
level socket (`audio.sock`) makes. Reconnect and read a fresh `hello` if that
happens.

Each segment carries an `id` unique within the meeting, a `start_ms` of its
own (one segment per utterance, not per chunk), and `speaker_id` plus
`speaker_label` once a name has been given with `voxtype meeting label`. The
`hello` backlog is in order of `start_ms`. A `segments` event covers one chunk
of one track, so the microphone and loopback events for the same stretch of
time arrive one after the other: insert by `start_ms` rather than appending.
Microphone echo of the remote side is removed before an event is sent.

`speaker_renamed` means every segment already received with `speaker_id`
equal to `from` now has `to`. Hybrid diarization sends it when a second
remote voice is heard: the remote side, `Remote` until then, becomes
`Remote 1`, and the new voice arrives as `Remote 2`. The event comes before
the `segments` event that holds the new voice, a name given to the old ID
moves to the new one, and the next `hello` already carries the new ID.

`segment_updated` replaces one segment already received, matched by `id`,
with the copy it carries. It follows `voxtype meeting reassign` (or
`--reset`): the segment's `speaker_id` and `speaker_label` are the speaker
the user chose, and `original_speaker` holds the speaker the diarizer had
given it (absent after a reset). A moved line is never moved back by a later
chunk, and a `speaker_renamed` applies to it like any other: rename by
`speaker_id`, and rename `original_speaker` too if you keep it. The next
`hello` already carries the change.

A segment the user annotated (`voxtype meeting annotate`) carries
`"annotation": {"text": "...", "highlight": true}`; the field is absent when
there is none, and `text` is empty for a bare highlight. Adding, changing or
removing an annotation during a meeting sends `segment_updated`.

`notes_updated` means the meeting's notes (`notes.md`) changed. It carries
no text: read it with `voxtype meeting notes <meeting_id> --json`, which
prints `{"meeting_id": "...", "notes": "...", "updated_at": "..."}`
(`updated_at` is null when there are no notes).

Events fire roughly once per `chunk_duration_secs` (default 30s), not per
word — meeting mode transcribes in chunks, it does not stream. A UI built on
this socket should show a "listening…" affordance between updates, not
imitate word-by-word streaming it cannot actually deliver.

`speakers` (live speaker naming) and `summary` (auto-generated meeting
summary) events are planned for a later release and are not emitted yet.

### The meeting folder and `meeting.json`

Every meeting has a folder under the meeting storage path
(`~/.local/share/voxtype/meetings/<date>-<title>/` by default), and every
folder has a `meeting.json` manifest describing the meeting and the files
beside it. Read the manifest rather than guessing at file names or at
`metadata.json`, which is voxtype's internal record and may change.

```json
{
  "manifest_version": 1,
  "id": "b6b2e5b2-4c1e-4a57-9a0e-2f6f3c1d8e41",
  "title": "Weekly standup",
  "url": null,
  "status": "completed",
  "started_at": "2026-11-11T15:00:04Z",
  "ended_at": "2026-11-11T15:31:12Z",
  "duration_secs": 1868,
  "engine": "parakeet",
  "model": "parakeet-tdt-0.6b-v3",
  "diarization": "hybrid",
  "speakers": [
    { "id": "Remote 1", "name": "Dana" },
    { "id": "Remote 2", "name": "Remote 2" },
    { "id": "You", "name": "Pete" }
  ],
  "audio_retained": true,
  "files": [
    { "name": "transcript.json", "kind": "transcript", "bytes": 48211 },
    { "name": "transcript.md", "kind": "transcript_markdown", "bytes": 21980 },
    { "name": "notes.md", "kind": "notes", "bytes": 412 },
    { "name": "summary.md", "kind": "summary", "bytes": 1893 },
    { "name": "mic.opus", "kind": "audio_mic", "bytes": 4981233 },
    { "name": "loopback.opus", "kind": "audio_loopback", "bytes": 5310877 },
    { "name": "mixed.opus", "kind": "audio_mixed", "bytes": 5402115 }
  ],
  "voxtype_version": "1.3.0"
}
```

This is a stable contract:

- Every field above is always present. A value that is unknown or does not
  apply is `null` (`title`, `url`, `ended_at` and `duration_secs` while the
  meeting records, `engine`, `model` and `diarization` for meetings recorded
  before voxtype kept them, `diarization` when it was off).
- New fields may be added. Ignore fields you do not know.
- `manifest_version` changes only if an existing field changes meaning.
- Keys are written in the order shown, pretty-printed, with a trailing
  newline. The file is replaced atomically, so a reader never sees half of
  it.

The fields:

- `status`: `active`, `paused`, `completed` or `cancelled`. A meeting left
  `active` by a crash becomes `completed` the next time the daemon starts.
- `started_at`, `ended_at`: RFC 3339, UTC. `duration_secs` is wall-clock
  time from start to stop.
- `engine`, `model`: the transcription engine and its model.
  `voxtype_version` is the version that recorded the meeting.
- `speakers`: each speaker in the transcript, sorted by `id`. `id` is the
  speaker as the diarizer named it (the `speaker_id` in `transcript.json`, or
  `You`/`Remote` without diarization); `name` is the name given with
  `voxtype meeting label`, or `id` when there is none. Two speakers given the
  same name keep separate entries.
- `files`: the meeting's files that exist, in the order shown, with their
  size in bytes when the manifest was written. `kind` is one of
  `transcript`, `transcript_markdown`, `notes`, `summary`, `audio_mic`,
  `audio_loopback` or `audio_mixed`. Names are relative to the folder.
  Audio files end in `.opus` (Ogg Opus, 16 kHz mono) or, when ffmpeg was
  missing, `.wav` (16-bit PCM, 16 kHz mono); match on `kind`, not on the
  name.
- `audio_retained`: whether the meeting kept its audio (`retain_audio`).

The manifest is written when a meeting starts and rewritten when it stops,
when its notes or summary change, and when a daemon restart completes a
meeting a crash left open. To act once a meeting is finished, use
`post_meeting_command` (see [Meeting Mode](MEETING_MODE.md#running-a-command-after-a-meeting)),
which runs after the final rewrite and gets the folder and the manifest's
path in `VOXTYPE_MEETING_DIR` and `VOXTYPE_MEETING_MANIFEST`.

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
