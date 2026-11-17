#!/usr/bin/env python3
"""Parakeet's "Decoder type" is hidden for a GGUF model, offscreen.

`config schema --json` flags keys that only apply to an ONNX model with
`onnx_model_only: true` (today only `parakeet.model_type`). The form hides them
while the engine's configured model is a `.gguf` file.

Scenarios:
  onnx    ONNX model: Decoder type shown (section and search).
  gguf    GGUF model: hidden in both.
  legacy  GGUF model, schema from a voxtype without the flag: shown, as before.
  switch  ONNX, then the config moves to GGUF behind an open panel: hidden
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

SCENARIOS = ("onnx", "gguf", "legacy", "switch")
GGUF = "parakeet-ultra-Q8_0.gguf"


def run(scenario, shell, evidence):
    model = GGUF if scenario in ("gguf", "legacy") else "parakeet-tdt-0.6b-v3"
    state = h.base_state(model=model, legacy_schema=scenario == "legacy")
    with h.Run(shell, state) as r:
        gguf_state = r.root / "state-gguf.json"
        gguf_state.write_text(json.dumps(dict(state, model=GGUF)))
        r.dev_json(r.fake)
        output, code = r.run(h.HERE / "onnx-model-only.qml", {
            "__SCENARIO__": scenario, "__STATE__": str(r.state), "__GGUF_STATE__": str(gguf_state),
        })
        calls = r.calls()
        if evidence:
            out = evidence / scenario
            out.mkdir(parents=True, exist_ok=True)
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(r.log.read_text())
    problems = h.driver_problems(output, code, "ONNX_MODEL_ONLY", scenario)
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
