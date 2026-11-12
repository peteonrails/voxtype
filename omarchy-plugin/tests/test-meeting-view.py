#!/usr/bin/env python3
"""The live meeting view, offscreen, against a fake meeting.sock and CLI.

Runs the real plugin (Panel.qml and components/meetings/) under Quickshell with
QT_QPA_PLATFORM=offscreen. tests/fake-meeting-sock.py binds meeting.sock in a
private XDG_RUNTIME_DIR and speaks the daemon's NDJSON contract;
tests/fake-voxtype-meeting.py stands in for `voxtype meeting …` and tells the
fake socket what the daemon would do. The plugin's shell is a stand-in that
records summons, so auto-open is observed without an Omarchy shell running.

Scenarios:
  live        a meeting already running: hello backlog, new segments inserted
              by start time, speaker_renamed, chip rename and merge through
              `meeting label`, pause, resume, stop through finalizing to Ended
  reassign    a line's speaker opens the chooser; Escape closes it; picking a
              speaker runs `meeting reassign` and moves the line
  reassign-fails  a refused reassign is rolled back and its stderr shown
  new-speaker     a typed name moves the line to a new speaker, or to an
                  existing one matched case-insensitively
  restore     Restore original runs `meeting reassign --reset`
  external-update segment_updated from elsewhere: original_speaker drives the
                  mark and Restore, and survives a reconnect
  ended-edit  an ended meeting stays on screen and can still be edited, until a
              new meeting starts or the view closes
  notes       the Notes tab autosaves through `meeting notes --set-file`,
              reloads on notes_updated, and never overwrites unsaved typing
  annotate    Add note on a line (`meeting annotate`), the margin marker, a
              highlight from elsewhere, and removing the note
  ask         a question streams its answer; a [m:ss] citation click shows
              that line of the transcript
  ask-chips   the three quick questions
  ask-notify  a quick question with the view closed goes out with --notify;
              the notification's Open raises the view on the Ask tab
  consent     first use of an agent asks; Not now asks nothing; Send allows it
  consent-choose  several agents and none allowed: one button each, or local
  local-only  the agent chip's menu: Local model only on and off, another
              agent, and (Omarchy) Change in Settings
  summary     stop with auto-summary: Summarizing, then summary_ready read
              through `show --json`, rendered with chapter links
  regenerate  Summarize, then Regenerate with new text
  scroll-tabs Transcript and Summary keep their scroll positions
  view-request  `voxtype meeting view` requests: a tab, a stale request, and a
              saved meeting that is not on screen
  list        no meeting: past meetings from `meeting list --json`, newest
              first, with date, length, speakers and a Summary badge; more on
              request past the first 50
  past-view   pick a past meeting (`meeting show --json`), see it as Ended,
              Back to the list
  past-summary  a past meeting's summary on the Summary tab
  past-summarize  a past meeting without one: Summarize renders the output
  past-reassign   moving a line of a past meeting sticks
  start-while-browsing  a meeting that starts while a past one is open takes
              over; when it ends, Back shows it at the top of the list
  list-old    a voxtype whose list has no has_summary/speaker_count: the rows
              read their folders instead
  cli-missing no voxtype to run: the list, Start and the assistant say so
  daemon-old  the daemon answers but publishes no meeting.sock
  agent-missing  no usable agent: the Ask tab says what to install; an
              unsupported agent shows its note in the menu
  old-voxtype a voxtype without the 1.3.0 commands: each feature says to update
  copy        Copy on an answer and on the summary (wl-copy), and wl-copy missing
  line-edit   Edit a line's text inline: Enter saves, Escape cancels, "edited"
              and Restore original text (`edit --reset`), a refusal rolls back
  line-delete Delete a line with Undo; it commits after the window, and the
              daemon's segment_deleted (or one from elsewhere) removes lines; a
              restored line (segment_updated for a line not shown) comes back
              in its place
  line-delete-flag  the same with deletion as segment_updated "deleted": true;
              closing the view commits a pending delete
  export      More > Export (markdown) into Documents/Voxtype, then Open folder;
              Open meeting folder
  export-past an SRT export of a past meeting (all five formats offered),
              and its folder
  minimize    the pill while recording (red dot, elapsed, the newest line),
              sized to what it says up to 460 px, where a long line elides;
              on the card's outer edge and dragged with it;
              click to expand, the --minimize/--expand requests
  pill-states the pill through a meeting: Paused, Finishing..., Ended with
              its length, Summarizing with the agent, Summary ready
  pill-idle   no meeting: "No meeting" and Start, which starts one; before
              anything is said the pill shows the title, a long one elided
  minimize-autoopen  a meeting start while minimized expands the one window
  resize      the inner-edge handle (clamped 280 px to half the screen) and
              compact height
  move        dragging the header: margin math, clamping, edge snapping, and
              the position surviving into a new instance from the state file
  opacity     [meeting] view_opacity, opaque on hover and while typing, back
              after the linger; the keyboard is asked for only while typing
  view-state  meeting-view.state: visible, minimized, heartbeat, and the
              close write
  play        ▶ on a line plays from it (`meeting play --segment`); the header
              transport and Stop; the line being heard follows position_ms;
              a stop from elsewhere arrives as a playback event
  play-no-audio  a meeting without kept audio: no ▶, one "Audio not kept" hint
  play-errors the CLI's refusals shown as they are: no kept audio (then no ▶
              and the hint), missing tools
  ask-error   an ask that fails shows the error event's message
  summary-error  a summary that fails after the meeting replaces Summarizing
              with the error card at the top; Retry writes it; no "Use …
              instead" when an agent was what failed
  summary-local-error  summaries going to a local model (summary_backend_name)
              while questions go to Claude: each chip names its own; the
              local failure offers "Use Claude instead" (`--agent claude`)
  summary-error-cli  Summarize failing in the CLI (a local model's 500) clears
              Summarizing into the card, twice; local_only hides "Use …"
  summary-stale  an error event naming no meeting (an older daemon) still
              ends Summarizing; a status without summary_backend falls back
  soundcheck  Sound check from the idle screen: both meters move, the mic and
              echo handling shown, Stop drops the meters, Check again runs to
              its end ("Done."), and back to the list
  soundcheck-no-loopback  no call audio and no echo cancellation said so;
              leaving the panel while it runs stops the check
  soundcheck-error  the command's error shown as it words it
  levels      live meters in the card from `levels` events: hidden until the
              first, the call meter only once loopback carries a number, a
              sound check's or another meeting's levels ignored, and the bars
              fall when levels stop
  search      the search box on the list: Searching..., results with the
              time, speaker and highlighted match, a result opening its
              meeting on the line, the query kept on Back, Escape back to the
              list, no matches, a title match
  search-old  a voxtype without `meeting search` is called too old
  recovery    unfinished meetings from `meeting recover --json` (the list
              calls them interrupted): the "Unfinished recording found" card
              with what is left to transcribe; Finish runs `recover --finish`
              and the card goes; Discard asks first, Keep it backs out,
              Discard for good runs `recover --discard`
  recovery-socket  the daemon's `recoverable` event brings a card while the
              view is open
  recovery-old  a voxtype without `meeting recover`: found from the list's
              active status, Finish says it finishes when voxtype next
              starts, Discard runs `meeting delete --force`
  undo-edit   Undo after an edit: `edit --reset` for a first correction, the
              earlier text for a later one; the bar leaves after its window
  undo-reassign  Undo after moving a line: `reassign --reset`; a refused move
              leaves nothing to undo
  seek        a line's time plays the kept audio from that line, and moves a
              running playback; without kept audio it is not a link
  waveform    an ended meeting with kept audio shows its waveform (the
              proposed `meeting waveform`); a click plays from that point
              (`play --at`), the playhead follows, Stop audio
  waveform-missing  a voxtype without `meeting waveform`: no strip, no error
  recovery-real  `meeting recover` answered by a real voxtype (REAL_VOXTYPE,
              run only when set) over a store seeded with two interrupted
              meetings: both cards, Finish and Discard for real
  live-labels  names given mid-meeting stick: a renamed Remote 1 keeps its
              name on lines that arrive later, and after the view reconnects
              (a fresh hello) both a talking and a silent renamed speaker
              keep theirs
  share       More > Share...: the preview of exactly what would leave and
              what it includes, the transcript toggle re-previewing, Copy,
              Save... (with Open folder) and Email... each sending only on a
              click, the sheet closing with Escape
  share-unavailable  greyed destinations say why (the CLI's reason, or what
              they need), the configured command offered and run, a refused
              send shown as the CLI words it
  share-old   a voxtype without `meeting share` is called too old
  share-real  Share against a real voxtype (REAL_VOXTYPE, run only when set)
              over a stored copy of the meeting with a summary and notes:
              the real preview, Copy into the clipboard, Save... writing a
              real file, Email... opening a mailto: draft
  share-real-unavailable  the real voxtype with no wl-copy or xdg-open on
              its PATH and a share command configured: greyed Copy and
              Email, the command run for real, a refused save shown
  autoopen    a meeting started elsewhere opens the closed view
  autoopen-off   ... unless [meeting] show_view_on_start is false
  autoopen-oldcli  a CLI without the key reads as on
  autoopen-gtk4  ... unless [meeting] view sends meetings to another view
  autoopen-settings  an open Settings panel is not taken over
  reconnect   the socket going away and coming back (daemon drop, restart,
              which leaves the meeting on screen as Ended)
  idle        the idle state, a failed start, a start with a title
  nodaemon    no socket at all, then the daemon arrives
  scroll      auto-scroll follows new lines until the user scrolls up
  notice      the recording-consent notice: shown at the top of a running
              meeting while unacknowledged, also on a meeting that starts
              with the view open; "Got it" hides it and runs
              `meeting notice --ack`
  notice-acked  acknowledged already: never shown, at open or at a start

Run inside a desktop-isolated sandbox (no WAYLAND_DISPLAY/DISPLAY, private
XDG_RUNTIME_DIR); it refuses to run with a display in the environment.
"""
import argparse
import calendar
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import time

