# Meeting Mode

Meeting mode provides continuous transcription for meetings, lectures, interviews, or any long-form audio capture. Unlike push-to-talk dictation (which transcribes short bursts when you hold a hotkey), meeting mode records continuously and processes audio in chunks, building a timestamped transcript you can export and search later.

## Table of Contents

- [When to Use Meeting Mode](#when-to-use-meeting-mode)
- [Quick Start](#quick-start)
- [Commands](#commands)
  - [Starting a Meeting](#starting-a-meeting)
  - [Stopping a Meeting](#stopping-a-meeting)
  - [Pausing and Resuming](#pausing-and-resuming)
  - [Checking Status](#checking-status)
  - [Listing Past Meetings](#listing-past-meetings)
  - [Viewing Meeting Details](#viewing-meeting-details)
  - [Exporting Transcripts](#exporting-transcripts)
  - [Labeling Speakers](#labeling-speakers)
  - [AI Summarization](#ai-summarization)
  - [Deleting Meetings](#deleting-meetings)
- [Configuration](#configuration)
  - [Basic Settings](#basic-settings)
  - [Audio Settings](#audio-settings)
  - [Diarization Settings](#diarization-settings)
  - [Summarization Settings](#summarization-settings)
- [Storage](#storage)
- [Tips for Best Results](#tips-for-best-results)

---

## When to Use Meeting Mode

Use meeting mode when you need a transcript of a longer recording session. Good use cases:

- Video calls and meetings (Zoom, Teams, Google Meet)
- Lectures and presentations
- Interviews
- Brainstorming sessions

For short dictation (a sentence or two at a time), stick with the normal push-to-talk workflow.

---

## Quick Start

1. Enable meeting mode in your config:

```toml
[meeting]
enabled = true
```

2. Start the voxtype daemon if it is not already running:

```bash
voxtype
```

3. Start a meeting:

```bash
voxtype meeting start --title "Weekly Standup"
```

4. When the meeting ends:

```bash
voxtype meeting stop
```

5. Export the transcript:

```bash
voxtype meeting export latest --format markdown --output standup.md
```

---

## Commands

### Starting a Meeting

```bash
voxtype meeting start
voxtype meeting start --title "Project Kickoff"
voxtype meeting start -t "1:1 with Alice"
```

The `--title` flag is optional. If omitted, the meeting is named by its date and time (e.g., "Meeting 2026-02-16 14:30"). The daemon must be running, and meeting mode must be enabled in config.

Only one meeting can run at a time. Starting a second meeting while one is active will fail.

Recording starts as soon as the daemon reads the request, before any model loads, so nothing said in the first seconds is lost.

#### Engine, model, language and link

A meeting transcribes with the same engine and model as dictation unless you choose otherwise for that meeting:

```bash
voxtype meeting start --model large-v3-turbo --language de
voxtype meeting start --engine parakeet --model parakeet-ultra-Q8_0.gguf
voxtype meeting start --title "Planning" --url https://meet.example.com/abc-defg-hij
```

| Flag | Environment | Meaning |
|------|-------------|---------|
| `--engine` | `VOXTYPE_MEETING_ENGINE` | Engine for this meeting. Alone, it uses that engine's configured model. |
| `--model` | `VOXTYPE_MEETING_MODEL` | Model for this meeting, as `voxtype info models` names it. Its engine is implied. |
| `--language` | `VOXTYPE_MEETING_LANGUAGE` | Language hint, for checkpoints that take one (multilingual Whisper, Cohere). |
| `--url` | | Link to the call. Stored with the meeting; `meeting show` and exports with `--metadata` print it. |

A choice the meeting could not use is refused before anything starts, with the fix: an engine this binary was built without (with how to get a build that has it), a model that is not downloaded (with the download command), a model that belongs to a different engine than `--engine`, or a language the checkpoint does not take.

#### Fast start

When the meeting uses dictation's engine and model, and dictation keeps its model loaded (`on_demand_loading = false`, the default), the meeting transcribes with that loaded model instead of loading a second copy, so it is ready at once. This covers the GGUF worker and the in-process engines. Dictation keeps working during the meeting; the two take turns on the model, so a dictation that lands while a meeting chunk is being transcribed waits for it.

A different engine, model or language loads a separate copy for the meeting, as does Whisper with `gpu_isolation = true` (which starts a worker per transcription either way) and Soniox (meetings use its async API). With `on_demand_loading = true` nothing is resident, and the meeting loads its model when it starts.

### Stopping a Meeting

```bash
voxtype meeting stop
```

Stops recording, processes any remaining audio, saves the transcript, and returns to idle. You can stop a meeting whether it is active or paused.

### Pausing and Resuming

```bash
voxtype meeting pause
voxtype meeting resume
```

Pause temporarily stops recording without ending the meeting. Audio during the paused period is not captured. Resume picks up where you left off.

This is useful for breaks, side conversations you do not want transcribed, or when switching contexts temporarily.

### Checking Status

```bash
voxtype meeting status
voxtype meeting status --json
```

Shows whether a meeting is recording, paused, finalizing, or idle, along with the meeting ID if one is in progress. The transcript keeps updating while a meeting is in progress, so `voxtype meeting show latest` is accurate mid-meeting too, not just after `stop`.

The daemon acts on meeting commands between audio chunks, so while it is transcribing a chunk a request can take a few seconds to land. Until it does, status reports the request itself: `starting`, `pausing`, `resuming` or `stopping`. A second `voxtype meeting start` is refused as soon as the first one is issued, not only once that meeting is recording.

### Listing Past Meetings

```bash
voxtype meeting list
voxtype meeting list --limit 5
voxtype meeting list --json
```

Lists recent meetings with their ID, title, date, duration, status, and chunk count. Defaults to showing the 10 most recent. Meetings are sorted by start time, newest first.

### Viewing Meeting Details

```bash
voxtype meeting show latest
voxtype meeting show <meeting-id>
voxtype meeting show latest --json
```

Shows detailed information about a meeting: title, start/end times, duration, word count, number of chunks, speakers detected, and the transcription engine used.

You can use `latest` as a shorthand for the most recent meeting's ID. This works with all commands that take a meeting ID.

`--json` on `status`, `list`, and `show` (and `meeting_active`/`meeting_status` in `voxtype status --format json --extended`) are documented as an integration contract in [`INTEGRATIONS.md`](./INTEGRATIONS.md#meeting-mode), along with the `meeting.sock` NDJSON socket that streams the live transcript and status for building a HUD or bar widget.

### Exporting Transcripts

```bash
# Markdown to stdout
voxtype meeting export latest

# Plain text to a file
voxtype meeting export latest --format text --output meeting.txt

# JSON with timestamps and speaker labels
voxtype meeting export latest --format json --timestamps --speakers

# Subtitle formats
voxtype meeting export latest --format srt --output meeting.srt
voxtype meeting export latest --format vtt --output meeting.vtt
```

**Supported formats:**

| Format | Flag | Description |
|--------|------|-------------|
| Markdown | `markdown` or `md` | Default. Readable with headers and speaker labels. |
| Plain text | `text` or `txt` | Just the words, no formatting. |
| JSON | `json` | Structured data with all segment metadata. |
| SRT | `srt` | SubRip subtitle format. |
| VTT | `vtt` | WebVTT subtitle format. |

**Export options:**

| Flag | Description |
|------|-------------|
| `--format`, `-f` | Output format (default: markdown) |
| `--output`, `-o` | Write to file instead of stdout |
| `--timestamps` | Include timestamps in output |
| `--speakers` | Include speaker labels |
| `--metadata` | Include a metadata header (title, date, duration) |

Every format lists the conversation in the order it was spoken: your lines and remote lines are interleaved by start time, and each line carries its own timestamp. Within each chunk the daemon transcribes every utterance separately, splitting at pauses of 0.7 seconds or more, so a line's timestamp is when that utterance began rather than when its chunk did.

### Labeling Speakers

Every transcript names its speakers. With simple diarization they are `You` (your microphone) and `Remote` (system audio); with ML diarization they are auto-generated IDs like `SPEAKER_00`, `SPEAKER_01`, etc.; with hybrid diarization they are `You` plus `Remote`, or `Remote 1`, `Remote 2` and so on once a second remote voice has spoken. You can replace any of them with real names:

```bash
voxtype meeting label latest You "Pete"
voxtype meeting label latest Remote "Dana"
voxtype meeting label latest "Remote 2" "Sam"
voxtype meeting label latest SPEAKER_00 "Alice"
voxtype meeting label latest 1 "Bob"
```

The speaker can be the ID as the transcript shows it (matched without regard to case), just the number for an ML speaker (`0` for `SPEAKER_00`), or a name you already assigned, which renames every speaker carrying it. A speaker the meeting does not contain is an error that lists the speakers it does contain, so a typo never reports success.

Labels are saved to the database and applied to the transcript, so subsequent exports use the names you assigned. You can label speakers while the meeting is still recording: the name sticks for the rest of the meeting and appears on segments streamed to the meeting socket from then on. Giving two speakers the same name merges them: exports show one speaker, and consecutive lines from either are grouped under one heading.

### Moving a Line to Another Speaker

When the diarizer gives a single line to the wrong person, move that line on its own:

```bash
voxtype meeting show latest --json          # find the line's "id"
voxtype meeting reassign latest 28 "Remote 2"
voxtype meeting reassign latest 28 Dana      # a name you gave with `meeting label`
voxtype meeting reassign latest 28 "Casey"   # nobody yet: creates the speaker Casey
voxtype meeting reassign latest 28 --reset   # back to what the diarizer chose
```

The meeting can be its full ID, the start of it (`f82f0c11`), or `latest`. The line is the segment's `id` from `meeting show --json`; an ID the meeting does not have is an error that names the valid range. The speaker is matched the way `meeting label` matches it: an ID from the transcript (`You`, `Remote 1`, `SPEAKER_00` or just `0`) or a name already given, ignoring case. Text that matches no one creates a new speaker with that name, under an internal ID of the form `Person 1` that no diarizer produces. Add `--json` to print the updated segment.

The move is stored per segment and applied whenever the transcript is saved or read, so exports, `meeting show` and the meeting socket all show it, and neither later chunks nor diarization can move the line back. If the diarizer renames the speaker you moved the line to (hybrid's `Remote` becoming `Remote 1`), the line follows. The segment keeps the diarizer's own choice in `original_speaker`, which is what `--reset` restores.

This works during a meeting too. The command stores the move and the daemon applies it before the next chunk, sending the changed line to the meeting socket.

### Notes

Each meeting has free-text notes, kept as `notes.md` in the meeting's folder. You can write them during the meeting or after it; the daemon never overwrites them.

```bash
voxtype meeting notes latest                         # print them
voxtype meeting notes latest --append "Ask Dana about travel"
voxtype meeting notes latest --set-file agenda.md    # replace them ("-" reads stdin)
voxtype meeting notes latest --json                  # {meeting_id, notes, updated_at}
```

Exports and `transcript.md` show the notes at the top, under "Notes".

### Annotating a Line

Attach a note or a highlight to one transcript line, by the segment's `id` from `meeting show --json`:

```bash
voxtype meeting annotate latest 28 "Check this number"
voxtype meeting annotate latest 28 "Decision" --highlight
voxtype meeting annotate latest 28 --highlight       # highlight without a note
voxtype meeting annotate latest 28 --remove
```

A line has at most one annotation; annotating it again replaces it. Annotations are stored beside speaker moves and applied whenever the transcript is saved or read, so later chunks cannot remove them. Exports show each one as a quote under its line (`> Note: ...`, or `> **Highlight: ...**`), and `meeting show --json` and the meeting socket carry it on the segment as `"annotation": {"text": "...", "highlight": true}`. Both commands work during a meeting.

### The Meeting Folder

Each meeting folder holds `metadata.json`, `transcript.json` (the segments, with speaker names, moves and annotations applied), `transcript.md` (the same as `meeting export --timestamps --speakers --metadata` in Markdown, rewritten after every chunk and every change), and `notes.md` once you write notes.

### AI Summarization

Generate a summary with key points, action items, and decisions:

```bash
# Markdown summary to stdout
voxtype meeting summarize latest

# JSON format
voxtype meeting summarize latest --format json

# Save to file
voxtype meeting summarize latest --output summary.md
```

Summarization requires a configured backend. See [Summarization Settings](#summarization-settings) below.

The summary includes:
- A brief overview of the meeting
- Key discussion points
- Action items (with assignees when mentioned)
- Decisions made

### Deleting Meetings

```bash
voxtype meeting delete <meeting-id> --force
```

Permanently deletes the meeting record, transcript, and any associated audio files. The `--force` flag is required to confirm deletion.

---

## Configuration

All meeting settings live under the `[meeting]` section in `~/.config/voxtype/config.toml`.

### Basic Settings

```toml
[meeting]
# Enable meeting mode (required)
enabled = true

# Duration of each audio chunk in seconds (default: 30)
# Shorter chunks mean faster partial results but more processing overhead
chunk_duration_secs = 30

# Where to store meeting data (default: auto)
# "auto" uses ~/.local/share/voxtype/meetings/
storage_path = "auto"

# Keep raw audio files after transcription (default: false)
# Enable if you want to re-transcribe with a different model later
retain_audio = false

# Maximum meeting duration in minutes (default: 180, 0 = unlimited)
max_duration_mins = 180
```

### Audio Settings

```toml
[meeting.audio]
# Microphone device (default: "default", uses your main audio device)
mic_device = "default"

# Loopback device for capturing remote participants' audio
# "auto" = auto-detect, "disabled" = mic only, or a specific device name
loopback_device = "auto"

# RMS threshold for meeting voice activity detection (default: 0.01)
# Lower to 0.001 for quiet mics; set 0.0 to disable this pre-transcription gate
vad_threshold = 0.01
```

Setting `loopback_device = "auto"` lets voxtype capture system audio (the other side of a call). When loopback is active, speaker attribution can distinguish between "You" (from the mic) and "Remote" (from system audio).

Set `loopback_device = "disabled"` if you only want to capture your own microphone, or if loopback detection is causing problems.

### Diarization Settings

Speaker diarization identifies who said what in the transcript.

```toml
[meeting.diarization]
# Enable speaker diarization (default: true)
enabled = true

# Backend: "simple", "ml", "hybrid", or "subprocess" (default: "simple")
backend = "simple"

# Maximum speakers to detect (default: 10)
max_speakers = 10
```

**Backends:**

- **simple**: Uses audio source (mic vs loopback) to attribute speech as "You" or "Remote". No ML model needed.
- **ml**: Uses ONNX-based speaker embeddings (ECAPA-TDNN) to identify individual remote speakers. The model is downloaded automatically on first use. **Experimental:** speaker clustering works best with longer speech segments; short segments may produce too many unique speaker IDs.
- **hybrid**: Your microphone is always `You`, and speaker clustering runs only over the system audio to tell the remote participants apart. A call with one remote voice shows `Remote`, as with `simple`. When a second voice speaks, the first becomes `Remote 1` (in segments already saved and on the live meeting socket, and any name you gave `Remote` moves with it) and the new one `Remote 2`. Because the microphone is never clustered, your own voice cannot be split into several speakers and echo of the call on your microphone cannot turn into a speaker. Needs the same `ml-diarization` feature and ECAPA-TDNN model as `ml`; without them it falls back to `simple` and logs a warning.
- **subprocess**: Same as `ml` but runs in a separate process for memory isolation.

For most users, `simple` is sufficient. Use `hybrid` if you need to distinguish between multiple remote participants on a call, and `ml` for a recording where everyone is in the same room on one microphone.

`similarity_threshold`, `min_segment_ms` and the `vad_*` settings tune the clustering for both `ml` and `hybrid`; see the [configuration reference](CONFIGURATION.md#meetingdiarization). If one remote voice is split into `Remote 1` and `Remote 2`, lower `similarity_threshold`; if two remote voices share one name, raise it.

To try hybrid for a single meeting without changing your config:

```bash
voxtype meeting start --diarization hybrid
```

### Summarization Settings

```toml
[meeting.summary]
# Backend: "local", "remote", or "disabled" (default: "disabled")
backend = "local"

# Ollama settings (for local backend)
ollama_url = "http://localhost:11434"
ollama_model = "llama3.2"

# Remote API settings (for remote backend)
# remote_endpoint = "https://api.example.com/summarize"
# remote_api_key = "your-api-key"

# Request timeout in seconds (default: 120)
timeout_secs = 120
```

**Using Ollama for local summarization:**

1. Install Ollama: https://ollama.ai
2. Pull a model: `ollama pull llama3.2`
3. Set `backend = "local"` in config
4. Run `voxtype meeting summarize latest`

Ollama runs entirely on your machine. No transcript data leaves your computer. Any Ollama-compatible model works, but `llama3.2` is a good default for meeting summarization.

---

## Storage

Meeting data is stored at `~/.local/share/voxtype/meetings/` by default (or the path you set in `storage_path`).

Each meeting gets its own directory named by date and title:

```
~/.local/share/voxtype/meetings/
  index.db                          # SQLite database with meeting metadata
  2026-02-16-weekly-standup/
    metadata.json                   # Meeting metadata
    transcript.json                 # Full transcript with segments
  2026-02-14-project-kickoff/
    metadata.json
    transcript.json
```

The `index.db` SQLite database stores meeting metadata for fast listing and lookup. Transcripts are stored as JSON files alongside the metadata for easy access and portability.

---

## Tips for Best Results

**Choose the right model.** Meeting transcription processes many audio chunks, so model choice affects both speed and accuracy. A fast model like `base.en` keeps up with real-time audio on most hardware. Larger models like `large-v3-turbo` are more accurate but need a capable GPU to keep pace with 30-second chunks.

**Use a good microphone.** Transcription accuracy depends heavily on audio quality. A dedicated microphone or headset works much better than a laptop's built-in mic, especially in rooms with echo or background noise.

**Set chunk duration appropriately.** The default 30 seconds works well for most cases. Shorter chunks (15-20s) give faster partial results but increase processing overhead. Longer chunks (45-60s) can improve accuracy for slower hardware since the transcription engine has more context.

**Label speakers after the meeting.** Run `voxtype meeting list` to find the meeting ID, then use `voxtype meeting label` to assign names to auto-detected speaker IDs. This makes the exported transcript much more readable.

**Export in multiple formats.** You can export the same meeting in different formats for different purposes: markdown for reading, JSON for processing in other tools, SRT/VTT for adding subtitles to a recording.
