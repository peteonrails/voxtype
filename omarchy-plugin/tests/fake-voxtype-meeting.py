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

  meeting assistant status --json   the real shape, derived from
                                    state["assistant"] (default: Claude found
                                    and allowed, a local llama3.2)
  meeting assistant allow AGENT     marks AGENT allowed
  config set meeting.assistant.local_only true|false
  config set meeting.assistant.agent ID
  meeting ask M "Q" [--last 5m] [--notify] --json
                                    the NDJSON stream: start, deltas of
                                    state["answer"], answer with citations, or
                                    error (consent_required while blocked,
                                    agent_failed with state["fail"]["ask"]);
                                    each answer is saved for `show --json`
  meeting summarize M [--regenerate] [--agent ID] --format json
                                    the saved MeetingSummary, or a new one
                                    (state["summary_json"] or a default); refused
                                    on stderr while consent is missing
  meeting list --json [--limit N]   the socket's last meeting, then
                                    state["meetings"] (with or without
                                    has_summary/speaker_count, as the test
                                    wrote them), newest first
  meeting show M --json             MeetingData for M: a past meeting's folder,
                                    or the fake socket's snapshot with its summary
  meeting notes M [--set-file F] [--json]
                                    notes kept in FAKE_MEETING_DATA; --json gives
                                    {meeting_id, notes, updated_at}; a set also
                                    makes the socket send notes_updated
  meeting annotate M SEG [TEXT] [--highlight] [--json] | M SEG --remove [--json]
                                    `annotation` {text, highlight} on the segment;
                                    --json prints the segment

  meeting edit M SEG TEXT|--reset [--json]
                                    a line's new text, keeping what was
                                    transcribed in `original_text`; --reset puts
                                    it back; empty TEXT is refused; prints the
                                    segment
  meeting delete-line M SEG [--json]
                                    removes the line; the socket announces it as
                                    {"event":"segment_deleted"} (the daemon's
                                    shape) or, with state["delete_shape"] ==
                                    "flag", as segment_updated with "deleted":
                                    true, which the view also tolerates
  meeting export M --format F --output PATH [--timestamps --speakers --metadata]
                                    writes a small stand-in file at PATH

  meeting play M --segment S [--track T] --json | --status --json | --stop --json
                                    playback without a player: a state file in
                                    FAKE_MEETING_DATA, `position_ms` advancing at
                                    state["play_speed"] (default 1) times real
                                    time, and a `playback` event on the socket.
                                    state["play_error"] is the stderr of a
                                    refused play (exit 1)

`show --json` reports metadata.audio_retained from state["audio_retained"]
(default true). `export` accepts text, markdown and json, as the real one.

A meeting ID may be a unique prefix, as with the real CLI. Edits to a past
meeting rewrite its transcript.json; edits to the socket's meetings go through
the socket, which announces live ones as segment_updated.

