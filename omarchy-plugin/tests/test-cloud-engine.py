#!/usr/bin/env python3
"""An engine with no model to download (Deepgram) can be selected, offscreen.

`voxtype info engines --json` reports Deepgram compiled and `info models
--json` has no entry for it, so the card has no model row to Use. It offers
"Use Deepgram" instead, which runs `config set engine deepgram`
(`config select-model` refuses an engine without models).

Scenarios:
  use             engine = "parakeet"; browse Deepgram, Use. Expect exactly one
                  `config set engine deepgram`, then Deepgram configured, the
                  card headed "DEEPGRAM" with the no-model line.
  current         engine = "deepgram": marked configured, no Use, no writes.
  with-models     control: Parakeet and Omnilingual keep their model rows,
                  model Use and "MODELS FOR" header, with no engine Use and no
                  no-model line.
  engines-unread  `info engines` fails: Use is still offered (as the dropdown
                  offers every engine then) and works.
  switch-fails    `config set engine` fails: one attempt, "Could not switch to
                  this engine", Parakeet still configured, Use still offered.
  model-fails     control: a failed `config select-model` on Omnilingual keeps
                  "Could not use this model".

Run inside a desktop-isolated sandbox (no WAYLAND_DISPLAY/DISPLAY, private
XDG_RUNTIME_DIR); it refuses to run with a display in the environment.
"""
import argparse
import json
import os
from pathlib import Path
import sys

import panel_harness as h

SCENARIOS = ("use", "current", "with-models", "engines-unread", "switch-fails", "model-fails")
V3 = "parakeet-tdt-0.6b-v3"
V3_GGUF = "parakeet-tdt-0.6b-v3-Q8_0.gguf"


def run(scenario, shell, evidence):
    state = h.base_state(model=V3, installed=[V3, V3_GGUF], cloud_engines=["deepgram"],
                         extra_engines={"omnilingual": ["omnilingual-300m"]},
                         extra_models={"omnilingual": "omnilingual-1b"})
    expected = []
    if scenario in ("use", "engines-unread", "switch-fails"):
        expected = [["config", "set", "engine", "deepgram"]]
    if scenario == "current":
        state["engine"] = "deepgram"
    elif scenario == "engines-unread":
        state["info_fail"] = {"engines": "engines broke"}
    elif scenario == "switch-fails":
        state["set_engine_fail"] = "Error: could not write the config file"
    elif scenario == "model-fails":
        state["select_model_fail"] = "Error: could not write the config file"
        expected = [["config", "select-model", "omnilingual", "omnilingual-300m"]]
    start = state.get("engine", "parakeet")
    with h.Run(shell, state) as r:
        r.dev_json(r.fake)
        output, code = r.run(h.HERE / "cloud-engine.qml", {"__SCENARIO__": scenario, "__START__": start})
        calls = r.calls()
        if evidence:
            out = evidence / scenario
            out.mkdir(parents=True, exist_ok=True)
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(r.log.read_text())
    problems = h.driver_problems(output, code, "CLOUD_ENGINE", scenario)
    writes = [c for c in calls if c[:1] == ["config"] and c[:2] != ["config", "schema"]]
    if writes != expected:
        problems.append("writes " + json.dumps(writes) + ", expected " + json.dumps(expected))
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
