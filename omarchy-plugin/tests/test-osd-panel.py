#!/usr/bin/env python3
"""OSD offer, style picker, stale restart banner and unset Threads, offscreen.

Runs the real panel against fake-voxtype-migrate.py, with fake-systemctl.py
first on PATH as `systemctl`, so the unit's main pid and the panel's restart
flow are both under the test's control. No config, daemon or network is
touched.

Scenarios:
  osdoffer          configured gtk4, Quickshell available: the offer shows on the
                    OSD and Engine sections (not Audio); Switch runs
                    `setup osd --use-auto`, then the panel's restart flow, and the
                    offer goes.
  osdusefail        `--use-auto` fails: the error shows in the offer, no restart.
  osdnotnow         Not now runs `setup osd --dismiss-offer`, re-reads, hides.
  osdnotnowfail     Not now whose recording fails still hides, for the session.
  osdnoqs           configured gtk4, Quickshell not available: no offer.
  osddismissed      dismissed earlier: no offer.
  osdold            a CLI without `setup osd --status`: no offer, and a style
                    pick writes only osd.style, as before.
  stylequickshell   style pick on gtk4 with Quickshell available writes the style
                    and osd.frontend = auto, and the row says so.
  stylenoqs         style pick without Quickshell writes nothing and says why;
                    the picker goes back to the configured style.
  styleactive       style pick with Quickshell already in use writes only the style.
  stalebanner       "Changes take effect after restart" survives a poll with the
                    same daemon pid, clears when the pid changes on the poll, and
                    clears on reopen after a restart made while closed.
  threads           parakeet.threads unset shows "default", not 1, and writes
                    nothing until a number is entered; reset brings it back.

Run inside a desktop-isolated sandbox (no WAYLAND_DISPLAY/DISPLAY, private
XDG_RUNTIME_DIR); it refuses to run with a display in the environment.
"""
import argparse
import json
import os
from pathlib import Path
import sys

import panel_harness as h

UNIT = {"state": "active", "pid": 1000}
GTK4_QS = {"configured": "gtk4", "quickshell_available": True, "offer_dismissed": False}
GTK4_NO_QS = {"configured": "gtk4", "quickshell_available": False, "offer_dismissed": False}
AUTO_QS = {"configured": "auto", "quickshell_available": True, "offer_dismissed": False}

SCENARIOS = {
    "osdoffer": {"osd": GTK4_QS},
    "osdusefail": {"osd": GTK4_QS, "osd_use_auto_fail": True},
    "osdnotnow": {"osd": GTK4_QS},
    "osdnotnowfail": {"osd": GTK4_QS, "osd_dismiss_fail": True},
    "osdnoqs": {"osd": GTK4_NO_QS},
    "osddismissed": {"osd": dict(GTK4_QS, offer_dismissed=True)},
    "osdold": {"osd": None},
    "stylequickshell": {"osd": GTK4_QS},
    "stylenoqs": {"osd": GTK4_NO_QS},
    "styleactive": {"osd": AUTO_QS},
    "stalebanner": {"osd": AUTO_QS},
    "threads": {"osd": AUTO_QS, "threads_key": True},
}

STATUS = ["setup", "osd", "--status", "--json"]
USE_AUTO = ["setup", "osd", "--use-auto"]
DISMISS = ["setup", "osd", "--dismiss-offer"]
VALIDATE = ["config", "validate-selection", "--require-idle"]
RESTART = ["systemctl", "--user", "restart", "voxtype"]

# Every write and OSD action each scenario may make, in order.
EXPECTED_ACTIONS = {
    "osdoffer": [USE_AUTO, VALIDATE, RESTART],
    "osdusefail": [USE_AUTO],
    "osdnotnow": [DISMISS],
    "osdnotnowfail": [DISMISS],
    "osdold": [["config", "set", "osd.style", "pill"]],
    "stylequickshell": [["config", "set", "osd.style", "pill"], ["config", "set", "osd.frontend", "auto"]],
    "styleactive": [["config", "set", "osd.style", "pill"]],
    # The two restarts here are the test's own, made outside the panel.
    "stalebanner": [["config", "set", "osd.style", "pill"], RESTART,
                    ["config", "set", "osd.style", "hud"], RESTART],
    "threads": [["config", "set", "parakeet.threads", "8"], ["config", "unset", "parakeet.threads"]],
}


def actions(calls):
    out = []
    for c in calls:
        if c[:2] in (["config", "set"], ["config", "unset"], ["config", "select-model"]) or c == VALIDATE:
            out.append(c)
        elif c[:2] == ["setup", "osd"] and c != STATUS:
            out.append(c)
        elif c[:1] == ["systemctl"] and c[1:3] != ["--user", "show"]:
            out.append(c)
        elif c[:1] == ["setup"] and c[:2] not in (["setup", "osd"], ["setup", "migrate"], ["setup", "omarchy"]):
            out.append(c)
        elif c[:2] == ["setup", "migrate"] and c != ["setup", "migrate", "--json"]:
            out.append(c)
    return out


def run(scenario, shell, evidence):
    state = h.base_state(osd_keys=True, unit=dict(UNIT))
    state.update(SCENARIOS[scenario])
    if state["osd"] is not None:
        state["osd"] = dict(state["osd"])
    with h.Run(shell, state) as r:
        bindir = r.root / "bin"
        bindir.mkdir()
        systemctl = bindir / "systemctl"
        systemctl.write_text("#!/bin/sh\nexec " + sys.executable + " -I " + str(h.HERE / "fake-systemctl.py") + ' "$@"\n')
        systemctl.chmod(0o755)
        r.dev_json(r.fake)
        output, code = r.run(h.HERE / "osd-panel.qml", {"__SCENARIO__": scenario},
                             env={"PATH": str(bindir) + os.pathsep + os.environ.get("PATH", "")}, timeout=90)
        calls = r.calls()
        launched = r.launched()
        final = json.loads(r.state.read_text())
        if evidence:
            out = evidence / scenario
            out.mkdir(parents=True, exist_ok=True)
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(r.log.read_text())
            (out / "final-state.json").write_text(json.dumps(final, indent=2))

    problems = h.driver_problems(output, code, "OSD_PANEL", scenario)
    got = actions(calls)
    want = EXPECTED_ACTIONS.get(scenario, [])
    if got != want:
        problems.append("actions " + json.dumps(got) + ", expected " + json.dumps(want))
    if STATUS not in calls:
        problems.append("setup osd --status --json was never asked")
    if scenario in ("osdnotnow", "osdnotnowfail", "osdoffer") and calls.count(STATUS) < 2:
        problems.append("OSD status not re-read after the action")
    if scenario == "osdoffer" and (final["osd"]["configured"] != "auto" or final["unit"].get("restarts") != 1):
        problems.append("final state " + json.dumps(final))
    if scenario == "osdnotnow" and final["osd"].get("offer_dismissed") is not True:
        problems.append("dismissal not recorded: " + json.dumps(final["osd"]))
    if scenario == "osdnotnowfail" and "could not record Not now for the OSD offer" not in output:
        problems.append("failed dismissal was not logged")
    if launched:
        problems.append("unexpected terminal launches " + json.dumps(launched))
    return output, problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--shell", type=Path, default=Path(os.environ.get("OMARCHY_PATH", "/usr/share/omarchy")) / "shell")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--scenario", choices=sorted(SCENARIOS), action="append")
    args = parser.parse_args()
    h.guard_environment(parser, args.shell)
    failed = False
    for scenario in args.scenario or list(SCENARIOS):
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