state["fail"] maps an action ("pause", "label", …) to the stderr it should
fail with (exit 1), changing nothing. state["old"] makes the commands newer
than 1.3.0 (assistant, ask, summarize, notes, annotate, reassign) fail the way
an older voxtype does: "unrecognized subcommand", exit 2. `status --format
json` exits 0 only with state["daemon_running"].
"""
import json
import os
import re
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


# The assistant as configured: which agent is set (or none: the first found
# and supported one answers), local_only, and per candidate whether it is
# found, supported and allowed. `assistant status --json` derives the rest the
# way the real command does (docs/INTEGRATIONS.md).
DEFAULT_ASSISTANT = {
    "agent": None, "local_only": False,
    "candidates": [{"id": "claude", "name": "Claude", "found": True, "supported": True, "consent": True}],
}

CODEX_NOTE = ("its command tool cannot be turned off, so it runs in a bwrap jail that sees no files "
              "from your home but its auth.json")

DEFAULT_ANSWER = ("[00:35] The build went green overnight, and Pete will cut the tag after lunch [00:50]. "
                  "Nobody raised a blocker.")

DEFAULT_SUMMARY = {
    "summary": "A short standup about the release.", "key_points": [], "decisions": ["Ship today."],
    "action_items": [{"description": "Cut the tag", "assignee": "Pete", "completed": False}],
    "chapters": [{"start_ms": 4000, "title": "Greetings"}, {"start_ms": 35000, "title": "Release status"}],
    "generated_at": "2026-11-11T16:00:00Z", "model": "claude",
}


def save(state):
    with open(os.environ["FAKE_VOXTYPE_STATE"], "w") as f:
        json.dump(state, f)


def assistant_config(state):
    if "assistant" not in state:
        state["assistant"] = json.loads(json.dumps(DEFAULT_ASSISTANT))
    return state["assistant"]


def status(state):
    a = assistant_config(state)
    cands = []
    for c in a["candidates"]:
        cand = {"id": c["id"], "name": c["name"], "path": "/usr/bin/" + c["id"], "found": c.get("found", True),
                "supported": c.get("supported", True), "tool_less": c["id"] == "claude",
                "sandbox": "bwrap" if c["id"] == "codex" else None, "consent": c.get("consent", False),
                "sources": ["path"]}
        if c.get("note") or c["id"] == "codex":
            cand["note"] = c.get("note", CODEX_NOTE)
        cands.append(cand)
    usable = [c for c in cands if c["found"] and c["supported"]]
    agent = next((c for c in cands if c["id"] == a.get("agent")), None) if a.get("agent") else None
    choice = False
    if agent is None:
        allowed = [c for c in usable if c["consent"]]
        agent = allowed[0] if allowed else (usable[0] if len(usable) == 1 else None)
        choice = agent is None and len(usable) > 1
    local = {"model": "llama3.2", "url": "http://localhost:11434", "name": "Local (llama3.2)"}
    if a.get("local_only"):
        backend, name, blocked = "local", local["name"], None
    elif agent and agent["consent"]:
        backend, name, blocked = "agent", agent["name"], None
    elif agent or choice:
        backend, name, blocked = "none", None, "consent_required"
    else:
        backend, name, blocked = "none", None, "no_backend"
    out = {"agent": agent["id"] if agent else None, "agent_name": agent["name"] if agent else None,
            "agent_path": agent["path"] if agent else None, "source": "config" if a.get("agent") else "path",
            "consent": bool(agent and agent["consent"]), "choice_required": choice, "problem": None,
            "omarchy_default": None, "local_only": bool(a.get("local_only")), "local": local,
            "backend": backend, "backend_name": name, "blocked": blocked, "auto_summary": False,
            "auto_summary_configured": None, "consent_file": "/tmp/assistant-consent.json", "candidates": cands}
    # Where summaries go: a legacy [meeting.summary] backend can differ from
    # where questions go. `legacy_status` is a voxtype from before the field.
    if a.get("summary_backend"):
        out["summary_backend"], out["summary_backend_name"] = a["summary_backend"], a["summary_backend_name"]
    elif not a.get("legacy_status"):
        out["summary_backend"], out["summary_backend_name"] = backend, name
    return out


def assistant(state, args):
    a = assistant_config(state)
    if args == ["status", "--json"]:
        print(json.dumps(status(state), indent=2))
        return 0
    if args[:1] == ["allow"] and len(args) == 2:
        for c in a["candidates"]:
            if c["id"] == args[1]:
                c["consent"] = True
        save(state)
        return 0
    return 2


def config_set(state, key, value):
    a = assistant_config(state)
    if key == "meeting.assistant.local_only":
        a["local_only"] = value == "true"
    elif key == "meeting.assistant.agent":
        a["agent"] = value
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


def asks_path(meeting):
    return os.path.join(os.environ["FAKE_MEETING_DATA"], "asks-%s.jsonl" % meeting)


def load_asks(meeting):
    try:
        with open(asks_path(meeting)) as f:
            return [json.loads(line) for line in f if line.strip()]
    except FileNotFoundError:
        return []


def emit(ev):
    sys.stdout.write(json.dumps(ev) + "\n")
    sys.stdout.flush()


def ask(state, args):
    """meeting ask M "Q" [--last D] [--notify] [--json]: the NDJSON stream."""
    rest = [a for a in args if a not in ("--notify", "--json")]
    last = None
    if "--last" in rest:
        i = rest.index("--last")
        last = rest[i + 1]
        rest = rest[:i] + rest[i + 2:]
    if len(rest) != 2:
        return 2
    meeting = resolve(state, rest[0])
    st = status(state)
    if st["backend"] == "none":
        if st["blocked"] == "consent_required":
            message = "Meeting transcripts can't go to %s until you allow it." % (st["agent_name"] or "an agent")
        else:
            message = "No agent or local model is available (%s)." % st["blocked"]
        emit({"event": "error", "code": st["blocked"], "message": message})
        return 1
    if state.get("fail", {}).get("ask"):
        emit({"event": "error", "code": "agent_failed", "message": state["fail"]["ask"]})
        return 1
    secs = {"5m": 300, "2m": 120}.get(last)
    start = {"event": "start", "meeting_id": meeting, "agent": st["agent"] or "local:llama3.2",
             "agent_name": st["backend_name"], "question": rest[1]}
    if secs:
        start["last_secs"] = secs
    emit(start)
    text = state.get("answer", DEFAULT_ANSWER)
    words = text.split(" ")
    for i in range(0, len(words), 4):
        emit({"event": "delta", "text": " ".join(words[i:i + 4]) + (" " if i + 4 < len(words) else "")})
        time.sleep(0.15)
    cites = []
    for ts in re.findall(r"\[(\d{2}:\d{2})\]", text):
        m, sec = ts.split(":")
        cites.append({"ts": ts, "ms": (int(m) * 60 + int(sec)) * 1000})
    record = {"id": "ask-%d" % (len(load_asks(meeting)) + 1), "time": "2026-11-11T16:00:00Z", "question": rest[1],
              "answer": text, "agent": start["agent"], "agent_name": st["backend_name"], "citations": cites}
    if secs:
        record["last_secs"] = secs
    with open(asks_path(meeting), "a") as f:
        f.write(json.dumps(record) + "\n")
    emit(dict(record, event="answer", meeting_id=meeting))
    return 0


def summarize(state, args):
    """meeting summarize M [--regenerate] [--format json]: prints the saved one unless --regenerate."""
    meeting = resolve(state, args[0])
    path = os.path.join(os.environ["FAKE_MEETING_DATA"], "summary-%s.json" % meeting)
    folder = past_dir(meeting)
    saved = None
    if os.path.exists(path):
        with open(path) as f:
            saved = json.load(f)
    elif os.path.exists(os.path.join(folder, "metadata.json")):
        with open(os.path.join(folder, "metadata.json")) as f:
            saved = json.load(f).get("summary")
    agent = args[args.index("--agent") + 1] if "--agent" in args else None
    if saved is None or "--regenerate" in args or agent:
        st = status(state)
        # `summary_fail`: the configured summary backend fails (a local model
        # answering 500); `--agent` goes around it.
        if state.get("summary_fail") and not agent:
            print("Summarizing...", file=sys.stderr)
            time.sleep(0.4)
            print("Error: " + state["summary_fail"], file=sys.stderr)
            return 1
        if st["backend"] == "none" and not agent:
            print("Error: Meeting transcripts can't go to %s until you allow it (consent_required)"
                  % (st["agent_name"] or "an agent"), file=sys.stderr)
            return 1
        print("Summarizing...", file=sys.stderr)
        time.sleep(0.6)
        saved = state.get("summary_json", DEFAULT_SUMMARY)
        with open(path, "w") as f:
            json.dump(saved, f)
    print(json.dumps(saved, indent=2))
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


def edit_text(state, args):
    want_json = "--json" in args
    rest = [a for a in args if a != "--json"]
    if len(rest) != 3:
        return 2
    meeting, _, seg = find_segment(state, rest[0], rest[1])
    if seg is None:
        return 1
    if rest[2] == "--reset":
        if "original_text" in seg:
            seg["text"] = seg.pop("original_text")
    elif rest[2].strip() == "":
        print("error: the new text is empty; use `voxtype meeting delete-line` to remove a line", file=sys.stderr)
        return 1
    else:
        seg.setdefault("original_text", seg["text"])
        seg["text"] = rest[2]
        if seg["text"] == seg["original_text"]:
            seg.pop("original_text")
    if not put_segment(state, meeting, seg):
        print("error: the fake socket did not apply the change", file=sys.stderr)
        return 1
    print(json.dumps(seg, indent=2) if want_json else "Edited segment %s" % rest[1])
    return 0


def delete_line(state, args):
    want_json = "--json" in args
    rest = [a for a in args if a != "--json"]
    if len(rest) != 2:
        return 2
    meeting, _, seg = find_segment(state, rest[0], rest[1])
    if seg is None:
        return 1
    path = os.path.join(past_dir(meeting), "transcript.json")
    if os.path.exists(path):
        with open(path) as f:
            t = json.load(f)
        t["segments"] = [s for s in t["segments"] if s["id"] != seg["id"]]
        with open(path, "w") as f:
            json.dump(t, f)
    else:
        nonce = "delete-%f" % time.time()
        control({"cmd": "delete", "meeting_id": meeting, "segment_id": seg["id"],
                 "shape": state.get("delete_shape", "event"), "nonce": nonce})
        if not wait_applied(nonce):
            print("error: the fake socket did not apply the change", file=sys.stderr)
            return 1
    print(json.dumps(dict(seg, deleted=True), indent=2) if want_json else "Deleted segment %s" % rest[1])
    return 0


def export(state, args):
    meeting = resolve(state, args[0])
    if meeting is None:
        print("error: Meeting not found: " + args[0], file=sys.stderr)
        return 1
    fmt = args[args.index("--format") + 1]
    if fmt not in ("text", "txt", "markdown", "md", "json", "srt", "vtt"):
        print("Error: Unknown export format '%s'. Valid formats: text, markdown, json, srt, vtt" % fmt, file=sys.stderr)
        return 1
    out = args[args.index("--output") + 1]
    flags = [a for a in args if a in ("--timestamps", "--speakers", "--metadata")]
    with open(out, "w") as f:
        f.write("FAKE EXPORT %s %s %s\n" % (meeting, fmt, " ".join(flags)))
        for s in segments_of(state, meeting) or []:
            f.write(s["text"] + "\n")
    print("Exported to %s" % out, file=sys.stderr)
    return 0


def play_path():
    return os.path.join(os.environ["FAKE_MEETING_DATA"], "play.json")


def play_state(state):
    try:
        with open(play_path()) as f:
            p = json.load(f)
    except FileNotFoundError:
        return {"state": "stopped", "meeting_id": None, "track": None, "from_ms": None, "pid": None,
                "started_at": None, "position_ms": None}
    if p["state"] == "playing":
        speed = state.get("play_speed", 1)
        p["position_ms"] = int(p["from_ms"] + (time.time() - p["_t0"]) * 1000 * speed)
    else:
        p["position_ms"] = None
    return p


def play(state, args):
    if "--status" in args:
        p = play_state(state)
        p.pop("_t0", None)
        print(json.dumps(p))
        return 0
    if "--stop" in args:
        p = play_state(state)
        if p.get("meeting_id"):
            p["state"] = "stopped"
            p["position_ms"] = None
            with open(play_path(), "w") as f:
                json.dump(p, f)
            control({"cmd": "playback", "meeting_id": p["meeting_id"], "track": p["track"], "state": "stopped",
                      "from_ms": p["from_ms"]})
        p.pop("_t0", None)
        print(json.dumps(p))
        return 0
    if state.get("play_error"):
        print(state["play_error"], file=sys.stderr)
        return 1
    meeting = resolve(state, args[0])
    seg_id = args[args.index("--segment") + 1]
    track = args[args.index("--track") + 1] if "--track" in args else "mixed"
    seg = next((s for s in segments_of(state, meeting) or [] if str(s["id"]) == seg_id), None)
    if seg is None:
        print("Error: no segment %s in meeting %s" % (seg_id, meeting), file=sys.stderr)
        return 1
    p = {"state": "playing", "meeting_id": meeting, "track": track, "from_ms": seg["start_ms"], "pid": 4242,
         "started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "_t0": time.time()}
    with open(play_path(), "w") as f:
        json.dump(p, f)
    control({"cmd": "playback", "meeting_id": meeting, "track": track, "state": "playing", "from_ms": seg["start_ms"]})
    print(json.dumps({"meeting_id": meeting, "track": track, "from_ms": seg["start_ms"], "pid": 4242}))
    return 0


def list_meetings(state, args):
    """meeting list --json [--limit N]: the socket's last meeting, then the past ones."""
    limit = int(args[args.index("--limit") + 1]) if "--limit" in args else 10
    out = []
    stored = snapshot().get("stored")
    if stored and stored.get("meeting_id"):
        out.append({"id": stored["meeting_id"], "title": stored.get("title"), "started_at": stored.get("started_at"),
                    "duration_secs": 70, "status": "completed", "chunk_count": 3, "audio_retained": False,
                    "has_summary": False,
                    "speaker_count": len({segment_key(s) for s in stored.get("segments", [])})})
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
        saved = os.path.join(os.environ["FAKE_MEETING_DATA"], "summary-%s.json" % meeting)
        if os.path.exists(saved):
            with open(saved) as f:
                meta["summary"] = json.load(f)
        print(json.dumps({"metadata": meta, "transcript": transcript, "asks": load_asks(meeting)}, indent=2))
        return 0
    snap = snapshot()
    target = next(m for m in (snap.get("live"), snap.get("stored")) if m and m.get("meeting_id") == meeting)
    meta = {"id": meeting, "title": target.get("title"), "started_at": target.get("started_at"),
            "status": "completed", "chunk_count": 3, "audio_retained": state.get("audio_retained", True),
            "storage_path": os.path.join(os.environ["FAKE_MEETING_DATA"], "live-" + meeting)}
    if snap.get("summary"):
        meta["summary"] = snap["summary"]
    saved = os.path.join(os.environ["FAKE_MEETING_DATA"], "summary-%s.json" % meeting)
    if os.path.exists(saved):
        with open(saved) as f:
            meta["summary"] = json.load(f)
    print(json.dumps({"metadata": meta, "transcript": {"segments": target["segments"], "total_chunks": 3},
                      "asks": load_asks(meeting)}, indent=2))
    return 0


