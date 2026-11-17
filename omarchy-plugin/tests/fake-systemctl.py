#!/usr/bin/env python3
"""Fake `systemctl` for the Settings panel tests, put first on PATH.

Answers only `systemctl --user show voxtype -p ActiveState -p MainPID --value`
and `systemctl --user restart voxtype`, from state["unit"] in the file named by
FAKE_VOXTYPE_STATE: {"state": "active", "pid": 1000}. A restart moves the
pid on, as a real one gives the daemon a new process. Every argv is appended
to FAKE_VOXTYPE_LOG prefixed with "systemctl". Anything else exits 1.
"""
import json
import os
import sys

STATE = os.environ["FAKE_VOXTYPE_STATE"]
LOG = os.environ["FAKE_VOXTYPE_LOG"]


def main():
    args = sys.argv[1:]
    with open(LOG, "a") as f:
        f.write(json.dumps(["systemctl"] + args) + "\n")
    with open(STATE) as f:
        state = json.load(f)
    unit = state.get("unit")
    if unit is None:
        return 1
    if args[:3] == ["--user", "show", "voxtype"] and "--value" in args:
        print(unit["state"])
        print(unit["pid"])
        return 0
    if args == ["--user", "restart", "voxtype"]:
        unit["pid"] = int(unit["pid"]) + 1
        unit["state"] = "active"
        unit["restarts"] = unit.get("restarts", 0) + 1
        tmp = STATE + ".tmp"
        with open(tmp, "w") as f:
            json.dump(state, f)
        os.replace(tmp, STATE)
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
