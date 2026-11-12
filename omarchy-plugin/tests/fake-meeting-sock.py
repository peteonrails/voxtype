#!/usr/bin/env python3
"""Fake voxtype meeting.sock for the meeting view tests. Never touches a daemon.

    fake-meeting-sock.py SOCKET CONTROL LOG SNAPSHOT

Binds SOCKET and speaks the meeting.sock contract (docs/INTEGRATIONS.md, "Live
transcript and status"): every new connection gets a `hello` snapshot of the
current state, then whatever is broadcast. It keeps that state the way the
daemon's MeetingHudState does (src/meeting/events.rs): segments in start_ms
order, speaker renames applied to the backlog, and everything reset on idle.

It is driven by JSON lines appended to CONTROL, by the QML driver and by
fake-voxtype-meeting.py (which stands in for `voxtype meeting …`):

  {"cmd": "state", ...}            set hello fields (status, meeting_id, title,
                                   started_at, chunk_count, segments); no event
  {"cmd": "segments", "chunk_id": N, "segments": [...]}
  {"cmd": "rename", "from": A, "to": B}         a speaker_renamed event
  {"cmd": "status", "status": S}   as update_meeting_state: idle resets
  {"cmd": "start", "title": T}     a new meeting, then status recording
  {"cmd": "stop"}                  status finalizing, then idle 1 s later
  {"cmd": "error", "message": M}
  {"cmd": "label", "key": K, "name": N}  later segments with K carry N
  {"cmd": "update", "segment": S, "key": "event"|"type", "meeting_id": M}
                                   replace segment S (by id). In the live meeting
                                   it is broadcast as segment_updated, tagged
                                   with `key` (default `event`, the daemon's);
                                   in the stored one (after the meeting ended)
                                   nothing is sent, as the daemon only announces
                                   live changes
  {"cmd": "notes_updated", "meeting_id": M, "notes": N?}
  {"cmd": "autosummary", "summary": {...}, "error": {"code", "message"}?}
                                   the next stop goes finalizing, idle, then,
                                   as the daemon does after a meeting,
                                   {"event":"status","status":"summarizing",
                                   meeting_id} and summary_ready (or, with
                                   `error`, an error event naming the meeting,
                                   or naming none with "anonymous": true)
  {"cmd": "delete", "segment_id": N, "shape": "event"|"flag", "meeting_id": M?}
                                   remove a line; announce it as segment_deleted
                                   or as segment_updated with "deleted": true
  {"cmd": "playback", "meeting_id", "track", "state", "from_ms"}
                                   a playback event, as the daemon sends one
  {"cmd": "drop"}                  close every connection, keep listening
  {"cmd": "down"}                  close everything and remove the socket
  {"cmd": "up"}                    bind again
  {"cmd": "quit"}

Each handled command is appended to LOG, so a driver can wait for it. After
each one, SNAPSHOT holds {"live": ..., "stored": ...}: the running meeting and
the last one that ended, each {meeting_id, segments}, which is what
fake-voxtype-meeting.py edits for `meeting reassign`.
"""
import json
import os
import selectors
import socket
import sys
import time
import uuid
from datetime import datetime, timezone


def now_iso():
    # chrono's format, nanoseconds and all: the view must cope with it.
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f") + "123Z"


