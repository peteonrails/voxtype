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
              `meeting label`, pause, resume, stop through finalizing to idle
  autoopen    a meeting started elsewhere opens the closed view
  autoopen-off   ... unless [meeting] show_view_on_start is false
  autoopen-oldcli  a CLI without the key reads as on
  autoopen-settings  an open Settings panel is not taken over
  reconnect   the socket going away and coming back (daemon drop, restart)
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
import subprocess
import sys
import time

import panel_harness as h

SCENARIOS = ("live", "autoopen", "autoopen-off", "autoopen-oldcli", "autoopen-settings", "reconnect",
             "idle", "nodaemon", "scroll")

FAKE_CLI = h.HERE / "fake-voxtype-meeting.py"
FAKE_SOCK = h.HERE / "fake-meeting-sock.py"

MEETING_WINDOW_OLD = """PanelWindow {
  id: window
  anchors { top: true; right: true; bottom: true }
  implicitWidth: Style.space(460)
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


INITIAL = {
    "live": [running_meeting()],
    "reconnect": [running_meeting()],
    "scroll": [scroll_meeting()],
}

STATE = {
    "autoopen-off": {"show_view_on_start": False},
    "autoopen-oldcli": {"no_show_key": True},
    "idle": {"fail": {"start": "Error: Meeting mode is disabled in config."}},
}


def run(scenario, shell, evidence):
    with h.Run(shell, STATE.get(scenario, {})) as r:
        h.patch(r.plugin / "components/meetings/MeetingWindow.qml", MEETING_WINDOW_OLD, MEETING_WINDOW_NEW)
        h.patch(r.plugin / "Panel.qml", "readonly property alias formTest: form",
                "readonly property alias formTest: form\n  readonly property alias meetingsTest: meetings")
        fake = r.root / "voxtype-meeting-fake"
        fake.write_text("#!/bin/sh\nexec " + sys.executable + " -I " + str(FAKE_CLI) + ' "$@"\n')
        fake.chmod(0o755)
        r.dev_json(fake)

        runtime = r.root / "run"
        (runtime / "voxtype").mkdir(parents=True)
        ctl = r.root / "meeting-ctl.jsonl"
        ctl.write_text("".join(json.dumps(c) + "\n" for c in INITIAL.get(scenario, [])))
        srv_log = r.root / "meeting-srv.jsonl"
        srv_log.write_text("")
        sock = runtime / "voxtype" / "meeting.sock"
        shots = ""
        if evidence:
            shots = str((evidence / scenario).resolve())
            Path(shots).mkdir(parents=True, exist_ok=True)

        server = subprocess.Popen([sys.executable, "-I", str(FAKE_SOCK), str(sock), str(ctl), str(srv_log)])
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
                "__EVIDENCE__": shots, "__MEETING_ID__": MEETING_ID,
            }, env={"XDG_RUNTIME_DIR": str(runtime), "FAKE_MEETING_CTL": str(ctl)}, timeout=90)
        finally:
            with open(ctl, "a") as f:
                f.write(json.dumps({"cmd": "quit"}) + "\n")
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
        calls = r.calls()
        if evidence:
            out = evidence / scenario
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(r.log.read_text())
            (out / "socket.jsonl").write_text(srv_log.read_text())

    problems = h.driver_problems(output, code, "MEETING_VIEW", scenario)
    problems += check_calls(scenario, calls)
    return output, problems, calls


def check_calls(scenario, calls):
    """What the view ran, beyond what the driver saw on screen."""
    problems = []
    meeting = [c for c in calls if c[:1] == ["meeting"]]
    config = [c for c in calls if c[:1] == ["config"]]
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
    elif scenario in ("reconnect", "nodaemon", "scroll"):
        expect(meeting, [], "meeting commands")
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--shell", type=Path, default=Path(os.environ.get("OMARCHY_PATH", "/usr/share/omarchy")) / "shell")
    parser.add_argument("--evidence", type=Path, help="save logs and screenshots per scenario here")
    parser.add_argument("--scenario", choices=SCENARIOS, action="append")
    args = parser.parse_args()
    h.guard_environment(parser, args.shell)
    failed = False
    for scenario in args.scenario or SCENARIOS:
        output, problems, calls = run(scenario, args.shell, args.evidence)
        if problems:
            failed = True
            print(output)
            print("FAIL " + scenario + ": " + "; ".join(problems))
        else:
            print("PASS " + scenario)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
