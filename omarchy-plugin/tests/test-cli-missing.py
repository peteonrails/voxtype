#!/usr/bin/env python3
"""Settings panel when its voxtype cannot run, offscreen against a fake CLI.

Scenarios:
  missing   dev.json names a binary that does not exist: the error names the
            binary and the dev.json, nothing stale is shown, and Install /
            Update launches Omarchy's installer (falling back to the old name).
  nopath    no dev.json and no voxtype on PATH.
  vanish    the binary disappears between two summons (the 1.2.0 report: a
            stale dev.json kept an old release and model on screen); putting it
            back recovers on the next summon.
  infofail  `info engines/devices/styles` exit non-zero: an error strip names
            each, their lists are dropped, the rest of the panel keeps working.

Run inside a desktop-isolated sandbox (no WAYLAND_DISPLAY/DISPLAY, private
XDG_RUNTIME_DIR); it refuses to run with a display in the environment.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import sys

import panel_harness as h

SCENARIOS = ("missing", "nopath", "vanish", "infofail")

INFO_FAIL = {"engines": "engines broke", "devices": "no audio backend", "styles": "styles broke"}


def check_installer(command):
    """The launched string must prefer the new installer and fall back to the old."""
    problems = []
    with tempfile.TemporaryDirectory(prefix="voxtype-installer-test-") as directory:
        root = Path(directory)
        for name, which in (("omarchy-install-dictation-voxtype", "new"), ("omarchy-voxtype-install", "old")):
            bindir = root / which
            bindir.mkdir()
            script = bindir / name
            script.write_text("#!/bin/sh\necho " + which + "\n")
            script.chmod(0o755)
        for path, expected in ((str(root / "new") + ":" + str(root / "old"), "new"),
                               (str(root / "old"), "old")):
            result = subprocess.run(["/bin/bash", "-c", command], env={"PATH": path},
                                    text=True, capture_output=True, timeout=10)
            if result.stdout.strip() != expected or result.returncode != 0:
                problems.append("installer with PATH=" + path + " ran " + repr(result.stdout.strip()))
    return problems


def run(scenario, shell, evidence):
    state = h.base_state(omarchy="linked")
    with h.Run(shell, state) as r:
        env = {}
        missing = r.root / "gone" / "voxtype"
        fail_state = r.root / "state-fail.json"
        good_state = r.root / "state-good.json"
        fail_state.write_text(json.dumps(dict(state, info_fail=INFO_FAIL)))
        good_state.write_text(json.dumps(state))
        if scenario == "missing":
            r.dev_json(missing)
        elif scenario == "nopath":
            empty = r.root / "empty-path"
            empty.mkdir()
            env["PATH"] = str(empty)
        else:
            r.dev_json(r.fake)
        output, code = r.run(h.HERE / "cli-missing.qml", {
            "__SCENARIO__": scenario, "__MISSING__": str(missing), "__FAKE__": str(r.fake),
            "__DEVJSON__": str(r.plugin / "dev.json"), "__STATE__": str(r.state),
            "__FAIL_STATE__": str(fail_state), "__GOOD_STATE__": str(good_state),
        }, env=env)
        calls = r.calls()
        launched = r.launched()
        if evidence:
            out = evidence / scenario
            out.mkdir(parents=True, exist_ok=True)
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(r.log.read_text())
            (out / "terminal.jsonl").write_text(r.terminal_log.read_text())

    problems = h.driver_problems(output, code, "CLI_MISSING", scenario)
    if scenario in ("missing", "nopath") and calls:
        problems.append("nothing should have reached a voxtype: " + json.dumps(calls))
    if scenario == "missing":
        if len(launched) != 1 or len(launched[0]) != 1:
            problems.append("expected one installer launch: " + json.dumps(launched))
        else:
            problems += check_installer(launched[0][0])
    elif launched:
        problems.append("unexpected terminal launches " + json.dumps(launched))
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
