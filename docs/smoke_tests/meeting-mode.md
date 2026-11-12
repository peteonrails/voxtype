# Meeting Mode

A manual pass over meeting mode as a user meets it, from the detection prompt
to playback, in about 15 minutes. Each step names what to do and what you
should see. Stop at the first step whose result differs and note which.

Needs:

- A voxtype 1.3.0 build installed, with the Omarchy shell's Voxtype plugin
  (Omarchy) or voxtype's Quickshell shell (step 12) to show the view.
- Hyprland, Chromium, and a Google Meet call with someone else, or a second
  device joined to it, so there is a remote side. Speakers rather than
  headphones also exercise echo removal.
- `jq` for the JSON checks.
- About two minutes of talking during steps 2 to 6. The meeting records the
  whole time; let it run.

Step 6 sends the transcript to your coding agent if you choose **Send to**.
Choose **Use a local model instead** to keep it on the machine.

## 0. Prepare (1 minute)

```bash
voxtype --version
voxtype config set meeting.enabled true
voxtype config set meeting.retain_audio true
voxtype config set meeting.show_view_on_start true
voxtype config set meeting.status_notifications auto
systemctl --user restart voxtype
voxtype meeting suggest --dry-run | head -1
voxtype meeting assistant status
```

Expected:

- `--version` shows the build under test.
- `suggest` starts with `Meeting detection in the daemon: on (default on Omarchy)`.
- `assistant status` names the agent that would answer and whether it is
  allowed. If it is allowed, step 6 shows no consent dialog; run
  `voxtype meeting assistant revoke <agent>` first to see it.
- In Omarchy, run `omarchy-restart-shell` once after installing a new build,
  or the shell keeps the old QML.

## 1. The detection prompt (2 minutes)

Open a Meet call in Chromium, keep its tab active, and join.

Expected:

- About 5 seconds after joining, one notification: **Google Meet call
  detected**, body `Record "Meet - <code>" as a meeting?`, with **Record** and
  **Dismiss**.
- No prompt during the lobby's mic preview is acceptable, but at most one
  prompt for the call.

If nothing appears, run `voxtype meeting suggest --dry-run` during the call
and keep its output: it says what it sees and why it did not prompt.

## 2. Record and the live view (2 minutes)

Click **Record**. Then type a few characters in the Meet chat.

Expected:

- The meeting view opens as a card at the right edge, titled
  `Meet - <code>`, showing **Recording** and a running time, with **Pause**
  and **Stop**.
- Your typing still goes to the Meet chat: the card did not take focus.
- No "Meeting Started" notification, because the view is showing it.
- `voxtype meeting status --json` shows `"status": "recording"` and the ID.

Talk, and have the other side talk. Within about 30 seconds:

- Lines appear as **You** and **Remote** (or **Remote 1**, **Remote 2**
  with hybrid diarization), each with its own timestamp, in the order they
  were spoken.
- Between updates the card says `Listening… last update N s ago.`
- With speakers, the other side's words do not show up under You.

## 3. Minimize and move (1 minute)

- Drag the header to the left edge of the screen and let go near it.
  Expected: the card moves and snaps to the edge.
- Drag the card's inner edge. Expected: it gets narrower or wider, no
  narrower than about 280 px.
- Click **↥** and then **↧**. Expected: compact (40 % height), then full
  height again.
- Click **–**. Expected: a small pill with the state, the time and the last
  line. A new chunk updates the last line.
- Click the pill. Expected: the full card comes back where it was.
- In a terminal: `voxtype meeting view --minimize`, then
  `voxtype meeting view --expand`. Expected: pill, then card.
- `cat "$XDG_RUNTIME_DIR/voxtype/meeting-view.state"`. Expected:
  `"visible": true`, `"host": "omarchy"`, and an `updated_at` within the last
  10 seconds.
- **⋯ → Reset position and size**. Expected: back at the right edge, default
  width.

## 4. Fix the transcript: rename, reassign, edit, delete (2 minutes)

- Click the **Remote** chip and name it `Dana`. Expected: every Remote line
  now says Dana.
- Click the speaker name on one of your own lines, then type a new name in
  **New speaker…** and press **Move**. Expected: only that line changes, and
  a chip for the new name appears. Click its speaker again: **Restore
  original (You)** puts it back.
- Hover a line, click **Edit**, change a word, press Enter. Expected: the
  line shows the new text.
- Hover another line and click **Delete**. Expected: it disappears with
  **Line deleted.** and **Undo**. Let the Undo time out.

Check from a terminal:

```bash
voxtype meeting show latest --json | jq '.transcript.segments[] | select(.original_text or .original_speaker) | {id, text, original_text, speaker_id, original_speaker}'
voxtype meeting show latest --json | jq '.transcript.segments | length'
```

Expected: the edited line has `original_text`, a moved line (if you left
one moved) has `original_speaker`, and the deleted line is not in the list.
Bring it back with `voxtype meeting delete-line latest <id> --restore`.
Expected: the line reappears in the view, in its place, within a few seconds.