import panel_harness as h

SCENARIOS = ("live", "live-labels", "autoopen", "autoopen-off", "autoopen-oldcli", "autoopen-gtk4", "autoopen-settings", "reconnect",
             "idle", "nodaemon", "scroll", "reassign", "reassign-fails", "new-speaker", "restore",
             "external-update", "ended-edit", "notes", "annotate", "ask", "ask-chips", "ask-notify", "consent",
             "consent-choose", "local-only", "summary", "regenerate", "scroll-tabs", "view-request",
             "list", "past-view", "past-summary", "past-summarize", "past-reassign", "start-while-browsing",
             "ask-error", "summary-error", "summary-local-error", "summary-error-cli", "summary-stale", "list-old", "cli-missing", "daemon-old", "agent-missing",
             "old-voxtype", "copy", "line-edit", "line-delete", "line-delete-flag", "export", "export-past",
             "minimize", "pill-states", "pill-idle", "minimize-autoopen", "resize", "move", "opacity", "view-state", "play", "play-no-audio",
             "play-errors", "soundcheck", "soundcheck-no-loopback", "soundcheck-error",
             "levels", "search", "search-old", "recovery", "recovery-socket", "recovery-old",
             "recovery-real", "share", "share-unavailable", "share-old",
             "share-real", "share-real-unavailable",
             "undo-edit", "undo-reassign", "seek", "waveform", "waveform-missing",
             "notice", "notice-acked")

FAKE_CLI = h.HERE / "fake-voxtype-meeting.py"
DEFAULT_ANSWER = ("[00:35] The build went green overnight, and Pete will cut the tag after lunch [00:50]. "
                  "Nobody raised a blocker.")
STANDALONE = h.PLUGIN.parent / "quickshell"
HOSTS = ("omarchy", "standalone")
# Scenarios about the Omarchy Settings panel itself.
OMARCHY_ONLY = {"autoopen-settings"}
FAKE_SOCK = h.HERE / "fake-meeting-sock.py"

MEETING_WINDOW_OLD = """PanelWindow {
  id: window
  anchors { top: true; left: true; bottom: window.placement.fullHeight }
  margins { top: window.placement.top; left: window.placement.left; bottom: window.placement.fullHeight ? window.placement.gap : 0 }
  implicitWidth: window.placement.windowWidth
  implicitHeight: window.placement.windowHeight
  color: "transparent"
  exclusionMode: ExclusionMode.Normal
  WlrLayershell.namespace: "voxtype-meetings"
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.keyboardFocus: window.theme.inputDemand > 0 ? WlrKeyboardFocus.OnDemand : WlrKeyboardFocus.None
"""
# A plain window of the card's size (full height is 780 here), over a busy
# backdrop so a screenshot shows the translucency. The keyboard policy the
# real window would ask for is kept as a property the driver can read.
MEETING_WINDOW_NEW = """FloatingWindow {
  id: window
  implicitWidth: window.placement.windowWidth
  implicitHeight: window.placement.fullHeight ? 780 : window.placement.windowHeight
  color: "#20242c"
  readonly property string keyboardPolicy: window.theme.inputDemand > 0 ? "on-demand" : "none"
  // The backdrop and the card in one item, for a screenshot of both.
  readonly property alias shotItem: shotRoot
  // Sized like the real window would be (an offscreen window keeps its first
  // size), so a narrow card or the pill looks the way it will.
  Item {
    id: shotRoot
    width: window.implicitWidth
    height: window.implicitHeight
    Grid {
      width: 480
      height: 800
      columns: 6
      Repeater {
        model: 120
        Rectangle {
          width: 80
          height: 40
          color: ["#3b6ea5", "#a5503b", "#3ba56e", "#a59a3b", "#6e3ba5", "#3ba5a0"][(index + Math.floor(index / 6)) % 6]
          Text { anchors.centerIn: parent; text: "Zoom call"; color: "white"; font.pixelSize: 11 }
        }
      }
    }
  }
  Component.onCompleted: window.view.parent = shotRoot
"""

MEETING_ID = "6f0c2a8e-1b7d-4c55-9a43-2d1e5f7b9c10"


