#!/usr/bin/env python3
"""Settings can switch back to Parakeet after a GGUF model was picked, offscreen.

After Use on a generic GGUF row, the Parakeet catalog showed Parakeet Ultra as
"installed" with no Use button, so the panel had no way back (1.2.1 demo bug).

Scenarios:
  from-gguf    engine = "gguf"; browse Parakeet, Use on Ultra. Expect one
               `config select-model parakeet <ultra>` and Ultra configured.
  same-engine  engine = "parakeet" on another model; Use on Ultra still goes
               through `config set parakeet.model`, as before.
  parakeet-to-cohere  engine = "parakeet"; browse Cohere, Use on a Cohere
               model. Expect one `config select-model cohere <model>`.

Run inside a desktop-isolated sandbox (no WAYLAND_DISPLAY/DISPLAY, private
XDG_RUNTIME_DIR); it refuses to run with a display in the environment.
"""
import argparse
import json
import os
from pathlib import Path
import sys

import panel_harness as h

SCENARIOS = ("from-gguf", "same-engine", "parakeet-to-cohere")
ULTRA = "parakeet-ultra-Q8_0.gguf"
V3 = "parakeet-tdt-0.6b-v3-Q8_0.gguf"
COHERE = "cohere-transcribe-03-2026-Q4_K_M.gguf"


def run(scenario, shell, evidence):
    if scenario == "from-gguf":
        state = h.base_state(model=ULTRA, engine="gguf", gguf_model="Qwen3-ASR-0.6B-Q8_0.gguf",
                             installed=[ULTRA])
        start, expected = "gguf", [["config", "select-model", "parakeet", ULTRA]]
        target, model = "parakeet", ULTRA
    elif scenario == "parakeet-to-cohere":
        state = h.base_state(model=ULTRA, installed=[ULTRA], cohere_models=[COHERE])
        start, expected = "parakeet", [["config", "select-model", "cohere", COHERE]]
        target, model = "cohere", COHERE
    else:
        state = h.base_state(model=V3, installed=[V3, ULTRA])
        start, expected = "parakeet", [["config", "set", "parakeet.model", ULTRA]]
        target, model = "parakeet", ULTRA
    with h.Run(shell, state) as r:
        r.dev_json(r.fake)
        output, code = r.run(h.HERE / "engine-switch-back.qml",
                             {"__SCENARIO__": scenario, "__START__": start,
                              "__TARGET__": target, "__MODEL__": model})
        calls = r.calls()
        if evidence:
            out = evidence / scenario
            out.mkdir(parents=True, exist_ok=True)
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(r.log.read_text())
    problems = h.driver_problems(output, code, "ENGINE_SWITCH_BACK", scenario)
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