class Server:
    def __init__(self, path, control, log, snapshot):
        self.path = path
        self.control = control
        self.log = log
        self.snapshot = snapshot
        self.stored = None
        self.summary = None
        self.autosummary = None
        self.pending = []
        self.offset = 0
        self.sel = selectors.DefaultSelector()
        self.listener = None
        self.clients = []
        self.labels = {}
        self.pending_idle = None
        self.idle()
        self.up()

    def idle(self):
        if getattr(self, "state", None) and self.state.get("meeting_id"):
            self.stored = {"meeting_id": self.state["meeting_id"], "segments": self.state["segments"],
                           "title": self.state.get("title"), "started_at": self.state.get("started_at")}
        self.state = {"status": "idle", "meeting_id": None, "title": None, "started_at": None,
                      "chunk_count": 0, "diarization": {"enabled": False}, "segments": []}
        self.labels = {}

    def up(self):
        if self.listener:
            return
        try:
            os.unlink(self.path)
        except FileNotFoundError:
            pass
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(self.path)
        self.listener.listen(8)
        self.listener.setblocking(False)
        self.sel.register(self.listener, selectors.EVENT_READ)

    def drop(self):
        for c in self.clients:
            try:
                c.close()
            except OSError:
                pass
        self.clients = []

    def down(self):
        self.drop()
        if self.listener:
            self.sel.unregister(self.listener)
            self.listener.close()
            self.listener = None
        try:
            os.unlink(self.path)
        except FileNotFoundError:
            pass

    def hello(self):
        ev = {"event": "hello", "status": self.state["status"], "chunk_count": self.state["chunk_count"],
              "diarization": self.state["diarization"], "segments": self.state["segments"]}
        for key in ("meeting_id", "title", "started_at"):
            if self.state.get(key):
                ev[key] = self.state[key]
        return ev

    def send(self, conn, ev):
        try:
            conn.sendall((json.dumps(ev) + "\n").encode())
            return True
        except OSError:
            return False

    def broadcast(self, ev):
        self.clients = [c for c in self.clients if self.send(c, ev)]

    def set_status(self, status):
        if status == "idle":
            self.idle()
        else:
            self.state["status"] = status
        self.broadcast({"event": "status", "status": status})

    def labelled(self, seg):
        seg = dict(seg)
        key = seg.get("speaker_id") or {"microphone": "You", "loopback": "Remote"}.get(seg.get("source"), "Unknown")
        if key in self.labels:
            seg["speaker_label"] = self.labels[key]
        return seg

    def handle(self, cmd):
        kind = cmd.get("cmd")
        if kind == "state":
            for key, value in cmd.items():
                if key != "cmd":
                    self.state[key] = value
            self.state["segments"] = sorted(self.state.get("segments", []), key=lambda s: s["start_ms"])
        elif kind == "segments":
            segs = [self.labelled(s) for s in cmd["segments"]]
            for s in segs:
                at = len([x for x in self.state["segments"] if x["start_ms"] <= s["start_ms"]])
                self.state["segments"].insert(at, s)
            self.state["chunk_count"] = cmd["chunk_id"] + 1
            self.broadcast({"event": "segments", "chunk_id": cmd["chunk_id"], "segments": segs})
        elif kind == "rename":
            for s in self.state["segments"]:
                if s.get("speaker_id") == cmd["from"]:
                    s["speaker_id"] = cmd["to"]
            if cmd["from"] in self.labels and cmd["to"] not in self.labels:
                self.labels[cmd["to"]] = self.labels.pop(cmd["from"])
            self.broadcast({"event": "speaker_renamed", "from": cmd["from"], "to": cmd["to"]})
        elif kind == "status":
            self.set_status(cmd["status"])
        elif kind == "start":
            self.idle()
            self.state.update({"status": "recording", "meeting_id": str(uuid.uuid4()),
                               "title": cmd.get("title") or None, "started_at": now_iso(),
                               "diarization": {"enabled": True, "backend": "hybrid"}})
            self.broadcast({"event": "status", "status": "recording"})
        elif kind == "stop":
            self.set_status("finalizing")
            self.pending_idle = time.monotonic() + 1.0
            if self.autosummary is not None:
                auto, mid = self.autosummary, self.state.get("meeting_id")
                self.autosummary = None
                if auto.get("error"):
                    done = dict(auto["error"], event="error")
                    # `anonymous`: an older daemon whose error names no meeting.
                    if not auto.get("anonymous"):
                        done["meeting_id"] = mid
                else:
                    done = {"event": "summary_ready", "meeting_id": mid, "agent": "claude", "agent_name": "Claude",
                            "path": "/tmp/meetings/%s/summary.md" % mid, "summary": auto["summary"]}
                now = time.monotonic()
                self.pending += [(now + 1.4, {"event": "status", "status": "summarizing", "meeting_id": mid}, None),
                                 (now + 2.8, done, auto.get("summary"))]
        elif kind == "autosummary":
            self.autosummary = cmd
        elif kind == "notes_updated":
            ev = {"event": "notes_updated", "meeting_id": cmd["meeting_id"]}
            if "notes" in cmd:
                ev["notes"] = cmd["notes"]
            self.broadcast(ev)
        elif kind == "error":
            self.broadcast({"event": "error", "message": cmd["message"]})
        elif kind == "label":
            # As the daemon does since `meeting label` pokes it: later lines
            # carry the name, and lines already sent get it too, each
            # published as segment_updated and kept in the hello backlog.
            self.labels[cmd["key"]] = cmd["name"]
            for i, seg in enumerate(self.state["segments"]):
                named = self.labelled(seg)
                if named.get("speaker_label") != seg.get("speaker_label"):
                    self.state["segments"][i] = named
                    self.broadcast({"event": "segment_updated", "meeting_id": self.state.get("meeting_id"),
                                    "segment": named})
        elif kind == "update":
            seg = cmd["segment"]
            target = None
            if not cmd.get("meeting_id") or cmd.get("meeting_id") == self.state.get("meeting_id"):
                target = self.state
            elif self.stored and cmd.get("meeting_id") == self.stored["meeting_id"]:
                target = self.stored
            if target is not None:
                target["segments"] = [seg if s["id"] == seg["id"] else s for s in target["segments"]]
                if target is self.state:
                    self.broadcast({cmd.get("key", "event"): "segment_updated", "segment": seg})
        elif kind == "delete":
            mid = cmd.get("meeting_id") or self.state.get("meeting_id")
            if mid == self.state.get("meeting_id"):
                gone = [s for s in self.state["segments"] if s["id"] == cmd["segment_id"]]
                self.state["segments"] = [s for s in self.state["segments"] if s["id"] != cmd["segment_id"]]
                if cmd.get("shape") == "flag" and gone:
                    self.broadcast({"event": "segment_updated", "segment": dict(gone[0], deleted=True)})
                else:
                    self.broadcast({"event": "segment_deleted", "meeting_id": mid, "segment_id": cmd["segment_id"]})
            elif self.stored and mid == self.stored["meeting_id"]:
                self.stored["segments"] = [s for s in self.stored["segments"] if s["id"] != cmd["segment_id"]]
        elif kind == "playback":
            self.broadcast({"event": "playback", "meeting_id": cmd["meeting_id"], "track": cmd["track"],
                            "state": cmd["state"], "from_ms": cmd["from_ms"]})
        elif kind == "drop":
            self.drop()
        elif kind == "down":
            self.down()
        elif kind == "up":
            self.up()
        elif kind == "quit":
            return False
        with open(self.log, "a") as f:
            f.write(json.dumps(cmd) + "\n")
        self.write_snapshot()
        return True

    def write_snapshot(self):
        live = {"meeting_id": self.state.get("meeting_id"), "segments": self.state["segments"],
                "title": self.state.get("title"), "started_at": self.state.get("started_at")}
        tmp = self.snapshot + ".tmp"
        with open(tmp, "w") as f:
            json.dump({"live": live, "stored": self.stored, "summary": self.summary}, f)
        os.replace(tmp, self.snapshot)

    def poll_control(self):
        try:
            with open(self.control) as f:
                f.seek(self.offset)
                data = f.read()
        except FileNotFoundError:
            return True
        # Only whole lines; a writer may be mid-append.
        end = data.rfind("\n")
        if end < 0:
            return True
        self.offset += len(data[:end + 1].encode())
        for line in data[:end + 1].splitlines():
            if line.strip() and not self.handle(json.loads(line)):
                return False
        return True

    def run(self):
        while True:
            for key, _ in self.sel.select(timeout=0.03):
                if self.listener and key.fileobj is self.listener:
                    conn, _ = self.listener.accept()
                    if self.send(conn, self.hello()):
                        self.clients.append(conn)
            due = [p for p in self.pending if p[0] <= time.monotonic()]
            for p in due:
                self.pending.remove(p)
                ev, summary = p[1], p[2]
                # These name a meeting that is already over; the state stays.
                if summary is not None:
                    self.summary = summary
                self.broadcast(ev)
                self.write_snapshot()
            if self.pending_idle and time.monotonic() >= self.pending_idle:
                self.pending_idle = None
                self.set_status("idle")
                self.write_snapshot()
            if not self.poll_control():
                self.down()
                return


if __name__ == "__main__":
    Server(sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]).run()