def search(state, args):
    """meeting search QUERY [--limit N] --json: every word in one line, as the
    real index does, over the past meetings' lines and titles; best (earliest
    meeting first here) results in the real shape. state["search_delay"]
    slows it, to see the view's Searching... state."""
    rest = []
    i = 0
    limit = 20
    while i < len(args):
        if args[i] == "--limit":
            limit = int(args[i + 1])
            i += 2
            continue
        if args[i] != "--json":
            rest.append(args[i])
        i += 1
    words = " ".join(rest).lower().replace('"', " ").replace("*", " ").split()
    if not words:
        print("Error: search needs at least one word", file=sys.stderr)
        return 1
    time.sleep(state.get("search_delay", 0))

    def hit(text):
        low = text.lower()
        if not all(w in low for w in words):
            return None
        marks = []
        for w in words:
            at = low.find(w)
            while at != -1:
                marks.append([at, at + len(w)])
                at = low.find(w, at + len(w))
        marks.sort()
        snippet = ""
        pos = 0
        for a, b in marks:
            snippet += text[pos:a] + "\u00ab" + text[a:b] + "\u00bb"
            pos = b
        return text, marks, snippet + text[pos:]

    results = []
    for m in sorted(state.get("meetings", []), key=lambda m: m["started_at"], reverse=True):
        base = {"meeting_id": m["id"], "title": m.get("title") or m["started_at"], "date": m["started_at"]}
        h = hit(m.get("title") or "")
        if h:
            results.append(dict(base, kind="title", segment_id=None, start_ms=None, speaker=None,
                                snippet=h[2], snippet_plain=h[0], highlights=h[1], rank=-2.0))
        for seg in segments_of(state, m["id"]) or []:
            h = hit(seg["text"])
            if h:
                results.append(dict(base, kind="segment", segment_id=seg["id"], start_ms=seg["start_ms"],
                                    speaker=seg.get("speaker_label") or seg.get("speaker_id"),
                                    snippet=h[2], snippet_plain=h[0], highlights=h[1], rank=-1.0))
    print(json.dumps({"query": " ".join(rest), "speaker": None, "since": None, "results": results[:limit]}))
    return 0


