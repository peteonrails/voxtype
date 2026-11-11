#!/usr/bin/env python3
"""Fake `voxtype` for the meeting view tests. Never touches a real daemon.

Appends every argv to FAKE_VOXTYPE_LOG and answers the commands the meeting
view runs, standing in for the daemon by appending what it would do to the
fake meeting.sock's control file (FAKE_MEETING_CTL; see fake-meeting-sock.py):

  meeting pause | resume        status paused | recording
  meeting stop                  finalizing, then idle
  meeting start [--title T]     a new recording meeting
  meeting label M KEY NAME      later segments for KEY carry NAME; the socket
                                announces nothing, as the daemon does not
  config get meeting.show_view_on_start
                                state["show_view_on_start"] (default true);
                                state["no_show_key"] exits 2 like a CLI that
                                predates the key

state["fail"] maps an action ("pause", "label", …) to the stderr it should
fail with (exit 1), changing nothing.
"""
import json
import os
import sys


def load():
    try:
        with open(os.environ["FAKE_VOXTYPE_STATE"]) as f:
            return json.load(f)
    except (KeyError, FileNotFoundError):
        return {}


def control(cmd):
    with open(os.environ["FAKE_MEETING_CTL"], "a") as f:
        f.write(json.dumps(cmd) + "\n")


def main():
    args = sys.argv[1:]
    log = os.environ.get("FAKE_VOXTYPE_LOG")
    if log:
        with open(log, "a") as f:
            f.write(json.dumps(args) + "\n")
    state = load()

    if args == ["config", "get", "meeting.show_view_on_start"]:
        if state.get("no_show_key"):
            print("error: unknown config key 'meeting.show_view_on_start'.", file=sys.stderr)
            return 2
        print("true" if state.get("show_view_on_start", True) else "false")
        return 0

    if args[:1] != ["meeting"] or len(args) < 2:
        return 2
    action = args[1]
    failing = state.get("fail", {})
    if action in failing:
        print(failing[action], file=sys.stderr)
        return 1
    if action == "pause":
        control({"cmd": "status", "status": "paused"})
    elif action == "resume":
        control({"cmd": "status", "status": "recording"})
    elif action == "stop":
        control({"cmd": "stop"})
    elif action == "start":
        title = args[args.index("--title") + 1] if "--title" in args else ""
        control({"cmd": "start", "title": title})
    elif action == "label" and len(args) == 5:
        control({"cmd": "label", "key": args[3], "name": args[4]})
        print("Labeled %s as '%s' in meeting %s" % (args[3], args[4], args[2]))
    else:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
