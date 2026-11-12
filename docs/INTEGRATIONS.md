# Integrating with Voxtype

This document is the contract for programs that drive Voxtype: desktop shells,
bar widgets, and AI agent harnesses. Anything described here is intended to stay
stable; anything not described here is an implementation detail, including the
files under `$XDG_RUNTIME_DIR/voxtype/` that
[the table below](#files-under-the-runtime-and-state-directories) does not
mark as a contract.

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

Everything a meeting client can read or drive: the meeting's state, its
saved data, the live feed on `meeting.sock`, the commands that change a
transcript, the assistant, playback, the meeting folder, and the files the
meeting view uses. [Meeting Mode](MEETING_MODE.md) describes the same
features for people using them.

Some rules hold throughout:

- A `<meeting>` argument is a meeting ID, a unique start of one (`a32baa9b`),
  or `latest`.
- A segment is named by its `id`, which is unique within the meeting and
  never reused. `meeting show --json` lists them.
- JSON from `meeting` commands and from `meeting.sock` uses the same
  snake_case shapes as the files on disk. `meeting export --format json` is
  the exception: it predates this contract and keeps its own camelCase
  schema for the tools that already read it.
- Fields may be added. Ignore the ones you do not know. A field documented
  as absent when it does not apply is left out, not set to `null`, unless
  the table says otherwise.
- The commands that change a transcript work during a meeting and after it.
  After a meeting they rewrite its files directly; during one they hand the
  change to the daemon, which applies it before the next chunk and announces
  it on `meeting.sock`.

The examples below were produced by voxtype against one meeting, so their
IDs and times agree with each other.

### Meeting status

```sh
voxtype meeting status --json
```

```json
{
  "active": true,
  "echo_cancel": "webrtc",
  "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed",
  "status": "recording"
}
```

`echo_cancel` is the echo handling the running meeting uses: `webrtc`
(PipeWire's WebRTC echo canceller), `gtcrn` (the speech enhancer) or `none`.
It is `null` when no meeting is running. The meeting socket's `hello` carries
the same `echo_cancel` once the meeting has started.

`status` is one of `idle`, `starting`, `recording`, `pausing`, `paused`,
`resuming`, `stopping` or `finalizing`. `starting`, `pausing`, `resuming` and
`stopping` mean the command was issued and the daemon has not acted on it
yet: it acts between audio chunks, which can take a few seconds.
`finalizing` is the time between `meeting stop` and the meeting ending, while
the last partial chunk is transcribed. Treat it as "still running, about to
end". With no meeting, `active` is `false` and `meeting_id` is `null`.

A status bar that already reads `voxtype status --format json --extended`
gets the meeting there too, after the dictation fields:

```json
{"text": "", "alt": "idle", "class": "idle", "tooltip": "...", "model": "base.en", "device": "default", "backend": "CPU (AVX2)", "model_upgrade": null, "osd_frontend_active": "none", "meeting_active": true, "meeting_status": "recording", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed"}
```

`meeting_status` there is `idle`, `recording`, `paused` or `finalizing`,
without the pending states. `meeting_id` is present only while a meeting is
in progress. None of the three fields appear without `--extended`.

### Listing and reading meetings

```sh
voxtype meeting list --json [--limit N]
```

An array, newest first (default limit 10). Each entry is the meeting's
metadata plus two fields read from the meeting index, so a long list stays
fast:

```json
[
  {
    "id": "a32baa9b-b9df-4051-8830-b422d21495ed",
    "title": "Release sync",
    "started_at": "2026-11-11T15:00:04Z",
    "ended_at": "2026-11-11T15:31:12Z",
    "duration_secs": 1868,
    "status": "completed",
    "chunk_count": 63,
    "storage_path": "/home/me/.local/share/voxtype/meetings/2026-11-11-release-sync",
    "audio_retained": true,
    "url": "https://meet.google.com/abc-defg-hij",
    "has_summary": true,
    "speaker_count": 3
  }
]
```

- `status` is `active`, `paused`, `completed`, `cancelled` or `interrupted`
  (the daemon stopped while it recorded; see
  [Meetings that never finished](#meetings-that-never-finished-meeting-recover)).
- `url` is the call's link from `meeting start --url`, absent otherwise.
- `has_summary`: a summary is saved (`meeting summarize`, or the summary at
  stop).
- `speaker_count`: distinct speakers as the transcript shows them, with names
  and moved lines applied. Speakers given the same name count once; a
  speaker whose only lines are empty does not count. It is refreshed whenever
  the transcript is saved. A meeting with no transcript yet has 0.

```sh
voxtype meeting show <meeting> --json
```

One object: `metadata` (as in `list`, plus `summary` once there is one),
`transcript` (`segments` and `total_chunks`), `notes` (present when there
are notes) and `asks` (the saved questions and answers, see
[Asking a meeting](#asking-a-meeting-meeting-ask---json)). Shortened:

```json
{
  "metadata": {
    "id": "a32baa9b-b9df-4051-8830-b422d21495ed",
    "title": "Release sync",
    "status": "completed",
    "summary": {"summary": "Release sync for 1.2.1. ...", "key_points": ["..."], "decisions": ["..."], "action_items": [{"description": "Draft the release notes", "assignee": "Dana", "completed": false}], "chapters": [{"start_ms": 1000, "title": "Build status"}], "generated_at": "2026-11-11T15:33:06.967Z", "model": "claude"}
  },
  "notes": "Tag at 10 tomorrow.\n",
  "transcript": {
    "segments": [
      {"id": 0, "start_ms": 1410, "end_ms": 3090, "text": "Okay, let's start the release sync.", "source": "microphone", "speaker_id": "You", "chunk_id": 0},
      {"id": 4, "start_ms": 23040, "end_ms": 30000, "text": "Only one. The restart banner stays visible after the daemon restarts.", "source": "loopback", "speaker_id": "Remote 1", "speaker_label": "Dana", "annotation": {"text": "Known issue for the notes", "highlight": true}, "chunk_id": 0}
    ],
    "total_chunks": 2
  },
  "asks": [
    {"id": "8b1fba46-aa7b-430c-b3da-6dbd515b6cd3", "time": "2026-11-11T15:33:06.959Z", "question": "What blockers came up?", "answer": "Dana found one blocker: ... [00:23].", "agent": "claude", "agent_name": "Claude", "citations": [{"ts": "00:23", "ms": 23000}], "last_secs": 300}
  ]
}
```

`show` keeps reading the transcript while a meeting records, so it is
current to the last chunk.

#### Segments

`meeting show`, `meeting.sock` and the commands below all carry segments in
the same shape:

| Field | Meaning |
|-------|---------|
| `id` | Unique within the meeting |
| `start_ms`, `end_ms` | From the start of the meeting. One segment per utterance, so each line has its own start |
| `text` | What the line says, with any correction applied |
| `source` | `microphone` or `loopback` |
| `speaker_id` | The speaker: `You`, `Remote`, `Remote 1`, `SPEAKER_00`, or `Person 1` for a speaker created by `meeting reassign` |
| `speaker_label` | The name given with `meeting label` or `meeting reassign`; absent when there is none |
| `original_speaker` | Present on a moved line: the speaker the diarizer gave it |
| `original_text` | Present on a corrected line: what was transcribed |
| `annotation` | Present on an annotated line: `{"text": "...", "highlight": bool}`, `text` empty for a bare highlight |
| `deleted` | `true` on a hidden line. Only `transcript.json` and the `delete-line` command show hidden lines; skip them |
| `chunk_id` | The audio chunk the line came from |

Show a line as `speaker_label`, falling back to `speaker_id`. Two speakers
with the same `speaker_label` are one person to the user. Microphone echo of
the remote side is removed before segments are saved or sent.

With `[meeting] my_name` set, every `You` line without a label of its own
carries `my_name` as its `speaker_label` in `show`, exports and
`meeting.sock`, while `speaker_id` stays `You`. Only `transcript.json` itself
leaves it out. `voxtype meeting my-name --json` prints
`{"my_name": "Pete" | null, "suggestion": "Pete Jackson" | null}`, the
suggestion being the account's full name, offered only while `my_name` is
unset. Voxtype never sets it; a view may offer the suggestion and run
`voxtype config set meeting.my_name "<name>"` when the user accepts.

### Searching meetings: `meeting search --json`

```sh
voxtype meeting search <words...> [--limit N] [--speaker NAME] [--since DATE] --json
```

Full-text search over transcript lines as displayed (labels, moved lines and
corrections applied, deleted lines left out), titles, notes and summaries.
Every word must appear in the same line; `"a phrase"` stays together and
`word*` matches a prefix. Matching ignores case and accents. Best matches
come first.

```json
{
  "query": "deploy",
  "speaker": null,
  "since": null,
  "results": [
    {
      "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed",
      "title": "Release sync",
      "date": "2026-11-11T15:00:04Z",
      "kind": "segment",
      "segment_id": 4,
      "start_ms": 23040,
      "speaker": "Dana",
      "snippet": "Can we ship the «deploy» tool first?",
      "snippet_plain": "Can we ship the deploy tool first?",
      "highlights": [[16, 22]],
      "rank": -1.73
    }
  ]
}
```

| Field | Meaning |
|-------|---------|
| `meeting_id`, `title`, `date` | The meeting. `title` falls back to the date-based name; `date` is its start, RFC 3339 UTC |
| `kind` | `segment` (a transcript line), `title`, `notes` or `summary` |
| `segment_id`, `start_ms` | The line, as in `meeting show`; `null` unless `kind` is `segment`. Open the meeting at `start_ms`, or `meeting play <meeting> --segment <segment_id>` |
| `speaker` | The line's speaker as displayed, `my_name` applied; `null` unless `kind` is `segment` |
| `snippet` | The matching text around the match, each match wrapped in `«` and `»`. Long text is cut with `…` |
| `snippet_plain`, `highlights` | The same text without the marks, and each match as `[start, end)` offsets into it, in Unicode characters (code points) |
| `rank` | SQLite FTS5 bm25 score: lower is a better match. Results are already sorted |

`--speaker` matches the displayed speaker ignoring case, and `my_name` also
matches unlabelled `You` lines. `--since` takes `2026-11-01` (local
midnight) or an RFC 3339 time. A query with no words exits non-zero with a
message on stderr. The first search after an upgrade indexes older meetings.

### The live feed: `meeting.sock`

The daemon publishes meeting events as newline-delimited JSON on a Unix
socket at `$XDG_RUNTIME_DIR/voxtype/meeting.sock`, so a view gets the live
transcript without polling. The socket is bound whenever the daemon runs,
with or without a meeting. It is one-way: connect and read lines.

```sh
socat - UNIX-CONNECT:"$XDG_RUNTIME_DIR/voxtype/meeting.sock"
```

`voxtype meeting watch --json` relays the socket to stdout line for line
(every event but `levels`; add `--levels` for those too, or use `--levels`
alone for only them),
waiting for the daemon if it is not running and reconnecting after it
restarts, so a script or a QML `Process` can follow a meeting without a
socket client of its own:

```sh
voxtype meeting watch --json | jq -c 'select(.event == "segments") | .segments[].text'
```

The first line on every connection is a `hello` snapshot, so a client that
connects mid-meeting is correct at once. Delivery is best effort: a client
that falls behind is disconnected rather than allowed to slow the daemon.
Reconnect and read a fresh `hello` when that happens, or when the daemon
restarts.

Transcript events arrive about once per `chunk_duration_secs` (30 seconds by
default), not per word: meetings are transcribed in chunks, not streamed.
Show a "listening" state between updates. Only `levels` is frequent, about
ten a second while a meeting records.

| Event | When |
|-------|------|
| `hello` | First line of every connection |
| `segments` | A chunk of one track was transcribed |
| `speaker_renamed` | Every line of one speaker moved to another ID |
| `segment_updated` | One line changed: moved, corrected, annotated, or restored |
| `segment_deleted` | One line was hidden |
| `notes_updated` | The meeting's notes changed |
| `title_changed` | The meeting was retitled, by hand or from the calendar |
| `status` | The meeting's state changed, or a summary started |
| `summary_ready` | The summary at stop is saved |
| `playback` | `meeting play` started or stopped |
| `levels` | About ten times a second while recording or sound-checking: each track's level |
| `soundcheck` | A sound check started or stopped |
| `import_progress` | `meeting import` or `meeting retranscribe` finished a chunk |
| `recoverable` | Meetings the daemon never finished: after `hello` while there are any, and when one is finished or discarded |
| `error` | Something failed |

#### `hello`

```json
{"event": "hello", "status": "recording", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "title": "Release sync", "started_at": "2026-11-11T15:00:04Z", "chunk_count": 1, "diarization": {"enabled": true, "backend": "hybrid"}, "echo_cancel": "webrtc", "recording_notice": false, "segments": [{"id": 0, "start_ms": 1410, "end_ms": 3090, "text": "Okay, let's start the release sync.", "source": "microphone", "speaker_id": "You", "chunk_id": 0}, {"id": 1, "start_ms": 3750, "end_ms": 8310, "text": "The build for 1.2.1 passed on every variant last night.", "source": "microphone", "speaker_id": "You", "chunk_id": 0}]}
```

`status` is `idle`, `recording`, `paused` or `finalizing`. With no meeting it
is `idle`, `meeting_id`, `title` and `started_at` are absent, and
`segments` is empty.

`recording_notice` is `true` while the user has not acknowledged the
recording-consent notice ([Meeting Mode](MEETING_MODE.md#recording-consent)).
A view should show the notice on a running meeting while it is `true`, with
a way to acknowledge it that runs `voxtype meeting notice --ack`. Every
`status` event carries the same field, so an acknowledgement made anywhere
reaches every view at once: the daemon repeats the current `status` with
`recording_notice: false` when it sees one. An older daemon sends no
`recording_notice`; read that as `false`.

`segments` is every line so far, in order of
`start_ms`, with every change below already applied and hidden lines left
out.

#### `segments`

```json
{"event": "segments", "chunk_id": 1, "segments": [{"id": 2, "start_ms": 10320, "end_ms": 16290, "text": "Great! I tested the Omarchy package this morning.", "source": "loopback", "speaker_id": "Remote 1", "speaker_label": "Dana", "chunk_id": 0}, {"id": 3, "start_ms": 18150, "end_ms": 21060, "text": "Good. Are there any blockers left before we tag it?", "source": "microphone", "speaker_id": "You", "chunk_id": 0}]}
```

One event covers one chunk of one track, so the microphone and loopback
events for the same stretch of time arrive one after the other. Insert each
segment by `start_ms` rather than appending.

#### `speaker_renamed`

```json
{"event": "speaker_renamed", "from": "Remote", "to": "Remote 1"}
```

Every segment already received with `speaker_id` equal to `from` now has
`to`; rename `original_speaker` too if you keep it. Hybrid diarization sends
this when a second remote voice is heard: `Remote` becomes `Remote 1` and the
new voice arrives as `Remote 2`. It comes before the `segments` event that
holds the new voice, a name given to `Remote` moves to `Remote 1`, and the
next `hello` already has the new ID.

#### `segment_updated`

```json
{"event": "segment_updated", "segment": {"id": 4, "start_ms": 23040, "end_ms": 30000, "text": "Only one. The restart banner stays visible after the daemon restarts.", "source": "loopback", "speaker_id": "Remote 1", "speaker_label": "Dana", "annotation": {"text": "Known issue for the notes", "highlight": true}, "chunk_id": 0}}
```

Replace the segment with the same `id` by this copy. It follows
`meeting reassign` (and `--reset`), `meeting edit` (and `--reset`),
`meeting annotate` (and `--remove`), and `meeting delete-line --restore`. A
restored line is one you do not have: put it back in order of `start_ms`. A
moved line is never moved back by a later chunk.

`meeting label` during a meeting sends one for every line already sent under
that speaker, now carrying `speaker_label`, and the `hello` backlog has the
name from then on. Lines transcribed later carry it in `segments`.

#### `segment_deleted`

```json
{"event": "segment_deleted", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "segment_id": 6}
```

Drop the segment. It follows `meeting delete-line`.

#### `notes_updated`

```json
{"event": "notes_updated", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "updated_at": "2026-11-11T15:33:07.023Z"}
```

The notes changed. The event carries no text: read them with
`voxtype meeting notes <meeting_id> --json`.

#### `title_changed`

```json
{"event": "title_changed", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "title": "Pete <> Chad Bi-Weekly"}
```

Show the new title. It follows `voxtype meeting title`, or a title taken from the calendar just after the meeting started. The `hello` snapshot carries the new title from then on. Lines whose speakers the new title names follow as `segment_updated`.

#### `status`

```json
{"event": "status", "status": "paused", "recording_notice": false}
{"event": "status", "status": "recording", "recording_notice": true}
{"event": "status", "status": "finalizing", "recording_notice": false}
{"event": "status", "status": "idle", "recording_notice": false}
{"event": "status", "status": "summarizing", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "recording_notice": false}
```

`recording_notice` is as in `hello`. A `status` repeating the current state
with a changed `recording_notice` is the daemon passing on an
acknowledgement; nothing else changed.

A meeting goes `recording` (when it starts and on resume), `paused`,
`finalizing` after `meeting stop`, and `idle` once it has ended and its
transcript is saved. After `idle`, the next `hello` is the idle snapshot.

`summarizing` is different: it is about a meeting that has already ended,
names it in `meeting_id`, and does not change the `hello` snapshot. When the
summary at stop is on (see [Meeting Mode](MEETING_MODE.md#ai-summarization)),
the daemon summarizes in the background after the transcript is saved, and
by then it may be `idle` or recording the next meeting. It ends with
`summary_ready`, or with an `error` that names the same meeting. A meeting
with the summary at stop off, or with no transcript, sends neither.

#### `summary_ready`

```json
{"event": "summary_ready", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "agent": "claude", "agent_name": "Claude", "path": "/home/me/.local/share/voxtype/meetings/2026-11-11-release-sync/summary.md", "summary": {"summary": "Release sync for 1.2.1. The build passed on every variant; one known issue goes in the release notes.", "key_points": ["The Omarchy package and settings panel work"], "action_items": [{"description": "Draft the release notes", "assignee": "Dana", "completed": false}, {"description": "Tag the release", "assignee": "You", "completed": false}], "decisions": ["Ship with the restart banner as a known issue", "Tag tomorrow at 10"], "chapters": [{"start_ms": 1000, "title": "Build status"}, {"start_ms": 23000, "title": "Known issue"}], "generated_at": "2026-11-11T15:33:06.967Z", "model": "claude"}}
```

`agent` is `claude`, `codex`, `pi`, `local:<model>`, or `ollama` / `remote`
for the older summarizers; `agent_name` is how to show it. `path` is absent
for a meeting without a folder. The same summary is in
`meeting show --json` under `metadata.summary` and in the folder's
`summary.md`.

#### `playback`

```json
{"event": "playback", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "track": "mixed", "state": "playing", "from_ms": 23040}
```

Follows `meeting play`: `state` is `playing` when a playback starts and
`stopped` when it is stopped or reaches the end. It describes the playback
when the daemon sends it, so changes in quick succession can arrive as one
event with the latest state. It is sent whether or not a meeting is
recording, and not at all while the daemon is down; ask
`voxtype meeting play --status --json` when you need certainty.

#### `levels`

```json
{"event": "levels", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "t_ms": 61200, "mic": -23.4, "mic_peak": -15.1, "loopback": -41.0, "loopback_peak": -33.2}
```

For a level meter per track. While a meeting records and is not paused,
the daemon sends one every 100 ms with the level of the audio that arrived
in those 100 ms. It reads them from the audio the meeting already captures,
and it sends them only while at least one client is connected.

| Field | Meaning |
|-------|---------|
| `mic`, `loopback` | RMS level of the window in dBFS, rounded to 0.1 dB: 0 is full scale, `-100` is silence. A speaking voice is typically -40 to -15 |
| `mic_peak`, `loopback_peak` | The loudest sample of the window, in dBFS |
| `t_ms` | Where the window ends on the meeting's timeline: microphone audio recorded so far, paused time left out, like `start_ms` on segments. During a sound check, time since it started |
| `meeting_id` | The meeting measured; `null` during a sound check |
| `soundcheck` | `true` during a sound check; absent otherwise |

A track field is `null` when the meeting records no loopback, or when no
audio from that track arrived in the window (for example right after a
start). There are no `levels` while paused, finalizing or idle, unless a
sound check runs, and they are not part of `hello`; a meter should fall to
silence when they stop.
They are best effort like every event: a client too slow for ten lines a
second is disconnected, so read them promptly or use `--levels` from
`voxtype meeting watch`, which reconnects.

To drive a 0 to 1 meter, map dBFS linearly from -60 to 0:
`max(0, min(1, (db + 60) / 60))`.

```sh
voxtype meeting watch --levels | jq -c '[.t_ms, .mic, .loopback]'
```

#### `soundcheck`

```json
{"event": "soundcheck", "state": "running", "seconds": 30, "echo_cancel": "webrtc", "mic": "voxtype_meeting_aec_source", "loopback": true}
{"event": "soundcheck", "state": "stopped", "reason": "done"}
{"event": "soundcheck", "state": "stopped", "reason": "error", "message": "Audio connection failed: ..."}
```

`voxtype meeting soundcheck` (below) asks the daemon to open the meeting's
microphone and loopback exactly as a meeting would, record and transcribe
nothing, and send `levels` with `"soundcheck": true` until it stops.

- `running` says what the check, and so the next meeting, uses:
  `echo_cancel` (`webrtc`, `gtcrn` or `none`, as `hello` and
  `meeting status --json` report for a meeting), `mic` (the device, or the
  canceller's source), and whether `loopback` is captured. It is sent again,
  with the new `seconds`, when a check is extended.
- `stopped` gives a `reason`: `done` (its time ran out), `stopped` (asked to),
  `meeting` (a meeting needs the microphone; starting one ends the check) or
  `error`, with a `message`, when the audio could not be opened. A request
  made while a meeting records gets `stopped` with reason `meeting` at once.

#### `import_progress`

```json
{"event": "import_progress", "meeting_id": "...", "operation": "import", "state": "transcribing", "chunks_done": 2, "chunks_total": 5, "position_ms": 60000, "duration_ms": 150000}
```

Follows `voxtype meeting import` and `meeting retranscribe`, which run
outside the daemon and hand it their progress: see
[Importing and transcribing again](#importing-and-transcribing-again-meeting-import-and-meeting-retranscribe)
below. It is not sent while the daemon is down.

#### `recoverable`

```json
{"event": "recoverable", "meetings": [{"id": "a32baa9b-b9df-4051-8830-b422d21495ed", "title": "Release sync", "started_at": "2026-11-11T15:00:04Z", "last_chunk_ms": 1260000, "has_audio": true, "untranscribed_chunks": 1, "lines": 84, "stopped_at": "2026-11-11T15:21:40Z", "duration_secs": 1296}]}
{"event": "recoverable", "meetings": []}
```

Meetings the daemon never finished because it crashed or was killed while
they recorded, newest first, for a card that offers to finish or discard
them. A connection gets it right after `hello` while the list is not empty,
and every connection gets it again, possibly empty, after
`meeting recover --finish` or `--discard`. `title` is absent for an untitled
meeting.

- `last_chunk_ms`: where the transcript ends on the meeting's timeline (the
  end of the last chunk transcribed).
- `has_audio`: whether retained audio is in the folder.
- `untranscribed_chunks`: chunks of that audio past `last_chunk_ms`, which
  the daemon kept but never transcribed; `--finish` transcribes them.
  Always 0 without retained audio.
- `lines`: transcript lines on disk.
- `stopped_at`: when the meeting last wrote to its folder, about when the
  daemon stopped; `duration_secs` runs from `started_at` to it.

Act on an entry with `voxtype meeting recover` (below).

#### `error`

```json
{"event": "error", "message": "Failed to start meeting: no microphone"}
{"event": "error", "message": "Summary failed: Ollama (llama3:latest at localhost:11434) returned an error: mkdir /opt/models/manifests-v2: permission denied", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "code": "local_failed", "backend": "Local (llama3:latest)"}
```

`message` is for people, and names the backend that failed and its reason.
An error about the summary at stop also has `meeting_id`, a `code` from the
[assistant error codes](#assistant-error-codes), and `backend`, the name of
the backend that failed, as `summary_backend_name` gives it.

### Meetings that never finished: `meeting recover`

```sh
voxtype meeting recover [--json]
voxtype meeting recover --finish <id> [--json]
voxtype meeting recover --discard <id> [--json]
```

Without options it lists the meetings the daemon never finished; with
`--json`, as `{"meetings": [...]}` with the entries of the
[`recoverable`](#recoverable) event. It works with the daemon down too, and
then also lists meetings still marked `active`, which the next daemon start
would mark `interrupted`.

`--finish` completes one from its folder. First it transcribes the
`untranscribed_chunks`, with the engine and model the meeting recorded with
(the configured ones if this build cannot run those), and adds their lines
after the transcript's; then `ended_at` becomes `stopped_at`,
`transcript.md` is rewritten and the status becomes `completed`; then the
summary runs under the usual `[meeting.summary]` rules and
`post_meeting_command` runs, as after `meeting stop`. It waits for all of
it, so it can take as long as the transcription and the summary do. If the
leftover audio cannot be transcribed (no ffmpeg, no model), it says why and
finishes with the transcript as it is.

With `--json` it prints the meeting's metadata, as `meeting show --json`
has it under `metadata` (with `summary` once written), plus `recovered`:

```json
{"id": "a32baa9b-b9df-4051-8830-b422d21495ed", "title": "Release sync", "started_at": "2026-11-11T15:00:04Z", "ended_at": "2026-11-11T15:21:40Z", "duration_secs": 1296, "status": "completed", "chunk_count": 86, "storage_path": "/home/me/.local/share/voxtype/meetings/2026-11-11-release-sync", "audio_retained": true, "recovered": {"transcribed_chunks": 1, "summary": {"state": "written", "agent": "claude", "path": "/home/me/.local/share/voxtype/meetings/2026-11-11-release-sync/summary.md"}, "post_meeting_command": "succeeded"}}
```

`recovered.transcribed_chunks` counts the chunks transcribed from the
leftover audio (a trailing fraction of a second counts as one).
`recovered.summary.state` is `written`, `skipped` (auto-summary off, no
consent, or no transcript) or `failed`, with `message` and `backend`.
`recovered.post_meeting_command` is `null` when none is configured,
otherwise `succeeded`, `failed (exit N)`, `timed out` or
`could not start: ...`.

`--discard` deletes the meeting and its folder and prints
`{"discarded": "<id>"}`. Both take a full ID or a unique prefix, exit 1 for
a meeting that is not interrupted (finished, or recording now), and have the
daemon send `recoverable` again.

### The recording-consent notice: `meeting notice`

```sh
voxtype meeting notice --json
voxtype meeting notice --ack [--json]
```

```json
{"notice": "Laws on recording and transcribing conversations differ by place, and some require everyone on the call to agree. Voxtype doesn't tell the other people for you, so let them know you're using a meeting assistant.", "acknowledged": false, "acknowledged_at": null}
```

`notice` is the text to show, word for word. `--ack` records the
acknowledgement (the first time is kept in `acknowledged_at`, RFC 3339 UTC)
in `$XDG_STATE_HOME/voxtype/recording-notice.json` and tells a running
daemon, which repeats its current `status` with `recording_notice: false`.
Neither form prompts or blocks. `meeting start` prints the notice on stderr,
prefixed `Note:`, while it is unacknowledged; its stdout is unchanged.

### Changing a transcript

Each of these prints the segment it changed with `--json`, in the
[segment shape](#segments). During a meeting the daemon also sends
`segment_updated` (or `segment_deleted`) on `meeting.sock`.

```sh
voxtype meeting reassign <meeting> <segment> <speaker> --json
voxtype meeting reassign <meeting> <segment> --reset --json
```

```json
{
  "id": 6,
  "start_ms": 37680,
  "end_ms": 40590,
  "text": "I'll draft the notes today and send them to you by five.",
  "source": "loopback",
  "speaker_id": "Remote 1",
  "speaker_label": "Dana",
  "original_speaker": "Remote 2",
  "chunk_id": 0
}
```

The speaker is matched like `meeting label` matches it: an ID from the
transcript (`Remote 1`, `SPEAKER_00` or just `0`) or a name already given,
ignoring case. Here `Dana` was the name of `Remote 1`. A name that matches
no one creates a speaker with an ID of the form `Person 1` and that name as
`speaker_label`. `--reset` returns the line to `original_speaker` and drops
that field.

```sh
voxtype meeting edit <meeting> <segment> "<text>" --json
voxtype meeting edit <meeting> <segment> --reset --json
```

```json
{
  "id": 2,
  "start_ms": 10320,
  "end_ms": 16290,
  "text": "Great! I tested the Omarchy package this morning.",
  "source": "loopback",
  "speaker_id": "Remote 1",
  "speaker_label": "Dana",
  "original_text": "Great! I tested the Omarchy package this morning, and the settings panel opened without any problems.",
  "chunk_id": 0
}
```

```sh
voxtype meeting annotate <meeting> <segment> ["<note>"] [--highlight] --json
voxtype meeting annotate <meeting> <segment> --remove --json
```

```json
{
  "id": 4,
  "start_ms": 23040,
  "end_ms": 30000,
  "text": "Only one. The restart banner stays visible after the daemon restarts.",
  "source": "loopback",
  "speaker_id": "Remote 1",
  "speaker_label": "Dana",
  "annotation": {"text": "Known issue for the notes", "highlight": true},
  "chunk_id": 0
}
```

A line has at most one annotation; annotating it again replaces it.

```sh
voxtype meeting delete-line <meeting> <segment> --json
voxtype meeting delete-line <meeting> <segment> --restore --json
```

```json
{
  "id": 7,
  "start_ms": 42450,
  "end_ms": 45990,
  "text": "Perfect, then we tag tomorrow morning at 10. Thanks, Dana.",
  "source": "microphone",
  "speaker_id": "You",
  "deleted": true,
  "chunk_id": 0
}
```

`--restore` prints the line without `deleted`. A hidden line stays in
`transcript.json` with `"deleted": true` and appears nowhere else: not in
`hello`, `meeting show`, exports, or what the assistant sees.

A segment ID the meeting does not have is an error (exit 1) that names the
valid range.

#### Notes

```sh
voxtype meeting notes <meeting> --json
voxtype meeting notes <meeting> --append "<text>" --json
voxtype meeting notes <meeting> --set-file <path|-> --json
```

```json
{
  "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed",
  "notes": "Tag at 10 tomorrow.\n",
  "updated_at": "2026-11-11T15:33:07.023Z"
}
```

`notes` is the whole of `notes.md`, empty when there are none, and then
`updated_at` is `null`. `--append` adds a line, `--set-file` replaces them.
The daemon never writes notes itself.

### Asking a meeting: `meeting ask --json`

```sh
voxtype meeting ask <meeting> "<question>" --json [--last 5m] [--agent claude|codex|pi] [--local] [--notify]
```

One JSON object per line:

```json
{"event": "start", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "agent": "claude", "agent_name": "Claude", "question": "What blockers came up?", "last_secs": 300}
{"event": "delta", "text": "Dana found one blocker: "}
{"event": "delta", "text": "the restart banner stays visible after the daemon restarts [00:23]."}
{"event": "answer", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "id": "8b1fba46-aa7b-430c-b3da-6dbd515b6cd3", "time": "2026-11-11T15:33:06.959Z", "question": "What blockers came up?", "answer": "Dana found one blocker: the restart banner stays visible after the daemon restarts [00:23].", "agent": "claude", "agent_name": "Claude", "citations": [{"ts": "00:23", "ms": 23000}], "last_secs": 300}
```

- `delta` events stream the answer as it is written. Pi answers whole, so
  with Pi there is one `delta` or none.
- `citations` are the `[mm:ss]` times in the answer, with `ms` from the start
  of the meeting, for jumping to the line.
- `last_secs` is present only with `--last`.
- The `answer` object without `event` and `meeting_id` is the record saved
  to the meeting's `asks.jsonl` and listed under `asks` in `meeting show
  --json`.

On failure the last line is an `error` and the exit status is 1. `backend`
names the backend that failed, or is `null` when none was chosen:

```json
{"event": "error", "code": "consent_required", "message": "Meeting transcripts have not been allowed to go to Claude.\n  Allow it once with: voxtype meeting assistant allow claude\n  Or answer with the local model: add --local, or set [meeting.assistant] local_only = true", "backend": null}
{"event": "error", "code": "agent_failed", "message": "Claude failed: Not logged in. Run claude /login", "backend": "Claude"}
```

`--notify` also shows the answer as a notification. Its Open action runs
`voxtype meeting view "$VOXTYPE_MEETING_ID" --tab ask` through the same
binary, or `[meeting.assistant] open_command` with `VOXTYPE_MEETING_ID` and
`VOXTYPE_ASK_ID` set (an empty `open_command` drops the action). With
`--notify` the command waits for the notification to be answered, up to 15
minutes.

`ask` does not need the daemon: it reads the saved transcript.

#### Assistant error codes

Used by `ask`, `meeting summarize` and the summary at stop:
`consent_required`, `agent_unavailable`, `agent_failed`, `timeout`,
`local_failed`, `no_backend`, `empty_transcript`, `storage`, `parse`.

### Summaries: `meeting summarize --format json`

```sh
voxtype meeting summarize <meeting> --format json [--regenerate] [--agent ...] [--local]
```

Prints the summary object (the `summary` in `summary_ready` above) on
stdout, and progress on stderr. A meeting that already has a summary prints
it without calling the agent; `--regenerate` writes a new one.

### Assistant state: `meeting assistant status --json`

What a view needs for an agent chip and a consent dialog:

```json
{
  "agent": "claude",
  "agent_name": "Claude",
  "agent_path": "/usr/bin/claude",
  "auto_summary": false,
  "auto_summary_configured": null,
  "backend": "none",
  "backend_name": null,
  "blocked": "consent_required",
  "summary_backend": "none",
  "summary_backend_name": null,
  "summary_blocked": "consent_required",
  "summary_note": null,
  "candidates": [
    {"id": "claude", "name": "Claude", "path": "/usr/bin/claude", "found": true, "supported": true, "tool_less": true, "sandbox": null, "consent": false, "sources": ["omarchy", "path"]},
    {"id": "codex", "name": "Codex", "path": "/usr/bin/codex", "found": true, "supported": true, "tool_less": false, "sandbox": "bwrap", "consent": false, "note": "its command tool cannot be turned off, so it runs in a bwrap jail that sees no files from your home but its auth.json", "sources": ["config"]}
  ],
  "choice_required": false,
  "consent": false,
  "consent_file": "/home/me/.local/state/voxtype/assistant-consent.json",
  "local": {"model": "llama3.2", "name": "Local (llama3.2)", "url": "http://localhost:11434"},
  "local_only": false,
  "omarchy_default": "claude",
  "problem": null,
  "source": "omarchy"
}
```

- `agent` is the agent `ask` would use, `source` says why: `config`,
  `omarchy` (the Omarchy default agent), `path`, or `none`.
- `backend` is what `ask` would use now: `agent`, `local`, or `none` with the
  reason in `blocked` (an [error code](#assistant-error-codes)).
  `backend_name` is the chip's label ("Claude", "Local (llama3.2)").
- `summary_backend` is what a summary would use now (at stop, `summarize`,
  Regenerate): `agent`, `local`, `remote`, or `none` with the reason in
  `summary_blocked`. `summary_backend_name` names it ("Claude",
  "Local (llama3:latest)", "Remote (host)"). The two follow one order
  ([Meeting Mode](MEETING_MODE.md#which-backend-answers)) and differ only
  for an older `[meeting.summary] backend = "remote"`; then `summary_note`
  says so in words, otherwise it is `null`.
- `consent` is whether `agent` is allowed. Each candidate has its own
  `consent`, and `consented_at` once allowed.
- `choice_required` is true when several agents could answer and none is
  allowed yet: offer the candidates that are `found` and `supported`.
- `sandbox` is `"bwrap"` for codex, which keeps a command tool and so only
  runs in a jail, and `null` for the agents that run with no tools. Without
  `bwrap` codex is listed with `supported: false` and the reason in `note`.
- `auto_summary` is whether the summary at stop will run;
  `auto_summary_configured` is `[meeting.summary] auto` as set, `null` when
  unset.
- `problem` explains a configured agent that cannot be used.

The consent dialog's answers map to `voxtype meeting assistant allow <id>`,
`voxtype config set meeting.assistant.local_only true` (or `ask --local` for
one question), and doing nothing. `voxtype meeting assistant revoke <id>`
withdraws consent.

### Checking the audio: `meeting soundcheck`

```sh
voxtype meeting soundcheck [--seconds N] [--json]
voxtype meeting soundcheck --stop
```

Opens the meeting's microphone and loopback as a meeting would, records and
transcribes nothing, and prints both tracks' levels for `N` seconds (default
30, at most 600) or until Ctrl+C. With `--json` it prints NDJSON: a
`soundcheck` `running` event, `levels` events, and a `soundcheck` `stopped`
event, in the shapes above. Without it, it draws two meters.

While the daemon runs, the daemon does the check, because it owns the audio:
the command asks for it through a trigger file and relays the events from
meeting.sock, where every other client (the meeting view) sees them too. A
second request while one runs extends it. `--stop` ends it from anywhere.
The daemon ends it after `N` seconds even if the asking command is gone.
Without the daemon, the command opens the audio itself the same way and
prints the same lines.

A meeting that is recording already has its levels, so the command refuses
(exit 1) and points at `voxtype meeting watch --levels`. Exit status 1 also
covers a check the audio could not be opened for, or one a starting meeting
ended.

### Playing meeting audio: `meeting play`

A meeting recorded with `retain_audio = true` can be played from any point.
The command starts playback in the background (`ffmpeg` decoding into
`pw-play`, `paplay` or `aplay`, the first one found) and returns at once.
One playback runs at a time; starting another stops the one before.

```sh
voxtype meeting play <meeting> [--segment <id> | --at <time>] [--track mixed|mic|loopback] [--json]
voxtype meeting play --status [--json]
voxtype meeting play --stop [--json]
```

`--segment` starts at that segment's `start_ms`. `--at` takes seconds (`95`,
`95.5`), `mm:ss` or `hh:mm:ss` from the start of the meeting. Without either
it starts at 0. `--track` defaults to `mixed`.

Starting prints, with `--json`:

```json
{"meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "track": "mixed", "from_ms": 23040, "pid": 479}
```

`pid` is the playback's process group. `--status --json` prints the current
or last playback, and `--stop --json` what it stopped, already `stopped`:

```json
{"state": "playing", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "track": "mixed", "from_ms": 23040, "pid": 479, "started_at": "2026-11-11T15:33:07.054Z", "position_ms": 23044}
{"state": "stopped", "meeting_id": "a32baa9b-b9df-4051-8830-b422d21495ed", "track": "mixed", "from_ms": 23040, "pid": 479, "started_at": "2026-11-11T15:33:07.054Z", "position_ms": null}
```

`state` is `playing` or `stopped`; before anything has played, every other
field is `null`. `position_ms` estimates where playback is now (`from_ms`
plus the time since `started_at`), for highlighting the line being heard,
and is `null` unless playing.

A failure exits 1 with the reason on stderr: the meeting kept no audio, the
track is missing (a meeting without loopback has no `loopback` track), the
segment is unknown, or `ffmpeg` or a player is not installed.

### Drawing a waveform: `meeting waveform`

For a player's scrub bar: the peak level of each time bucket of a meeting's
kept audio, per track.

```sh
voxtype meeting waveform <meeting> [--buckets N] --json
```

```json
{"duration_ms": 2843210, "bucket_ms": 2844, "mic": [-100.0, -41.3, -18.9, ...], "loopback": [-37.2, -12.4, ...], "mic_track": "mic"}
```

- `duration_ms` is the longer track's length, and `bucket_ms` the time each
  value covers: the meeting split into at most `--buckets` (default 1000,
  1 to 100000) buckets of a whole number of milliseconds. The last bucket
  may be shorter. `mic` and `loopback` have the same number of values;
  a shorter track is padded with silence.
- Values are peak dBFS, rounded to 0.1, with -100 for silence (the floor
  the live `levels` use).
- `loopback` is `null` for a meeting recorded without loopback.
  `mic_track` is `mic`, or `mixed` for a meeting that kept only both sides
  together (an import without `--channels`): `mic` then holds the mixed
  track.

The audio is decoded with `ffmpeg`. The result is cached in the meeting
folder as `waveform.json`, one entry per bucket count, so asking again is
instant; it is recomputed when an audio file changes, which includes a
meeting that is still recording. A meeting that kept no audio prints the
reason on stderr and exits 2; other failures (no `ffmpeg`, an unreadable
file) exit 1.

### Importing and transcribing again: `meeting import` and `meeting retranscribe`

```sh
voxtype meeting import <file|meeting-recorder-folder> [--title T] [--engine E] [--model M] [--language L]
                       [--date RFC3339] [--channels mono|mic,loopback,...] [--diarization B] [--json]
voxtype meeting retranscribe <meeting> [--engine E] [--model M] [--language L] [--diarization B]
                             [--keep-edits | --discard-edits] [--json]
```

Both run in the calling process, not in the daemon, and can take minutes.
With `--json` they print one JSON object per line on stdout: a `progress`
line when they start, one after each chunk and one at the end, then the
result. Logs go to stderr.

```json
{"event": "progress", "meeting_id": "...", "operation": "import", "state": "transcribing", "chunks_done": 3, "chunks_total": 5, "position_ms": 89000, "duration_ms": 150000}
{"event": "progress", "meeting_id": "...", "operation": "import", "state": "done", "chunks_done": 5, "chunks_total": 5, "position_ms": 150000, "duration_ms": 150000}
{"event": "imported", "meeting_id": "...", "title": "Client call", "folder": "/home/me/.local/share/voxtype/meetings/2026-11-02-client-call", "started_at": "2026-11-02T15:00:00Z", "duration_ms": 150000, "channels": "mic,loopback", "segments": 42, "engine": "whisper", "model": "base.en", "audio_files": ["mic.opus", "loopback.opus", "mixed.opus"]}
```

```json
{"event": "retranscribed", "meeting_id": "...", "previous_transcript": "transcript.v1.json", "segments": 40, "engine": "whisper", "model": "large-v3-turbo", "channels": "mic,loopback", "edits": {"edits_carried": 3, "edits_dropped": 1, "edits_discarded": 0, "annotations_carried": 2, "annotations_dropped": 0}}
```

- `operation` is `import` or `retranscribe`; `state` is `transcribing`,
  `done` or `failed`.
- `chunks_total` is estimated from the duration and settles at the end, when
  it equals `chunks_done`. `position_ms` is how far through the audio it is.
- The same object, with `"event": "import_progress"`, goes to `meeting.sock`
  when the daemon is running, so a view can show the progress of an import
  started elsewhere.
- `channels` is `mono`, the `--channels` map, or `meeting-recorder` for a
  Meeting Recorder folder (imported as it is, with nothing transcribed).
- `previous_transcript` is `null` when the meeting had no transcript.
  `edits` counts speaker moves, text corrections and deleted lines
  (`edits_*`) and annotations separately; `edits_discarded` is non-zero
  only with `--discard-edits`.
- A failure exits 1 with the reason on stderr.

### The meeting folder and `meeting.json`

Every meeting has a folder under the storage path
(`~/.local/share/voxtype/meetings/<date>-<title>/` by default), and every
folder has a `meeting.json` manifest describing the meeting and the files
beside it. Read the manifest rather than guessing file names, and rather than
`metadata.json`, which is voxtype's internal record.

```json
{
  "manifest_version": 1,
  "id": "a32baa9b-b9df-4051-8830-b422d21495ed",
  "title": "Release sync",
  "url": "https://meet.google.com/abc-defg-hij",
  "status": "completed",
  "started_at": "2026-11-11T15:00:04Z",
  "ended_at": "2026-11-11T15:31:12Z",
  "duration_secs": 1868,
  "engine": "parakeet",
  "model": "parakeet-tdt-0.6b-v3",
  "diarization": "hybrid",
  "echo_cancel": "webrtc",
  "speakers": [
    { "id": "Remote 1", "name": "Dana" },
    { "id": "Remote 2", "name": "Remote 2" },
    { "id": "You", "name": "You" }
  ],
  "audio_retained": true,
  "files": [
    { "name": "transcript.json", "kind": "transcript", "bytes": 48211 },
    { "name": "transcript.md", "kind": "transcript_markdown", "bytes": 21980 },
    { "name": "notes.md", "kind": "notes", "bytes": 20 },
    { "name": "summary.md", "kind": "summary", "bytes": 483 },
    { "name": "mic.opus", "kind": "audio_mic", "bytes": 4981233 },
    { "name": "loopback.opus", "kind": "audio_loopback", "bytes": 5310877 },
    { "name": "mixed.opus", "kind": "audio_mixed", "bytes": 5402115 }
  ],
  "voxtype_version": "1.3.0"
}
```

This is a stable contract:

- Every field above is always present. A value that is unknown or does not
  apply is `null`: `title` and `url` when not given; `ended_at` and
  `duration_secs` while the meeting records; `engine`, `model` and
  `diarization` for meetings recorded before voxtype kept them;
  `diarization` when it was off.
- New fields may be added. `manifest_version` changes only if an existing
  field changes meaning.
- Keys are written in the order shown, pretty-printed, with a trailing
  newline. The file is replaced atomically, so a reader never sees half of
  it.

The fields:

- `status`: `active`, `paused`, `completed`, `cancelled` or `interrupted`. A
  meeting left `active` by a crash becomes `interrupted` the next time the
  daemon starts, and `completed` once `meeting recover --finish` finishes it.
- `started_at`, `ended_at`: RFC 3339, UTC. `duration_secs` is wall-clock
  time from start to stop.
- `engine`, `model`: the transcription engine and model the meeting used,
  including a `meeting start --engine` or `--model` override.
  `voxtype_version` is the version that recorded the meeting.
- `speakers`: each speaker with at least one line in the transcript, sorted
  by `id`; lines deleted with `meeting delete-line` do not count. `id` is the
  speaker as the diarizer named it (the `speaker_id` in `transcript.json`, or
  `You`/`Remote` without diarization); `name` is the name given with
  `voxtype meeting label`, or `id` when there is none. Two speakers given the
  same name keep separate entries.
- `files`: the meeting's files that exist, in the order shown, with their
  size when the manifest was written. `kind` is one of `transcript`,
  `transcript_markdown`, `notes`, `summary`, `transcript_previous` (a
  transcript `meeting retranscribe` replaced, `transcript.v1.json` and on,
  oldest first), `audio_mic`, `audio_loopback` or `audio_mixed`. Names are relative to the folder. Audio is `.opus` (Ogg
  Opus, 16 kHz mono), or `.wav` (16-bit PCM, 16 kHz mono) when ffmpeg was
  missing; match on `kind`, not on the name.
- `echo_cancel`: the echo handling the meeting used: `webrtc` (PipeWire's
  WebRTC echo canceller), `gtcrn` (the speech enhancer) or `none`. `null`
  for meetings recorded before 1.3.0.
- `audio_retained`: whether the meeting kept its audio (`retain_audio`).

The folder can hold files the manifest does not list: `asks.jsonl` (one
saved answer per line, read them through `meeting show --json`),
`assistant-context.json` (the assistant's running summary of a long
meeting) and `metadata.json`. They are internal.

The manifest is written when a meeting starts and rewritten after every
chunk, when it stops, when its transcript is edited (labels, moves, text
edits, deleted lines), when its notes or summary change, and when a daemon
restart completes a meeting a crash left open. To act once a meeting is finished, use
`[meeting] post_meeting_command` (see
[Meeting Mode](MEETING_MODE.md#running-a-command-after-a-meeting)), which
runs after the final rewrite with the folder and the manifest's path in
`VOXTYPE_MEETING_DIR` and `VOXTYPE_MEETING_MANIFEST`, and the ID in
`VOXTYPE_MEETING_ID`.

### The meeting view

The meeting view runs in the Voxtype plugin for the Omarchy shell and in
voxtype's standalone Quickshell shell. Two files connect it to everything
else, and another host (a different shell, a GTK window) can use both.

#### Opening it: `meeting-view.request`

`voxtype meeting view` writes `$XDG_RUNTIME_DIR/voxtype/meeting-view.request`
and every running host acts on it, so callers do not need to know which host
is running:

```sh
voxtype meeting view [<meeting>] [--tab transcript|notes|ask|summary] [--ask-id <id>] [--ask last-5m|last-speaker|actions] [--no-open] [--minimize | --expand]
```

```json
{"ask": null, "ask_id": null, "meeting_id": "latest", "open": true, "requested_at": 1794440137340, "tab": "notes", "window": "expand"}
```

| Field | Meaning |
|-------|---------|
| `meeting_id` | The meeting to show, as given (an ID, the start of one, or `latest`); `null` for the current or last meeting |
| `ask_id` | A saved answer to show on the Ask tab; implies `tab: "ask"` |
| `tab` | The tab to open on, or `null` |
| `ask` | A quick question to ask, or `null` |
| `open` | Whether to open the view; `false` with `ask` means the answer arrives as a notification |
| `window` | `minimize`, `expand`, or `null` |
| `requested_at` | Unix time in milliseconds |

The file is written whole and renamed into place. A host reads it when it
changes, deletes it, and ignores a request more than 30 seconds old, so a
host that starts later does not act on an old one. `meeting_id` and
`ask_id` default to `VOXTYPE_MEETING_ID` and `VOXTYPE_ASK_ID`, which is how
an Ask notification's Open action lands on its answer.

#### Is it on screen: `meeting-view.state`

A host keeps `$XDG_RUNTIME_DIR/voxtype/meeting-view.state` current so the
daemon can hold back its meeting notifications while the view already shows
the same thing (`[meeting] status_notifications = "auto"`):

```json
{"visible": true, "minimized": false, "host": "omarchy", "pid": 12345, "updated_at": 1794434659}
```

`host` is `omarchy` or `standalone` for voxtype's own hosts. The file is
replaced whole on every show, hide and minimize, and every 10 seconds while
visible; closing the view writes `"visible": false`. A minimized pill counts
as visible. A shell that dies leaves the file behind, so a reader counts the
view as visible only while `pid` is alive and `updated_at` is under 30
seconds old.

### Files under the runtime and state directories

The sockets and files above are the contract. Everything else voxtype keeps
in `$XDG_RUNTIME_DIR/voxtype/` and `$XDG_STATE_HOME/voxtype/` is internal and
can change in any release; use the command named instead.

| File | Contract | Instead, use |
|------|----------|--------------|
| `$XDG_RUNTIME_DIR/voxtype/meeting.sock` | Yes | |
| `$XDG_RUNTIME_DIR/voxtype/meeting-view.request` | Yes | `voxtype meeting view` to write it |
| `$XDG_RUNTIME_DIR/voxtype/meeting-view.state` | Yes | |
| `<transcript>.done` beside a `--file` transcript | Yes | `voxtype record stop --wait` |
| `$XDG_RUNTIME_DIR/voxtype/meeting_state` | No | `voxtype meeting status --json` |
| `$XDG_RUNTIME_DIR/voxtype/meeting_start`, `meeting_stop`, `meeting_pause`, `meeting_resume` and the other request files | No | the `voxtype meeting` commands |
| `$XDG_RUNTIME_DIR/voxtype/meeting_playback.json` | No | `voxtype meeting play --status --json` |
| `$XDG_RUNTIME_DIR/voxtype/meeting-controls.flag` | No | `voxtype meeting view`; touching it toggles the standalone shell's view, for old keybindings |
| `$XDG_STATE_HOME/voxtype/assistant-consent.json` | No | `voxtype meeting assistant status --json`, `allow`, `revoke` |
| `$XDG_STATE_HOME/voxtype/meeting-view.json` | No | Where the view was left, per host and screen; the view's **Reset position and size** |
| `$XDG_STATE_HOME/voxtype/meeting_offer.json` | No | The answers to the "Turn on meetings" offer |
| `$XDG_STATE_HOME/voxtype/meeting-recover-notified.json` | No | `voxtype meeting recover --json` lists the meetings; this only remembers which were announced |

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