def soundcheck(state, args):
    """meeting soundcheck [--seconds N] [--json] | --stop

    Prints the real NDJSON: soundcheck running, levels about ten a second,
    soundcheck stopped. Time runs at state["sc_time_scale"] (default 0.1, so
    30 s take 3). state: sc_echo, sc_no_loopback, sc_error (stopped with
    reason error, exit 1), sc_meeting_after (seconds of fake time before a
    meeting takes the mic: reason meeting, exit 1). --stop writes a flag the
    running check sees and ends with reason stopped. Opens no audio."""
    flag = os.path.join(os.environ["FAKE_MEETING_DATA"], "soundcheck.stop")
    if "--stop" in args:
        open(flag, "w").close()
        return 0
    seconds = int(args[args.index("--seconds") + 1]) if "--seconds" in args else 30
    if os.path.exists(flag):
        os.remove(flag)
    if state.get("sc_error"):
        print("Opening the meeting audio...", file=sys.stderr)
        emit({"event": "soundcheck", "state": "stopped", "reason": "error", "message": state["sc_error"]})
        return 1
    loopback = not state.get("sc_no_loopback")
    emit({"event": "soundcheck", "state": "running", "seconds": seconds,
          "echo_cancel": state.get("sc_echo", "webrtc"),
          "mic": "voxtype_meeting_aec_source", "loopback": loopback})
    scale = state.get("sc_time_scale", 0.1)
    meeting_after = state.get("sc_meeting_after")
    t = 0
    while True:
        time.sleep(0.1)
        t += 100
        fake_s = t / 1000.0 / scale
        if os.path.exists(flag):
            os.remove(flag)
            emit({"event": "soundcheck", "state": "stopped", "reason": "stopped"})
            return 0
        if meeting_after is not None and fake_s >= meeting_after:
            emit({"event": "soundcheck", "state": "stopped", "reason": "meeting"})
            return 1
        if fake_s >= seconds:
            emit({"event": "soundcheck", "state": "stopped", "reason": "done"})
            return 0
        phase = (t // 100) % 10
        mic = -45.0 + phase * 3.0
        emit({"event": "levels", "meeting_id": None, "soundcheck": True, "t_ms": t,
              "mic": mic, "mic_peak": mic + 4.0,
              "loopback": (-20.0 if loopback else None), "loopback_peak": (-12.0 if loopback else None)})


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

    if args == ["config", "get", "meeting.view_opacity"]:
        print(state.get("view_opacity", 0.8))
        return 0

    if args[:3] == ["status", "--format", "json"]:
        if state.get("daemon_running"):
            print(json.dumps({"text": "", "alt": "idle", "class": "idle"}))
            return 0
        print("voxtype daemon is not running", file=sys.stderr)
        return 1

    if state.get("old") and args[:1] == ["meeting"] and len(args) > 1 and args[1] in (
            "assistant", "ask", "summarize", "notes", "annotate", "reassign", "edit", "delete-line",
            "soundcheck", "search"):
        print("error: unrecognized subcommand '%s'" % args[1], file=sys.stderr)
        return 2

    if args[:2] == ["config", "set"] and len(args) == 4:
        return config_set(state, args[2], args[3])

    if args[:1] != ["meeting"] or len(args) < 2:
        return 2
    action = args[1]
    failing = state.get("fail", {})
    # A failing ask answers in its own NDJSON form.
    if action in failing and action != "ask":
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
        return summarize(state, args[2:])
    elif action == "show" and len(args) == 4 and args[3] == "--json":
        return show(state, args[2])
    elif action == "list" and "--json" in args:
        return list_meetings(state, args)
    elif action == "notes":
        return notes(state, args[2:])
    elif action == "annotate":
        return annotate(state, args[2:])
    elif action == "edit":
        return edit_text(state, args[2:])
    elif action == "delete-line":
        return delete_line(state, args[2:])
    elif action == "export":
        return export(state, args[2:])
    elif action == "play":
        return play(state, args[2:])
    elif action == "search":
        return search(state, args[2:])
    elif action == "soundcheck":
        return soundcheck(state, args[2:])
    elif action == "label" and len(args) == 5:
        control({"cmd": "label", "key": args[3], "name": args[4]})
        print("Labeled %s as '%s' in meeting %s" % (args[3], args[4], args[2]))
    else:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
