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
  autoopen    a meeting started elsewhere opens the closed view
  autoopen-off   ... unless [meeting] show_view_on_start is false
  autoopen-oldcli  a CLI without the key reads as on
  autoopen-settings  an open Settings panel is not taken over
  reconnect   the socket going away and coming back (daemon drop, restart,
              which leaves the meeting on screen as Ended)
  idle        the idle state, a failed start, a start with a title
  nodaemon    no socket at all, then the daemon arrives
  scroll      auto-scroll follows new lines until the user scrolls up

Run inside a desktop-isolated sandbox (no WAYLAND_DISPLAY/DISPLAY, private
XDG_RUNTIME_DIR); it refuses to run with a display in the environment.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

import panel_harness as h

SCENARIOS = ("live", "autoopen", "autoopen-off", "autoopen-oldcli", "autoopen-settings", "reconnect",
             "idle", "nodaemon", "scroll", "reassign", "reassign-fails", "new-speaker", "restore",
             "external-update", "ended-edit", "notes", "annotate", "ask", "ask-chips", "ask-notify", "consent",
             "consent-choose", "local-only", "summary", "regenerate", "scroll-tabs", "view-request")

FAKE_CLI = h.HERE / "fake-voxtype-meeting.py"
STANDALONE = h.PLUGIN.parent / "quickshell"
HOSTS = ("omarchy", "standalone")
# Scenarios about the Omarchy Settings panel itself.
OMARCHY_ONLY = {"autoopen-settings"}
FAKE_SOCK = h.HERE / "fake-meeting-sock.py"

MEETING_WINDOW_OLD = """PanelWindow {
  id: window
  anchors { top: true; right: true; bottom: true }
  implicitWidth: 460
  color: "transparent"
  exclusionMode: ExclusionMode.Normal
  WlrLayershell.namespace: "voxtype-meetings"
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.keyboardFocus: WlrKeyboardFocus.OnDemand
"""
MEETING_WINDOW_NEW = """FloatingWindow {
  id: window
  implicitWidth: 460
  implicitHeight: 780
  color: "black"
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
}

CLAUDE = {"id": "claude", "label": "Claude", "consented": True}
CODEX = {"id": "codex", "label": "Codex", "consented": True}
LONG_SUMMARY = "## Overview\n\n" + "\n\n".join("### Section %d\n\nA paragraph of summary text for section %d, long "
                                                  "enough to take a couple of lines in the narrow view." % (i, i)
                                                  for i in range(1, 41))

STATE = {
    "autoopen-off": {"show_view_on_start": False},
    "autoopen-oldcli": {"no_show_key": True},
    "idle": {"fail": {"start": "Error: Meeting mode is disabled in config."}},
    "reassign-fails": {"fail": {"reassign": "Error: segment 2 cannot be reassigned: the transcript is locked."}},
    "consent": {"assistant": {"local_only": False, "local_model": "llama3.2", "agent": "claude", "agent_label": "Claude",
                              "consented": False, "candidates": [dict(CLAUDE, consented=False)]}},
    "consent-choose": {"assistant": {"local_only": False, "local_model": "llama3.2", "agent": None, "agent_label": None,
                                     "consented": False, "candidates": [dict(CLAUDE, consented=False),
                                                                         dict(CODEX, consented=False)]}},
    "local-only": {"assistant": {"local_only": False, "local_model": "llama3.2", "agent": "claude", "agent_label": "Claude",
                                 "consented": True, "candidates": [CLAUDE, CODEX]}},
    "scroll-tabs": {"summary_md": LONG_SUMMARY},
}


def run(scenario, host, shell, evidence):
    with h.Run(shell, STATE.get(scenario, {})) as r:
        h.patch(r.plugin / "components/meetings/MeetingWindow.qml", MEETING_WINDOW_OLD, MEETING_WINDOW_NEW)
        # The standalone shell's tree, with the same window swap. Its
        # voxtype-meetings is the directory the plugin's link points at.
        shutil.copytree(STANDALONE, r.root / "standalone")
        h.patch(r.root / "standalone/voxtype-meetings/MeetingWindow.qml", MEETING_WINDOW_OLD, MEETING_WINDOW_NEW)
        data = r.root / "meeting-data"
        data.mkdir()
        h.patch(r.plugin / "Panel.qml", "readonly property alias formTest: form",
                "readonly property alias formTest: form\n  readonly property alias meetingsTest: meetings")
        fake = r.root / "voxtype-meeting-fake"
        fake.write_text("#!/bin/sh\nexec " + sys.executable + " -I " + str(FAKE_CLI) + ' "$@"\n')
        fake.chmod(0o755)
        r.dev_json(fake)

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

        server = subprocess.Popen([sys.executable, "-I", str(FAKE_SOCK), str(sock), str(ctl), str(srv_log),
                                   str(snapshot)])
        try:
            # The server binds before its first poll; wait for it so the plugin's
            # first attempt finds it (except where its absence is the point).
            for _ in range(100):
                if sock.exists():
                    break
                time.sleep(0.02)
            if scenario == "nodaemon":
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
            }, env={"XDG_RUNTIME_DIR": str(runtime), "FAKE_MEETING_CTL": str(ctl),
                    "FAKE_MEETING_LOG": str(srv_log), "FAKE_MEETING_SNAPSHOT": str(snapshot),
                    "FAKE_MEETING_DATA": str(data)}, timeout=120)
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

    problems = h.driver_problems(output, code, "MEETING_VIEW", scenario)
    problems += check_calls(scenario, calls, runtime_notes=str(runtime / "voxtype"))
    if scenario == "notes" and saved_notes != "Mine":
        problems.append("notes on disk: %r, expected 'Mine'" % saved_notes)
    return output, problems, calls


def read_only(argv):
    """Reads the view makes on its own (notes, assistant status, show)."""
    if argv[:3] == ["meeting", "assistant", "status"] or argv[:2] == ["meeting", "show"]:
        return True
    return argv[:2] == ["meeting", "notes"] and "--set-file" not in argv


def ask_call(question, *extra):
    return ["meeting", "ask", MEETING_ID, question] + list(extra)


def summarize_call():
    return ["meeting", "summarize", MEETING_ID, "--regenerate"]


def reassign(segment, speaker):
    return ["meeting", "reassign", MEETING_ID, segment, speaker, "--json"]


def check_calls(scenario, calls, runtime_notes=""):
    """What the view ran, beyond what the driver saw on screen."""
    problems = []
    meeting = [c for c in calls if c[:1] == ["meeting"] and not read_only(c)]
    config = [c for c in calls if c[:2] == ["config", "set"] or c[:2] == ["config", "get"]]
    show_key = ["config", "get", "meeting.show_view_on_start"]

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
    elif scenario in ("autoopen", "autoopen-off", "autoopen-oldcli"):
        expect(config, [show_key], "config reads")
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
        expect(meeting, [["meeting", "annotate", MEETING_ID, "3", "check this number"],
                         ["meeting", "annotate", MEETING_ID, "3", "--remove"]], "meeting commands")
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
    elif scenario in ("summary", "view-request"):
        expect(meeting, [["meeting", "stop"]], "meeting commands")
    elif scenario == "regenerate":
        expect(meeting, [["meeting", "stop"], summarize_call(), summarize_call()], "meeting commands")
    elif scenario == "scroll-tabs":
        expect(meeting, [["meeting", "stop"], summarize_call()], "meeting commands")
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
