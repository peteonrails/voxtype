#!/usr/bin/env python3
"""Fake `voxtype` for the model-upgrade banner test. Never touches a real config.

Speaks only the subcommands the panel calls, against a JSON state file named by
FAKE_VOXTYPE_STATE, and appends every argv to FAKE_VOXTYPE_LOG. `setup migrate`
follows the contract the real CLI implements:

  --json                          the status object
  --accept <id> --yes             NDJSON progress, then switch (exit 0)
  --decline                       mark offers declined
  --cleanup                       forget the cleanup offer

`setup omarchy --status --json` answers from state["omarchy"]: a link state
such as "copy" or "linked" (contract 2.2 of the 1.2.1 plan), or absent for a
build without the subcommand, which exits 2 like an unknown subcommand does.

`copy_dismissed` is reported only when state["copy_dismissed"] is not None,
as an older CLI omits it. `setup omarchy --dismiss-copy` sets it, or with
state["dismiss_fail"] exits 1 and changes nothing.

state["gguf_model"] adds the generic GGUF engine with that model installed, and
state["engine"] names the configured engine ("parakeet" when absent).
state["installed"] lists the Parakeet models on disk (default: the configured
one). `config select-model` and `config set parakeet.model` update the state.

state["info_fail"] maps an `info` reader ("engines", "devices", "styles") to
the stderr it should fail with (exit 1).

`setup osd --status --json` answers from state["osd"] (contract C3 of the
1.2.2 plan): {"configured", "quickshell_available", "offer_dismissed"}, with
"active" and "offer" derived the way the real CLI derives them. Absent, it
exits 2 like a CLI that predates --status. `--use-auto` and `--dismiss-offer`
(C4) change it; state["osd_use_auto_fail"] / state["osd_dismiss_fail"] make
them exit 1 instead.

state["osd_keys"] adds the OSD section's Frontend and Style rows (and a
second style) to the schema; state["threads_key"] adds parakeet.threads, unset
unless state["values"] holds it. `config set` / `config unset` write
state["values"] for exactly those keys and reject anything else (exit 2);
`osd.frontend` writes go to state["osd"]["configured"] as well.
`config validate-selection` exits 0, so the panel's restart flow runs through
to `systemctl --user restart voxtype` (see fake-systemctl.py).

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


FRONTENDS = ["auto", "gtk4", "native", "quickshell"]
SETTABLE = {"osd.style", "osd.frontend", "parakeet.threads"}


def osd_status(state):
    osd = state["osd"]
    configured = osd["configured"]
    available = osd["quickshell_available"]
    if configured == "quickshell" or (configured == "auto" and available):
        active = "quickshell" if available else "none"
    elif configured == "auto":
        active = "gtk4"
    else:
        active = configured
    dismissed = osd.get("offer_dismissed", False)
    return {"configured": configured, "active": active, "reason": "fake: " + active,
            "quickshell_available": available,
            "offer": configured in ("gtk4", "native") and available and not dismissed,
            "offer_dismissed": dismissed}


def value_of(state, key, default=None):
    return state.get("values", {}).get(key, default)


def schema(state):
    model_type = {"key": "parakeet.model_type", "type": "enum", "choices": ["tdt", "ctc"], "section": "Engine",
                  "label": "Decoder type", "description": "", "value": None, "file_value": None,
                  "engine": "parakeet", "compiled": True, "restart_required": True}
    # Older CLIs never emitted the flag; state["legacy_schema"] reproduces them.
    if not state.get("legacy_schema"):
        model_type["onnx_model_only"] = True
    engine = state.get("engine", "parakeet")
    choices = ["whisper", "parakeet"] + (["gguf"] if "gguf_model" in state else []) \
        + (["cohere"] if "cohere_models" in state else [])
    keys = [
        {"key": "engine", "type": "enum", "choices": choices, "section": "Engine",
         "label": "Transcription engine", "description": "", "value": engine,
         "file_value": engine, "engine": None, "compiled": True, "restart_required": True},
        {"key": "parakeet.model", "type": "dynamic_enum", "source": "models", "section": "Engine",
         "label": "Model", "description": "", "value": state["model"], "file_value": state["model"],
         "engine": "parakeet", "compiled": True, "restart_required": True},
        model_type,
    ]
    if "cohere_models" in state:
        cohere = state.get("cohere_model", state["cohere_models"][0])
        keys.append({"key": "cohere.model", "type": "dynamic_enum", "source": "models", "section": "Engine",
                     "label": "Model", "description": "", "value": cohere, "file_value": cohere,
                     "engine": "cohere", "compiled": True, "restart_required": True})
    if "gguf_model" in state:
        keys.append({"key": "gguf.model", "type": "dynamic_enum", "source": "models", "section": "Engine",
                     "label": "Model", "description": "", "value": state["gguf_model"],
                     "file_value": state["gguf_model"], "engine": "gguf", "compiled": True,
                     "restart_required": True})
    if state.get("threads_key"):
        # Like the real schema: an unset Option<u32> is null in both fields.
        threads = value_of(state, "parakeet.threads")
        keys.append({"key": "parakeet.threads", "type": "int", "min": 1, "max": 256, "section": "Engine",
                     "label": "Threads", "description": "ONNX Runtime intra-op threads. Unset uses 4.",
                     "value": threads, "file_value": threads, "engine": "parakeet", "compiled": True,
                     "restart_required": True})
    keys.append({"key": "audio.device", "type": "string", "section": "Audio", "label": "Device",
                 "description": "", "value": "default", "file_value": None, "engine": None,
                 "compiled": True, "restart_required": True})
    if state.get("osd_keys"):
        frontend = state["osd"]["configured"] if state.get("osd") else value_of(state, "osd.frontend", "gtk4")
        style = value_of(state, "osd.style")
        keys += [
            {"key": "osd.frontend", "type": "enum", "choices": FRONTENDS, "section": "OSD", "label": "Frontend",
             "description": "Which OSD renderer the daemon launches.", "value": frontend,
             "file_value": frontend, "engine": None, "compiled": True, "restart_required": True},
            {"key": "osd.style", "type": "dynamic_enum", "source": "styles", "section": "OSD", "label": "Style",
             "description": "Quickshell style name.", "value": style or "hud",
             "file_value": style, "engine": None, "compiled": True, "restart_required": True},
        ]
    return {
        "schema_version": 1, "voxtype_version": "1.2.0-rc4", "config_path": "",
        "engine": engine, "replacements": {}, "keys": keys,
    }


def models(state):
    names = ["parakeet-tdt-0.6b-v3", "parakeet-tdt-0.6b-v3-Q8_0.gguf", "parakeet-ultra-Q8_0.gguf",
             "parakeet-tdt-0.6b-v2"]
    installed = state.get("installed", [state["model"]])
    rows = [{"name": n, "installed": n in installed, "languages": []} for n in names]
    # Legacy ONNX rows: listed only while configured (one-way door).
    rows[0]["legacy"] = True
    rows[3]["legacy"] = True
    # Optional license fields: one row with a link, one with a name but no
    # URL (no link), one like an older build (neither field).
    rows[0].update(license="CC-BY-4.0", license_url="https://creativecommons.org/licenses/by/4.0/")
    rows[1].update(license="CC-BY-4.0", license_url="")
    engines = {"parakeet": {"models": rows}}
    if "gguf_model" in state:
        engines["gguf"] = {"models": [{"name": state["gguf_model"], "installed": True, "languages": ["auto"]}]}
    if "cohere_models" in state:
        engines["cohere"] = {"models": [{"name": n, "installed": True, "languages": ["en"]}
                                        for n in state["cohere_models"]]}
    return {"engines": engines}


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


def omarchy_status(state):
    link = state["omarchy"]
    packaged = "/usr/share/voxtype/omarchy-plugin/io.voxtype.settings"
    return {
        "omarchy": True,
        "packaged_dir": packaged,
        "packaged_version": "1.2.1",
        "state": link,
        "path": "/home/u/.config/omarchy/plugins/io.voxtype.settings",
        "installed_version": "1.2.0" if link == "copy" else "1.2.1",
    } | ({} if state.get("copy_dismissed") is None else {"copy_dismissed": state["copy_dismissed"]})


def main():
    args = sys.argv[1:]
    with open(LOG, "a") as f:
        f.write(json.dumps(args) + "\n")
    state = load()
    failing = state.get("info_fail", {})
    if args[:1] == ["info"] and len(args) > 1 and args[1] in failing:
        print(failing[args[1]], file=sys.stderr)
        return 1
    if args[:3] == ["config", "schema", "--json"]:
        print(json.dumps(schema(state)))
    elif args[:3] == ["info", "models", "--json"]:
        print(json.dumps(models(state)))
    elif args[:3] == ["info", "engines", "--json"]:
        engines = [{"name": "whisper", "compiled": True}, {"name": "parakeet", "compiled": True}]
        if "gguf_model" in state:
            engines.append({"name": "gguf", "compiled": True})
        if "cohere_models" in state:
            engines.append({"name": "cohere", "compiled": True})
        print(json.dumps(engines))
    elif args[:3] == ["info", "devices", "--json"]:
        print(json.dumps([{"name": "Built-in Microphone", "default": True}]))
    elif args[:3] == ["info", "styles", "--json"]:
        styles = [{"name": "hud", "description": "Heads-up display"}]
        if state.get("osd_keys"):
            styles.append({"name": "pill", "description": "Compact pill"})
        print(json.dumps(styles))
    elif args[:2] == ["config", "validate-selection"]:
        return 0
    elif args[:2] in (["config", "set"], ["config", "unset"]) and len(args) >= 3 and args[2] in SETTABLE:
        key = args[2]
        values = state.setdefault("values", {})
        if args[1] == "unset":
            values.pop(key, None)
        elif key == "osd.frontend" and args[3] not in FRONTENDS:
            print("error: invalid osd.frontend", file=sys.stderr)
            return 2
        elif key == "parakeet.threads":
            values[key] = int(args[3])
        else:
            values[key] = args[3]
        if key == "osd.frontend" and state.get("osd"):
            state["osd"]["configured"] = values.get(key, "auto")
        save(state)
    elif args[:2] == ["setup", "osd"]:
        if state.get("osd") is None:
            print("error: unexpected argument found", file=sys.stderr)
            return 2
        if args[2:] == ["--status", "--json"]:
            print(json.dumps(osd_status(state)))
        elif args[2:] == ["--use-auto"]:
            if state.get("osd_use_auto_fail"):
                print("error: could not write osd.frontend", file=sys.stderr)
                return 1
            state["osd"]["configured"] = "auto"
            state["osd"]["offer_dismissed"] = False
            save(state)
        elif args[2:] == ["--dismiss-offer"]:
            if state.get("osd_dismiss_fail"):
                print("error: could not write the dismissal", file=sys.stderr)
                return 1
            state["osd"]["offer_dismissed"] = True
            save(state)
        else:
            return 2
    elif args[:3] == ["status", "--format", "json"]:
        print(json.dumps({"class": "idle"}))
    elif args[:2] == ["config", "select-model"] and len(args) >= 4:
        engine, model = args[2], args[3]
        if engine not in ("gguf", "parakeet", "cohere") or ("--language" in args and engine != "gguf"):
            print("error: select-model rejected " + " ".join(args[2:]), file=sys.stderr)
            return 2
        state["engine"] = engine
        state[{"gguf": "gguf_model", "cohere": "cohere_model"}.get(engine, "model")] = model
        save(state)
    elif args[:3] == ["config", "set", "parakeet.model"] and len(args) == 4:
        state["model"] = args[3]
        save(state)
    elif args[:2] == ["setup", "omarchy"]:
        if state.get("omarchy") is None:
            print("error: unrecognized subcommand 'omarchy'", file=sys.stderr)
            return 2
        if args[2:] == ["--status", "--json"]:
            print(json.dumps(omarchy_status(state)))
        elif args[2:] == ["--dismiss-copy"]:
            if state.get("dismiss_fail"):
                print("error: could not write the dismissal", file=sys.stderr)
                return 1
            state["copy_dismissed"] = True
            save(state)
        else:
            return 2
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
        # info accel and anything else: "not in this build".
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
