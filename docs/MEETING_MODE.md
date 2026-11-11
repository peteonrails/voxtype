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
  - [Asking About a Meeting](#asking-about-a-meeting)
  - [AI Summarization](#ai-summarization)
  - [The Meeting Assistant and Consent](#the-meeting-assistant-and-consent)
  - [Deleting Meetings](#deleting-meetings)
- [Live Meeting View](#live-meeting-view)
- [Meeting Detection](#meeting-detection)
- [Configuration](#configuration)
  - [Basic Settings](#basic-settings)
  - [Audio Settings](#audio-settings)
  - [Diarization Settings](#diarization-settings)
  - [Summarization Settings](#summarization-settings)
  - [Assistant Settings](#assistant-settings)
- [Privacy: Where Transcripts Go](#privacy-where-transcripts-go)
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

1. Enable meeting mode. A config written by `voxtype setup` for a new install already has it on. On an existing install, choose **Turn on meetings...** in the tray, or add to your config and restart the daemon:

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

On Linux, the system tray menu has the same controls: **Start Meeting** (with the default title), then **Stop Meeting** and **Pause Meeting** / **Resume Meeting** while one runs.

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

### Asking About a Meeting

Ask a question about a meeting while it is recording or after it ends. The answer streams to the terminal and cites the `[mm:ss]` of the lines it relies on:

```bash
voxtype meeting ask latest "What did Steve just say?" --last 2m
voxtype meeting ask latest "Summarize the last few minutes" --last 5m
voxtype meeting ask latest "What are the action items so far?"
```

The assistant gets the transcript as `meeting show` displays it (speaker names you gave, lines moved with `meeting reassign`, and your annotations next to their lines), your meeting notes, and the question. `--last` limits it to the end of the transcript: `5m`, `90s`, `1h`, `2m30s`, or a bare number of minutes. It is a time window, so "the last few minutes" means minutes, not a number of words.

A meeting longer than `max_transcript_chars` is sent as a running summary of its earlier part plus as many recent lines as fit. The running summary is kept in the meeting folder (`assistant-context.json`), so asking again during a meeting only summarizes what was said since.

Every question and answer is saved to `asks.jsonl` in the meeting folder, and `voxtype meeting show <id> --json` includes them as `asks`.

Other options:

| Option | Effect |
|--------|--------|
| `--json` | One JSON event per line: `start`, `delta` while the answer streams, then `answer` (the saved record) or `error` |
| `--notify` | Also show the answer as a desktop notification. Its Open action raises the meeting view on the Ask tab (`[meeting.assistant] open_command` replaces that; `""` removes it) |
| `--agent claude` | Use this agent for this question |
| `--local` | Answer with the local model (Ollama) |

`ask` does not need the daemon: it reads the same saved transcript `meeting show` does.

### AI Summarization

Generate a summary with an overview, key points, decisions, action items with owners, and chapters with timestamps:

```bash
# Markdown summary to stdout
voxtype meeting summarize latest

# JSON format
voxtype meeting summarize latest --format json

# Save to file
voxtype meeting summarize latest --output summary.md

# Write a new one after naming speakers or moving lines
voxtype meeting summarize latest --regenerate
```

The summary is saved with the meeting: `summary.md` in the meeting folder, `metadata.json`, and the meeting index, so it survives a restart. Running `summarize` again prints the saved summary; `--regenerate` writes a new one. Your notes go in as guidance, so the summary expands on what you wrote.

The summary comes from the [meeting assistant](#the-meeting-assistant-and-consent) unless `[meeting.summary] backend` is `"local"` or `"remote"`, which keep the older Ollama and API summarizers.

**Summary at stop.** Once you have allowed an agent, each meeting is summarized when it stops. This happens in the background after the transcript is saved: the meeting socket reports `summarizing` and then `summary_ready` (see [INTEGRATIONS.md](INTEGRATIONS.md)). If the summary fails, the meeting and its transcript are unaffected; the failure is logged and reported on the socket, and `voxtype meeting summarize <id> --regenerate` retries it. Set `[meeting.summary] auto = false` to summarize only by hand, or `auto = true` to summarize at stop with the local model or older summarizers too.

### The Meeting Assistant and Consent

`ask` and summaries go through your own coding agent, run headless with every tool switched off: it gets the transcript as text and returns text. It never gets a shell, file access or network tools in this path. Which agent:

1. `[meeting.assistant] agent` in config.
2. Otherwise the Omarchy default agent (`omarchy-default-agent`), when that command exists. Voxtype works the same without Omarchy.
3. Otherwise a supported agent CLI found on PATH: Claude Code or Pi. With both found and neither allowed yet, the consent prompt asks which to use.

| Agent | Support |
|-------|---------|
| Claude Code (`claude`) | Supported, no tools: `claude -p --tools "" --strict-mcp-config ...` |
| Codex (`codex`) | Supported when named in config or as the Omarchy default, and only with bubblewrap (`bwrap`) installed: `codex exec` with its shell and app tools disabled and a read-only sandbox, run inside a bwrap jail. Its command tool cannot be switched off, so the jail is what keeps it away from your files: it sees `/usr`, the codex install, a few `/etc` files for DNS and TLS, and `~/.codex/auth.json` read-only, and nothing else from your home. It is never picked from PATH on its own |
| Pi (`pi`) | Supported, no tools: `pi --print --no-tools --no-mcp --no-extensions --no-skills --no-context-files --no-session ...`, with the prompt attached as a file. Its answer arrives whole rather than streaming |
| OpenCode (`opencode`) | Not supported: `opencode run` has no flag that turns its tools off, and its agents allow shell commands and web fetches, so even a filesystem jail could not keep a transcript from leaving the machine |
| Hermes (`hermes`) | Not supported: its one-shot mode loads tools and auto-approves them, with no documented way to turn them all off |

The first time transcripts would go to an agent, voxtype asks once ("Send to Claude / Use a local model / Not now") and remembers the answer per agent. Without a terminal to ask on, `ask` and `summarize` refuse with a message naming the command that allows it:

```bash
voxtype meeting assistant status          # which agent, what was found, consent
voxtype meeting assistant allow claude    # allow transcripts to go to Claude
voxtype meeting assistant revoke claude   # stop
voxtype meeting assistant status --json   # for the meeting view and Settings panel
```

With no agent found, or with `local_only = true` or `--local`, answers and summaries come from the local Ollama model in `[meeting.summary]`.

### Deleting Meetings

```bash
voxtype meeting delete <meeting-id> --force
```

Permanently deletes the meeting record, transcript, and any associated audio files. The `--force` flag is required to confirm deletion.

---

## Live Meeting View

The meeting view follows a meeting as it happens and stays useful after it. It runs in two places with one implementation: the Voxtype plugin in the Omarchy shell, and voxtype's standalone Quickshell shell (`voxtype setup quickshell`, then `qs -p ~/.local/share/voxtype/quickshell`). It opens by itself when a meeting starts, whether from `voxtype meeting start`, the detection prompt's **Record** button, or the tray, and sits at the right edge of the screen without taking keyboard focus from your call.

The header shows the meeting title, how long it has been running, and whether it is recording, paused or finalizing, with **Pause**/**Resume** and **Stop**. Below it are four tabs:

- **Transcript**: one line per utterance with its timestamp and speaker, in the order it was spoken. It scrolls to each new line unless you have scrolled up; **Jump to latest** takes you back.
  - A chip for each speaker. Click one to name the speaker, which runs `voxtype meeting label` for you. Give two speakers the same name and they become one chip.
  - Click the speaker name on a line to move just that line: pick one of the meeting's speakers, type a new name, or, for a line already moved, **Restore original**. This runs `voxtype meeting reassign`; if it is refused, the line goes back and the error is shown.
  - Hover a line for **Add note** (`voxtype meeting annotate`): a note, a **Highlight**, or both. Lines with either get a marker in the margin.
- **Notes**: your notes for the meeting. They save as you type and go into the summary as guidance.
- **Ask**: ask about what has been said, or use a quick question: "Last 5 minutes", "What did *the last remote speaker* just say?", "Action items so far". Answers stream in, say which agent gave them, and cite times like [1:58] that jump to that line.
- **Summary**, once the meeting stops: overview, decisions, action items and chapters, "Summarizing…" while it is written, and **Regenerate** after you fix speakers or lines.

Ask and Summary show the agent in use ("Claude", or "Local (llama3.2)"); click it to pick another agent, switch to **Local model only**, or (in Omarchy) open the settings. The first time an agent is needed the view asks whether to send meeting transcripts to it, with **Use a local model instead** and **Not now**.

Between updates the view says "Listening…": lines arrive once per chunk (`chunk_duration_secs`, 30 seconds by default), not word by word.

When the meeting ends its transcript stays on screen marked **Ended**, with its length and end time, so you can still fix lines, name speakers, take notes and ask questions after the call. It stays until a new meeting starts (**Start meeting** is offered there too), you go **‹ Meetings** back to the list, or you close the view.

With no meeting in progress the view lists your recent meetings, newest first: title, date and time, length, how many speakers, and a **Summary** badge on those that have one, under **Start meeting** and an optional title. Pick one to open it the same way as an ended meeting, with its transcript, speakers and summary, and fix it the same way. A meeting with no summary yet has **Summarize** on its Summary tab. The list shows 50 meetings at first; **Show older meetings** loads more. A meeting that starts while you are reading an old one takes over the view, and once it ends it is at the top of the list.

Open it yourself, in whichever shell is running:

```bash
voxtype meeting view                      # the current or last meeting
voxtype meeting view latest --tab summary
voxtype meeting view --ask last-5m --no-open   # answer arrives as a notification
```

In Omarchy, `omarchy-shell io.voxtype.settings meetings` works too. In the standalone shell, the `meeting-controls.flag` keybinding that `voxtype setup quickshell` prints toggles it.

An answer that arrives as a notification (a question asked while the view is closed) has an **Open** action that runs `[meeting.assistant] open_command`. `voxtype meeting view` is all it needs: it reads the meeting and the answer from `VOXTYPE_MEETING_ID` and `VOXTYPE_ASK_ID`, which the notification sets, and the view opens on that answer in whichever shell is running.

```toml
[meeting.assistant]
open_command = "voxtype meeting view"
```

To stop it opening on its own:

```bash
voxtype config set meeting.show_view_on_start false
```

The view reads the daemon's meeting socket, described in [INTEGRATIONS.md](./INTEGRATIONS.md#live-transcript-and-status-meetingsock). It does not open over an open Voxtype Settings panel.

---

## Meeting Detection

Voxtype can notice a call starting and offer to record it. When Zoom, Microsoft Teams, or Google Meet or Teams in a browser takes the microphone, a notification asks "Record ... as a meeting?" with **Record** and **Dismiss**. Clicking the notification itself also records. Record starts the meeting exactly as `voxtype meeting start --title` would, titled from the call window (for example "Meet - abc-defg-hij").

Detection is on by default on Omarchy and off elsewhere. It only runs while meeting mode is enabled:

```toml
[meeting]
enabled = true

[meeting.detect]
enabled = true          # unset: on for Omarchy, off elsewhere
auto_record_apps = []   # e.g. ["Zoom"] to record Zoom calls without asking
debounce_secs = 5
```

The setting is read when the daemon starts, so restart it after changing it.

### How calls are recognized

- **The microphone.** A call app opening a PipeWire capture stream is the main signal. Native clients (Zoom, `teams-for-linux`) count on that alone.
- **The window title.** A browser opens the microphone for many things, so a browser's stream only counts when a window title confirms a call: "Meet - ..." or "Google Meet" for Meet, "| Microsoft Teams" for Teams on the web. Titles come from Hyprland; on other compositors browser calls are not detected.
- **Voxtype's own capture never counts**, nor do other dictation apps or the camera-effects virtual microphone.

### When it prompts

- Once per call. The app must hold the microphone for `debounce_secs` (default 5) first, so a mic test or a permission check does not prompt.
- Not while a meeting is already recording.
- Not while you are dictating: the prompt waits until dictation ends.
- When the call app releases the microphone while a meeting is recording, a second notification offers **Stop meeting**. Voxtype never stops a meeting on its own.

Detection never records by itself, except for apps you list in `auto_record_apps`.

### Checking what it sees

```bash
# Read the current state once and print what would be prompted and why
voxtype meeting suggest --dry-run

# Keep watching and print each decision as calls start and end
voxtype meeting suggest --dry-run --watch
```

Neither shows a notification or starts anything. Example output during a Meet call in Chromium:

```text
Meeting detection in the daemon: on (default on Omarchy)
windows: 4 from Hyprland
mic streams: 3
  ignored mic stream 105 (PipeWire ALSA [hex]): another dictation app
  ignored mic stream 129 (Camera Effects): camera-effects virtual mic
evidence: Chromium has the mic (stream 211) and window "Meet - abc-defg-hij - Chromium" matches
would prompt: Record "Meet - abc-defg-hij"? (Google Meet)
```

Without `--dry-run`, `voxtype meeting suggest` runs the detector in the foreground with real prompts. It refuses while the daemon is already prompting.

To add another call service, see `apps` under [`[meeting.detect]`](CONFIGURATION.md#meetingdetect).

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

# Open the live meeting view in the Omarchy shell when a meeting starts
# (default: true; no effect on other desktops)
show_view_on_start = true
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

`backend = "disabled"`, the default, does not turn summaries off for `meeting summarize`: it uses the meeting assistant. `auto` controls the summary at stop:

```toml
[meeting.summary]
# Summarize each meeting when it stops. Unset: on once an agent is allowed,
# off otherwise.
# auto = true
```

### Assistant Settings

```toml
[meeting.assistant]
# Agent for ask and summaries: "claude", "codex" or "pi".
# Unset: the Omarchy default agent, then claude or pi found on PATH.
# agent = "claude"

# Never send transcripts to an agent; use the local Ollama model.
local_only = false

# Seconds to wait for an answer or a summary (default: 180)
timeout_secs = 180

# Transcript characters sent verbatim (default: 100000). Longer meetings send
# a summary of the earlier part plus the most recent lines.
max_transcript_chars = 100000

# Command the Open action of an `ask --notify` notification runs, through
# sh -c, with VOXTYPE_MEETING_ID and VOXTYPE_ASK_ID set.
# Unset: open the meeting view on its Ask tab. "": no Open action.
# open_command = "..."
```

---

## Privacy: Where Transcripts Go

Recording and transcription stay on your machine. The assistant is the one part of meeting mode that can send a transcript elsewhere:

- **An agent** (`claude`, `codex`) sends the prompt to whichever account that CLI is signed in to, usually the vendor's cloud service, under that account's terms. That prompt holds the meeting title, date, speaker names, the transcript (or the part of it that fits, plus a summary of the rest), your notes and annotations, and your question. Nothing goes to an agent until you allow it for that agent, and `voxtype meeting assistant revoke <agent>` stops it.
- **The local model** (Ollama at `ollama_url`, `localhost` by default) keeps everything on your machine. `local_only = true` makes it the only option.
- The agent runs in an empty temporary directory with its tools off, so it does not read your files or project instructions, and its session is not saved (`--no-session-persistence` for Claude, `--ephemeral` for Codex). The prompt reaches it through a file in a private temporary directory that is removed when the agent exits.
- Codex keeps a command tool that cannot be turned off, and a transcript is untrusted text: anyone on the call can say something meant for the model. So codex runs in a bubblewrap jail where your home holds nothing but its `auth.json` (read-only), and without `bwrap` voxtype does not use codex at all.
- Consent is stored in `$XDG_STATE_HOME/voxtype/assistant-consent.json` (default `~/.local/state/voxtype/`).
- Answers are stored in the meeting folder (`asks.jsonl`, `summary.md`), next to the transcript, and deleted with it.

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