def seg(id, start_s, text, source, speaker=None, chunk=0):
    s = {"id": id, "start_ms": int(start_s * 1000), "end_ms": int(start_s * 1000) + 1500, "text": text,
         "source": source, "chunk_id": chunk}
    if speaker:
        s["speaker_id"] = speaker
    return s


def running_meeting(seconds_ago=65):
    started = time.time() - seconds_ago
    iso = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(started)) + ".123456789Z"
    return {"cmd": "state", "status": "recording", "meeting_id": MEETING_ID, "title": "Standup",
            "started_at": iso, "chunk_count": 2, "diarization": {"enabled": True, "backend": "hybrid"},
            "segments": [
                seg(1, 4, "Morning. Can everyone hear me?", "microphone", "You", 0),
                seg(2, 9, "Yes, loud and clear.", "loopback", "Remote", 0),
                seg(3, 21, "Great, let's start with the release.", "microphone", "You", 0),
                seg(4, 35, "The build went green overnight.", "loopback", "Remote", 1),
            ]}


def seed_interrupted(real, data_home):
    """A real voxtype's meetings store holding the two STALE meetings, marked
    interrupted as the daemon's startup pass leaves them. The real binary
    makes the schema; the rows and folders are written here."""
    env = dict(os.environ, XDG_DATA_HOME=str(data_home))
    subprocess.run([real, "meeting", "list", "--json"], env=env, capture_output=True, check=True)
    store = data_home / "voxtype" / "meetings"
    db = sqlite3.connect(store / "index.db")
    now = int(time.time())
    for mid, title, ago in ((STALE[0], "Planning", 7200), (STALE[1], "Vendor call", 10800)):
        folder = store / mid
        folder.mkdir(parents=True)
        (folder / "transcript.json").write_text(json.dumps({"segments": [
            seg(0, 1, "Let's plan the week.", "microphone", "You", 0),
            seg(1, 31, "Sounds good.", "loopback", "Remote", 1)], "total_chunks": 2}))
        db.execute("INSERT INTO meetings (id, title, started_at, status, chunk_count, storage_path, audio_retained) "
                   "VALUES (?, ?, ?, 'interrupted', 2, ?, 0)", (mid, title, now - ago, str(folder)))
    db.commit()
    db.close()
def hybrid_meeting():
    """Hybrid diarization with two remote voices."""
    m = running_meeting()
    m["segments"] = [
        seg(1, 4, "Morning. Can everyone hear me?", "microphone", "You", 0),
        seg(2, 9, "Yes, loud and clear.", "loopback", "Remote 1", 0),
        seg(3, 15, "This meeting is being recorded.", "loopback", "Remote 2", 0),
        seg(4, 21, "Great, let's start with the release.", "microphone", "You", 0),
    ]
    return m


def seed_shared_meeting(real, data_home, config_home, folder, command=None):
    """The running meeting (MEETING_ID) stored the way a real voxtype keeps
    one, with a summary and notes, for `meeting share` to read; and a
    config naming the share folder (never the real ~/Documents)."""
    env = dict(os.environ, XDG_DATA_HOME=str(data_home), XDG_CONFIG_HOME=str(config_home))
    (config_home / "voxtype").mkdir(parents=True)
    conf = "[meeting.share]\nfolder = %s\n" % json.dumps(str(folder))
    if command:
        conf += "command = %s\n" % json.dumps(command)
    (config_home / "voxtype" / "config.toml").write_text(conf)
    subprocess.run([real, "meeting", "list", "--json"], env=env, capture_output=True, check=True)
    store = data_home / "voxtype" / "meetings"
    meeting_dir = store / MEETING_ID
    meeting_dir.mkdir(parents=True)
    segs = running_meeting()["segments"]
    (meeting_dir / "transcript.json").write_text(json.dumps({"segments": segs, "total_chunks": 2}))
    (meeting_dir / "notes.md").write_text("follow up with Steve\n")
    summary = {"summary": "A short standup about the release.", "key_points": [], "decisions": ["Ship today."],
               "action_items": [{"description": "Cut the tag", "assignee": "Pete", "completed": False}],
               "generated_at": "2026-11-12T16:00:00Z", "model": "claude"}
    now = int(time.time())
    db = sqlite3.connect(store / "index.db")
    db.execute("INSERT INTO meetings (id, title, started_at, ended_at, duration_secs, status, chunk_count, storage_path, "
               "audio_retained) VALUES (?, 'Standup', ?, ?, 70, 'completed', 2, ?, 0)",
               (MEETING_ID, now - 600, now - 530, str(meeting_dir)))
    db.execute("INSERT INTO meeting_summaries (meeting_id, summary_json, generated_by, generated_at) VALUES (?, ?, ?, ?)",
               (MEETING_ID, json.dumps(summary), "claude", now - 500))
    db.commit()
    db.close()