## 5. Notes (1 minute)

Open **Notes** and type `Follow up on the release date`. Then:

```bash
voxtype meeting notes latest
voxtype meeting notes latest --append "Added from the terminal"
```

Expected: the CLI prints what you typed, and after `--append` the Notes tab
shows the new line without reopening.

## 6. Ask (2 minutes)

Open **Ask** and click **Last 5 minutes**.

Expected:

- If no agent is allowed yet: a dialog, **Send meeting transcripts to
  Claude?** (or your agent), with **Send to Claude**, **Use a local model
  instead** and **Not now**. **Not now** asks nothing.
- After choosing: the answer streams in, says **Answered by Claude** (or
  `Local (llama3.2)`), and cites times like `[1:58]`. Clicking a time jumps
  to that line on the Transcript tab.
- `voxtype meeting show latest --json | jq '.asks | length'` counts the
  question.

## 7. Stop and the summary (2 minutes)

Leave the Meet call while still recording.

Expected:

- About 5 seconds later: **Call ended**, `Chromium released the microphone.
  Stop the meeting recording?`, with **Stop meeting** and **Keep recording**.
  The meeting keeps recording until you choose.

Click **Stop meeting**.

Expected:

- The header shows **Finalizing**, then **Ended** with the meeting's length
  and end time. The card stays open and lines can still be edited.
- No "Meeting Ended" notification while the card shows.
- With an agent allowed (or `[meeting.summary] auto = true`), the Summary tab
  shows `Summarizing…` and then an overview, decisions, action items and
  chapters. Without one, it offers **Summarize**.
- `voxtype meeting status --json` shows `"status": "idle"`.

## 8. Past meetings (1 minute)

Click **‹ Meetings**.

Expected:

- **Recent meetings**, newest first, with this meeting at the top: title,
  date and time, length, speaker count, and a **Summary** badge if it has
  one.
- Clicking an older meeting opens its transcript, speakers and summary, and
  it can be fixed the same way. **Show older meetings** appears when there
  are more than 50.
- `voxtype meeting list --json | jq '.[0] | {title, has_summary, speaker_count}'`
  agrees with the list.

## 9. Export (1 minute)

Open this meeting again, then **⋯ → Export to Documents/Voxtype →
Markdown (.md)**.

Expected: **Exported to …** with **Open folder**. The file in
`~/Documents/Voxtype/` has the title, your notes, the summary, and the
transcript with Dana's name and your corrections, without the deleted line.

```bash
voxtype meeting export latest --timestamps --speakers | less
```

Expected: the same content: notes, then the summary, then the transcript
with a timestamp on each line.

## 10. Kept audio and playback (2 minutes)

```bash
dir=$(voxtype meeting show latest --json | jq -r .metadata.storage_path)
ls "$dir"
jq '.audio_retained, [.files[].kind]' "$dir/meeting.json"
id=$(voxtype meeting show latest --json | jq '.transcript.segments[3].id')
voxtype meeting play latest --segment "$id"
voxtype meeting play --status --json
voxtype meeting play latest --at 0:05 --track loopback
voxtype meeting play --stop
```

Expected:

- The folder has `meeting.json`, `transcript.json`, `transcript.md`,
  `notes.md`, `asks.jsonl`, `mic.opus`, `loopback.opus` and `mixed.opus`,
  plus `summary.md` if step 7 summarized.
- `audio_retained` is `true` and `files` lists `audio_mic`,
  `audio_loopback` and `audio_mixed`.
- `play --segment` returns at once and you hear the meeting from that line.
- `--status --json` shows `"state": "playing"` and a `position_ms` that
  grows each time you run it.
- `--track loopback` replaces the first playback with only the other side.
- `--stop` ends it and prints `Playback stopped`.

## 11. The tray offer on a fresh config (2 minutes)

An install from before meeting mode has no `[meeting]` section. This runs a
second daemon on a scratch config to see what such a user gets. Stop the
real one first, so the two do not compete:

```bash
systemctl --user stop voxtype
mkdir -p /tmp/vt-offer/state/voxtype
printf '[hotkey]\nenabled = false\n\n[whisper]\non_demand_loading = true\n' > /tmp/vt-offer/config.toml
# Pretend the offer first applied a day ago, so its notification is due now.
printf '{"first_seen": %s}\n' $(( $(date +%s) - 90000 )) > /tmp/vt-offer/state/voxtype/meeting_offer.json
XDG_STATE_HOME=/tmp/vt-offer/state voxtype --config /tmp/vt-offer/config.toml
```

Expected:

- The tray menu has **Turn on meetings...** where **Start Meeting** would be.
- A notification: **Turn on Voxtype meetings?**, with **Turn on**, **Not
  now** and **No, don't ask again**. (Without the `first_seen` line, a
  plain user sees only the tray entry until the next day; someone with
  Meeting Recorder installed gets the notification at once.)
