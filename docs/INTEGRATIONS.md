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
voxtype meeting list --json            # an array of MeetingMetadata, plus two fields
```

Each `meeting list --json` entry is the meeting's metadata with two more
fields, read from the meeting index rather than each transcript, so a list
of many meetings stays fast:

- `has_summary` (bool): a summary is saved (`meeting summarize`, or the
  summary at stop).
- `speaker_count` (number): distinct speakers as the transcript displays
  them, with labels and moved lines applied; speakers given the same name
  count once, and a speaker whose only lines are empty does not count. It is
  refreshed whenever the transcript is saved, so it follows labels and moves
  made during or after the meeting. A meeting with no transcript yet has 0.

A meeting started with `--url` has `"url"` in its metadata; the key is absent otherwise. Something that starts meetings for a calendar event (a detection prompt, a calendar plugin) can pass `--url` with `--title`, and read the link back from `metadata.url`.

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
{"event": "segment_deleted", "meeting_id": "...", "segment_id": 31}
{"event": "notes_updated", "meeting_id": "...", "updated_at": "2026-11-11T20:00:00Z"}
{"event": "status", "status": "paused"}
{"event": "playback", "meeting_id": "...", "track": "mixed", "state": "playing", "from_ms": 61250}
{"event": "import_progress", "meeting_id": "...", "operation": "import", "state": "transcribing", "chunks_done": 2, "chunks_total": 5, "position_ms": 60000, "duration_ms": 150000}
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

A segment whose text the user corrected (`voxtype meeting edit`) has the
correction in `text` and what was transcribed in `original_text`; the field
is absent on unedited lines and after `--reset`. A correction during a
meeting sends `segment_updated`.

`segment_deleted` means the user hid that segment (`voxtype meeting
delete-line`): drop it. `--restore` sends `segment_updated` with the whole
segment, to be put back in order of `start_ms`. Deleted segments never
appear in `hello`, `meeting show --json` or exports. Only `transcript.json`
keeps them, marked `"deleted": true`, so a reader of that file should skip
them.

`notes_updated` means the meeting's notes (`notes.md`) changed. It carries
no text: read it with `voxtype meeting notes <meeting_id> --json`, which
prints `{"meeting_id": "...", "notes": "...", "updated_at": "..."}`
(`updated_at` is null when there are no notes).

Events fire roughly once per `chunk_duration_secs` (default 30s), not per
word — meeting mode transcribes in chunks, it does not stream. A UI built on
this socket should show a "listening…" affordance between updates, not
imitate word-by-word streaming it cannot actually deliver.

`playback` follows `voxtype meeting play` (below): `state` is `playing`
when a playback starts and `stopped` when it is stopped or reaches the end
of the track. It describes the current playback when the daemon sends it, so
changes made in quick succession can arrive as one event with the latest
state. It is sent whether or not a meeting is recording, and not at all
while the daemon is down; poll `meeting play --status --json` if you need
certainty.

#### Summary at stop: `summarizing` and `summary_ready`

When a meeting stops and auto-summary is on, the daemon summarizes it in the
background after the transcript is saved. These events name the meeting they
are about, because by then the daemon is already `idle` and a new meeting may
have started. They do not change the `hello` snapshot.

```json
{"event": "status", "status": "summarizing", "meeting_id": "..."}
{"event": "summary_ready", "meeting_id": "...", "agent": "claude", "agent_name": "Claude", "path": "/home/me/.local/share/voxtype/meetings/2026-11-11-sync/summary.md", "summary": {"summary": "...", "key_points": [], "decisions": [], "action_items": [{"description": "...", "assignee": "Steve", "completed": false}], "chapters": [{"start_ms": 41000, "title": "..."}], "generated_at": "...", "model": "claude"}}
{"event": "error", "message": "Summary failed: ...", "meeting_id": "...", "code": "consent_required"}
```

A summary that fails ends with the `error` event instead of `summary_ready`;
`code` is one of `consent_required`, `agent_unavailable`, `agent_failed`,
`timeout`, `local_failed`, `no_backend`, `empty_transcript`, `storage`,
`parse`. A meeting with auto-summary off, or with no transcript, produces
neither event. `agent` is `claude`, `codex`, `local:<model>`, or `ollama` /
`remote` for the older summarizers. The saved summary is also in
`voxtype meeting show <id> --json` under `metadata.summary`.

`import_progress` follows `voxtype meeting import` and `meeting
retranscribe`, which run outside the daemon and hand it their progress:
see [Importing and transcribing again](#importing-and-transcribing-again-meeting-import-and-meeting-retranscribe)
below. It is not sent while the daemon is down.

`speakers` (live speaker naming) events are planned for a later release and
are not emitted yet.

### Asking a meeting: `meeting ask --json`

`voxtype meeting ask <id|latest> "<question>" --json` prints one JSON object
per line:

```json
{"event": "start", "meeting_id": "...", "agent": "claude", "agent_name": "Claude", "question": "...", "last_secs": 300}
{"event": "delta", "text": "Steve said "}
{"event": "delta", "text": "we ship Friday [00:41]."}
{"event": "answer", "meeting_id": "...", "id": "...", "time": "...", "question": "...", "answer": "...", "agent": "claude", "agent_name": "Claude", "citations": [{"ts": "00:41", "ms": 41000}], "last_secs": 300}
```

or, on failure, `{"event": "error", "code": "...", "message": "..."}` with
exit status 1 (`code` as above). `last_secs` is present only with `--last`.
The `answer` object, without `event` and `meeting_id`, is the line saved to
the meeting's `asks.jsonl` and listed under `asks` in `meeting show --json`.
`--notify` additionally shows the answer as a notification whose Open action
runs `voxtype meeting view "$VOXTYPE_MEETING_ID" --tab ask` through the same
binary that answered (or `[meeting.assistant] open_command`, with
`VOXTYPE_MEETING_ID` and `VOXTYPE_ASK_ID` set; an empty `open_command` drops
the action). The command waits for the notification to be answered, up to 15
minutes. A view host that wants to scroll to the answer reads
`VOXTYPE_ASK_ID` through a custom `open_command`.

### Assistant state: `meeting assistant status --json`

What a UI needs for an agent chip and a consent dialog:

```json
{
  "agent": "claude", "agent_name": "Claude", "agent_path": "/usr/bin/claude",
  "source": "omarchy",
  "consent": false,
  "choice_required": false,
  "problem": null,
  "omarchy_default": "claude",
  "local_only": false,
  "local": {"model": "llama3.2", "url": "http://localhost:11434", "name": "Local (llama3.2)"},
  "backend": "none", "backend_name": null, "blocked": "consent_required",
  "auto_summary": false, "auto_summary_configured": null,
  "consent_file": "/home/me/.local/state/voxtype/assistant-consent.json",
  "candidates": [
    {"id": "claude", "name": "Claude", "path": "/usr/bin/claude", "found": true, "supported": true, "tool_less": true, "consent": false, "sources": ["omarchy", "path"]},
    {"id": "codex", "name": "Codex", "path": "/usr/bin/codex", "found": true, "supported": true, "tool_less": false, "sandbox": "bwrap", "consent": false, "note": "its command tool cannot be turned off, so it runs in a bwrap jail that sees no files from your home but its auth.json", "sources": ["path"]}
  ]
}
```

- `source` is `config`, `omarchy`, `path` or `none`.
- `sandbox` is `"bwrap"` for codex, which runs in a filesystem jail because its
  command tool cannot be turned off, and `null` for claude, which runs with no
  tools. Without `bwrap`, codex is listed with `supported: false` and the
  reason in `note`.
- `backend` is what `ask` would use now: `agent`, `local`, or `none` with the
  reason in `blocked`. `backend_name` is the chip label ("Claude",
  "Local (llama3.2)").
- `choice_required` is true when several agents could answer and none is
  allowed yet: offer the `candidates` that are `supported` and `found`.
- The consent dialog's three answers map to `voxtype meeting assistant allow
  <id>`, `voxtype config set meeting.assistant.local_only true` (or `ask
  --local` for one question), and doing nothing.

### Playing meeting audio: `meeting play`

A meeting kept with `retain_audio = true` can be played from any point. The
command starts the playback in the background (`ffmpeg` seeking and decoding
into `pw-play`, `paplay` or `aplay`, whichever is found first) and returns at
once. One playback runs at a time; starting another stops the one before.

```sh
voxtype meeting play <meeting> [--segment <id> | --at <time>] [--track mixed|mic|loopback] [--json]
voxtype meeting play --status [--json]
voxtype meeting play --stop [--json]
```

- `<meeting>` is a meeting ID, a unique start of one, or `latest`.
- `--segment` starts at that segment's `start_ms` (`id` from `transcript.json`
  or `meeting show --json`). `--at` takes seconds (`95`, `95.5`), `mm:ss` or
  `hh:mm:ss`, from the start of the meeting. Neither starts at 0.
- `--track` defaults to `mixed`.

Starting prints, with `--json`, one line:

```json
{"meeting_id": "b6b2e5b2-...", "track": "mixed", "from_ms": 61250, "pid": 48211}
```

`pid` is the playback's process group. `--status --json` and `--stop --json`
print the current or last playback (`--stop` prints what it stopped, with
`state` already `stopped`):

```json
{"state": "playing", "meeting_id": "b6b2e5b2-...", "track": "mixed", "from_ms": 61250, "pid": 48211, "started_at": "2026-11-11T22:58:38.164Z", "position_ms": 64105}
```

- `state` is `playing` or `stopped`. Before anything has played, every other
  field is `null`.
- `position_ms` estimates where playback is now (`from_ms` plus the time
  since `started_at`), for highlighting the line being heard. It is `null`
  unless playing.

A failure exits 1 with the reason on stderr: the meeting has no kept audio
("This meeting was recorded without keeping audio. Turn on
meeting.retain_audio for future meetings."), the track is missing (a
meeting recorded without loopback has no `loopback` track), the segment is
unknown, or `ffmpeg` or a player is not installed (the message names what is
missing).

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

- `url`: the call's link, from `voxtype meeting start --url`.
- `status`: `active`, `paused`, `completed` or `cancelled`. A meeting left
  `active` by a crash becomes `completed` the next time the daemon starts.
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
  size in bytes when the manifest was written. `kind` is one of
  `transcript`, `transcript_markdown`, `notes`, `summary`,
  `transcript_previous` (a transcript `meeting retranscribe` replaced,
  `transcript.v1.json` and on, oldest first), `audio_mic`, `audio_loopback`
  or `audio_mixed`. Names are relative to the folder.
  Audio files end in `.opus` (Ogg Opus, 16 kHz mono) or, when ffmpeg was
  missing, `.wav` (16-bit PCM, 16 kHz mono); match on `kind`, not on the
  name.
- `audio_retained`: whether the meeting kept its audio (`retain_audio`).

The manifest is written when a meeting starts and rewritten after every
chunk, when it stops, when its transcript is edited (labels, moves, text
edits, deleted lines), when its notes or summary change, and when a daemon
restart completes a meeting a crash left open. To act once a meeting is finished, use
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