def scroll_meeting():
    m = running_meeting()
    m["segments"] = [seg(i + 1, i * 3, "Line number %d of a long meeting, long enough to wrap onto a second "
                                        "line in the narrow view." % (i + 1),
                         "microphone" if i % 2 == 0 else "loopback", "You" if i % 2 == 0 else "Remote", i // 10)
                     for i in range(40)]
    return m


# Built per run, so a meeting's start time is relative to its own scenario.
INITIAL = {
    "live": lambda: [running_meeting()],
    "live-labels": lambda: [hybrid_meeting()],
    "reconnect": lambda: [running_meeting()],
    "scroll": lambda: [scroll_meeting()],
    "reassign": lambda: [running_meeting()],
    "reassign-fails": lambda: [running_meeting()],
    "new-speaker": lambda: [running_meeting()],
    "restore": lambda: [running_meeting()],
    "external-update": lambda: [running_meeting()],
    "ended-edit": lambda: [running_meeting()],
    "notes": lambda: [running_meeting()],
    "annotate": lambda: [running_meeting()],
    "ask": lambda: [running_meeting()],
    "ask-chips": lambda: [running_meeting()],
    "ask-notify": lambda: [running_meeting()],
    "consent": lambda: [running_meeting()],
    "consent-choose": lambda: [running_meeting()],
    "local-only": lambda: [running_meeting()],
    "summary": lambda: [running_meeting()],
    "regenerate": lambda: [running_meeting()],
    "scroll-tabs": lambda: [scroll_meeting()],
    "view-request": lambda: [running_meeting()],
    "ask-error": lambda: [running_meeting()],
    "agent-missing": lambda: [running_meeting()],
    "old-voxtype": lambda: [running_meeting()],
    "copy": lambda: [running_meeting()],
    "line-edit": lambda: [running_meeting()],
    "line-delete": lambda: [running_meeting()],
    "line-delete-flag": lambda: [running_meeting()],
    "export": lambda: [running_meeting()],
    "minimize": lambda: [running_meeting()],
    "pill-states": lambda: [running_meeting()],
    "resize": lambda: [running_meeting()],
    "move": lambda: [running_meeting()],
    "opacity": lambda: [running_meeting()],
    "view-state": lambda: [running_meeting()],
    "play": lambda: [running_meeting()],
    "play-no-audio": lambda: [running_meeting()],
    "play-errors": lambda: [running_meeting()],
    "summary-error": lambda: [running_meeting()],
    "levels": lambda: [running_meeting()],
    "share": lambda: [running_meeting()],
    "share-unavailable": lambda: [running_meeting()],
    "share-old": lambda: [running_meeting()],
    "share-real": lambda: [running_meeting()],
    "share-real-unavailable": lambda: [running_meeting()],
    "undo-edit": lambda: [running_meeting()],
    "undo-reassign": lambda: [running_meeting()],
    "seek": lambda: [running_meeting()],
    "waveform": lambda: [running_meeting()],
    "waveform-missing": lambda: [running_meeting()],
    "summary-local-error": lambda: [running_meeting()],
    "summary-error-cli": lambda: [running_meeting()],
    "summary-stale": lambda: [running_meeting()],
    "notice": lambda: [running_meeting(), {"cmd": "notice", "pending": True}],
    "notice-acked": lambda: [running_meeting()],
}

STALE = ["c3000000-0000-4000-8000-000000000003", "c3000000-0000-4000-8000-000000000004"]
CLAUDE = {"id": "claude", "name": "Claude", "found": True, "supported": True, "consent": True}
CODEX = {"id": "codex", "name": "Codex", "found": True, "supported": True, "consent": True}
LONG_SUMMARY = {"summary": "\n\n".join("### Section %d\n\nA paragraph of summary text for section %d, long "
                                       "enough to take a couple of lines in the narrow view." % (i, i)
                                       for i in range(1, 41)),
                "key_points": [], "decisions": [], "action_items": [], "generated_at": "2026-11-11T16:00:00Z"}

STATE = {
    "autoopen-off": {"show_view_on_start": False},
    "autoopen-oldcli": {"no_show_key": True},
    "autoopen-gtk4": {"view": "gtk4"},
    "idle": {"fail": {"start": "Error: Meeting mode is disabled in config."}},
    "reassign-fails": {"fail": {"reassign": "Error: segment 2 cannot be reassigned: the transcript is locked."}},
    "consent": {"assistant": {"agent": None, "local_only": False, "candidates": [dict(CLAUDE, consent=False)]}},
    "consent-choose": {"assistant": {"agent": None, "local_only": False,
                                     "candidates": [dict(CLAUDE, consent=False), dict(CODEX, consent=False)]}},
    "local-only": {"assistant": {"agent": "claude", "local_only": False, "candidates": [CLAUDE, CODEX]}},
    "ask-error": {"fail": {"ask": "Claude failed: the agent exited with status 1."}},
    "daemon-old": {"daemon_running": True},
    "agent-missing": {"assistant": {"agent": None, "local_only": False, "candidates": [
        dict(CLAUDE, found=False),
        dict(CODEX, supported=False, note="bwrap is not installed, so codex cannot run in its jail")]}},
    "old-voxtype": {"old": True},
    "line-delete-flag": {"delete_shape": "flag"},
    "opacity": {"view_opacity": 0.5},
    "play": {"play_speed": 10},
    "play-no-audio": {"audio_retained": False},
    "play-errors": {"play_error": "This meeting was recorded without keeping audio. "
                                  "Turn on meeting.retain_audio for future meetings."},
    "scroll-tabs": {"summary_json": LONG_SUMMARY},
    "soundcheck-no-loopback": {"sc_no_loopback": True, "sc_echo": "none", "sc_time_scale": 0.02},
    "search": {"search_delay": 0.6},
    "recovery-old": {"recover_missing": True},
    "share-unavailable": {"share_available": {"clipboard": False, "email": False},
                          "share_why": {"clipboard": "wl-copy is not installed. Install wl-clipboard."},
                          "share_command": "post-standup --channel team",
                          "share_fail": {"file": "Documents/Voxtype is not writable. Check its permissions."}},
    "share-old": {"old": True},
    "recovery-socket": {"recover_array": True},
    "recovery": {"recover_summary": {"state": "failed", "message": "the local model answered 500"},
                 "recoverable": [
        {"id": STALE[0], "title": "Planning", "started_at": "2026-11-11T09:00:00Z", "last_chunk_ms": 760000,
         "has_audio": True, "untranscribed_chunks": 2},
        {"id": STALE[1], "title": "Vendor call", "started_at": "2026-11-11T08:00:00Z", "last_chunk_ms": 300000,
         "has_audio": False, "untranscribed_chunks": 1}]},
    "seek": {"play_speed": 10},
    "waveform": {"play_speed": 10},
    "waveform-missing": {"waveform_missing": True},
    "search-old": {"old": True},
    "soundcheck-error": {"sc_error": "Audio connection failed: no such device 'usb-mic'"},
    "summary-local-error": {"assistant": {"agent": None, "local_only": False, "candidates": [CLAUDE],
                                          "summary_backend": "local", "summary_backend_name": "Local (llama3.2)"}},
    "summary-error-cli": {"assistant": {"agent": None, "local_only": True, "candidates": [CLAUDE]},
                          "summary_fail": "Local model (llama3.2 at http://localhost:11434) failed: "
                                          "LLM request failed: http://localhost:11434/api/generate: status code 500"},
    "summary-stale": {"assistant": {"agent": None, "local_only": False, "candidates": [CLAUDE], "legacy_status": True}},
}


PAST = {
    "design": "a1000000-0000-4000-8000-000000000001",
    "one2one": "a1000000-0000-4000-8000-000000000002",
}

PAST_SUMMARY = {
    "summary": "The team agreed the new layout and who builds which part.",
    "key_points": [], "decisions": ["Ship the narrow card first."],
    "action_items": [{"description": "Draft the empty state", "assignee": "Dana", "completed": False}],
    "chapters": [{"start_ms": 3000, "title": "Layout"}, {"start_ms": 61000, "title": "Who builds what"}],
    "generated_at": "2026-11-10T16:00:00Z", "model": "claude",
}


def past_meetings(data, scenario):
    """Write past meetings the way storage keeps them; return the list entries."""
    entries = []

    def add(mid, title, started, minutes, segs, summary=None):
        folder = data / "meetings" / mid
        folder.mkdir(parents=True)
        start = calendar.timegm(time.strptime(started, "%Y-%m-%dT%H:%M:%SZ"))
        ended = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(start + minutes * 60))
        meta = {"id": mid, "title": title, "started_at": started, "ended_at": ended,
                "duration_secs": minutes * 60, "status": "completed", "chunk_count": minutes * 2,
                "storage_path": str(folder), "audio_retained": False}
        if title is None:
            meta.pop("title")
        entry = dict(meta)
        if scenario == "list-old":
            pass
        else:
            # What `meeting list --json` adds. The folder is out of reach here,
            # so the row can only have these from the list.
            entry["has_summary"] = summary is not None
            entry["speaker_count"] = len({s.get("speaker_id") or s["source"] for s in segs})
            entry["storage_path"] = "/nonexistent/" + mid
        entries.append(entry)
        if summary:
            meta["summary"] = summary
            (folder / "summary.md").write_text("## Overview\n")
        (folder / "metadata.json").write_text(json.dumps(meta))
        (folder / "transcript.json").write_text(json.dumps({"segments": segs, "total_chunks": 2}))

    add(PAST["design"], "Design review", "2026-11-10T15:00:00Z", 45, [
        seg(0, 3, "Let's look at the new layout.", "microphone", "You"),
        seg(1, 20, "I like the narrow card.", "loopback", "Remote 1"),
        seg(2, 61, "Who builds the empty state?", "loopback", "Remote 2"),
        seg(3, 75, "I can take it.", "loopback", "Remote 1"),
    ], PAST_SUMMARY)
    add(PAST["one2one"], "1:1 with Sam", "2026-11-09T10:30:00Z", 25, [
        seg(0, 2, "How was the week?", "microphone", "You"),
        seg(1, 9, "Busy, but good.", "loopback", "Remote"),
        seg(2, 30, "Any blockers?", "microphone", "You"),
    ])
    if scenario.startswith("recovery"):
        # Left active by a crash: no end, no duration.
        for mid, title, started in ((STALE[0], "Planning", "2026-11-11T09:00:00Z"),
                                    (STALE[1], "Vendor call", "2026-11-11T08:00:00Z")):
            if scenario == "recovery-old" and mid == STALE[1]:
                continue
            add(mid, title, started, 20, [seg(0, 1, "Let's plan the week.", "microphone", "You")])
            # The startup pass marks them interrupted; an older daemon left
            # them active.
            for e in entries:
                if e["id"] == mid:
                    e.update(status="active" if scenario == "recovery-old" else "interrupted",
                             duration_secs=None, ended_at=None)
    if scenario == "list":
        for i in range(58):
            day = 7 - i // 10
            add("b2%06d-0000-4000-8000-000000000000" % i, None if i == 0 else "Standup %d" % (58 - i),
                "2026-11-%02dT%02d:00:00Z" % (max(day, 1), 18 - i % 10), 15, [seg(0, 1, "Morning.", "microphone", "You")])
    return entries