- **Turn on** shows **Meetings are on**, `Restart Voxtype to start using
  them: systemctl --user restart voxtype` (the scratch daemon is not the
  service, so it cannot restart itself), the tray entry becomes
  **Start Meeting (restart Voxtype first)**, and `/tmp/vt-offer/config.toml`
  now has `[meeting]` with `enabled = true`.

Stop it with Ctrl+C, then:

```bash
systemctl --user start voxtype
rm -rf /tmp/vt-offer
```

## 12. The standalone Quickshell host (2 minutes)

The same view runs in voxtype's own Quickshell shell, for desktops without
Omarchy.

```bash
voxtype setup quickshell --force
qs -p ~/.local/share/voxtype/quickshell &
voxtype meeting view latest --tab summary
```

Expected:

- A card opens on the last meeting's Summary tab, styled by the Quickshell
  shell's theme. If the Omarchy shell is running, its card opens as well:
  every host acts on the request.
- `touch "$XDG_RUNTIME_DIR/voxtype/meeting-controls.flag"` closes the
  standalone card; touching it again opens it.
- `voxtype meeting start --title "Standalone check"` opens the card on the
  new meeting; `voxtype meeting stop` ends it.

Stop the shell with `kill %1`.

## 13. Speaker echo in a real room (4 minutes, at a normal hour)

This plays a talk out loud through the speakers, so do it when sound is
welcome. It needs speakers, not headphones, and the microphone you use for
calls.

```bash
pactl get-default-sink
pactl -f json list sinks | jq -r '.[] | select(.name == "'"$(pactl get-default-sink)"'") | .active_port'
voxtype meeting start --title "Echo check"
voxtype meeting status --json | jq .echo_cancel
pactl list short sources | grep voxtype_meeting_aec_source
journalctl --user -u voxtype --since "1 minute ago" | grep "echo handling"
```

Then, with the meeting recording:

1. Play a talk (any interview or lecture video) through the speakers at your
   usual listening volume for about a minute.
2. Stay silent for the first 30 seconds of it.
3. For the next 30 seconds, say a few sentences of your own over it.
4. Stop the video, then the meeting:

```bash
voxtype meeting stop
voxtype meeting show latest --json \
  | jq -r '.transcript.segments[] | select(.source == "microphone") | .text'
jq .echo_cancel "$(voxtype meeting show latest --json | jq -r .metadata.storage_path)/meeting.json"
```

Expected:

- The active port is a speaker (for example `[Out] Speaker`), `echo_cancel`
  is `"webrtc"` in the status and in `meeting.json`, the source
  `voxtype_meeting_aec_source` is listed while recording, and the log says
  `Meeting echo handling: webrtc`.
- The microphone lines hold your own sentences and none of the talk. The
  talk appears only under Remote (the loopback track), if at all.
- A mixer (`wpctl status`, pavucontrol) lists "Voxtype meeting echo
  canceller" while the meeting records, and your default microphone does not
  change. After `meeting stop` it is gone.

For comparison, run the same minute with the canceller off, then put it
back:

```bash
voxtype config set meeting.audio.echo_cancel gtcrn && systemctl --user restart voxtype
# record the same 30 s of the talk with no speech, stop, check the mic lines
voxtype config set meeting.audio.echo_cancel auto && systemctl --user restart voxtype
```

With `gtcrn` some of the talk shows up on the microphone lines; that is the
echo the canceller removes.

Headphones and a crash:

```bash
# Plug in headphones (the default output moves to them), then:
voxtype meeting start --title "Headphones"
voxtype meeting status --json | jq .echo_cancel      # "gtcrn"
journalctl --user -u voxtype --since "1 minute ago" | grep "echo handling"
voxtype meeting stop
# Unplug, start a meeting on speakers again, and kill the daemon mid-meeting:
voxtype meeting start --title "Crash"
systemctl --user kill -s KILL voxtype
sleep 2; pactl list short sources | grep -c voxtype_meeting_aec_source   # 0
systemctl --user start voxtype
```

Expected: `gtcrn` with "the output is headphones" in the log; after the
kill no `voxtype_meeting_aec_source` is left and no `pipewire -c
…/meeting-aec.conf` process runs (`pgrep -af meeting-aec.conf` prints
nothing).

If your default microphone is a filter such as `camera-effects-mic`, the
canceller records from it; note whether the result differs from a plain
microphone.

## Clean up

```bash
voxtype meeting list
voxtype meeting delete <id> --force    # the meetings this test made, if you do not want them
voxtype config set meeting.retain_audio false   # if you do not keep audio normally
systemctl --user restart voxtype
```

## Results

| Step | Result | Notes |
|------|--------|-------|
| 0 Prepare | | |
| 1 Detection prompt | | |
| 2 Record and live view | | |
| 3 Minimize and move | | |
| 4 Fix the transcript | | |
| 5 Notes | | |
| 6 Ask | | |
| 7 Stop and summary | | |
| 8 Past meetings | | |
| 9 Export | | |
| 10 Kept audio and playback | | |
| 11 Tray offer | | |
| 12 Standalone host | | |
