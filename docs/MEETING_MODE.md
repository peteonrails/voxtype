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
```

Shows whether a meeting is active, paused, or idle, along with the meeting ID if one is in progress.

### Listing Past Meetings

```bash
voxtype meeting list
voxtype meeting list --limit 5
```

Lists recent meetings with their ID, title, date, duration, status, and chunk count. Defaults to showing the 10 most recent. Meetings are sorted by start time, newest first.

### Viewing Meeting Details

```bash
voxtype meeting show latest
voxtype meeting show <meeting-id>
```

Shows detailed information about a meeting: title, start/end times, duration, word count, number of chunks, speakers detected, and the transcription engine used.

You can use `latest` as a shorthand for the most recent meeting's ID. This works with all commands that take a meeting ID.

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

### Labeling Speakers

When diarization detects multiple speakers, they are assigned auto-generated IDs like `SPEAKER_00`, `SPEAKER_01`, etc. You can replace these with real names:

```bash
voxtype meeting label latest SPEAKER_00 "Alice"
voxtype meeting label latest 1 "Bob"
```

The speaker ID can be the full form (`SPEAKER_00`) or just the number (`0`). Labels are saved to the database and applied to the transcript, so subsequent exports will use the names you assigned.

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
```

Setting `loopback_device = "auto"` lets voxtype capture system audio (the other side of a call). When loopback is active, speaker attribution can distinguish between "You" (from the mic) and "Remote" (from system audio).

Set `loopback_device = "disabled"` if you only want to capture your own microphone, or if loopback detection is causing problems.

### Diarization Settings

Speaker diarization identifies who said what in the transcript.

```toml
[meeting.diarization]
# Enable speaker diarization (default: true)
enabled = true

# Backend: "simple", "ml", or "subprocess" (default: "simple")
backend = "simple"

# Maximum speakers to detect (default: 10)
max_speakers = 10
```

**Backends:**

- **simple**: Uses audio source (mic vs loopback) to attribute speech as "You" or "Remote". No ML model needed.
- **ml**: Uses ONNX-based speaker embeddings to identify individual speakers. Requires the `ml-diarization` feature and a downloaded model.
- **subprocess**: Same as `ml` but runs in a separate process for memory isolation.

For most users, `simple` is sufficient. Use `ml` if you need to distinguish between multiple remote participants.

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