PAST_SCENARIOS = {"list", "past-view", "past-summary", "past-summarize", "past-reassign", "start-while-browsing",
                  "list-old", "export-past", "search", "search-old", "recovery", "recovery-socket", "recovery-old"}


def run(scenario, host, shell, evidence):
    state = dict(STATE.get(scenario, {}))
    with h.Run(shell, state) as r:
        h.patch(r.plugin / "components/meetings/MeetingWindow.qml", MEETING_WINDOW_OLD, MEETING_WINDOW_NEW)
        # The standalone shell's tree, with the same window swap. Its
        # voxtype-meetings is the directory the plugin's link points at.
        shutil.copytree(STANDALONE, r.root / "standalone")
        h.patch(r.root / "standalone/voxtype-meetings/MeetingWindow.qml", MEETING_WINDOW_OLD, MEETING_WINDOW_NEW)
        data = r.root / "meeting-data"
        data.mkdir()
        if scenario in PAST_SCENARIOS:
            state["meetings"] = past_meetings(data, scenario)
            r.state.write_text(json.dumps(state))
        h.patch(r.plugin / "Panel.qml", "readonly property alias formTest: form",
                "readonly property alias formTest: form\n  readonly property alias meetingsTest: meetings")
        fake = r.root / "voxtype-meeting-fake"
        fake.write_text("#!/bin/sh\nexec " + sys.executable + " -I " + str(FAKE_CLI) + ' "$@"\n')
        fake.chmod(0o755)
        if scenario == "cli-missing":
            fake = r.root / "no-such-voxtype"
        r.dev_json(fake)
        # wl-copy that keeps what it was given.
        bindir = r.root / "bin"
        bindir.mkdir()
        clipboard = r.root / "clipboard.txt"
        (bindir / "wl-copy").write_text("#!/bin/sh\ncat >> " + str(clipboard) + "\nprintf '\\n<<END>>\\n' >> "
                                        + str(clipboard) + "\n")
        (bindir / "wl-copy").chmod(0o755)
        opened = r.root / "opened.txt"
        (bindir / "xdg-open").write_text("#!/bin/sh\nprintf '%s\\n' \"$*\" >> " + str(opened) + "\n")
        (bindir / "xdg-open").chmod(0o755)
        documents = r.root / "Documents"

        runtime = r.root / "run"
        (runtime / "voxtype").mkdir(parents=True)
        ctl = r.root / "meeting-ctl.jsonl"
        ctl.write_text("".join(json.dumps(c) + "\n" for c in INITIAL.get(scenario, lambda: [])()))
        srv_log = r.root / "meeting-srv.jsonl"
        srv_log.write_text("")
        snapshot = r.root / "meeting-snapshot.json"
        sock = runtime / "voxtype" / "meeting.sock"
        shots = ""
        label = host + "-" + scenario
        if evidence:
            shots = str((evidence / label).resolve())
            Path(shots).mkdir(parents=True, exist_ok=True)

        real_env = {}
        if scenario.startswith("share-real"):
            real = os.environ["REAL_VOXTYPE"]
            tools = r.root / "real-tools"
            tools.mkdir()
            if scenario == "share-real":
                # The scenario's own wl-copy and xdg-open, which record.
                shutil.copy(bindir / "wl-copy", tools / "wl-copy")
                shutil.copy(bindir / "xdg-open", tools / "xdg-open")
            for prog in ("sh", "cat"):
                (tools / prog).symlink_to(shutil.which(prog))
            share_folder = documents / "Voxtype"
            command = None
            if scenario == "share-real-unavailable":
                command = "cat > " + str(r.root / "shared-by-command.txt")
                share_folder.mkdir(parents=True)
                share_folder.chmod(0o500)
            seed_shared_meeting(real, r.root / "xdg-data", r.root / "xdg-config", share_folder, command)
            real_env = {"REAL_VOXTYPE": real, "XDG_DATA_HOME": str(r.root / "xdg-data"),
                        "XDG_CONFIG_HOME": str(r.root / "xdg-config"), "REAL_SHARE_PATH": str(tools)}
        if scenario == "recovery-real":
            real = os.environ["REAL_VOXTYPE"]
            seed_interrupted(real, r.root / "xdg-data")
            real_env = {"REAL_VOXTYPE": real, "XDG_DATA_HOME": str(r.root / "xdg-data")}
        server = subprocess.Popen([sys.executable, "-I", str(FAKE_SOCK), str(sock), str(ctl), str(srv_log),
                                   str(snapshot)])
        try:
            # The server binds before its first poll; wait for it so the plugin's
            # first attempt finds it (except where its absence is the point).
            for _ in range(100):
                if sock.exists():
                    break
                time.sleep(0.02)
            if scenario in ("nodaemon", "daemon-old"):
                with open(ctl, "a") as f:
                    f.write(json.dumps({"cmd": "down"}) + "\n")
                for _ in range(100):
                    if not sock.exists():
                        break
                    time.sleep(0.02)
            output, code = r.run(h.HERE / "meeting-view.qml", {
                "__SCENARIO__": scenario, "__CTL__": str(ctl), "__STATE__": str(r.state),
                "__EVIDENCE__": shots, "__MEETING_ID__": MEETING_ID, "__HOST__": host,
                "__FAKE_BIN__": str(fake), "__RUNTIME__": str(runtime / "voxtype"), "__DATA__": str(data),
                "__PAST_DESIGN__": PAST["design"], "__PAST_ONE2ONE__": PAST["one2one"],
                "__DOCUMENTS__": str(documents),
            }, env={"XDG_RUNTIME_DIR": str(runtime), "FAKE_MEETING_CTL": str(ctl),
                    "FAKE_MEETING_LOG": str(srv_log), "FAKE_MEETING_SNAPSHOT": str(snapshot),
                    "FAKE_MEETING_DATA": str(data),
                    "PATH": str(bindir) + os.pathsep + os.environ.get("PATH", ""),
                    "XDG_DOCUMENTS_DIR": str(documents), "XDG_STATE_HOME": str(r.root / "state"),
                    **real_env}, timeout=120)
        finally:
            with open(ctl, "a") as f:
                f.write(json.dumps({"cmd": "quit"}) + "\n")
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
        calls = r.calls()
        if evidence:
            out = evidence / label
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(r.log.read_text())
            (out / "socket.jsonl").write_text(srv_log.read_text())

        notes_file = data / ("notes-%s.md" % MEETING_ID)
        saved_notes = notes_file.read_text() if notes_file.exists() else None
        copied = clipboard.read_text() if clipboard.exists() else None
        opened_paths = opened.read_text().splitlines() if opened.exists() else []
        exports = sorted(str(p.relative_to(documents)) for p in documents.rglob("*") if p.is_file()) \
            if documents.exists() else []
        export_head = [(documents / e).read_text().splitlines()[0] for e in exports]
        export_text = {e: (documents / e).read_text() for e in exports}
        shared_by_command = r.root / "shared-by-command.txt"
        shared_by_command = shared_by_command.read_text() if shared_by_command.exists() else ""
        if (documents / "Voxtype").exists():
            (documents / "Voxtype").chmod(0o700)

    problems = h.driver_problems(output, code, "MEETING_VIEW", scenario)
    problems += check_calls(scenario, calls, runtime_notes=str(runtime / "voxtype"))
    if scenario == "notes" and saved_notes != "Mine":
        problems.append("notes on disk: %r, expected 'Mine'" % saved_notes)
    if scenario == "share" and opened_paths != ["/home/u/Documents/Voxtype"]:
        problems.append("opened: %s" % opened_paths)
    if scenario == "share-real":
        # Save... wrote the share; Email... with the transcript saved the
        # transcript to attach beside it.
        saved = [e for e in exports if e.startswith("Voxtype/") and e.endswith(".md")]
        shares = [e for e in saved if not e.endswith(" transcript.md")]
        attach = [e for e in saved if e.endswith(" transcript.md")]
        if len(shares) != 1 or len(attach) != 1 or "A short standup about the release." not in export_text[shares[0]]:
            problems.append("saved share: %s" % saved)
        if not copied or "A short standup about the release." not in copied or "Morning. Can everyone hear me?" not in copied:
            problems.append("clipboard: %r" % copied)
        drafts = [p for p in opened_paths if p.startswith("mailto:")]
        if len(drafts) != 1 or str(documents / "Voxtype") not in opened_paths:
            problems.append("opened: %s" % opened_paths)
    if scenario == "share-real-unavailable":
        sent = shared_by_command
        if "A short standup about the release." not in sent:
            problems.append("command got: %r" % sent)
    if scenario == "export":
        name = "Voxtype/" + time.strftime("%Y-%m-%d", time.localtime()) + " Standup.md"
        if exports != [name] or export_head != ["FAKE EXPORT %s markdown --timestamps --speakers --metadata" % MEETING_ID]:
            problems.append("exports: %s %s, expected %s" % (exports, export_head, name))
        if opened_paths != [str(documents / "Voxtype"), str(data / ("live-" + MEETING_ID))]:
            problems.append("opened: %s" % opened_paths)
    if scenario == "export-past":
        name = "Voxtype/2026-11-10 Design review.srt"
        if exports != [name] or export_head != ["FAKE EXPORT %s srt " % PAST["design"]]:
            problems.append("exports: %s %s" % (exports, export_head))
        if opened_paths != [str(data / "meetings" / PAST["design"])]:
            problems.append("opened: %s" % opened_paths)
    # The answer, then the summary's markdown.
    if scenario == "copy":
        parts = (copied or "").split("\n<<END>>\n")
        if len(parts) < 2 or parts[0] != DEFAULT_ANSWER or not parts[1].startswith("## Overview"):
            problems.append("clipboard: %r" % copied)
    return output, problems, calls


