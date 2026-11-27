# Meeting Mode

Meeting mode provides continuous transcription for meetings, lectures, interviews, or any long-form audio capture. Unlike push-to-talk dictation (which transcribes short bursts when you hold a hotkey), meeting mode records continuously and processes audio in chunks, building a timestamped transcript you can export and search later.

## Table of Contents

- [When to Use Meeting Mode](#when-to-use-meeting-mode)
- [Recording Consent](#recording-consent)
  - [Announcing the Recording](#announcing-the-recording)
- [Quick Start](#quick-start)
- [Recording a Meeting](#recording-a-meeting)
  - [Starting a Meeting](#starting-a-meeting)
  - [Stopping a Meeting](#stopping-a-meeting)
  - [Pausing and Resuming](#pausing-and-resuming)
  - [Checking Status](#checking-status)
- [Live Meeting View](#live-meeting-view)
  - [Watching a Meeting in the Terminal](#watching-a-meeting-in-the-terminal)
- [Meeting Detection](#meeting-detection)
  - [How calls are recognized](#how-calls-are-recognized)
  - [When it prompts](#when-it-prompts)
  - [When a call is over](#when-a-call-is-over)
  - [Titles from Your Calendar](#titles-from-your-calendar)
  - [Checking what it sees](#checking-what-it-sees)
- [Working with Transcripts](#working-with-transcripts)
  - [Listing Past Meetings](#listing-past-meetings)
  - [Viewing a Meeting](#viewing-a-meeting)
  - [Retitling a Meeting](#retitling-a-meeting)
  - [Labeling Speakers](#labeling-speakers)
  - [Your Name](#your-name)
  - [Searching Meetings](#searching-meetings)
  - [Moving a Line to Another Speaker](#moving-a-line-to-another-speaker)
  - [Correcting and Deleting Lines](#correcting-and-deleting-lines)
  - [Annotating a Line](#annotating-a-line)
  - [Notes](#notes)
  - [Watching a Meeting in the Terminal](#watching-a-meeting-in-the-terminal)
  - [Exporting Transcripts](#exporting-transcripts)
  - [Sharing a Meeting](#sharing-a-meeting)
  - [Deleting Meetings](#deleting-meetings)
  - [Recovering an Interrupted Meeting](#recovering-an-interrupted-meeting)
- [Meeting Audio](#meeting-audio)
  - [Checking the Audio](#checking-the-audio)
  - [Keeping the Audio](#keeping-the-audio)
  - [Playing a Meeting Back](#playing-a-meeting-back)
  - [Importing a Recording](#importing-a-recording)
  - [Transcribing a Meeting Again](#transcribing-a-meeting-again)
- [The Meeting Assistant](#the-meeting-assistant)
  - [Asking About a Meeting](#asking-about-a-meeting)
  - [AI Summarization](#ai-summarization)
  - [Action Items](#action-items)
  - [The Meeting Assistant and Consent](#the-meeting-assistant-and-consent)
  - [Privacy: Where Transcripts Go](#privacy-where-transcripts-go)
- [Automation](#automation)
  - [Running a Command After a Meeting](#running-a-command-after-a-meeting)
  - [Integrations](#integrations)
- [Configuration](#configuration)
  - [Basic Settings](#basic-settings)
  - [Audio Settings](#audio-settings)
  - [Echo Cancellation](#echo-cancellation)
  - [Echo Guard](#echo-guard)
  - [Diarization Settings](#diarization-settings)
  - [Summarization Settings](#summarization-settings)
  - [Assistant Settings](#assistant-settings)
- [Storage](#storage)
- [Tips for Best Results](#tips-for-best-results)
- [Troubleshooting](#troubleshooting)

---

## When to Use Meeting Mode

Use meeting mode when you need a transcript of a longer recording session. Good use cases:

- Video calls and meetings (Zoom, Teams, Google Meet)
- Lectures and presentations
- Interviews
- Brainstorming sessions

For short dictation (a sentence or two at a time), stick with the normal push-to-talk workflow.

---

## Recording Consent

Laws on recording and transcribing conversations differ by place, and some require everyone on the call to agree. Voxtype doesn't tell the other people for you, so let them know you're using a meeting assistant.

With the [recording announcement](#announcing-the-recording) on, the notice reads instead: "Voxtype will announce the recording to the call. You're still responsible for getting consent where the law requires it." If the announcement then plays only on your speakers, the meeting view and `meeting status` say the call did not hear it.

You are responsible for following the rules that apply to you.

Until you acknowledge this, voxtype reminds you:

- `voxtype meeting start` prints it on stderr, starting with `Note:`. It never waits for an answer, and stdout stays the same for scripts.
- The meeting view shows it at the top of a running meeting, with **Got it**.
- The call-detection prompt always adds "Let the others know you're transcribing."

```bash
voxtype meeting notice          # show it, and whether you acknowledged it
voxtype meeting notice --ack    # acknowledge it; the reminders stop
voxtype meeting notice --json   # {"notice", "announce_recording", "acknowledged", "acknowledged_at"}
```

**Got it** in the view does the same as `--ack`. The acknowledgement is kept in `~/.local/state/voxtype/recording-notice.json`.

### Announcing the Recording

Voxtype can say it for you, once, when a meeting starts: "This meeting is being recorded and transcribed with Voxtype." It is off by default.

```toml
[meeting]
announce_recording = true
```

Or for one meeting, whatever the config says:

```bash
voxtype meeting start --announce
voxtype meeting start --no-announce
```

`VOXTYPE_MEETING_ANNOUNCE_RECORDING=true` turns it on too, and Settings > Meeting has an **Announce recording** switch. `meeting start` reads the setting each time, so a change applies to the next meeting without restarting the daemon.

**The announcement is a courtesy, not legal compliance.** It does not ask anyone for consent, record that they gave it, or wait for them to agree, and a muted mic in the call app silences it like any other sound. You remain responsible for the laws on recording calls where you and the other people are.

#### How the others hear it

Playing it on your speakers would not reach the call: headphones keep it in, and the call app's echo cancellation removes speaker sound from your mic. So voxtype plays it into the call app's own microphone stream. PipeWire mixes it with your real mic for that app only, and nothing else that records the mic hears it. It also plays on your speakers so you hear it.

The call app is found the way [Meeting Detection](#meeting-detection) finds it: Zoom or Teams by their mic stream, a call in a browser by the mic stream plus a call window (Hyprland only): a window title for Meet and Teams, the web client's own window for Zoom. So a meeting you start by hand in the middle of a Zoom call reaches the call whatever tab is in front. For Meet and Teams the call's tab has to be in front when the announcement looks for it. When a meeting starts before the call does, voxtype waits up to two minutes for a call app to open the mic. It announces once the app has kept the same microphone stream for five seconds. Call apps open the mic on their pre-join screen, where nobody on the call hears anything, and open a new stream when you join, so an announcement played at the first sign of the mic was over before you were in the meeting. A meeting that call detection starts already has the call, and announces after the same five seconds. If you stay on a pre-join screen with the mic open for longer than that, the announcement still plays there.

When it cannot reach the call (no call app opened the mic in time, a browser call off Hyprland, PipeWire's tools missing, or the link failed), it plays on your speakers only, logs why, and says so:

```
$ voxtype meeting status
Meeting Status: recording
Meeting ID: ...
Announcement: played on your speakers only; the call did not hear it (no call app opened the microphone within 120 s)
```

When it reached the call, the line reads `Announcement: played into Zoom's microphone and on your speakers`. `meeting status --json` has the same as `announcement` (`state` is `waiting`, `call`, `local` or `failed`), and meeting.sock sends it as an `announcement` event and in `hello`.

The announcement comes back through the loopback, and maybe the mic, so the meeting keeps it out of the transcript and the summary: lines in the seconds it played that say its words lose them.

#### How loud

By default the others hear the announcement at half the clip's full level. At full level, the only level in earlier releases, the other people on a call found it too loud. Set it with `announcement_volume`, from 0.0 (silent) to 1.0 (full level):

```toml
[meeting]
announcement_volume = 0.5   # default
```

The number is a linear gain on the clip, passed to `pw-play --volume`: 0.5 is about 6 dB down. Hearing is not linear, so 0.5 still sounds a bit more than half as loud; about 0.3 (10 dB down) sounds roughly half as loud. Voxtype sets the level on its own stream into the call, so a volume your mixer remembers for `pw-play` does not change what the call hears.

For one meeting, `voxtype meeting start --announcement-volume 0.3`; `VOXTYPE_MEETING_ANNOUNCEMENT_VOLUME` and Settings > Meeting set it too. A value outside 0.0 to 1.0 is refused by the flag, the environment variable and `voxtype config set`, and clamped if it is written into the config file by hand. `voxtype meeting announcement` shows the volume in use.

The copy on your own speakers is not affected. It plays like any other sound, at the volume your mixer gives `pw-play`, so turn that down in your mixer if it is too loud for you.

#### Your own wording or language

Record your own clip (WAV, FLAC or Ogg) and point voxtype at it:

```toml
[meeting]
announce_recording = true
announcement_file = "~/.config/voxtype/announcement-de.wav"
```

`--announcement-file` and `VOXTYPE_MEETING_ANNOUNCEMENT_FILE` do the same. A file that does not exist refuses the start, with the fix. Voxtype does not know what your clip says, so it keeps out of the transcript any line that falls mostly within the time the clip played.

#### The voice

The built-in clip is AI-generated speech in the Kokoro-82M voice `af_heart`. It is in the voxtype binary, plays offline, and is MIT like the rest of voxtype. Voxtype never calls a speech service for it.

You can switch to an ElevenLabs voice, Jessica, instead:

```toml
[meeting]
announcement_voice = "jessica"   # "kokoro" (default) or "jessica"
```

Jessica is not part of voxtype, so it is downloaded once, from voxtype's models server, the first time a meeting wants it. Voxtype checks it against its published sha256 and keeps it in `~/.local/share/voxtype/sounds/`. Its terms are not MIT: it may be redistributed only unmodified, as part of voxtype, and it may not be used to train or improve AI models. Voxtype shows them when it downloads, and writes them beside the clip.

To download it before your first meeting, and see what's in use:

```bash
voxtype meeting announcement --voice jessica
voxtype meeting announcement
```

A meeting that needs Jessica and doesn't have it yet waits up to 15 seconds for the download. If that fails (offline, a firewall, a damaged file), the built-in voice plays instead and the meeting goes on. `meeting status` says so: `Announcement: played into Zoom's microphone and on your speakers. Note: the Jessica voice could not be downloaded (...), so the built-in voice played`.

`--announcement-voice` on `meeting start` and `VOXTYPE_MEETING_ANNOUNCEMENT_VOICE` choose the voice too. Your own `announcement_file` beats both voices.

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

   The [meeting view](#live-meeting-view) opens at the right edge of the screen in the Omarchy shell or in voxtype's Quickshell shell, and shows the transcript as it arrives.

4. When the meeting ends:

```bash
voxtype meeting stop
```

5. Export the transcript:

```bash
voxtype meeting export latest --format markdown --output standup.md
```

---

## Recording a Meeting

### Starting a Meeting

```bash
voxtype meeting start
voxtype meeting start --title "Project Kickoff"
voxtype meeting start -t "1:1 with Alice"
```

The `--title` flag is optional. If omitted, the meeting is named by its date and time (e.g., "Meeting 2026-02-16 14:30"). The daemon must be running, and meeting mode must be enabled in config.

Only one meeting can run at a time. Starting a second meeting while one is active will fail.

On Linux, the system tray menu has the same controls: **Start Meeting** (with the default title), then **Stop Meeting** and **Pause Meeting** / **Resume Meeting** while one runs. **Meetings...** near the top of the menu opens the meeting view.

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
| `--announce` / `--no-announce` | `VOXTYPE_MEETING_ANNOUNCE_RECORDING` | Play the [recording announcement](#announcing-the-recording) for this meeting, or not. |
| `--announcement-file` | `VOXTYPE_MEETING_ANNOUNCEMENT_FILE` | Your own announcement clip for this meeting. |
| `--announcement-voice` | `VOXTYPE_MEETING_ANNOUNCEMENT_VOICE` | The built-in announcement's voice for this meeting: `kokoro` or `jessica`. |
| `--announcement-volume` | `VOXTYPE_MEETING_ANNOUNCEMENT_VOLUME` | How loud the call hears the announcement in this meeting, a linear gain from 0.0 to 1.0. |

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

Shows whether a meeting is recording, paused, finalizing, or idle, along with the meeting ID if one is in progress, and where the [recording announcement](#announcing-the-recording) got to when the meeting announces. The transcript keeps updating while a meeting is in progress, so `voxtype meeting show latest` is accurate mid-meeting too, not just after `stop`.

The daemon acts on meeting commands between audio chunks, so while it is transcribing a chunk a request can take a few seconds to land. Until it does, status reports the request itself: `starting`, `pausing`, `resuming` or `stopping`. A second `voxtype meeting start` is refused as soon as the first one is issued, not only once that meeting is recording.

---

## Live Meeting View

The meeting view follows a meeting as it happens and stays useful after it. It runs in two places with one implementation: the Voxtype plugin in the Omarchy shell, and voxtype's standalone Quickshell shell (`voxtype setup quickshell`, then `qs -p ~/.local/share/voxtype/quickshell`). When both are running, as on Omarchy with the Quickshell OSD, the plugin shows the view and the standalone shell leaves meetings to it, so you only ever see one. If omarchy-shell stops, the standalone shell takes over within about ten seconds. It opens by itself when a meeting starts, whether from `voxtype meeting start`, the detection prompt's **Record** button, or the tray, and sits at the right edge of the screen without taking keyboard focus from your call.

The header shows the meeting title (click it to rename the meeting: Enter saves with `voxtype meeting title`, Escape cancels; a rename made elsewhere, or a title taken from the calendar after the start, shows up at once; renaming a finished meeting to a two-person title such as "Pete <> Chad" shows the names on its lines right away), how long it has been running, and whether it is recording, paused or finalizing, with **Pause**/**Resume** and **Stop**. While it records, two small meters beside "Listening…" show your microphone (**You**) and, when the meeting records the call, the other side (**Call**), so a muted mic or a silent call shows up before the first chunk is transcribed. They empty while paused. Below it are four tabs:

- **Transcript**: one line per utterance with its timestamp and speaker, in the order it was spoken. It scrolls to each new line unless you have scrolled up; **Jump to latest** takes you back.
  - A chip for each speaker. Click one to name the speaker, which runs `voxtype meeting label` for you. Give two speakers the same name and they become one chip.
  - Every speaker has a colour: the dot on the chip, and the dot and the name on each of that speaker's lines. You are the theme's accent colour; the others get colours made from the theme, so they suit a light theme as well as a dark one and change with it. A speaker keeps the same colour for as long as the meeting is open, through a rename and through lines being moved to or away from them. Two speakers given the same name share the first one's colour. There are eight colours besides yours, and a ninth speaker shares the first's, so the names are always there to tell them apart.
  - Click the speaker name on a line to move just that line: pick one of the meeting's speakers, type a new name, or, for a line already moved, **Restore original**. This runs `voxtype meeting reassign`; if it is refused, the line goes back and the error is shown.
  - Hover a line for **▶**, **Edit**, **Add note** and **Delete**. ▶ plays the meeting's audio from that line (`voxtype meeting play --segment`); while it plays, the header shows **■ Stop** and the line being heard is lit and kept in view: when playback reaches a line and it, or the line after it, is not all on screen, the transcript moves to centre it. Scroll the transcript yourself (or follow a citation or a search result) and it stays where you put it; it follows the audio again when you next seek or start playback. A meeting that is still recording stays on its newest line instead, unless you have scrolled up. A meeting recorded without `[meeting] retain_audio = true` has no audio to play: the view says so once and offers no ▶.
  - **Edit** changes the line's text in place (Enter saves, Escape cancels; `voxtype meeting edit`); an edited line is marked "edited" and offers **Restore original text** (`voxtype meeting edit --reset`). A note can be text, a **Highlight**, or both (`voxtype meeting annotate`); lines with either get a marker in the margin. Delete removes the line at once and offers **Undo** for a few seconds before it runs `voxtype meeting delete-line`. An edit and a speaker move offer **Undo** for a few seconds too; it runs the opposite command (`edit --reset` or the earlier text, `reassign --reset` or the earlier speaker).
  - A line's time is a link when the meeting's audio was kept: it plays from that line, or moves a playback that is already running there. The transcript does not move for it, since the line is already on screen.
  - An ended meeting with its audio kept shows the whole meeting as a waveform under the header, your side above the line and the call below. Your side is the accent colour. Each bar of the call is in the colour of whoever was speaking then: the speaker whose transcript lines cover the most of that bar's stretch of time. A stretch no line covers stays the neutral colour. An import that kept one mixed track is a single row, coloured the same way for everyone in it. Hovering shows the time under the pointer and, where a line says so, who is speaking there: above the middle on your side, below it on the call. A click plays from there (`voxtype meeting play --at`) and shows that line of the transcript, centred and lit for a moment; press and drag and the transcript goes along, and playback starts where you let go. A playhead follows the audio, and the part already played is drawn stronger. The waveform needs `voxtype meeting waveform`; a voxtype without it shows no strip, and the lines still play.
- **Notes**: your notes for the meeting. They save as you type and go into the summary as guidance.
- **Ask**: ask about what has been said, or use a quick question: "Last 5 minutes", "What did *the last remote speaker* just say?", "Action items so far". Answers stream in, say which agent gave them, and cite times like [1:58] that jump to that line.
- **Summary**, once the meeting stops: overview, decisions, action items and chapters, "Summarizing with Claude…" (or whichever backend writes it) while it is written, and **Regenerate** after you fix speakers or lines. The agent chip in this tab's header names the backend summaries go to, which can differ from the one that answers questions when an older `[meeting.summary]` backend such as Ollama is configured. If the summary fails, a card at the top of the tab says why, with **Retry**, **Settings**, and, when a local model failed and you have allowed an agent, **Use Claude instead** (it runs `voxtype meeting summarize <id> --agent claude`).
  - Action items are rows of their own above the summary (`voxtype meeting actions`), each with its owner, the due date when one was said, and the time it was said, which jumps to that line. All are ticked to start with; untick any you don't want to send. **Send to Todoist** adds the ticked items as tasks (the first time, **Connect Todoist…** opens your browser to allow it), and **Remind me…** sets an Omarchy reminder in 15 minutes, an hour, or at the end of the day, when `omarchy-reminder` is installed. An item already sent shows ✓ and is not sent there twice.

**⋯** in the header exports the meeting as Markdown, plain text, JSON, SRT or WebVTT (`voxtype meeting export`) into `~/Documents/Voxtype/` (or `$XDG_DOCUMENTS_DIR/Voxtype/`), named by date and title, with **Open folder** once it is written. **Open meeting folder** shows the meeting's own files. All of this works during a meeting, after it, and on past meetings.

Ask and Summary show the agent in use ("Claude", or "Local (llama3.2)"); click it to pick another agent, switch to **Local model only**, or (in Omarchy) open the settings. The first time an agent is needed the view asks whether to send meeting transcripts to it, with **Use a local model instead** and **Not now**.

The card is an overlay: its background is translucent (`[meeting] view_opacity`, 0.8 by default) and turns opaque while the pointer is on it or you are typing in it. It never takes keyboard focus from your call unless you click into one of its text fields. Drag the header to move it anywhere; it snaps to a screen edge when dropped near one. Drag its inner edge to make it narrower or wider (280 px up to half the screen), and use **↥/↧** for full height or a compact 40 %. **–** shrinks it to a pill that shows the meeting's status: a red dot, the time and the newest line ("Dana: …") while recording, or the title until the first line arrives; "Paused" and the time; "Finishing…"; "Summarizing · Claude…"; "Ended · 21:03 · Summary ready" once the summary is in; or "No meeting" with a **Start** button. The pill is only as wide as what it says (up to 460 px, where a long line is cut short) and keeps the card's outer edge; click it to bring the card back. A meeting that starts while it is a pill expands it. Position and size are remembered per screen in `~/.local/state/voxtype/meeting-view.json`; **⋯ → Reset position and size** puts it back on the right edge. `voxtype meeting view --minimize` and `--expand` do the same from a keybinding.

Between updates the view says "Listening…": lines arrive once per chunk (`chunk_duration_secs`, 30 seconds by default), not word by word.

When the meeting ends its transcript stays on screen marked **Ended**, with its length and end time, so you can still fix lines, name speakers, take notes and ask questions after the call. It stays until a new meeting starts (**Start meeting** is offered there too), you go **‹ Meetings** back to the list, or you close the view.

With no meeting in progress the view lists your recent meetings, newest first: title, date and time, length, how many speakers, and a **Summary** badge on those that have one, under **Start meeting** and an optional title. Pick one to open it the same way as an ended meeting, with its transcript, speakers and summary, and fix it the same way. A meeting with no summary yet has **Summarize** on its Summary tab. The list shows 50 meetings at first; **Show older meetings** loads more. A meeting that starts while you are reading an old one takes over the view, and once it ends it is at the top of the list.

If voxtype stopped in the middle of a meeting (a crash, or the machine going down), the meeting is left unfinished; the daemon marks it interrupted when it next starts. The view shows an **Unfinished recording found** card for each one `voxtype meeting recover` lists, with where it stopped and what is left to transcribe. **Finish it** runs `voxtype meeting recover --finish <id>`, which transcribes what was left (when the audio was kept) and closes the meeting; **Discard** (after a second **Discard for good**) runs `voxtype meeting recover --discard <id>`. With a voxtype that has no `meeting recover`, the card says the meeting finishes the next time voxtype starts, and Discard deletes it.

**⋯ → Share…** sends the meeting's summary, notes and highlights somewhere (`voxtype meeting share`). The sheet shows exactly what would leave, what it includes, and a switch to add the transcript; nothing is sent until you pick where: **Copy** (the clipboard), **Save…** (a new file in `~/Documents/Voxtype`, with **Open folder**), **Email…** (a draft in your mail app; with the transcript, the file to attach), or the share command you configured with `meeting.share.command`. A destination that cannot work here is greyed, with what it needs.

The box above the list searches every meeting (`voxtype meeting search`, see [Searching Meetings](#searching-meetings)): transcript lines as you see them, titles, notes and summaries. Each result shows the meeting, the time and speaker of the line, and the matching words highlighted; picking one opens the meeting on that line. **Back** returns to the results, and Escape in the box clears it.

**Sound check…** under **Start meeting** checks the audio before a call: it runs `voxtype meeting soundcheck` for 30 seconds and shows a meter for your microphone and one for the call, the microphone it opened, and how echo is handled (PipeWire's WebRTC canceller, voxtype's GTCRN model, or none). Nothing is recorded. If no call audio is captured, the panel says so in place of the second meter. **Stop** ends it early, **Check again** runs it once more, and leaving the panel or closing the view stops it. A meeting that starts during the check takes the microphone, and the panel says so.

Open it yourself, in whichever shell is running:

```bash
voxtype meeting view                      # the current or last meeting
voxtype meeting view latest --tab summary
voxtype meeting view --ask last-5m --no-open   # answer arrives as a notification
```

`voxtype meeting view` opens the view that `[meeting] view` picks (see [Without Quickshell](#without-quickshell-the-meeting-window) below); on a Quickshell desktop that is this one.


In Omarchy, `omarchy-shell io.voxtype.settings meetings` works too. In the standalone shell, the `meeting-controls.flag` keybinding that `voxtype setup quickshell` prints toggles it; while the Omarchy plugin is running, that keybinding opens the plugin's view instead.

An answer that arrives as a notification (a question asked while the view is closed) has an **Open** action that runs `[meeting.assistant] open_command`. `voxtype meeting view` is all it needs: it reads the meeting and the answer from `VOXTYPE_MEETING_ID` and `VOXTYPE_ASK_ID`, which the notification sets, and the view opens on that answer in whichever shell is running.

```toml
[meeting.assistant]
open_command = "voxtype meeting view"
```

To stop it opening on its own:

```bash
voxtype config set meeting.show_view_on_start false
```

### Without Quickshell: the meeting window

On GNOME, KDE Plasma, Sway, X11 and other desktops without Quickshell, the meeting opens in an ordinary window, `voxtype-meetings-gtk4`, which you can move, resize and alt-tab to like any other. It opens by itself when a meeting starts (`show_view_on_start`, as above), and `voxtype meeting view` opens it or brings it forward. It shows the meeting as the Quickshell view's Transcript tab does:

- the state, title and elapsed time, with **Pause**/**Resume** and **Stop** (which asks first) in the title bar, and a level meter for your microphone and the call while the daemon sends levels. Click the title to rename the meeting (`voxtype meeting title`); a rename made elsewhere shows up at once. Once the daemon is done with a meeting, recording it and writing its summary, **Delete…** in the title bar deletes it (`voxtype meeting delete`, after asking);
- the transcript, one colored line per utterance in the order it was spoken, following new lines until you scroll up (**Follow new lines** goes back). A corrected line is marked "(edited)", a moved line says who the diarizer gave it to ("(was Remote 1)"), highlighted lines are tinted, notes sit under their lines, deleted lines disappear, and while the meeting's audio plays (`voxtype meeting play`) the line being heard has a ▶;
- a button per speaker that renames them (`voxtype meeting label`, once for each speaker merged under that name); **Move line to…** and **Note on line…** for the selected line; your **Notes** with **Save notes**; an **Ask** box whose answer streams in under it; and the **Summary** once it is written;
- with no meeting running, **Start meeting** with an optional title, and your recent meetings, each with a delete button that asks first. **Meetings** in the title bar goes back to that list.

The window has no waveform: the waveform, and with it the bars in each speaker's colour and the transcript following the audio, is in the Quickshell view only.

A past meeting opened from the list is read-only: its transcript, speakers and notes, and its summary as saved in `summary.md`. Ask still works on it, and its title can still be changed and the meeting deleted. To fix the rest of a past meeting, use the `voxtype meeting` commands, the Quickshell view or `voxtype meeting watch`. A meeting that starts while you are reading one takes over the window.

Every button runs the matching `voxtype meeting` command, so it behaves as the command line does. A second `voxtype meeting view` brings the open window forward instead of opening another. A window the daemon opened at the start of a meeting closes if the daemon is stopped or restarted (it runs as part of the voxtype service); one you opened yourself reconnects.

Choosing `gtk4` (or `terminal` or `none`) on a Quickshell desktop keeps the Quickshell view from opening on its own, and `voxtype meeting view` opens the chosen view instead.

Which view opens is `[meeting] view`:

| Value | Opens |
|-------|-------|
| `auto` (default) | The Quickshell view when Quickshell can run here, else the GTK4 window when it is installed and GTK 4 loads, else nothing |
| `quickshell` | The Quickshell view (the Omarchy plugin or the standalone shell) |
| `gtk4` | The GTK4 window |
| `terminal` | `voxtype meeting watch`, in this terminal or a new one through `xdg-terminal-exec` |
| `none` | Nothing |

```bash
voxtype config set meeting.view gtk4
VOXTYPE_MEETING_VIEW=terminal voxtype meeting view
```

The deb and rpm packages install the window as `/usr/lib/voxtype/voxtype-meetings-gtk4`; it needs GTK 4 (`libgtk-4-1` on Debian and Ubuntu, `gtk4` on Fedora and Arch) but not gtk4-layer-shell.

The view reads the daemon's meeting socket, described in [INTEGRATIONS.md](./INTEGRATIONS.md#the-live-feed-meetingsock). It does not open over an open Voxtype Settings panel.

### Watching a Meeting in the Terminal

`voxtype meeting watch` shows the running meeting in the terminal, on any desktop and over SSH:

```bash
voxtype meeting watch
```

- The header shows the state (recording, paused, finalizing, summarizing, ended), the title and the elapsed time.
- The transcript fills in as each chunk is transcribed, each line with its time and speaker in that speaker's color, and annotations under their lines. It follows new lines; scroll up (↑, PgUp, Home) to read back and it stops following, and End follows again.
- The side panel lists the speakers, merged by the names you gave them, and the summary once it is written.

| Key | Action |
|-----|--------|
| `p` | Pause or resume |
| `s` | Stop (asks first) |
| `r` | Move the selected line to another speaker: pick one, or "New speaker…" and type a name (`meeting reassign`) |
| `n` | Add a note to the selected line (`meeting annotate`) |
| `a` | Ask the assistant about the meeting; the answer streams into a pane at the bottom (`meeting ask`) |
| `Esc` | Close the answer, or go to the list of meetings |
| `q` | Quit |

Each key runs the matching `voxtype meeting` command, so it behaves exactly as typing it would, including the assistant's consent: if transcripts have not been allowed to go to an agent yet, the answer pane says how to allow it.

With no meeting running, `watch` lists recent meetings with their length, speaker count and whether they have a summary. Enter opens one (ended meetings still take `r`, `n` and `a`), Esc goes back, and a meeting that starts meanwhile appears at the top of the list. If the daemon restarts, the view says it is reconnecting and catches up when it is back.

For scripts, `voxtype meeting watch --json` prints the daemon's meeting socket as NDJSON (see [INTEGRATIONS.md](INTEGRATIONS.md#the-live-feed-meetingsock)), reconnecting on its own; every connection starts with a `hello` snapshot. `voxtype meeting watch --levels` prints the microphone and loopback levels instead, about ten times a second while a meeting records, for a level meter; add both flags for everything.

---

## Meeting Detection

Voxtype can notice a call starting and offer to record it. When Zoom, Microsoft Teams, or Google Meet or Teams in a browser takes the microphone, a notification asks "Record ... as a meeting?", followed by "Let the others know you're transcribing." (see [Recording Consent](#recording-consent)), with **Record** and **Dismiss**. Clicking the notification itself also records. Record starts the meeting as `voxtype meeting start --window-title` would, titled from the call window (and named from your calendar instead, when [it can be](#titles-from-your-calendar)) when it names the meeting ("Meet - Weekly sync" gives "Weekly sync", "Budget review | Microsoft Teams" gives "Budget review"). A window title that only names the app, such as "Join from Zoom Workplace app - Zoom", "Zoom Meeting" or a bare Meet code, gives "Zoom call", "Google Meet call" or "Teams call" instead. A Zoom call in the web client's own window is titled with that window's title, which is the meeting's topic.

Detection is on by default on Omarchy and off elsewhere. It only runs while meeting mode is enabled:

```toml
[meeting]
enabled = true

[meeting.detect]
enabled = true          # unset: on for Omarchy, off elsewhere
auto_record_apps = []   # e.g. ["Zoom"] to record Zoom calls without asking
debounce_secs = 5
on_call_end = "stop"    # when the call ends: "stop", "ask" or "keep"
stop_grace_secs = 20
```

The setting is read when the daemon starts, so restart it after changing it.

### How calls are recognized

- **The microphone.** A call app opening a PipeWire capture stream is the main signal. Native clients (Zoom, `teams-for-linux`) count on that alone.
- **The window.** A browser opens the microphone for many things, so a browser's stream only counts when a window confirms a call. Windows come from Hyprland; on other compositors browser calls are not detected.
  - **By its title:** "Meet - ..." or "Google Meet" for Meet, "| Microsoft Teams" for Teams on the web, "Zoom Meeting" or "Zoom Workplace" for Zoom on the web. A browser window's title is the title of its front tab, so the call's tab has to be in front for the call to be noticed this way.
  - **By its class, for the Zoom web client:** Omarchy opens a Zoom link in the web client's own app window, and the browser names that window after the address: its class is `chrome-app.zoom.us__wc_join_<meeting number>-Default`. Its title is the meeting's topic, which says nothing about Zoom, so the class is what finds it. With that window open and the browser on the microphone, it is a Zoom call, whatever tab is in front of the main browser window, and whether or not the "Join from Zoom Workplace app" tab that launched it is still there. The window with nobody on the microphone is not a call. The Zoom web app's home window is not counted: it is open whether or not you are in a meeting.
- **Voxtype's own capture never counts**, nor do other dictation apps or the camera-effects virtual microphone.

While the Zoom web client's window is open, anything in that browser that takes the microphone for `debounce_secs` counts as the Zoom call, so close the window when the call is over.

### When it prompts

- Once per call. The app must hold the microphone for `debounce_secs` (default 5) first, so a mic test or a permission check does not prompt. A call that lets go of the microphone and takes it again, or whose tab goes behind another and comes back, is still the same call: it is not offered a second time, and the recording is not announced again.
- Not while a meeting is already recording.
- Not while you are dictating: the prompt waits until dictation ends.

Detection never records by itself, except for apps you list in `auto_record_apps`.

### When a call is over

Once a call has been noticed, it goes on for as long as anything still shows it. It is over when nothing has for `debounce_secs`:

- **A call in a browser** is over when the browser has released the microphone and no window shows the call. One without the other is not the end:
  - The browser still has the microphone, but no window shows the call. You brought another tab to the front of that browser window, or closed the call's tab or window. The call goes on.
  - The browser released the microphone, but a window still shows the call. The call goes on, however long the microphone stays released.
  - Hyprland cannot be asked for its windows. Voxtype does not take that to mean the window is gone, and the call goes on until the windows can be read again.
- **A Zoom call in the web client's own window** is shown by that window alone once it has been seen. Tabs in the main browser window make no difference to it. When you leave the call and the web client's window closes, the call is over, even with the "Join from Zoom Workplace app" tab still in front: that tab's title is not the call.
- **A native client** (Zoom, `teams-for-linux`) has only its microphone stream. Its call is over when it releases the microphone, whatever windows it still has open.
- **A call back within `stop_grace_secs`** of ending (its app on the microphone again, or its window back) is the same call. A stop that was waiting is called off (with `"ask"`, the question is withdrawn), and nothing is offered again.

A call tab, or the Zoom web client's window, left open after you hang up keeps the call going by this rule, so the recording runs on until you close it, stop the meeting yourself, or it reaches `max_duration_mins`. Voxtype would rather record too long than stop in the middle of a call.

When the call is over while a meeting records, `on_call_end` decides:

- `"stop"` (default): a notification says **Call ended**, "Stopping the recording in 20 s.", with **Keep recording**. Unless you press it, the meeting then stops as `voxtype meeting stop` would, and the summary at stop runs as usual. `stop_grace_secs` sets the wait.
- `"ask"`: a notification offers **Stop meeting** and **Keep recording**, and nothing happens until you choose.
- `"keep"`: the meeting keeps recording.

A meeting you started by hand, with no call detected while it ran, is never stopped this way.

Each step is logged at info level, with what was seen. If the first command prints nothing, use `journalctl _SYSTEMD_USER_UNIT=voxtype.service | grep "Meeting detection"` ([the user journal has no voxtype entries](TROUBLESHOOTING.md#the-user-journal-has-no-voxtype-entries)):

```text
$ journalctl --user -u voxtype | grep "Meeting detection"
Meeting detection: call seen: Zoom (web) - Chromium input has the mic (stream 115) and its call window "Weekly sync" is open (class contains "zoom.us__wc_join_"); prompting if it lasts 5s
Meeting detection: call seen: Google Meet - Chromium has the mic (stream 211) and window "Meet - abc-defg-hij - Chromium" matches; prompting if it lasts 5s
Meeting detection: call continues: Google Meet still has the mic, but no window shows the call any more (another tab is in front, or its window was closed)
Meeting detection: call continues: Google Meet has the mic and its call window again
Meeting detection: call continues: Zoom (web) released the mic, but its call window is still open (muted, or reconnecting)
Meeting detection: call ended: Zoom (web) released the mic and no window shows the call any more while a meeting records ([meeting.detect] on_call_end decides)
Meeting detection: the Zoom (web) call ended; stopping the meeting in 20 s unless kept
Meeting detection: call is back: Zoom (web) is seen again within the grace period; the same call, so nothing is offered again
Meeting detection: not stopping the meeting: the call came back
```

### Titles from Your Calendar

A meeting that starts without a title you chose can take its name from your calendar. That covers a meeting started with no `--title`, one recorded from the detection prompt (whose title comes from the call window: "Weekly sync", or just "Zoom call", "Google Meet call" or "Teams call"), and the date-only default ("Meeting 2026-11-12 14:00"). A title you typed with `--title` or `voxtype meeting title` is never replaced.

The event used is the one in progress when the meeting starts, or starting within `[meeting.calendar] window_mins` (10 by default) before or after; with several, the one whose start is nearest. Its title becomes the meeting's, and its video call link (Zoom, Meet, Teams and the like) is stored when the meeting has none. Then a 1:1 title names the speakers as usual. The lookup runs alongside the recording and never delays the start: the title arrives a moment later, through the same path as [Retitling a Meeting](#retitling-a-meeting).

Voxtype asks these in order and takes the first answer:

1. **Your calendar's iCal link.** Works with any calendar that publishes one.
2. **A calendar command**, `[meeting.calendar] command`.
3. **The Omarchy Next Meeting widget**, when it is installed and on your bar and nothing above is set up.

Without any of them, the call window's title or the default stays.

#### The iCal link

Google Calendar, Outlook, iCloud and Fastmail all give each calendar a private link that serves it in iCalendar format. Set it once:

```bash
voxtype meeting calendar set-ics -        # paste the link, then Enter
voxtype meeting calendar status           # where titles come from, the next event
voxtype meeting calendar clear            # forget the link and what was fetched
```

In Omarchy you can also set it from Settings: the Meeting section has a **Calendar (iCal link)** row above the calendar settings. Paste the link and press Set; the field is masked and empties once the link is saved, and the row then shows the server, when it was last fetched, the next event and any fetch error, with Clear to forget the link. "Where to find it" shows the steps below.

`-` reads the link from the terminal, which keeps it out of your shell history; `set-ics <link>` works too. `set-ics` fetches the link once to check it, and refuses one that does not return a calendar (`--no-check` saves it anyway). `voxtype meeting calendar set-ics --help` says where each service keeps the link:

- **Google Calendar:** Settings, your calendar under "Settings for my calendars", Integrate calendar, then "Secret address in iCal format".
- **Outlook:** Settings, Calendar, Shared calendars, Publish a calendar. Pick the calendar and "Can view all details", Publish, then copy the ICS link.
- **iCloud:** on iCloud.com, the share button beside the calendar, tick "Public Calendar", then copy the `webcal://` link.
- **Fastmail:** Settings, Calendars, the calendar's Edit & share, then its read-only iCalendar URL.

**The link lets anyone who has it read your calendar.** Voxtype keeps it in `~/.local/state/voxtype/calendar.json`, readable only by you, never in `config.toml`, and never writes it to a log: messages name only the server (`calendar.google.com`). If the link leaks, reset the secret address in your calendar's settings and set the new one.

While meeting mode is on, the daemon fetches the link when it starts and every 15 minutes after, asking the server whether anything changed when the server supports that, and keeps the events of the next 48 hours (also readable only by you). Repeating events are expanded, including skipped and moved occurrences, times follow each event's time zone, and all-day and cancelled events are left out. A fetch that fails keeps the events from the last good one and is shown by `voxtype meeting calendar status`.

#### A calendar command

For anything else, point voxtype at a command that prints the next meeting as one JSON object, the format of the Omarchy [Next Meeting](https://github.com/peteonrails/omarchy-next-meeting) widget:

```json
{"title": "Design review", "start_iso": "2026-11-12T14:00", "minutes_until": -2, "ongoing": true, "url": "https://meet.google.com/abc-defg-hij",
 "next": {"title": "Retro", "start_iso": "2026-11-12T15:00", "minutes_until": 58}}
```

`{"empty": true}` means nothing is coming up, and `"error"` that the calendar could not be read. The first event counts when it is running or starts within the window; `next` counts on the same terms, and wins over a first event that has been running a long time.

```toml
[meeting.calendar]
command = "~/bin/next-meeting --json"   # for example
window_mins = 10
timeout_secs = 3
```

The command runs through `bash -lc`, with `~/.local/bin` and `~/bin` first on `PATH`. If you use the Next Meeting widget, there is nothing to set: voxtype runs the widget's own command, read from the bar's settings in `~/.config/omarchy/shell.json`, which it never changes.

Whatever goes wrong (no answer within `timeout_secs`, a failed fetch, output that is not the expected JSON, no event near the start) leaves the title as it was, with the reason logged at debug level.

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
would prompt: Record "Google Meet call"? (Google Meet)
```

Without `--dry-run`, `voxtype meeting suggest` runs the detector in the foreground with real prompts. It refuses while the daemon is already prompting.

To add another call service, see `apps` under [`[meeting.detect]`](CONFIGURATION.md#meetingdetect).

---

## Working with Transcripts

### Listing Past Meetings

```bash
voxtype meeting list
voxtype meeting list --limit 5
voxtype meeting list --json
```

Lists recent meetings, newest first, with their title, ID, date, duration and status. It shows the 10 most recent unless you pass `--limit`.

### Viewing a Meeting

```bash
voxtype meeting show latest
voxtype meeting show <meeting-id>
voxtype meeting show latest --json
```

Shows a meeting's title, ID, start and end times, duration, status, number of chunks, the call's link if it has one, and its transcript's segment count, word count and speakers. It reads the saved transcript, which grows with every chunk, so it is current during a meeting too.

Every command that takes a meeting accepts its full ID, the start of it (`a32baa9b`), or `latest` for the most recent.

### Retitling a Meeting

Give a meeting a new title at any point, while it records, while it is paused, or after it has finished:

```bash
voxtype meeting title latest "Pete <> Chad Bi-Weekly"
voxtype meeting title 3f2a "Q4 planning" --json
```

The title is stored with the meeting at once, so `meeting list`, `meeting show`, `metadata.json`, `meeting.json` and `transcript.md` all use it. The meeting's folder keeps the name it was created with. While the meeting is running, the live meeting view changes title straight away. An empty title is refused.

A two-person title names the speakers just as one given at the start does (see [Naming Speakers from the Title](#naming-speakers-from-the-title)): retitling a call to "Pete <> Chad" halfway through names the lines already there and every voice heard from then on. Names you gave by hand are kept.

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

If the meeting already has a summary, its action items follow the new name: an item owned by `Remote 2` is owned by `Chris` after `meeting label latest "Remote 2" Chris`, in the Summary tab, `meeting actions` (and `--mine`), exports, shares and search. Only owners that are exactly the old name change (ignoring case and surrounding spaces), so naming `Chris` never touches `Christopher`. Labelling a speaker back undoes it, and two speakers merged by the diarizer carry the surviving name. Nothing else in the summary changes and it is not summarized again: ticks and the record of what was sent stay. Items you own (`You`, or your [name](#your-name) on your side) stay yours, so `--mine` keeps finding them. A task already sent to Todoist, or a reminder already set, keeps the owner it was sent with: voxtype does not edit or resend it.

### Your Name

Rather than labelling `You` in every meeting, set your name once:

```bash
voxtype config set meeting.my_name "Pete"
```

Every meeting then shows your side as "Pete": the live view, exports, `transcript.md`, summaries and search. The meeting assistant is told who you are ("The user asking is Pete (You)."), so a question like "what did I agree to?" means your lines. It is a display name only, never saved into the transcript, and a label you set for one meeting still wins there. `voxtype meeting my-name` shows the setting, or the full name on your account as a suggestion when it is unset; voxtype never sets it on its own.

### Naming Speakers from the Title

A two-person meeting usually says who is in it: "Pete <> Andrew standup", "Andrew <> Pete Standup", "Pete / Chris", "Pete x Chris", "Pete & Chris", "Pete and Chris 1:1", "1:1 Pete/Chris" or "Chris - Pete sync". When a meeting's title reads like that, voxtype names the speakers for you. The side of the title that is you names your microphone, and the other side names every remote voice. In a 1:1 every remote voice is the same person, so this also tidies up a diarizer that heard one person as `Remote 1`, `Remote 2` and `Remote 3`.

You are recognized by [your name](#your-name) (`meeting.my_name`) when it is set, and otherwise, or as well, by your account's full name (the GECOS field, as `getent passwd $USER` shows it). Matching ignores case, and a first name is enough: an account named "Pete Jackson" matches a title that says "Pete". Your microphone is labelled with `my_name` when it is set, and otherwise with your full name.

Many accounts have no full name set (`getent passwd $USER` shows an empty field between the colons, as in `pete:x:1000:1000::/home/pete:/bin/bash`). Then your login name stands in as a first name: the login `pete` matches "Pete" in a title, and your microphone is labelled as the title spells it. A login name is only ever matched as a first name, and only when it is letters alone and not an account such as `root`, `user` or `deck`. To be matched by your full name, set [your name](#your-name) (`meeting.my_name`), which comes first, or set it on the account with `sudo chfn -f "Pete Jackson" $USER`.

The rules are strict on purpose, because a wrong name is worse than none. Each name is one or two capitalized words. Words like standup, sync, weekly, check-in and 1:1 before or after the names are ignored. Nothing is named when:

- the title names three or more people ("Pete, Andrew, Sam sync") or none ("Zoom call", "Q4 planning")
- neither side is you, or both could be
- the title has words voxtype does not recognize as names or meeting words ("Pete <> Andrew quick sync")

Names are given when the meeting starts, and each remote voice is named as it first speaks, so the live view shows names from the first line. A name you give with `voxtype meeting label` always wins: voxtype only names speakers that have no name yet. If a second remote voice really is someone else, relabel that speaker by its ID:

```bash
voxtype meeting label latest "Remote 2" "Sam"
```

Use the ID rather than the name here: `voxtype meeting label latest Andrew Sam` would rename every speaker the title named Andrew.

Imports name their speakers the same way when the file has separate microphone and remote channels (`--channels mic,loopback`), taking the title from `--title` or the file name. A single mixed track has no sides to tell apart, so nothing is named. `voxtype meeting retranscribe` names the speakers of the new transcript from the meeting's title too.

To turn this off:

```bash
voxtype config set meeting.auto_label_from_title false
```

### Searching Meetings

```bash
voxtype meeting search budget
voxtype meeting search budget review --speaker Dana
voxtype meeting search '"next quarter"' --since 2026-11-01
voxtype meeting search deploy* --limit 5 --json
```

Search covers every meeting's transcript as you see it (names, moved lines and corrections applied, deleted lines left out), plus titles, notes and summaries. Every word has to appear in the same line (or the same title, notes or summary). Matching ignores case and accents. Put a phrase in double quotes to keep it together, and end a word with `*` to match words that start with it. The best matches come first, each with the meeting, the time in the meeting, the speaker, and the matching text with the match marked « like this ».

- `--speaker NAME` keeps only lines from that speaker, as the transcript shows them. Your `my_name` also finds lines shown as "You".
- `--since DATE` keeps only meetings started on or after a date (`2026-11-01`) or a time (`2026-11-01T09:00:00Z`).
- `--limit N` caps the number of matches (default 20).
- `--json` prints the matches for scripts and the meeting view; see [INTEGRATIONS.md](INTEGRATIONS.md#searching-meetings-meeting-search---json).

Search uses an index in `index.db` that every change keeps current. Meetings recorded before search existed are indexed the first time you search, which can take a moment with many meetings.

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

### Correcting and Deleting Lines

Fix a line the engine misheard, or hide one that should not be there, by the segment's `id` from `meeting show --json`:

```bash
voxtype meeting edit latest 28 "We ship on Friday."
voxtype meeting edit latest 28 --reset           # back to the transcribed text
voxtype meeting delete-line latest 31
voxtype meeting delete-line latest 31 --restore
```

An edited line keeps what was transcribed as `original_text`. A deleted line is not destroyed: it stays in `transcript.json` marked `"deleted": true`, so `--restore` can bring it back, but exports, `transcript.md`, `meeting show` and the meeting assistant leave it out. Both are stored beside speaker moves and annotations and applied whenever the transcript is saved or read, so later chunks cannot undo them, and both work during a meeting. Add `--json` to print the segment.

### Annotating a Line

Attach a note or a highlight to one transcript line, by the segment's `id` from `meeting show --json`:

```bash
voxtype meeting annotate latest 28 "Check this number"
voxtype meeting annotate latest 28 "Decision" --highlight
voxtype meeting annotate latest 28 --highlight       # highlight without a note
voxtype meeting annotate latest 28 --remove
```

A line has at most one annotation; annotating it again replaces it. Annotations are stored beside speaker moves and applied whenever the transcript is saved or read, so later chunks cannot remove them. Exports show each one as a quote under its line (`> Note: ...`, or `> **Highlight: ...**`), and `meeting show --json` and the meeting socket carry it on the segment as `"annotation": {"text": "...", "highlight": true}`. Both commands work during a meeting.

### Notes

Each meeting has free-text notes, kept as `notes.md` in the meeting's folder. You can write them during the meeting or after it; the daemon never overwrites them.

```bash
voxtype meeting notes latest                         # print them
voxtype meeting notes latest --append "Ask Dana about travel"
voxtype meeting notes latest --set-file agenda.md    # replace them ("-" reads stdin)
voxtype meeting notes latest --json                  # {meeting_id, notes, updated_at}
```

Exports and `transcript.md` show the notes at the top, under "Notes".

### Watching a Meeting in the Terminal

`voxtype meeting watch` shows the running meeting in the terminal, on any desktop and over SSH:

```bash
voxtype meeting watch
```

- The header shows the state (recording, paused, finalizing, summarizing, ended), the title and the elapsed time. While a meeting records it also shows a level meter for your microphone and the call, if the daemon sends levels; while its audio plays (`voxtype meeting play`) it says "▶ playing" and marks the line being heard; and while `meeting import` or `meeting retranscribe` works on it, how far it has got.
- The transcript fills in as each chunk is transcribed, each line with its time and speaker in that speaker's color, and annotations under their lines. A corrected line is marked "(edited)", a line moved to another speaker shows who the diarizer gave it to ("(was Remote 1)"), highlighted lines are yellow, and deleted lines disappear (and come back if restored). It follows new lines; scroll up (↑, PgUp, Home) to read back and it stops following, and End follows again.
- The side panel lists the speakers, merged by the names you gave them, and the summary once it is written.

| Key | Action |
|-----|--------|
| `p` | Pause or resume |
| `s` | Stop (asks first) |
| `l` | Name a speaker: pick one, type the name (`meeting label`, once for each speaker merged under that name) |
| `r` | Move the selected line to another speaker: pick one, or "New speaker…" and type a name (`meeting reassign`) |
| `n` | Add a note to the selected line (`meeting annotate`) |
| `a` | Ask the assistant about the meeting; the answer streams into a pane at the bottom (`meeting ask`) |
| `Esc` | Close the answer, or go to the list of meetings |
| `q` | Quit |

Each key runs the matching `voxtype meeting` command, so it behaves exactly as typing it would, including the assistant's consent: if transcripts have not been allowed to go to an agent yet, the answer pane says how to allow it.

With no meeting running, `watch` lists recent meetings with their length, speaker count and whether they have a summary. Enter opens one (ended meetings still take `r`, `n` and `a`), Esc goes back, and a meeting that starts meanwhile appears at the top of the list. If the daemon restarts, the view says it is reconnecting and catches up when it is back.

For scripts, `voxtype meeting watch --json` prints the daemon's meeting socket as NDJSON (see [INTEGRATIONS.md](INTEGRATIONS.md)), reconnecting on its own; every connection starts with a `hello` snapshot.

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

Markdown and JSON exports start with your notes and the summary when the meeting has them, and plain text with the notes. Annotations appear under their lines, and deleted lines are left out.

Every format lists the conversation in the order it was spoken: your lines and remote lines are interleaved by start time, and each line carries its own timestamp. Within each chunk the daemon transcribes every utterance separately, splitting at pauses of 0.7 seconds or more, so a line's timestamp is when that utterance began rather than when its chunk did. Parakeet, ONNX or GGUF, also times every word, so an utterance holding several sentences gives a line per sentence, each at its own start, and a line ends with its last word rather than at the pause after it. Cohere has no word times (its GGUF model decodes without timestamps), so it keeps one line per utterance. Whisper, where a short clip costs as much as a whole chunk, transcribes the chunk in one call and uses the segments Whisper times itself, one per sentence or phrase. Each segment's start and end are then fitted to the speech inside it, since Whisper counts a pause before a sentence as part of it, and a segment over pure silence (a hallucinated "Thank you.") is dropped. This covers local Whisper with or without `gpu_isolation`, `whisper-cli` (`backend = "cli"`, which times every segment in its JSON output), and remote Whisper on a server that returns segments for `response_format=verbose_json` (OpenAI, whisper.cpp server and most compatible servers). The first meeting chunk asks the server for them; a server that refuses or ignores the format is remembered and gets one line per chunk, as before, for the rest of the daemon's run.

### Sharing a Meeting

`voxtype meeting share` sends a meeting's notes somewhere: the title, date, length and attendees, the summary's overview, key points, decisions, action items with their owners and chapters, your notes, and your highlighted or annotated lines. Add `--transcript` to include the transcript after them, as voxtype shows it: speaker names applied, deleted lines left out and edited lines as you corrected them.

```bash
voxtype meeting share latest                              # Copy the notes to the clipboard
voxtype meeting share latest --transcript                 # ...with the transcript
voxtype meeting share latest --to file                    # Save to Documents/Voxtype
voxtype meeting share latest --to email                   # Open a mail draft
voxtype meeting share latest --to command                 # Pipe to [meeting.share] command
voxtype meeting share latest --format html                # Copy as HTML, for rich-text editors
voxtype meeting share latest --transcript --preview       # Print what would be shared
```

| Destination | What happens |
|-------------|--------------|
| `clipboard` (default) | Copied with `wl-copy`: Markdown, plain text with `--format txt`, or `text/html` with `--format html` |
| `file` | A new file named `<date> <title>.md` (or `.txt`, `.html`) in `[meeting.share] folder`, `~/Documents/Voxtype` by default. Existing files are never overwritten. `--output` names the file or folder instead |
| `email` | Opens a draft in your mail app through a `mailto:` link, with the title and date as the subject and the notes as the body. A transcript is too long for a mail link, so with `--transcript` it is saved as a file next to your other shares and the draft tells you to attach it |
| `command` | Runs `[meeting.share] command` (or `--command`) through `sh -c` with the share on stdin |

Nothing is ever shared on its own: every share is a command you run or a menu item you click, and there is no hosted link or cloud copy. `--preview` prints exactly what would be sent without sending it, and `--preview --json` gives the same to the meeting view's share preview along with which destinations work on this machine and, for any that do not, why.

The share command is for anything voxtype does not do itself: posting to a chat webhook, converting with pandoc, or uploading somewhere. It gets these variables:

| Variable | Value |
|----------|-------|
| `VOXTYPE_MEETING_ID` | The meeting's ID |
| `VOXTYPE_MEETING_TITLE` | Its title |
| `VOXTYPE_MEETING_DIR` | Its folder |
| `VOXTYPE_SHARE_FORMAT` | `markdown`, `text` or `html` |
| `VOXTYPE_SHARE_FILE` | A temporary file holding the share, removed when the command ends |
| `VOXTYPE_SHARE_TRANSCRIPT` | `1` when the transcript is included, else `0` |

```toml
[meeting.share]
# Post the notes to a Slack channel
command = "jq -Rs '{text: .}' | curl -sf -X POST -H 'Content-Type: application/json' -d @- \"$SLACK_WEBHOOK_URL\""
```

```bash
# A PDF through pandoc, for this share only
voxtype meeting share latest --to command --command 'pandoc "$VOXTYPE_SHARE_FILE" -o ~/notes.pdf'
```

The command is stopped after `[meeting.share] timeout_secs` (60 by default). What it prints is shown after the share; a command that fails or times out makes the share fail with its error.

### Deleting Meetings

```bash
voxtype meeting delete <meeting-id> --force
```

Permanently deletes the meeting: its record, its folder and everything in it, audio included. Without `--force` it refuses and deletes nothing.

In the [meeting window](#without-quickshell-the-meeting-window) on desktops without Quickshell, each meeting in the list has a delete button, and an open meeting has **Delete…** in its title bar. Both ask first. A meeting that is still recording, finishing or being summarized cannot be deleted there.

### Recovering an Interrupted Meeting

A meeting is saved while it records: the transcript after every chunk, and the audio as it arrives when `retain_audio` is on. If the daemon crashes, is killed, or the computer loses power mid-meeting, all of that is still in the meeting's folder, but the meeting never ends: it has no end time, no summary, and `post_meeting_command` never ran. The next time the daemon starts it marks such meetings `interrupted`, the meeting view offers to finish them, and a notification says "A meeting didn't finish". For one meeting it has a **Finish** button, which finishes it as below, and an **Open** button for the meeting view; for several, **Open**. It comes once per meeting, not on every restart, and follows [`status_notifications`](CONFIGURATION.md#status_notifications): by default it is skipped while the meeting view is on screen, since the view shows the same.

From the command line:

```bash
# List meetings that never finished
voxtype meeting recover

# Finish one from what is on disk
voxtype meeting recover --finish 3f2a

# Or delete it and its folder
voxtype meeting recover --discard 3f2a
```

Finishing first transcribes any audio the daemon kept but had not transcribed yet, usually the last chunk, with the engine and model the meeting used. Then it ends the meeting when it last wrote to its folder, which is about when the daemon stopped, and does what stopping it would have: it writes `transcript.md`, writes the summary under the usual rules (see [AI Summarization](#ai-summarization)), and runs `post_meeting_command`. Without `retain_audio`, whatever was said after the last transcribed chunk, up to 30 seconds by default, is lost with the daemon. The meeting being recorded right now is never listed, finished or discarded.

---

## Meeting Audio

### Checking the Audio

Before a call, check that voxtype hears both sides:

```bash
voxtype meeting soundcheck                # 30 seconds of meters; Ctrl+C stops
voxtype meeting soundcheck --seconds 60
```

It opens the meeting's microphone and loopback the way a meeting would (the same devices, and on speakers the same echo canceller), records and transcribes nothing, and shows a meter for each. Speak: the mic meter should move. Play something on the computer: the loopback meter should move, and with speakers the mic meter should stay low, because the echo canceller takes the speakers out of the mic. The first line says which echo handling the next meeting will use.

While the daemon runs the check, the meeting views show its meters too: `voxtype meeting watch` beside **Meetings** ("Sound check  mic ▮▮▮▯▯ call ▮▮▮▮▯") and the GTK4 window in its title bar, under **Sound check**, while no meeting records.

During a meeting the levels are already live in `voxtype meeting watch --levels` and the meeting view, so the sound check refuses to run then. Starting a meeting ends a sound check.

### Keeping the Audio

With `retain_audio = true` the meeting folder also keeps the meeting's audio, as three Ogg Opus files at 16 kHz mono and 24 kbps:

- `mic.opus`: your microphone, as the transcript heard it. When echo cancellation runs (the WebRTC canceller on speakers, or GTCRN; see [Echo Cancellation](#echo-cancellation)), this is the cleaned-up mic, not the raw one: the raw mic carries the other side again through your speakers, and a mix of it with the loopback plays every remote voice twice. With echo cancellation off it is the raw mic.
- `loopback.opus`: the other side, from the loopback device. A meeting recorded without loopback has no loopback file.
- `mixed.opus`: the two summed, for listening back.

The three files are the same length and start when the meeting starts, so a transcript timestamp is the same moment in each. Time spent paused is left out of both the audio and the transcript.

Audio goes to disk chunk by chunk as the meeting records (every `chunk_duration_secs`, 30 seconds by default), not at the end. If voxtype crashes mid-meeting, the files keep everything up to the last chunk handed over before the crash and play as they are; the next daemon start marks the meeting completed in `meeting.json`.

Opus takes about 10 MB per hour per file for speech, so a one-hour meeting with all three files uses about 30 MB (less where a side is silent).

Encoding needs `ffmpeg` with libopus, which most distributions' `ffmpeg` package includes. Without it voxtype logs a warning and keeps the audio as `mic.wav`, `loopback.wav` and `mixed.wav` instead, at about 115 MB per hour per file. See [Troubleshooting](TROUBLESHOOTING.md#meeting-audio-is-saved-as-wav-instead-of-opus).

### Playing a Meeting Back

`voxtype meeting play` plays a meeting's kept audio from the start, from a transcript line, or from a time, and returns straight away while it plays in the background:

```bash
voxtype meeting play latest                       # from the start
voxtype meeting play latest --segment 28          # from line 28 (its id in meeting show --json)
voxtype meeting play latest --at 12:40            # 12 minutes 40 seconds in (also 760 or 1:12:40)
voxtype meeting play latest --track loopback      # only the other side (mixed, mic or loopback)
voxtype meeting play --status                     # what is playing
voxtype meeting play --stop
```

Only one playback runs at a time; starting another stops the one before. It plays `mixed.opus` by default.

Playback needs `ffmpeg` and one of `pw-play` (PipeWire), `paplay` (PulseAudio) or `aplay` (ALSA), and goes to your default output. A meeting recorded without `retain_audio` has nothing to play, and says so.

`voxtype meeting waveform latest` reports the peak level of each stretch of a meeting's kept audio, your side and the other side, the data a player draws its waveform from. `--json` gives the values (see [Integrations](INTEGRATIONS.md#drawing-a-waveform-meeting-waveform)) and `--buckets` how many stretches to split the meeting into. The result is kept in the meeting folder as `waveform.json`, so asking again is instant.

### Importing a Recording

`voxtype meeting import` turns a recording made elsewhere (a phone memo, a call recorded by another app, a video) into a meeting like any other: transcript, speakers, exports, the kept audio, the summary at the end when auto-summary is on, and `post_meeting_command`. Anything ffmpeg reads works.

```bash
voxtype meeting import call.mp3 --title "Client call"
voxtype meeting import memo.m4a --model large-v3-turbo --language de
voxtype meeting import recording.wav --channels mic,loopback
voxtype meeting import memo.m4a --date 2026-11-02T15:00:00Z
```

It runs in your terminal, not in the daemon, so it never interrupts a meeting being recorded, and prints its progress chunk by chunk. `--engine`, `--model` and `--language` work as they do for `meeting start`, and `--diarization` picks the backend for this import.

**Channels.** By default every channel is mixed into one track. Lines then have no side (they show as `Unknown` until you label them), and with `ml` diarization each voice gets its own speaker. A call recorded with you on one channel and the others on another can say so: `--channels mic,loopback` (or `loopback,mic`) names each channel in order, so lines are attributed to You and Remote, and `hybrid` diarization splits the remote side into Remote 1, Remote 2. `ignore` (or `-`) skips a channel.

**Date.** The meeting is dated by `--date` (RFC 3339), else by the file's `creation_time` tag, else by its modification time. Its length is the recording's.

**Audio.** The recording is kept in the meeting folder as `mixed.opus`, plus `mic.opus` and `loopback.opus` when channels are mapped, whatever `retain_audio` says, so the meeting can be played back and transcribed again. The original file is left where it is.

**From Omarchy Meeting Recorder.** Give `meeting import` a Meeting Recorder meeting folder (the one with the `.meeting-recorder` file) and the meeting comes over as it is: its transcript, speaker names, chapters (as the summary's chapters), notes, and audio (from `.tracks/`, the separate files, or `audio.ogg`). Nothing is transcribed; run `meeting retranscribe` afterwards to redo it with a voxtype engine. This importer was written from Meeting Recorder's source, not tested against folders it wrote.

```bash
voxtype meeting import ~/Documents/Meetings/'202511021500 Launch sync'
```

### Transcribing a Meeting Again

`voxtype meeting retranscribe` runs a meeting's kept audio through the transcriber again, for a better model, another engine or another language:

```bash
voxtype meeting retranscribe latest --model large-v3-turbo
voxtype meeting retranscribe b6b2 --engine parakeet --discard-edits
```

It needs the meeting's audio: a meeting recorded with `retain_audio = true`, or an imported one. The meeting keeps its ID, folder, notes, speaker names and summary, and its previous transcript is kept as `transcript.v1.json` (then `v2`, and so on).

Edits you made line by line are tied to the old lines, and the new transcript has new lines, so each old line is matched to the new line it overlaps most in time, on the same side:

- annotations always move to their matching line;
- speaker moves, text corrections and deleted lines move too with `--keep-edits` (the default), or are dropped with `--discard-edits`;
- an edit whose line has no match (the new transcript has nothing there, or splits it differently) is dropped.

The command says how many carried over. Speaker names carry over by speaker: with `simple` and `hybrid` diarization You and Remote stay put, but with `ml` the speaker numbers come from a fresh clustering, so check the names. A meeting that is still recording cannot be transcribed again until it stops.

---

## The Meeting Assistant

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

The summary comes from the [meeting assistant](#the-meeting-assistant-and-consent): the allowed agent, or the local model. The older Ollama and API summarizers (`[meeting.summary] backend = "local"` or `"remote"`) are used only while no agent is allowed; see [Which backend answers](#which-backend-answers).

**Summary at stop.** Once you have allowed an agent, each meeting is summarized when it stops. This happens in the background after the transcript is saved: the meeting socket reports `summarizing` and then `summary_ready` (see [INTEGRATIONS.md](INTEGRATIONS.md#status)). If the summary fails, the meeting and its transcript are unaffected; the failure is logged and reported on the socket, and `voxtype meeting summarize <id> --regenerate` retries it. Set `[meeting.summary] auto = false` to summarize only by hand, or `auto = true` to summarize at stop with the local model or older summarizers too.

### Action Items

A summary lists the meeting's action items: what to do, who took it on (a speaker name), the deadline if someone said one, and where in the meeting it came up. `voxtype meeting actions` lists them and sends them on:

```bash
voxtype meeting actions latest                           # List them
voxtype meeting actions latest --mine                    # Only yours
voxtype meeting actions latest --send todoist            # Add every item to Todoist
voxtype meeting actions latest --send todoist --pick 1,3 # Only items 1 and 3
voxtype meeting actions latest --send reminder --in 30   # Omarchy reminders in 30 minutes
voxtype meeting actions latest --json                    # For scripts and the meeting view
```

```
Action items from "Release sync":
   1. Tag the release (Dana, due Fri Nov 13, at 01:05)
   2. Write the notes (Pete (you), due Thu Nov 12 15:00)
   3. Book the room (due "next week")  [sent: Todoist]
```

A deadline is only kept when someone said it, and voxtype turns what was said into a day counted from the meeting's date: "Friday" in a Thursday meeting is the next day, "by end of day" is the meeting's day, "November 20" and "next Friday" are what you would expect. Something that names no day ("next week", "soon") stays as it was said. An item is yours when its owner is `You` or your [name](#your-name) (`[meeting] my_name`).

Every item is sent unless you narrow it with `--pick` (item numbers from the list, such as `1,3` or `2-4`) or `--mine`. Each send is remembered per item, so sending again skips what already went and never makes a second task; `--resend` sends it again anyway. An item that fails is reported and can simply be sent again. Items come from the summary, so a meeting without one has none: summarize it first with `voxtype meeting summarize`.

**Todoist.** Each item becomes a task in your Inbox, or in `[meeting.actions] todoist_project_id` (or `--project`). The task is the item's text, with the owner added when it is someone else's, due on the resolved day (and time, when one was said). Its description names the meeting, the date and the moment the item came up, and the command that opens the meeting. Connect your Todoist account once:

```bash
voxtype meeting todoist connect      # Approve in the browser
voxtype meeting todoist status       # Connected?
voxtype meeting todoist disconnect   # Forget the connection on this machine
```

`connect` opens Todoist's consent page in your browser and waits (up to five minutes) for you to approve; `--no-browser` prints the address instead. Voxtype asks only to add tasks: it cannot read your tasks, projects or anything else in your account. The browser comes back to `http://localhost` on one of ports 47815 to 47819, so one of them must be free. There is no client secret: voxtype registers itself with Todoist the first time and proves each sign-in with PKCE, and it keeps the client ID and tokens in `~/.local/state/voxtype/todoist.json`, readable only by you, never in `config.toml`. Tokens renew themselves; `disconnect` removes them here, and Todoist's Settings > Integrations removes voxtype's access altogether.

On a machine without a browser, use a personal API token instead (Todoist Settings > Integrations > Developer). Put it in a file only you can read and point voxtype at the file; the token itself never goes in `config.toml`:

```bash
install -m 600 /dev/null ~/.config/voxtype/todoist-token
$EDITOR ~/.config/voxtype/todoist-token
voxtype config set meeting.actions.todoist_token_file ~/.config/voxtype/todoist-token
```

**Omarchy reminders.** On Omarchy, `--send reminder` sets an `omarchy-reminder` notification for each item. It goes off after `[meeting.actions] remind_in_mins` (60 by default, or `--in`), or at the time said when that is later today ("by 3pm"). Omarchy's reminders are timed notifications, not a task list, so they suit "remind me about this later today"; Todoist is the place to keep tasks. Without `omarchy-reminder` installed, this option is not offered.

### The Meeting Assistant and Consent

`ask` and summaries go through your own coding agent, run headless with every tool switched off: it gets the transcript as text and returns text. It never gets a shell, file access or network tools in this path. Which agent:

1. `[meeting.assistant] agent` in config.
2. Otherwise the Omarchy default agent (`omarchy-default-agent`), when that command exists. Voxtype works the same without Omarchy.
3. Otherwise a supported agent CLI found on PATH: Claude Code or Pi. With both found and neither allowed yet, the consent prompt asks which to use.

| Agent | Support |
|-------|---------|
| Claude Code (`claude`) | Supported, no tools: `claude -p --tools "" --strict-mcp-config ...` |
| Codex (`codex`) | Supported when named in config or as the Omarchy default, and only with bubblewrap (`bwrap`) installed: `codex exec` with its shell and app tools disabled and a read-only sandbox, run inside a bwrap jail. Its command tool cannot be switched off, so the jail is what keeps it away from your files: it sees `/usr`, the codex install, a few `/etc` files for DNS and TLS, and `~/.codex/auth.json` read-only, and nothing else from your home. It is never picked from PATH on its own |
| Pi (`pi`) | Supported, no tools: `pi --print --no-tools --no-mcp --no-extensions --no-skills --no-context-files --no-session ...`, with the prompt attached as a file. Its answer arrives whole rather than streaming. When pi's default model is on Omarchy's `omarchy-local` provider, voxtype also passes `--extension ~/.pi/agent/extensions/omarchy-local.ts`, the one extension Omarchy writes to register that provider; every other extension stays off |
| OpenCode (`opencode`) | Not supported: `opencode run` has no flag that turns its tools off, and its agents allow shell commands and web fetches, so even a filesystem jail could not keep a transcript from leaving the machine |
| Hermes (`hermes`) | Not supported: its one-shot mode loads tools and auto-approves them, with no documented way to turn them all off |

The first time transcripts would go to an agent, voxtype asks once ("Send to Claude / Use a local model / Not now") and remembers the answer per agent. Without a terminal to ask on, `ask` and `summarize` refuse with a message naming the command that allows it:

```bash
voxtype meeting assistant status          # which agent, what was found, consent
voxtype meeting assistant allow claude    # allow transcripts to go to Claude
voxtype meeting assistant revoke claude   # stop
voxtype meeting assistant status --json   # for the meeting view and Settings panel
```

#### Omarchy's local model

On Omarchy, meeting summaries and Ask can use the text model Omarchy runs on your machine (`omarchy models`), with no agent and no consent prompt, since the transcript never leaves the machine:

```toml
[meeting.summary]
backend = "omarchy"
# omarchy_model = "qwen3.8-27b-gufo"   # optional; unset uses the last started model
```

A config that `voxtype setup` writes on an Omarchy install already has this. An existing config keeps whatever it chose.

Voxtype uses only what Omarchy documents for third-party clients: `omarchy-models status --json` for the running model and its loopback address, the recipe name as the model, and the key in `~/.config/omarchy/agent-local/key`, which is read for each request and never logged. It never starts, installs or downloads a model: a text model is usually not running (it stops at logout, and it takes most of the machine's memory). When none is running, the log says how to start one (`omarchy models start <recipe>`, or the local models panel), and questions and summaries go the way the table below describes, as if Omarchy were not chosen. If the model fails while writing a summary, the summary is written the same way instead, and `meeting summarize` names the model that failed.

`[meeting.summary] backend = "openai"` does the same with any OpenAI-compatible server (`openai_url`, `openai_model`, `openai_api_key_file`). It answers questions only when its URL is on this machine; a server elsewhere writes summaries only. See [CONFIGURATION.md](CONFIGURATION.md#meetingsummary).

`voxtype doctor` reports whether Omarchy was found, whether a text model is running, whether the key file can be read, and whether the model answers.

#### Which backend answers

Questions and summaries follow the same order, top row first. "The agent" is the one picked above, and "the local model" is Ollama at `[meeting.summary] ollama_url` with `ollama_model`.

| Situation | Questions (`ask`, the Ask tab) | Summaries (at stop, Summarize, Regenerate) |
|-----------|-------------------------------|--------------------------------------------|
| `[meeting.summary] backend = "omarchy"`, a model is running, and no `--agent` | Omarchy's model | Omarchy's model |
| `[meeting.summary] backend = "openai"`, and no `--agent` | That server, when it is on this machine | That server (a local one only, with `local_only` or `--local`) |
| `[meeting.assistant] local_only = true`, or `--local` | The local model | The local model |
| `--agent <id>` | That agent, once allowed | That agent, once allowed |
| The agent is allowed | The agent | The agent, whatever `[meeting.summary] backend` says |
| Not allowed, and `[meeting.summary] backend = "local"` | The local model | The older Ollama summarizer |
| Not allowed, and `[meeting.summary] backend = "remote"` | As in the rows below | The remote summarizer |
| Not allowed, and an agent was found | Asks to allow it | Asks to allow it |
| No agent found | The local model | The local model |

Allowing an agent is the newer and more explicit choice, so it wins over a `[meeting.summary] backend` written before the assistant existed. Someone who never allowed an agent keeps the summarizer they set up.

`voxtype meeting assistant status` shows both choices, and `--json` has them as `backend_name` (questions) and `summary_backend_name` (summaries), with a note when they differ.

When the backend fails, the error names it and gives its reason, for example `Ollama (llama3:latest at localhost:11434) returned an error: mkdir /opt/models/manifests-v2: permission denied`. A failure never sends the transcript somewhere else you did not choose: a failing local model is never replaced by an agent. The one fallback goes the other way. If the agent fails while writing a summary and `[meeting.summary] backend = "local"` is set, the local model writes it instead, provided it answers, and `meeting summarize` says so.

### Privacy: Where Transcripts Go

Recording and transcription stay on your machine. The assistant is the one part of meeting mode that can send a transcript elsewhere:

- **An agent** (`claude`, `codex`, `pi`) sends the prompt to whichever account that CLI is signed in to, usually the vendor's cloud service, under that account's terms. That prompt holds the meeting title, date, speaker names, the transcript (or the part of it that fits, plus a summary of the rest), your notes and annotations, and your question. Nothing goes to an agent until you allow it for that agent, and `voxtype meeting assistant revoke <agent>` stops it.
- **The local model** (Ollama at `ollama_url`, `localhost` by default) keeps everything on your machine. `local_only = true` makes it the only option.
- **Omarchy's local model** (`[meeting.summary] backend = "omarchy"`) keeps everything on your machine too: voxtype only talks to the loopback address Omarchy reports and refuses any other. The model runs in Omarchy's container with no network access.
- **An `"openai"` server** gets the prompt at `openai_url`. Only one on a loopback address counts as local.
- The agent runs in an empty temporary directory with its tools off, so it does not read your files or project instructions, and its session is not saved (`--no-session-persistence` for Claude, `--ephemeral` for Codex, `--no-session` for Pi). The prompt reaches it through a file in a private temporary directory that is removed when the agent exits.
- Codex keeps a command tool that cannot be turned off, and a transcript is untrusted text: anyone on the call can say something meant for the model. So codex runs in a bubblewrap jail where your home holds nothing but its `auth.json` (read-only), and without `bwrap` voxtype does not use codex at all.
- Consent is stored in `$XDG_STATE_HOME/voxtype/assistant-consent.json` (default `~/.local/state/voxtype/`).
- Answers are stored in the meeting folder (`asks.jsonl`, `summary.md`), next to the transcript, and deleted with it.

---

## Automation

### Running a Command After a Meeting

`post_meeting_command` runs a shell command once a meeting has stopped and its folder is final: after the transcript, audio and `meeting.json` are written, and after the automatic summary when one runs. Use it to copy the folder somewhere, file it in your notes, or start your own processing.

```toml
[meeting]
post_meeting_command = 'rsync -a "$VOXTYPE_MEETING_DIR" nas:meetings/'
post_meeting_timeout_secs = 300   # default
```

The command runs through `sh -c` in the meeting folder, with these variables set:

| Variable | Value |
|----------|-------|
| `VOXTYPE_MEETING_ID` | The meeting's ID |
| `VOXTYPE_MEETING_DIR` | The meeting folder |
| `VOXTYPE_MEETING_MANIFEST` | The path of its `meeting.json` |

It runs in the background, so the daemon carries on and you can start the next meeting straight away. Its output and exit status go to the daemon log (`journalctl --user -u voxtype`, or `journalctl _SYSTEMD_USER_UNIT=voxtype.service` when [the user journal has no voxtype entries](TROUBLESHOOTING.md#the-user-journal-has-no-voxtype-entries)). A command still running after `post_meeting_timeout_secs` is killed, along with anything it started. A command that fails or hangs never affects the daemon.

### Integrations

Everything a program can read or drive is documented as a stable contract in [INTEGRATIONS.md](INTEGRATIONS.md#meeting-mode): `--json` on `status`, `list`, `show`, `ask`, `assistant status`, `play` and the transcript-editing commands, the `meeting.sock` socket that streams the live transcript and status, the `meeting.json` manifest, and the files the meeting view reads and writes. Build a bar widget, a view of your own, or an agent workflow on those rather than on voxtype's internal files.

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

# Keep the meeting's audio in its folder as mic.opus, loopback.opus and
# mixed.opus (default: false). About 30 MB per hour; needs ffmpeg for Opus,
# otherwise WAV. See "Keeping the Audio".
retain_audio = false

# Shell command run once a meeting's folder is final (default: unset).
# Gets VOXTYPE_MEETING_ID, VOXTYPE_MEETING_DIR and VOXTYPE_MEETING_MANIFEST.
# post_meeting_command = 'notify-send "Meeting saved" "$VOXTYPE_MEETING_DIR"'

# Seconds before post_meeting_command is killed (default: 300)
post_meeting_timeout_secs = 300

# Maximum meeting duration in minutes (default: 180, 0 = unlimited)
max_duration_mins = 180

# Open the live meeting view when a meeting starts, in the Omarchy shell or
# voxtype's Quickshell shell (default: true)
show_view_on_start = true

# How opaque the meeting view's background is while the pointer is elsewhere,
# 0.3 to 1.0 (default: 0.8)
view_opacity = 0.8

# Meeting Started/Paused/Resumed/Ended notifications: "auto" skips them while
# the meeting view is on screen, "always", or "never" (default: "auto")
status_notifications = "auto"

# While meeting mode is off, offer to turn it on from the tray and a
# notification at most once a day (default: true)
offer_enable = true

# Say "This meeting is being recorded and transcribed with Voxtype" into
# the call app's mic, and on your speakers, when a meeting starts
# (default: false). A courtesy, not consent. See "Announcing the Recording".
announce_recording = false

# Your own announcement clip (WAV, FLAC or Ogg) instead of the built-in one
# (default: unset)
# announcement_file = "~/.config/voxtype/announcement-de.wav"

# The built-in announcement's voice: "kokoro" (built in, MIT) or "jessica"
# (ElevenLabs, downloaded once, not MIT) (default: "kokoro")
announcement_voice = "kokoro"

# How loud the call hears the announcement, as a linear gain: 1.0 is the
# clip at full level, 0.5 about 6 dB down, 0.0 silent. About 0.3 sounds
# roughly half as loud. Your speakers' copy follows your mixer. (default: 0.5)
announcement_volume = 0.5
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

# Echo cancellation for the mic (default: "auto"). See "Echo Cancellation".
echo_cancel = "auto"

# Keep the call heard through your speakers out of your lines (default: true);
# the misheard-words check looks at lines up to this many words (default: 4)
echo_guard = true
echo_fuzzy_max_words = 4
```

Setting `loopback_device = "auto"` lets voxtype capture system audio (the other side of a call). When loopback is active, speaker attribution can distinguish between "You" (from the mic) and "Remote" (from system audio).

Set `loopback_device = "disabled"` if you only want to capture your own microphone, or if loopback detection is causing problems.

### Echo Cancellation

On speakers, your microphone hears everything the speakers play: the other side of the call, and anything else, such as a video playing during the meeting. Without echo cancellation that audio is transcribed as You.

With `echo_cancel = "auto"` (the default), a meeting runs PipeWire's echo canceller (the WebRTC engine) for as long as the meeting runs, unless the output is headphones. The canceller listens to what your default output plays and removes it from the microphone before voxtype transcribes it, whatever is playing, not only the call. Your own voice comes through. With headphones or a headset there is no echo to remove, so voxtype skips the canceller and uses GTCRN, its speech enhancer, as before.

- **What counts as headphones:** the default output's active port is Headphones, Headset or Handset, or the device says it is headphones or a headset, or it is Bluetooth (unless it is a Bluetooth speaker). Everything else, including HDMI, line out, S/PDIF and anything voxtype cannot identify, gets the canceller.
- **Nothing in your PipeWire configuration changes.** The canceller is a small `pipewire` process voxtype starts when the meeting starts and stops when it stops. If voxtype crashes, the canceller stops with it. While it runs, a source called "Voxtype meeting echo canceller" shows in your mixer; it never becomes your default.
- **If your default microphone is a filter** over a real one (Camera Effects' "Microphone Effects", a noise-suppression filter chain), the canceller records the real microphone underneath. A filter's noise gate, auto level or voice effect, placed before the canceller, would bend the echo it removes. Voxtype finds the real microphone from the filter's own capture stream and says so in the log. If the filter hides the real microphones from other apps (Camera Effects can), voxtype cannot see them and records the filter, as before. Name a microphone in `mic_device` to choose it yourself.
- **A call on another output.** The canceller removes what your default output plays. If the call plays somewhere else and `loopback_device` names that output's monitor (`"<sink>.monitor"`), the canceller removes that output's sound instead, and decides speakers or headphones by it.
- **If your microphone is a monitor of that output** (a virtual mic fed from the speakers, not a microphone), everything it hears is what the canceller removes, so it would deliver silence. Voxtype skips the canceller, uses GTCRN, and says so in the log.
- **If you already run an echo canceller** as your default microphone (PipeWire's `echo-cancel` module, set up yourself), voxtype records from it and does not add a second one.
- **If the canceller cannot start** (PipeWire is missing its echo-cancel module or the WebRTC plugin, or it is too old), the meeting logs one warning and uses GTCRN. The meeting never fails because of it.
- **If the canceller hears nothing.** For its first 3 seconds voxtype checks that the canceller's mic carries any sound at all. A real microphone always picks up some room noise; a canceller that delivers digital silence is not getting the microphone, or is removing your voice along with the echo (when your microphone is a monitor of the very output it cancels, for one). The meeting then records the microphone directly with GTCRN, logs a warning, and reports `gtcrn` from then on. The first 3 seconds of your microphone are lost in that case; everything after is kept.
- **The two tracks stay in step.** The canceller hands the mic over about 37 ms late; voxtype drops that much from the start of the mic so the mic and the other side line up in the transcript and the kept audio.
- **It works on the meeting mic only.** Dictation is not affected.

`voxtype meeting status --json`, the meeting's `meeting.json` and the meeting socket's `hello` say which one a meeting used: `webrtc`, `gtcrn` or `none`.

| `echo_cancel` | What it does |
|---------------|--------------|
| `auto` (default) | The canceller unless the output is headphones, then GTCRN |
| `webrtc` | The canceller whatever the output; GTCRN if it cannot start |
| `gtcrn` | GTCRN only, as before 1.3.0 |
| `disabled` | Neither |

**A specific microphone.** With `mic_device` set to something other than `default`, the canceller records that microphone if the name is a PipeWire source (as `pactl list short sources` lists it). An ALSA device name is not one, so the meeting uses GTCRN instead and says so in the log.

The transcript dedup, which removes mic lines that also appear on the loopback track, and the echo guard below run whichever echo handling a meeting uses.

### Echo Guard

Echo cancellation removes most of what your speakers play from the microphone. What gets through, the echo guard (`echo_guard`, on by default) keeps out of your lines in the transcript:

- A microphone line whose sound is all explained by the loopback, whatever its length, is echo. When the loopback has the same speech at the time, the line is dropped, because the loopback already has those words. When it does not, the line moves to the remote speaker instead, so nothing the loopback missed is lost; `voxtype meeting reassign` moves it back if that was wrong.
- A short line that is a misheard version of the remote's words, such as "after the daemon restarts." while the remote said "after the demon restarts", is dropped.
- A short "Thank you." or "Thanks for watching." on a microphone with no voice of yours in it, while the other side talks, is dropped. Speech engines write these phrases for faint sound they cannot make out, such as what the echo canceller leaves behind. The same phrase on the other side's track is dropped when nobody there was speaking at the time, as when a call is joined and the engine writes "Thank you." for the silence or the join sound.
- Your own lines stay. Before touching a line the guard compares both tracks' audio levels: your voice is louder on the microphone than the echo can explain, so a line with any of your own speech in it stays yours, and "yes" said over the other side's "yes" stays too.

Short lines with exactly the remote's words, and runs of four or more of them, are removed whether the guard is on or off. See [`echo_guard`](CONFIGURATION.md#echo_guard) for the details.

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

## Storage

Meetings are kept under `~/.local/share/voxtype/meetings/` by default, or the `storage_path` you set. Each meeting gets a folder named by its date and title, and `index.db`, a SQLite database, indexes them all for fast listing:

```
~/.local/share/voxtype/meetings/
  index.db
  2026-11-11-release-sync/
    meeting.json
    transcript.json
    transcript.md
    notes.md
    summary.md
    asks.jsonl
    mic.opus
    loopback.opus
    mixed.opus
    metadata.json
  2026-11-10-weekly-standup/
    meeting.json
    transcript.json
    transcript.md
    metadata.json
```

A folder holds:

| File | What it is |
|------|------------|
| `meeting.json` | The manifest: title, times, engine and model, speakers and their names, and every file below with its size. A stable format for scripts; see [INTEGRATIONS.md](INTEGRATIONS.md#the-meeting-folder-and-meetingjson) |
| `transcript.json` | The segments, with speaker names, moves, annotations and text corrections applied. Deleted lines stay here, marked `"deleted": true`, and appear nowhere else |
| `transcript.md` | The same as `meeting export --timestamps --speakers --metadata` in Markdown, rewritten after every chunk and every change |
| `notes.md` | Your notes, once you write any |
| `summary.md` | The meeting summary, once there is one |
| `asks.jsonl` | Every question asked with `meeting ask` or the Ask tab, and its answer, one per line |
| `assistant-context.json` | During a long meeting, the assistant's running summary of the earlier part, so asking again only summarizes what is new |
| `mic.opus`, `loopback.opus`, `mixed.opus` | The meeting's audio, with `retain_audio = true` (below) |
| `metadata.json` | Voxtype's own record of the meeting; read `meeting.json` instead |

`meeting.json` is written when the meeting starts and rewritten when it stops, when its notes or summary change, and when a daemon restart finishes a meeting a crash left open.

`voxtype meeting delete` removes the folder along with the meeting's record. Copying a folder elsewhere keeps everything a reader needs, starting with `meeting.json`; [post_meeting_command](#running-a-command-after-a-meeting) does that automatically.

---

## Tips for Best Results

**Choose the right model.** Meeting transcription processes many audio chunks, so model choice affects both speed and accuracy. A fast model like `base.en` keeps up with real-time audio on most hardware. Larger models like `large-v3-turbo` are more accurate but need a capable GPU to keep pace with 30-second chunks.

**Use a good microphone.** Transcription accuracy depends heavily on audio quality. A dedicated microphone or headset works much better than a laptop's built-in mic, especially in rooms with echo or background noise.

**Set chunk duration appropriately.** The default 30 seconds works well for most cases. Shorter chunks (15-20s) give faster partial results but increase processing overhead. Longer chunks (45-60s) can improve accuracy for slower hardware since the transcription engine has more context.

**Label speakers after the meeting.** Run `voxtype meeting list` to find the meeting ID, then use `voxtype meeting label` to assign names to auto-detected speaker IDs. This makes the exported transcript much more readable.

**Export in multiple formats.** You can export the same meeting in different formats for different purposes: markdown for reading, JSON for processing in other tools, SRT/VTT for adding subtitles to a recording.

---

## Troubleshooting

The daemon tells you with a desktop notification when a meeting cannot start or starts without something it needs. Each problem is also logged; see the full detail with `journalctl --user -u voxtype`, or with `journalctl _SYSTEMD_USER_UNIT=voxtype.service` when [the user journal has no voxtype entries](TROUBLESHOOTING.md#the-user-journal-has-no-voxtype-entries).

**"Meeting Error: The meeting could not start".** The meeting was never created. The notification carries the error, for example a `[meeting] storage_path` that cannot be written or a transcription model that cannot be loaded. Fix the cause and run `voxtype meeting start` again.

**"Meeting started with limitations".** The meeting is recording, but part of it will be missing. One notification lists every problem:

| Message | Cause | Fix |
|---------|-------|-----|
| Remote audio is not recorded: pactl/parec is not installed | Loopback capture uses PulseAudio's command-line tools | Install `libpulse` (Arch) or `pulseaudio-utils` (Debian, Ubuntu, Fedora) |
| Remote audio is not recorded: pactl could not list sources | No PulseAudio server answers | On PipeWire, install `pipewire-pulse` and run `systemctl --user enable --now pipewire-pulse.socket` |
| Remote audio is not recorded: no monitor source was found | The audio server lists no `.monitor` source | Set `[meeting.audio] loopback_device` to a source from `pactl list short sources` |
| Speaker labels fall back to mic/remote only (or to You and Remote, for `backend = "hybrid"`) | The ML diarization model is missing or failed to load, or the binary has no ML diarization | Run the download command in the notification, use an ONNX binary, or set `backend = "simple"` |
| Echo cancellation is off | The meeting uses GTCRN (headphones, `echo_cancel = "gtcrn"`, or the WebRTC canceller could not start) and its model is missing or failed to load | Run the download command in the notification, or set `[meeting.audio] echo_cancel = "disabled"` |

Missing remote audio is reported at every meeting start, since every meeting loses that side of the conversation. Missing models are reported once per daemon run. If the echo canceller goes silent mid-meeting and the meeting switches to GTCRN without its model, a "Meeting continues with limitations" notification says so.

**"Meeting transcription failed".** A chunk could not be transcribed, so that part of the meeting is missing from the transcript. The meeting keeps recording. Only the first failure in a meeting raises a notification; later ones are logged.

**"One side of the meeting is not being transcribed".** Your microphone, or the call, has sound, but none of it reaches the transcript. Before a chunk of a track is transcribed it has to pass a speech check: 300 ms of it louder than `[meeting.audio] vad_threshold`. With the threshold above the level a track reaches, every chunk of that track is skipped as silence, and that side of the conversation is missing while the other side transcribes as usual. The notification names the side, the level its sound reaches and the threshold, both on the threshold's scale:

```
Your microphone has sound, but the speech check skips it, so nothing you say reaches the transcript. It reaches about 0.052 and [meeting.audio] vad_threshold is 0.11.

Lower the threshold to the default, 0.01:
voxtype config set meeting.audio.vad_threshold 0.01
It applies once voxtype restarts, so restart it after this meeting: systemctl --user restart voxtype

This meeting keeps its audio. With the threshold lowered, get the missing lines back after it ends:
voxtype meeting retranscribe b6b2a0c4
```

- Run the `voxtype config set` command it gives. The value is half the level the track reaches and never above the default, so a quiet track is sent below `0.01`. The running meeting keeps the threshold it started with; the new one applies to meetings started after `systemctl --user restart voxtype`. Restarting during a meeting interrupts it, so wait until it has ended.
- With `[meeting] retain_audio = true` the lines are not lost: once the meeting has ended and the threshold is lowered, `voxtype meeting retranscribe <meeting>` transcribes the kept audio again. Without kept audio, what was skipped cannot be recovered.
- If the threshold is already at or below the default, the track itself is too quiet, and the notification says that instead: raise the microphone's input volume (or the call's volume in the app that plays it), or lower the threshold further with the command it gives.

It is raised on evidence, not on silence, so a microphone that is quiet because you are only listening does not trigger it. A skipped chunk counts when it holds at least 0.6 s of sound that stands well above the track's own noise floor, comes in stretches of 150 ms or more (not clicks or key presses), and, on the microphone, does not coincide with sound on the call, which the microphone may only be hearing through your speakers. Three such chunks among a track's last ten, with no line from that track in between, raise it.

Each side is announced at most once per meeting. When both sides go missing at the same moment they share one notification, titled "Neither side of the meeting is being transcribed", with one command that covers the quieter of the two. `voxtype meeting status` shows it for as long as it lasts, as a `Not transcribed:` line (`untranscribed_tracks` in `--json`), and drops it when a line from that side arrives. The meeting view shows it in its banner, and the log has it as a warning with the numbers. The notification is shown whatever `[meeting] status_notifications` is set to, `"never"` included, and whether or not a meeting view is on screen, like the other meeting error notifications: it reports transcript being lost.
