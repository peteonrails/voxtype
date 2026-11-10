#!/usr/bin/env python3
"""Fake `voxtype` for the model-upgrade banner test. Never touches a real config.

Speaks only the subcommands the panel calls, against a JSON state file named by
FAKE_VOXTYPE_STATE, and appends every argv to FAKE_VOXTYPE_LOG. `setup migrate`
follows the contract the real CLI implements:

  --json                          the status object
  --accept <id> --yes             NDJSON progress, then switch (exit 0)
  --decline                       mark offers declined
  --cleanup                       forget the cleanup offer

Accept behavior is scripted per attempt by state["accept_script"]: "fail",
"slow" (one progress line, then wait to be cancelled) or "ok". After the script
runs out, attempts succeed.
"""
import json
import os
import sys
import time

STATE = os.environ["FAKE_VOXTYPE_STATE"]
LOG = os.environ["FAKE_VOXTYPE_LOG"]

ULTRA = {"id": "parakeet-ultra", "kind": "model", "engine": "parakeet",
         "model": "parakeet-ultra-Q8_0.gguf", "title": "Parakeet Ultra is now available",
         "detail": "Parakeet Ultra covers the same 25 European languages as Parakeet v3.",
         "download_bytes": 740363168, "recommended": True, "declined": False}
FORMAT = {"id": "format-gguf", "kind": "format", "engine": "parakeet",
          "model": "parakeet-tdt-0.6b-v3-Q8_0.gguf", "title": "Your model format has been updated",
          "detail": "Redownload parakeet-tdt-0.6b-v3 in the new format for better performance.",
          "download_bytes": 739508576, "recommended": False, "declined": False}


def load():
    with open(STATE) as f:
        return json.load(f)


def save(state):
    tmp = STATE + ".tmp"
    with open(tmp, "w") as f:
        json.dump(state, f)
    os.replace(tmp, STATE)


def status(state):
    return {
        "schema": 1,
        "current": {"engine": "parakeet", "model": state["model"], "format": "onnx"},
        "offers": state["offers"],
        "removal_notice": {
            "version": "2.0",
            "message": "Support for parakeet-tdt-0.6b-v3 (ONNX) will be removed in voxtype 2.0. "
                       "To keep using it, pin voxtype to this version.",
            "pin_command": "add IgnorePkg = voxtype-bin to /etc/pacman.conf",
        } if state["offers"] else None,
        "cleanup": state["cleanup"],
    }


def schema(state):
    return {
        "schema_version": 1, "voxtype_version": "1.2.0-rc4", "config_path": "",
        "engine": "parakeet", "replacements": {},
        "keys": [
            {"key": "engine", "type": "enum", "choices": ["whisper", "parakeet"], "section": "Engine",
             "label": "Transcription engine", "description": "", "value": "parakeet",
             "file_value": "parakeet", "engine": None, "compiled": True, "restart_required": True},
            {"key": "parakeet.model", "type": "dynamic_enum", "source": "models", "section": "Engine",
             "label": "Model", "description": "", "value": state["model"], "file_value": state["model"],
             "engine": "parakeet", "compiled": True, "restart_required": True},
            {"key": "audio.device", "type": "string", "section": "Audio", "label": "Device",
             "description": "", "value": "default", "file_value": None, "engine": None,
             "compiled": True, "restart_required": True},
        ],
    }


def models(state):
    names = ["parakeet-tdt-0.6b-v3", "parakeet-tdt-0.6b-v3-Q8_0.gguf", "parakeet-ultra-Q8_0.gguf",
             "parakeet-tdt-0.6b-v2"]
    rows = [{"name": n, "installed": n == state["model"], "languages": []} for n in names]
    # Legacy ONNX rows: listed only while configured (one-way door).
    rows[0]["legacy"] = True
    rows[3]["legacy"] = True
    # Optional license fields: one row with a link, one with a name but no
    # URL (no link), one like an older build (neither field).
    rows[0].update(license="CC-BY-4.0", license_url="https://creativecommons.org/licenses/by/4.0/")
    rows[1].update(license="CC-BY-4.0", license_url="")
    return {"engines": {"parakeet": {"models": rows}}}


def accept(state, args):
    offer_id = args[args.index("--accept") + 1]
    if "--yes" not in args:
        print("refusing without --yes", file=sys.stderr)
        return 2
    if state.get("reject_progress_flag") and "--progress-format" in args:
        print("error: unexpected argument '--progress-format' found", file=sys.stderr)
        return 2
    script = state.get("accept_script", [])
    step = script.pop(0) if script else "ok"
    save(state)
    def emit(obj):
        if "--progress-format" in args:
            print(json.dumps(obj), flush=True)
        else:
            print("Downloading... " + str(obj.get("pct", "")), flush=True)
    emit({"event": "progress", "file": "parakeet-ultra-Q8_0.gguf", "pct": 42})
    if step == "fail":
        emit({"event": "error", "message": "sha256 mismatch for " + offer_id + "; nothing was changed"})
        print("download failed", file=sys.stderr)
        return 1
    if step == "slow":
        time.sleep(60)
        return 0
    if step == "late":
        # Like the real CLI after verify: SIGTERM is ignored until the
        # switch and restart have finished.
        import signal
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        emit({"event": "progress", "file": "parakeet-ultra-Q8_0.gguf", "pct": 100})
        time.sleep(2)
    offer = next(o for o in state["offers"] if o["id"] == offer_id)
    emit({"event": "progress", "file": offer["model"], "pct": 100})
    emit({"event": "done"})
    state["cleanup"] = {"model": state["model"], "bytes": 2600000000}
    state["model"] = offer["model"]
    state["offers"] = []
    save(state)
    return 0


def main():
    args = sys.argv[1:]
    with open(LOG, "a") as f:
        f.write(json.dumps(args) + "\n")
    state = load()
    if args[:3] == ["config", "schema", "--json"]:
        print(json.dumps(schema(state)))
    elif args[:3] == ["info", "models", "--json"]:
        print(json.dumps(models(state)))
    elif args[:3] == ["info", "engines", "--json"]:
        print(json.dumps([{"name": "whisper", "compiled": True}, {"name": "parakeet", "compiled": True}]))
    elif args[:3] == ["status", "--format", "json"]:
        print(json.dumps({"class": "idle"}))
    elif args[:2] == ["setup", "migrate"]:
        if state.get("no_migrate"):
            print("error: unrecognized subcommand 'migrate'", file=sys.stderr)
            return 2
        if "--json" in args:
            print(json.dumps(status(state)))
        elif "--accept" in args:
            return accept(state, args)
        elif "--decline" in args:
            for offer in state["offers"]:
                offer["declined"] = True
            save(state)
            print("Keeping the current model.")
        elif "--cleanup" in args:
            state["cleanup"] = None
            save(state)
        else:
            return 2
    else:
        # info devices/styles/accel and anything else: "not in this build".
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