def read_only(argv):
    """Reads the view makes on its own (notes, assistant status, show)."""
    if argv[:3] == ["meeting", "assistant", "status"] or argv[:2] in (["meeting", "show"], ["meeting", "list"]):
        return True
    # The Settings panel's read of the name suggestion (Your name).
    if argv == ["meeting", "my-name", "--json"]:
        return True
    if argv == ["meeting", "play", "--status", "--json"] or argv[:2] == ["meeting", "waveform"] \
            or argv == ["meeting", "recover", "--json"]:
        return True
    return argv[:2] == ["meeting", "notes"] and "--set-file" not in argv


def documents_dir(runtime_notes):
    # run/voxtype -> the scenario root's Documents.
    return Path(runtime_notes).parent.parent / "Documents"


def ask_call(question, *extra):
    return ["meeting", "ask", MEETING_ID, question] + list(extra) + ["--json"]


def summarize_call(meeting=MEETING_ID, regenerate=False):
    return ["meeting", "summarize", meeting, "--format", "json"] + (["--regenerate"] if regenerate else [])


def reassign(segment, speaker):
    return ["meeting", "reassign", MEETING_ID, segment, speaker, "--json"]


def check_calls(scenario, calls, runtime_notes=""):
    """What the view ran, beyond what the driver saw on screen."""
    problems = []
    meeting = [c for c in calls if c[:1] == ["meeting"] and not read_only(c)]
    # Every open reads meeting.view_opacity; that is not what these check.
    config = [c for c in calls if (c[:2] == ["config", "set"] or c[:2] == ["config", "get"])
              and c != ["config", "get", "meeting.view_opacity"]]
    show_key = ["config", "get", "meeting.show_view_on_start"]
    view_key = ["config", "get", "meeting.view"]

    def expect(actual, wanted, what):
        if actual != wanted:
            problems.append("%s: %s, expected %s" % (what, json.dumps(actual), json.dumps(wanted)))

    if scenario == "live":
        expect(meeting, [
            ["meeting", "label", MEETING_ID, "Remote 1", "Alice"],
            ["meeting", "label", MEETING_ID, "Remote 2", "Alice"],
            ["meeting", "label", MEETING_ID, "Remote 1", "Team"],
            ["meeting", "label", MEETING_ID, "Remote 2", "Team"],
            ["meeting", "pause"], ["meeting", "resume"], ["meeting", "stop"],
        ], "meeting commands")
        expect(config, [], "config reads")
    elif scenario == "autoopen-off":
        expect(config, [show_key], "config reads")
        expect(meeting, [], "meeting commands")
    elif scenario in ("autoopen", "autoopen-oldcli", "autoopen-gtk4"):
        # On (or unreadable): then which view meetings go to.
        expect(config, [show_key, view_key], "config reads")
        expect(meeting, [], "meeting commands")
    elif scenario == "autoopen-settings":
        expect(show_key in config, False, "show_view_on_start read with Settings open")
        expect(meeting, [], "meeting commands")
    elif scenario == "idle":
        expect(meeting, [["meeting", "start", "--title", "Retro"], ["meeting", "start", "--title", "Retro"]],
               "meeting commands")
    elif scenario in ("reconnect", "nodaemon", "scroll", "external-update"):
        expect(meeting, [], "meeting commands")
    elif scenario in ("reassign", "reassign-fails"):
        expect(meeting, [reassign("2", "You")], "meeting commands")
    elif scenario == "new-speaker":
        expect(meeting, [reassign("4", "Dana"), reassign("2", "You")], "meeting commands")
    elif scenario == "restore":
        expect(meeting, [reassign("2", "You"), reassign("2", "--reset")], "meeting commands")
    elif scenario == "notes":
        sets = [c for c in meeting if c[:2] == ["meeting", "notes"]]
        want = ["meeting", "notes", MEETING_ID, "--set-file", runtime_notes + "/meeting-notes-%s.md" % MEETING_ID]
        if not sets or any(c != want for c in sets):
            problems.append("notes saves: %s, expected only %s" % (json.dumps(sets), json.dumps(want)))
        expect([c for c in meeting if c[:2] != ["meeting", "notes"]], [], "other meeting commands")
    elif scenario == "annotate":
        expect(meeting, [["meeting", "annotate", MEETING_ID, "3", "check this number", "--json"],
                         ["meeting", "annotate", MEETING_ID, "1", "--highlight", "--json"],
                         ["meeting", "annotate", MEETING_ID, "3", "--remove", "--json"]], "meeting commands")
    elif scenario == "ask":
        expect(meeting, [ask_call("What did we decide?")], "meeting commands")
    elif scenario == "ask-chips":
        expect(meeting, [ask_call("Summarize the last 5 minutes.", "--last", "5m"),
                         ask_call("What did Remote just say?", "--last", "2m"),
                         ask_call("What are the action items so far, and who owns each?")], "meeting commands")
    elif scenario == "ask-notify":
        expect(meeting, [ask_call("Summarize the last 5 minutes.", "--last", "5m", "--notify")], "meeting commands")
    elif scenario == "consent":
        expect(meeting, [["meeting", "assistant", "allow", "claude"], ask_call("What did we decide?"),
                         ask_call("And the date?")], "meeting commands")
    elif scenario == "consent-choose":
        expect(config, [["config", "set", "meeting.assistant.local_only", "true"]], "config writes")
        expect(meeting, [ask_call("What did we decide?")], "meeting commands")
    elif scenario == "local-only":
        expect(config, [["config", "set", "meeting.assistant.local_only", "true"],
                        ["config", "set", "meeting.assistant.local_only", "false"],
                        ["config", "set", "meeting.assistant.agent", "codex"],
                        ["config", "set", "meeting.assistant.local_only", "false"]], "config writes")
        expect(meeting, [], "meeting commands")
    elif scenario == "pill-states":
        expect(meeting, [["meeting", "pause"], ["meeting", "resume"], ["meeting", "stop"]], "meeting commands")
    elif scenario == "pill-idle":
        expect(meeting, [["meeting", "start"]], "meeting commands")
    elif scenario == "notice":
        expect(meeting, [["meeting", "stop"], ["meeting", "start", "--title", "Second"],
                         ["meeting", "notice", "--ack"]], "meeting commands")
    elif scenario == "notice-acked":
        expect(meeting, [["meeting", "stop"], ["meeting", "start", "--title", "Second"]], "meeting commands")
    elif scenario in ("cli-missing", "daemon-old", "old-voxtype", "minimize", "minimize-autoopen", "resize", "move",
                      "opacity", "view-state"):
        pass
    elif scenario == "line-edit":
        expect(meeting, [["meeting", "edit", MEETING_ID, "2", "Yes, very loud and clear.", "--json"],
                         ["meeting", "edit", MEETING_ID, "2", "--reset", "--json"],
                         ["meeting", "edit", MEETING_ID, "4", "It went red again.", "--json"]], "meeting commands")
    elif scenario in ("line-delete", "line-delete-flag"):
        expect(meeting, [["meeting", "delete-line", MEETING_ID, "2", "--json"]], "meeting commands")
    elif scenario == "export":
        expect(meeting, [["meeting", "export", MEETING_ID, "--format", "markdown", "--output",
                          str(documents_dir(runtime_notes)) + "/Voxtype/" + time.strftime("%Y-%m-%d") + " Standup.md",
                          "--timestamps", "--speakers", "--metadata"]], "meeting commands")
    elif scenario == "export-past":
        expect(meeting, [["meeting", "export", PAST["design"], "--format", "srt", "--output",
                          str(documents_dir(runtime_notes)) + "/Voxtype/2026-11-10 Design review.srt"]], "meeting commands")
    elif scenario == "play":
        start = ["meeting", "play", MEETING_ID, "--segment", "2", "--track", "mixed", "--json"]
        expect(meeting, [start, ["meeting", "play", "--stop", "--json"], start, ["meeting", "play", "--stop", "--json"]],
               "meeting commands")
    elif scenario == "play-no-audio":
        expect(meeting, [], "meeting commands")
    elif scenario == "play-errors":
        start = ["meeting", "play", MEETING_ID, "--segment", "2", "--track", "mixed", "--json"]
        expect(meeting, [start, start], "meeting commands")
    elif scenario == "soundcheck":
        check_ = ["meeting", "soundcheck", "--seconds", "30", "--json"]
        expect(meeting, [check_, ["meeting", "soundcheck", "--stop"], check_], "meeting commands")
    elif scenario == "soundcheck-no-loopback":
        expect(meeting, [["meeting", "soundcheck", "--seconds", "30", "--json"],
                         ["meeting", "soundcheck", "--stop"]], "meeting commands")
    elif scenario == "search":
        def q(text):
            return ["meeting", "search", "--limit", "30", "--json", "--", text]
        expect(meeting, [q("empty state"), q("zebra"), q("1:1")], "meeting commands")
    elif scenario == "search-old":
        expect(meeting, [["meeting", "search", "--limit", "30", "--json", "--", "layout"]], "meeting commands")
    elif scenario == "undo-edit":
        def ed(seg, *rest):
            return ["meeting", "edit", MEETING_ID, seg] + list(rest) + ["--json"]
        expect(meeting, [ed("2", "Yes, very loud and clear."), ed("2", "--reset"),
                         ed("4", "It went amber."), ed("4", "It went red."), ed("4", "It went amber.")],
               "meeting commands")
    elif scenario == "undo-reassign":
        expect(meeting, [reassign("2", "You"), ["meeting", "reassign", MEETING_ID, "2", "--reset", "--json"],
                         reassign("4", "You")], "meeting commands")
    elif scenario == "waveform":
        expect(meeting, [["meeting", "stop"],
                         ["meeting", "play", MEETING_ID, "--at", "20", "--track", "mixed", "--json"],
                         ["meeting", "play", "--stop", "--json"]], "meeting commands")
        waves = [c for c in calls if c[:2] == ["meeting", "waveform"]]
        if waves != [["meeting", "waveform", MEETING_ID, "--buckets", "160", "--json"]]:
            problems.append("waveform calls: " + json.dumps(waves))
    elif scenario == "waveform-missing":
        expect(meeting, [["meeting", "stop"]], "meeting commands")
    elif scenario == "seek":
        def play_from(seg):
            return ["meeting", "play", MEETING_ID, "--segment", seg, "--track", "mixed", "--json"]
        expect(meeting, [play_from("3"), play_from("1"), ["meeting", "play", "--stop", "--json"]], "meeting commands")
    elif scenario == "recovery":
        expect(meeting, [["meeting", "recover", "--finish", STALE[0], "--json"],
                         ["meeting", "recover", "--discard", STALE[1], "--json"]], "meeting commands")
    elif scenario == "share":
        sh = ["meeting", "share", MEETING_ID]
        expect(meeting, [sh + ["--preview", "--json"], sh + ["--preview", "--json", "--transcript"],
                         sh + ["--to", "clipboard", "--transcript", "--json"], sh + ["--to", "file", "--transcript", "--json"],
                         sh + ["--to", "email", "--transcript", "--json"]], "meeting commands")
    elif scenario == "share-unavailable":
        sh = ["meeting", "share", MEETING_ID]
        expect(meeting, [sh + ["--preview", "--json"], sh + ["--to", "command", "--json"], sh + ["--to", "file", "--json"]],
               "meeting commands")
    elif scenario == "share-real":
        sh = ["meeting", "share", MEETING_ID]
        expect(meeting, [sh + ["--preview", "--json"], sh + ["--preview", "--json", "--transcript"],
                         sh + ["--to", "clipboard", "--transcript", "--json"], sh + ["--to", "file", "--transcript", "--json"],
                         sh + ["--to", "email", "--transcript", "--json"]], "meeting commands")
    elif scenario == "share-real-unavailable":
        sh = ["meeting", "share", MEETING_ID]
        expect(meeting, [sh + ["--preview", "--json"], sh + ["--to", "command", "--json"], sh + ["--to", "file", "--json"]],
               "meeting commands")
    elif scenario == "share-old":
        expect(meeting, [["meeting", "share", MEETING_ID, "--preview", "--json"]], "meeting commands")
    elif scenario == "recovery-real":
        expect(meeting, [["meeting", "recover", "--finish", STALE[0], "--json"],
                         ["meeting", "recover", "--discard", STALE[1], "--json"]], "meeting commands")
    elif scenario == "recovery-socket":
        expect(meeting, [], "meeting commands")
    elif scenario == "recovery-old":
        expect(meeting, [["meeting", "delete", STALE[0], "--force"]], "meeting commands")
    elif scenario == "levels":
        expect(meeting, [], "meeting commands")
    elif scenario == "soundcheck-error":
        expect(meeting, [["meeting", "soundcheck", "--seconds", "30", "--json"]], "meeting commands")
    elif scenario == "live-labels":
        expect(meeting, [["meeting", "label", MEETING_ID, "Remote 1", "Chris"],
                         ["meeting", "label", MEETING_ID, "Remote 2", "Announcer"]], "meeting commands")
    elif scenario == "agent-missing":
        expect(meeting, [ask_call("What did we decide?")], "meeting commands")
    elif scenario == "copy":
        expect(meeting, [ask_call("What did we decide?"), ["meeting", "stop"], summarize_call()], "meeting commands")
    elif scenario == "ask-error":
        expect(meeting, [ask_call("What did we decide?")], "meeting commands")
    elif scenario in ("summary", "view-request", "summary-stale"):
        expect(meeting, [["meeting", "stop"]], "meeting commands")
    elif scenario == "summary-error":
        expect(meeting, [["meeting", "stop"], summarize_call()], "meeting commands")
    elif scenario == "summary-local-error":
        expect(meeting, [["meeting", "stop"], summarize_call() + ["--agent", "claude"]], "meeting commands")
    elif scenario == "summary-error-cli":
        expect(meeting, [["meeting", "stop"], summarize_call(), summarize_call()], "meeting commands")
    elif scenario == "regenerate":
        expect(meeting, [["meeting", "stop"], summarize_call(), summarize_call(regenerate=True)], "meeting commands")
    elif scenario == "scroll-tabs":
        expect(meeting, [["meeting", "stop"], summarize_call()], "meeting commands")
    elif scenario in ("list", "past-view", "past-summary", "list-old"):
        expect(meeting, [], "meeting commands")
        lists = [c for c in calls if c[:2] == ["meeting", "list"]]
        if not lists or any(c[2:4] != ["--json", "--limit"] for c in lists):
            problems.append("list calls: " + json.dumps(lists))
        if scenario == "list" and ["meeting", "list", "--json", "--limit", "100"] not in lists:
            problems.append("no second page asked for: " + json.dumps(lists))
    elif scenario == "past-summarize":
        expect(meeting, [summarize_call(PAST["one2one"])], "meeting commands")
    elif scenario == "past-reassign":
        expect(meeting, [["meeting", "reassign", PAST["one2one"], "1", "You", "--json"]], "meeting commands")
    elif scenario == "start-while-browsing":
        expect(meeting, [["meeting", "stop"]], "meeting commands")
    elif scenario == "ended-edit":
        expect(meeting, [["meeting", "stop"], reassign("2", "You"), ["meeting", "label", MEETING_ID, "Remote", "Sam"],
                         ["meeting", "start", "--title", "Second"], ["meeting", "stop"]], "meeting commands")
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--shell", type=Path, default=Path(os.environ.get("OMARCHY_PATH", "/usr/share/omarchy")) / "shell")
    parser.add_argument("--evidence", type=Path, help="save logs and screenshots per scenario here")
    parser.add_argument("--scenario", choices=SCENARIOS, action="append")
    parser.add_argument("--host", choices=HOSTS, action="append", help="default: both")
    args = parser.parse_args()
    h.guard_environment(parser, args.shell)
    failed = False
    for host in args.host or HOSTS:
        for scenario in args.scenario or SCENARIOS:
            if host != "omarchy" and scenario in OMARCHY_ONLY:
                continue
            # Needs a real voxtype with `meeting recover` / `meeting share`.
            if scenario in ("recovery-real", "share-real", "share-real-unavailable") \
                    and not os.environ.get("REAL_VOXTYPE"):
                continue
            output, problems, calls = run(scenario, host, args.shell, args.evidence)
            if problems:
                failed = True
                print(output)
                print("FAIL " + host + " " + scenario + ": " + "; ".join(problems))
            else:
                print("PASS " + host + " " + scenario)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
