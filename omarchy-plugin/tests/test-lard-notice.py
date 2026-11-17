#!/usr/bin/env python3
"""The LARD switch says when it is on with nothing to run, offscreen.

`config schema --json` adds `notice` to `text.lard_disfluency` while the
switch is on and the stage cannot run (no model installed, or dictation that
is not known to be English). The form shows it under the row, so the panel never
presents the switch as on without saying it is doing nothing.

Scenarios:
  missing    Schema carries the notice: shown under the LARD row.
  ready      No notice (a working install, or a voxtype that predates the
             field): nothing shown.
  installed  Notice, then the model is installed behind an open panel: gone
             after the next refresh.

Run inside a desktop-isolated sandbox (no WAYLAND_DISPLAY/DISPLAY, private
XDG_RUNTIME_DIR); it refuses to run with a display in the environment.
"""
import argparse
import json
import os
from pathlib import Path
import sys

import panel_harness as h

SCENARIOS = ("missing", "ready", "installed")
NOTICE = ("LARD disfluency cleanup is on but its model is not installed "
          "(/data/voxtype/models/lard-disfluency-v1.vxdf). Run: voxtype setup lard, "
          "or turn it off: voxtype config set text.lard_disfluency false")


def run(scenario, shell, evidence):
    lard = {} if scenario == "ready" else {"notice": NOTICE}
    state = h.base_state(lard=lard)
    with h.Run(shell, state) as r:
        ready_state = r.root / "state-ready.json"
        ready_state.write_text(json.dumps(dict(state, lard={})))
        r.dev_json(r.fake)
        output, code = r.run(h.HERE / "lard-notice.qml", {
            "__SCENARIO__": scenario, "__NOTICE__": NOTICE,
            "__STATE__": str(r.state), "__READY_STATE__": str(ready_state),
        })
        calls = r.calls()
        if evidence:
            out = evidence / scenario
            out.mkdir(parents=True, exist_ok=True)
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(r.log.read_text())
    problems = h.driver_problems(output, code, "LARD_NOTICE", scenario)
    writes = [c for c in calls if c[:1] == ["config"] and c[:2] != ["config", "schema"]]
    if writes:
        problems.append("unexpected writes " + json.dumps(writes))
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
