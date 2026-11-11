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
  meeting reassign M SEG SPEAKER|--reset [--json]
                                move one segment, as the real command: SPEAKER
                                is an existing speaker ID or display name
                                (case-insensitive) or a new name; --reset puts
                                it back. Edits the fake socket's snapshot
                                (FAKE_MEETING_SNAPSHOT) through its control
                                file, waits until the socket applied it, and
                                with --json prints the updated segment, which
                                carries `original_speaker` (the pipeline's key)
                                while it is overridden, as the daemon's does
  config get meeting.show_view_on_start
                                state["show_view_on_start"] (default true);
                                state["no_show_key"] exits 2 like a CLI that
                                predates the key

The assistant (1.3.0 contract, see voxtype-meetings/MeetingAssistant.qml):

  meeting assistant status --json   state["assistant"], defaulting to Claude,
                                    allowed, with a local llama3.2
  meeting assistant allow AGENT     marks AGENT allowed (and current if none is)
  config set meeting.assistant.local_only true|false
  config set meeting.assistant.agent ID
  meeting ask M "Q" [--last 5m] [--notify]
                                    streams state["answer"] (or a default with
                                    [m:ss] citations) in chunks
  meeting summarize M               prints state["summary_md"] (or a default)
  meeting list --json [--limit N]   the socket's last meeting, then
                                    state["meetings"], newest first
  meeting show M --json             MeetingData for M: a past meeting's folder,
                                    or the fake socket's snapshot with its summary
  meeting notes M [--set-file F] [--json]
                                    notes kept in FAKE_MEETING_DATA; --json gives
                                    {meeting_id, notes, updated_at}; a set also
                                    makes the socket send notes_updated
  meeting annotate M SEG [TEXT] [--highlight] [--json] | M SEG --remove [--json]
                                    `annotation` {text, highlight} on the segment;
                                    --json prints the segment

A meeting ID may be a unique prefix, as with the real CLI. Edits to a past
meeting rewrite its transcript.json; edits to the socket's meetings go through
the socket, which announces live ones as segment_updated.

state["fail"] maps an action ("pause", "label", …) to the stderr it should
fail with (exit 1), changing nothing.
"""
import json
import os
import sys
import time


def load():
    try:
        with open(os.environ["FAKE_VOXTYPE_STATE"]) as f:
            return json.load(f)
    except (KeyError, FileNotFoundError):
        return {}


def control(cmd):
    with open(os.environ["FAKE_MEETING_CTL"], "a") as f:
        f.write(json.dumps(cmd) + "\n")


def wait_applied(nonce):
    log = os.environ["FAKE_MEETING_LOG"]
    until = time.time() + 5
    while time.time() < until:
        try:
            with open(log) as f:
                if nonce in f.read():
                    return True
        except FileNotFoundError:
            pass
        time.sleep(0.02)
    return False


def segment_key(seg):
    return seg.get("speaker_id") or {"microphone": "You", "loopback": "Remote"}.get(seg.get("source"), "Unknown")


# Meetings live in three places: the fake socket's running meeting and the
# last one it ended (its snapshot, edited through its control file so the
# socket can announce live changes), and past meetings in FAKE_MEETING_DATA/
# meetings/<id>/ (metadata.json, transcript.json), listed by state["meetings"].

def past_dir(meeting_id):
    return os.path.join(os.environ["FAKE_MEETING_DATA"], "meetings", meeting_id)


def all_ids(state):
    ids = [m["id"] for m in state.get("meetings", [])]
    snap = snapshot()
    for name in ("live", "stored"):
        m = snap.get(name)
        if m and m.get("meeting_id"):
            ids.append(m["meeting_id"])
    return ids


def resolve(state, meeting):
    """The full ID for an ID or a unique prefix, as the real CLI accepts."""
    ids = all_ids(state)
    if meeting in ids:
        return meeting
    hits = [i for i in ids if i.startswith(meeting)]
    return hits[0] if len(hits) == 1 else None


def segments_of(state, meeting):
    snap = snapshot()
    for name in ("live", "stored"):
        m = snap.get(name)
        if m and m.get("meeting_id") == meeting:
            return m["segments"]
    path = os.path.join(past_dir(meeting), "transcript.json")
    if os.path.exists(path):
        with open(path) as f:
            return json.load(f)["segments"]
    return None


def put_segment(state, meeting, seg):
    """Store one changed segment; a socket meeting announces it."""
    path = os.path.join(past_dir(meeting), "transcript.json")
    if os.path.exists(path):
        with open(path) as f:
            t = json.load(f)
        t["segments"] = [seg if s["id"] == seg["id"] else s for s in t["segments"]]
        with open(path, "w") as f:
            json.dump(t, f)
        return True
    nonce = "edit-%f" % time.time()
    control({"cmd": "update", "meeting_id": meeting, "segment": seg, "key": "event", "nonce": nonce})
    return wait_applied(nonce)


def find_segment(state, args_meeting, seg_arg):
    meeting = resolve(state, args_meeting)
    if meeting is None:
        print("error: Meeting not found: " + args_meeting, file=sys.stderr)
        return None, None, None
    segs = segments_of(state, meeting)
    seg = next((s for s in segs or [] if str(s["id"]) == seg_arg), None)
    if seg is None:
        print("error: no segment %s in meeting %s" % (seg_arg, meeting), file=sys.stderr)
        return None, None, None
    return meeting, segs, dict(seg)


def reassign(state, args):
    """meeting reassign M SEG SPEAKER|--reset [--json]"""
    want_json = "--json" in args
    rest = [a for a in args if a != "--json"]
    if len(rest) != 3:
        return 2
    meeting, segs, seg = find_segment(state, rest[0], rest[1])
    if seg is None:
        return 1
    speaker = rest[2]
    current = segment_key(seg)
    original = seg.pop("original_speaker", None)
    if speaker == "--reset":
        if original is None:
            print("error: segment %s has not been reassigned" % rest[1], file=sys.stderr)
            return 1
        new = original
        seg.pop("speaker_label", None)
    else:
        new = None
        for s in segs:
            key = segment_key(s)
            if key.lower() == speaker.lower() or (s.get("speaker_label") or "").lower() == speaker.lower():
                new = key
                if s.get("speaker_label"):
                    seg["speaker_label"] = s["speaker_label"]
                else:
                    seg.pop("speaker_label", None)
                break
        if new is None:
            # A new speaker, as the daemon names one: "Person N", labelled.
            n = 1 + len({segment_key(s) for s in segs if segment_key(s).startswith("Person ")})
            new = "Person %d" % n
            seg["speaker_label"] = speaker
        if original is None:
            original = current
        if original != new:
            seg["original_speaker"] = original
    seg["speaker_id"] = new
    if not put_segment(state, meeting, seg):
        print("error: the fake socket did not apply the change", file=sys.stderr)
        return 1
    print(json.dumps(seg, indent=2) if want_json else "Moved segment %s to %s" % (rest[1], new))
    return 0


DEFAULT_ASSISTANT = {
    "local_only": False, "local_model": "llama3.2", "agent": "claude", "agent_label": "Claude",
    "consented": True, "candidates": [{"id": "claude", "label": "Claude", "consented": True}],
}

DEFAULT_ANSWER = ("[0:35] The build went green overnight, and Pete will cut the tag after lunch [0:50]. "
                  "Nobody raised a blocker.")

DEFAULT_SUMMARY = """## Overview

A short standup about the release.

## Decisions

- Ship today.

## Chapters

- [0:04] Greetings
- [0:35] Release status
"""


def save(state):
    with open(os.environ["FAKE_VOXTYPE_STATE"], "w") as f:
        json.dump(state, f)


def labels_for(a):
    a["agent_label"] = next((c.get("label") for c in a["candidates"] if c["id"] == a.get("agent")), a.get("agent"))
    a["consented"] = any(c["id"] == a.get("agent") and c.get("consented") for c in a["candidates"])


def assistant(state, args):
    a = state.setdefault("assistant", dict(DEFAULT_ASSISTANT, candidates=[dict(c) for c in DEFAULT_ASSISTANT["candidates"]]))
    if args == ["status", "--json"]:
        print(json.dumps(a))
        return 0
    if args[:1] == ["allow"] and len(args) == 2:
        for c in a["candidates"]:
            if c["id"] == args[1]:
                c["consented"] = True
        if not a.get("agent"):
            a["agent"] = args[1]
        labels_for(a)
        save(state)
        return 0
    return 2


def config_set(state, key, value):
    a = state.setdefault("assistant", dict(DEFAULT_ASSISTANT, candidates=[dict(c) for c in DEFAULT_ASSISTANT["candidates"]]))
    if key == "meeting.assistant.local_only":
        a["local_only"] = value == "true"
    elif key == "meeting.assistant.agent":
        a["agent"] = value
        labels_for(a)
    else:
        return 2
    save(state)
    return 0


def snapshot():
    try:
        with open(os.environ["FAKE_MEETING_SNAPSHOT"]) as f:
            return json.load(f)
    except FileNotFoundError:
        return {}


def ask(state, args):
    rest = [a for a in args if a not in ("--notify",)]
    if "--last" in rest:
        i = rest.index("--last")
        rest = rest[:i] + rest[i + 2:]
    if len(rest) != 2:
        return 2
    text = state.get("answer", DEFAULT_ANSWER)
    words = text.split(" ")
    for i in range(0, len(words), 4):
        sys.stdout.write(" ".join(words[i:i + 4]) + (" " if i + 4 < len(words) else ""))
        sys.stdout.flush()
        time.sleep(0.15)
    sys.stdout.write("\n")
    return 0


def notes(state, args):
    """meeting notes M [--set-file PATH] [--json]"""
    want_json = "--json" in args
    rest = [a for a in args if a != "--json"]
    meeting = resolve(state, rest[0]) if rest else None
    if meeting is None:
        print("error: Meeting not found", file=sys.stderr)
        return 1
    path = os.path.join(os.environ["FAKE_MEETING_DATA"], "notes-%s.md" % meeting)
    if len(rest) == 3 and rest[1] == "--set-file":
        with open(rest[2]) as src, open(path, "w") as dst:
            dst.write(src.read())
        control({"cmd": "notes_updated", "meeting_id": meeting})
    elif len(rest) != 1:
        return 2
    text = None
    if os.path.exists(path):
        with open(path) as f:
            text = f.read()
    if want_json:
        print(json.dumps({"meeting_id": meeting, "notes": text, "updated_at": None}, indent=2))
    elif text is not None:
        sys.stdout.write(text)
    return 0


def annotate(state, args):
    """meeting annotate M SEG [TEXT] [--highlight] [--json] | M SEG --remove [--json]"""
    want_json = "--json" in args
    highlight = "--highlight" in args
    rest = [a for a in args if a not in ("--json", "--highlight")]
    if len(rest) not in (2, 3):
        return 2
    meeting, _, seg = find_segment(state, rest[0], rest[1])
    if seg is None:
        return 1
    if len(rest) == 3 and rest[2] == "--remove":
        seg.pop("annotation", None)
    else:
        seg["annotation"] = {"text": rest[2] if len(rest) == 3 else "", "highlight": highlight}
    if not put_segment(state, meeting, seg):
        print("error: the fake socket did not apply the change", file=sys.stderr)
        return 1
    print(json.dumps(seg, indent=2) if want_json else "Annotated segment %s" % rest[1])
    return 0


def list_meetings(state, args):
    """meeting list --json [--limit N]: the socket's last meeting, then the past ones."""
    limit = int(args[args.index("--limit") + 1]) if "--limit" in args else 10
    out = []
    stored = snapshot().get("stored")
    if stored and stored.get("meeting_id"):
        out.append({"id": stored["meeting_id"], "title": stored.get("title"), "started_at": stored.get("started_at"),
                    "duration_secs": 70, "status": "completed", "chunk_count": 3, "audio_retained": False})
    out += sorted(state.get("meetings", []), key=lambda m: m["started_at"], reverse=True)
    print(json.dumps(out[:limit], indent=2))
    return 0


def show(state, args_meeting):
    meeting = resolve(state, args_meeting)
    if meeting is None:
        print("error: Meeting not found: " + args_meeting, file=sys.stderr)
        return 1
    folder = past_dir(meeting)
    if os.path.isdir(folder):
        with open(os.path.join(folder, "metadata.json")) as f:
            meta = json.load(f)
        with open(os.path.join(folder, "transcript.json")) as f:
            transcript = json.load(f)
        print(json.dumps({"metadata": meta, "transcript": transcript, "asks": []}, indent=2))
        return 0
    snap = snapshot()
    target = next(m for m in (snap.get("live"), snap.get("stored")) if m and m.get("meeting_id") == meeting)
    meta = {"id": meeting, "title": target.get("title"), "started_at": target.get("started_at"),
            "status": "completed", "chunk_count": 3, "audio_retained": False}
    if snap.get("summary"):
        meta["summary"] = snap["summary"]
    print(json.dumps({"metadata": meta, "transcript": {"segments": target["segments"], "total_chunks": 3},
                      "asks": []}, indent=2))
    return 0


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

    if args[:2] == ["config", "set"] and len(args) == 4:
        return config_set(state, args[2], args[3])

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
    elif action == "reassign":
        return reassign(state, args[2:])
    elif action == "assistant":
        return assistant(state, args[2:])
    elif action == "ask":
        return ask(state, args[2:])
    elif action == "summarize" and len(args) >= 3:
        time.sleep(0.6)
        print(state.get("summary_md", DEFAULT_SUMMARY))
    elif action == "show" and len(args) == 4 and args[3] == "--json":
        return show(state, args[2])
    elif action == "list" and "--json" in args:
        return list_meetings(state, args)
    elif action == "notes":
        return notes(state, args[2:])
    elif action == "annotate":
        return annotate(state, args[2:])
    elif action == "label" and len(args) == 5:
        control({"cmd": "label", "key": args[3], "name": args[4]})
        print("Labeled %s as '%s' in meeting %s" % (args[3], args[4], args[2]))
    else:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
