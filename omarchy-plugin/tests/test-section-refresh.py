#!/usr/bin/env python3
"""`section` IPC on an open Settings panel re-reads everything, offscreen.

The fake CLI logs every invocation. The driver opens the panel, changes the
fake's state (new configured model, a new upgrade offer, a hand-installed
plugin copy), then calls the IpcHandler's section() directly, as
`omarchy-shell io.voxtype.settings section Audio` would.

Scenarios:
  open    every reader open() uses (schema, models, engines, devices, styles,
          status, accel, upgrade offers, plugin link, OSD status) is invoked again and the
          new answers are shown; an unknown section also re-reads.
  closed  on a closed panel the verb goes through the shell and reads nothing.

Run inside a desktop-isolated sandbox (no WAYLAND_DISPLAY/DISPLAY, private
XDG_RUNTIME_DIR); it refuses to run with a display in the environment.
"""
import argparse
import json
import os
from pathlib import Path
import sys

import panel_harness as h

SCENARIOS = ("open", "closed")

READERS = {
    "schema": ["config", "schema", "--json"],
    "models": ["info", "models", "--json"],
    "engines": ["info", "engines", "--json"],
    "devices": ["info", "devices", "--json"],
    "styles": ["info", "styles", "--json"],
    "status": ["status", "--format", "json"],
    "accel": ["info", "accel", "--json"],
    "upgrade": ["setup", "migrate", "--json"],
    "link": ["setup", "omarchy", "--status", "--json"],
    "osd": ["setup", "osd", "--status", "--json"],
}

ULTRA_OFFER = {"id": "parakeet-ultra", "kind": "model", "engine": "parakeet", "model": "parakeet-ultra-Q8_0.gguf",
               "title": "Parakeet Ultra is now available", "detail": "", "download_bytes": 740363168,
               "recommended": True, "declined": False}


def run(scenario, shell, evidence):
    state = h.base_state(omarchy="linked")
    with h.Run(shell, state) as r:
        changed = r.root / "state-changed.json"
        changed.write_text(json.dumps(dict(state, model="parakeet-ultra-Q8_0.gguf", omarchy="copy",
                                           offers=[ULTRA_OFFER])))
        r.dev_json(r.fake)
        output, code = r.run(h.HERE / "section-refresh.qml", {
            "__SCENARIO__": scenario, "__STATE__": str(r.state), "__CHANGED_STATE__": str(changed),
        })
        calls = r.calls()
        if evidence:
            out = evidence / scenario
            out.mkdir(parents=True, exist_ok=True)
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(r.log.read_text())

    problems = h.driver_problems(output, code, "SECTION_REFRESH", scenario)
    counts = {name: calls.count(argv) for name, argv in READERS.items()}
    # open() reads each once. "open": section reads each again, and the
    # unknown-section call a third time. "closed": nothing after open().
    # Status and models also ride the 5 s poll, so they may run more often if
    # the machine is slow; the rest have no other trigger here.
    expected = 3 if scenario == "open" else 1
    for name, n in counts.items():
        polled = name in ("status", "models")
        if n < expected or (n > expected and not polled):
            problems.append("%s invoked %d times, expected %d" % (name, n, expected))
    writes = [c for c in calls if c[:1] == ["config"] and c[:2] != ["config", "schema"]]
    if writes:
        problems.append("unexpected writes " + json.dumps(writes))
    return output, problems, counts


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--shell", type=Path, default=Path(os.environ.get("OMARCHY_PATH", "/usr/share/omarchy")) / "shell")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--scenario", choices=SCENARIOS, action="append")
    args = parser.parse_args()
    h.guard_environment(parser, args.shell)
    failed = False
    for scenario in args.scenario or SCENARIOS:
        output, problems, counts = run(scenario, args.shell, args.evidence)
        if problems:
            failed = True
            print(output)
            print("FAIL " + scenario + ": " + "; ".join(problems))
        else:
            print("PASS " + scenario + " " + json.dumps(counts, sort_keys=True))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
