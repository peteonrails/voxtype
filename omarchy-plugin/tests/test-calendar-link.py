#!/usr/bin/env python3
"""The calendar link row in the Settings panel, offscreen.

The row (CalendarLinkRow) sits on the Meeting section right above the
meeting.calendar.* keys. Its link is a secret: the panel must hand it to
`voxtype meeting calendar set-ics -` on stdin, never in argv (which `ps`
shows), and must never show it again once saved. Every scenario checks the
fake CLI's argv log for the link; the set and fail scenarios also check that
it arrived on stdin.

Scenarios:
  set      no link yet: the help folds out, a link the CLI refuses stays in
           the field with the CLI's reason, a good one (typed, Enter) is
           saved, the field empties and the status line shows the host, the
           fetch and the next event; Clear removes it
  fail     the CLI refuses the link (a 404): its line is shown, the field
           keeps the link, and the status says the bar widget names meetings
  saved    a link already set whose last fetch failed: the status and the
           error line, then Clear
  keys     the keyboard cursor stops on the row; Enter puts it in the field,
           Escape gives the keys back without closing the panel
  search   searching for "ical" finds the row
  old-cli  a CLI without `meeting calendar` shows no row
  real     against a real voxtype (REAL_VOXTYPE, run only when set): the
           `meeting calendar` commands go to the real binary, which fetches
           a feed from a local HTTP server, so the store, the status and the
           stdin hand-off are all real

Run inside a desktop-isolated sandbox (no WAYLAND_DISPLAY/DISPLAY, private
XDG_RUNTIME_DIR); it refuses to run with a display in the environment.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import sys
import threading
import time

import panel_harness as h

SCENARIOS = ("set", "fail", "saved", "keys", "search", "old-cli", "real")
SECRET = "https://calendar.google.com/calendar/ical/pete%40example.com/private-0123456789abcdef/basic.ics"
SET_ARGV = ["meeting", "calendar", "set-ics", "-", "--json"]

STATES = {
    "set": {"calendar": {"link": None, "source_without_link": None}},
    "fail": {"calendar": {"link": None, "source_without_link": "plugin"},
             "calendar_set_fail": "The calendar server (calendar.google.com) answered 404: the link may have "
                                  "been reset. Copy the secret iCal address again and run voxtype meeting "
                                  "calendar set-ics."},
    "saved": {"calendar": {"link": {"host": "outlook.office365.com", "events": 0, "next": None,
                                    "fetched_at": 1762960000,
                                    "last_error": "The calendar server (outlook.office365.com) did not answer."},
                           "source_without_link": None}},
    "keys": {"calendar": {"link": None, "source_without_link": None}},
    "search": {"calendar": {"link": None, "source_without_link": None}},
    "old-cli": {"calendar_keys": True},
}


def ics_feed():
    """One event an hour from now, so the real CLI has a next event."""
    start = time.gmtime(time.time() + 3600)
    end = time.gmtime(time.time() + 5400)
    fmt = "%Y%m%dT%H%M%SZ"
    return ("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nBEGIN:VEVENT\r\nUID:one@test\r\n"
            "DTSTAMP:%s\r\nDTSTART:%s\r\nDTEND:%s\r\nSUMMARY:Design review\r\nEND:VEVENT\r\n"
            "END:VCALENDAR\r\n" % (time.strftime(fmt, start), time.strftime(fmt, start), time.strftime(fmt, end)))


class FeedServer:
    """A calendar feed on a free loopback port, recording each request path."""
    def __init__(self):
        paths = self.paths = []
        body = ics_feed().encode()

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                paths.append(self.path)
                self.send_response(200)
                self.send_header("Content-Type", "text/calendar")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        self.httpd = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        self.base = "http://127.0.0.1:%d" % self.httpd.server_port
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def close(self):
        self.httpd.shutdown()


def run(scenario, shell, evidence):
    state = h.base_state(**STATES.get(scenario, {"calendar_keys": True}))
    secret = SECRET
    feed = None
    with h.Run(shell, state) as r:
        env = {}
        if scenario == "real":
            # `meeting calendar` goes to the real binary; everything else
            # stays with the fake (the schema gets its calendar keys).
            real = os.environ["REAL_VOXTYPE"]
            feed = FeedServer()
            secret = feed.base + "/calendar/ical/pete/private-0123456789abcdef/basic.ics"
            shim = r.root / "voxtype-shim"
            shim.write_text("#!/bin/sh\n"
                            "if [ \"$1\" = meeting ] && [ \"$2\" = calendar ]; then\n"
                            "  printf '%s\\n' \"$*\" >> " + str(r.root / "real-argv.txt") + "\n"
                            "  exec " + real + " \"$@\"\nfi\n"
                            "exec " + str(r.fake) + " \"$@\"\n")
            shim.chmod(0o755)
            (r.root / "xdg-config" / "voxtype").mkdir(parents=True)
            (r.root / "xdg-config" / "voxtype" / "config.toml").write_text("")
            env = {"XDG_STATE_HOME": str(r.root / "xdg-state"), "XDG_CONFIG_HOME": str(r.root / "xdg-config"),
                   "XDG_DATA_HOME": str(r.root / "xdg-data")}
            r.dev_json(shim)
        else:
            r.dev_json(r.fake)
        shots = ""
        if evidence:
            shots = str((evidence / scenario).resolve())
            Path(shots).mkdir(parents=True, exist_ok=True)
        try:
            output, code = r.run(h.HERE / "calendar-link.qml", {
                "__SCENARIO__": scenario, "__SECRET__": secret, "__EVIDENCE__": shots,
                "__HOST__": secret.split("://", 1)[1].split("/", 1)[0],
            }, env=env, timeout=90)
        finally:
            if feed:
                feed.close()
        calls = r.calls()
        final = json.loads(r.state.read_text())
        real_argv = (r.root / "real-argv.txt").read_text().splitlines() \
            if (r.root / "real-argv.txt").exists() else []
        stored = None
        if scenario == "real":
            settings = r.root / "xdg-state" / "voxtype" / "calendar.json"
            stored = settings.exists()
        if evidence:
            out = evidence / scenario
            out.mkdir(parents=True, exist_ok=True)
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(r.log.read_text())

    problems = h.driver_problems(output, code, "CALENDAR_LINK", scenario)
    # The link never reaches argv, the fake's or the real binary's, or the log.
    if any(secret in " ".join(c) for c in calls) or any(secret in line for line in real_argv):
        problems.append("the link was passed in argv")
    if secret in output:
        problems.append("the link is in the shell's log")
    sets = [c for c in calls if c[:3] == ["meeting", "calendar", "set-ics"]] \
        + [line.split() for line in real_argv if line.startswith("meeting calendar set-ics")]
    if any(c != SET_ARGV for c in sets):
        problems.append("set-ics called as %s" % sets)
    if scenario == "set":
        if final.get("calendar_stdin") != ["calendar.example.com/feed\n", secret + "\n"]:
            problems.append("stdin: %r" % final.get("calendar_stdin"))
        if [c[:3] for c in calls if c[:2] == ["meeting", "calendar"] and c[2] != "status"] \
                != [["meeting", "calendar", "set-ics"]] * 2 + [["meeting", "calendar", "clear"]]:
            problems.append("calendar commands: %s" % [c for c in calls if c[:2] == ["meeting", "calendar"]])
    if scenario == "fail" and final.get("calendar_stdin") != [secret + "\n"]:
        problems.append("stdin: %r" % final.get("calendar_stdin"))
    if scenario in ("keys", "search", "old-cli") and sets:
        problems.append("set-ics ran: %s" % sets)
    if scenario == "real":
        if len(sets) != 2:
            problems.append("set-ics runs: %s" % sets)
        if feed and not any(p.endswith("/private-0123456789abcdef/basic.ics") for p in feed.paths):
            problems.append("the real CLI never fetched the feed: %s" % feed.paths)
        if stored is not False:
            # Cleared at the end of the run, so the settings file is gone.
            problems.append("the link is still stored after Clear: %s" % stored)
    return output, problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--shell", type=Path, default=Path(os.environ.get("OMARCHY_PATH", "/usr/share/omarchy")) / "shell")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--scenario", choices=SCENARIOS, action="append")
    args = parser.parse_args()
    h.guard_environment(parser, args.shell)
    failed = False
    for scenario in args.scenario or SCENARIOS:
        if scenario == "real" and not os.environ.get("REAL_VOXTYPE"):
            continue
        output, problems = run(scenario, args.shell, args.evidence)
        if problems:
            failed = True
            print(output)
            print("FAIL " + scenario + ": " + "; ".join(problems))
        else:
            print("PASS " + scenario)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
