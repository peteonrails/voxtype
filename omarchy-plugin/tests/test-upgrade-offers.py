#!/usr/bin/env python3
"""Model upgrade banner regression, against a fake `voxtype setup migrate`.

Runs the real Panel/SchemaForm/UpgradeOfferBanner/VoxtypeCli under Quickshell
with QT_QPA_PLATFORM=offscreen. The panel's layer-shell PanelWindow needs a
Wayland compositor, so the test copy swaps it for a FloatingWindow of the same
contents; nothing else in the plugin is replaced. The CLI is
fake-voxtype-migrate.py, so no config, model, daemon or network is touched.

Covers: offers shown with size, Ultra first when recommended, Keep current
model shows the selectable removal notice and pin command, the banner persists
after a decline, accept failure then Retry, cancel mid-download, accept success
(switch, no false restart prompt), cleanup offer, banner hidden when there is
nothing to offer, the --progress-format fallback, and a CLI without `setup
migrate`. The link* scenarios cover the "replace your copy" banner against a
fake `setup omarchy --status --json`: shown for a hand-installed copy (and its
button launches the replace in a terminal), hidden when linked, and hidden for
a CLI without the subcommand. linkcopy's CLI predates `copy_dismissed`, so it
also covers an older CLI; linkdismissed hides the banner from the status
alone; linknotnow records the dismissal and the banner goes after the re-read;
linknotnowfail hides it for the session even though recording failed.

Run inside a desktop-isolated sandbox (no WAYLAND_DISPLAY/DISPLAY, private
XDG_RUNTIME_DIR); it refuses to run with a display in the environment.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
PLUGIN = HERE.parent

WINDOW_OLD = """  PanelWindow {
    id: window
    visible: root.opened
    anchors { top: true; bottom: true; left: true; right: true }
    color: "transparent"
    exclusionMode: ExclusionMode.Ignore
    WlrLayershell.namespace: "voxtype-settings"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive
"""
WINDOW_NEW = """  FloatingWindow {
    id: window
    visible: root.opened
    implicitWidth: 1280
    implicitHeight: 900
    color: "black"
"""

SCENARIOS = {
    "main": {"accept_script": ["fail", "slow", "ok"]},
    "noflag": {"reject_progress_flag": True},
    "latecancel": {"accept_script": ["late"]},
    "nomigrate": {"no_migrate": True},
    "linkcopy": {"omarchy": "copy"},
    "linklinked": {"omarchy": "linked"},
    "linkunknown": {},
    "linkdismissed": {"omarchy": "copy", "copy_dismissed": True},
    "linknotnow": {"omarchy": "copy", "copy_dismissed": False},
    "linknotnowfail": {"omarchy": "copy", "copy_dismissed": False, "dismiss_fail": True},
}


def patch(path, old, new):
    text = path.read_text()
    assert old in text, (path, old[:60])
    path.write_text(text.replace(old, new, 1))


def run(scenario, shell, evidence):
    with tempfile.TemporaryDirectory(prefix="voxtype-upgrade-test-") as directory:
        root = Path(directory)
        for child in shell.iterdir():
            if child.is_dir():
                (root / child.name).symlink_to(child.resolve(), target_is_directory=True)
        plugin = root / "plugin"
        shutil.copytree(PLUGIN, plugin, ignore=shutil.ignore_patterns(".git", "__pycache__", "tests", "dev.json"))
        patch(plugin / "Panel.qml", WINDOW_OLD, WINDOW_NEW)
        patch(plugin / "Panel.qml", "id: root", "id: root\n  readonly property alias formTest: form\n"
              "  readonly property alias cliTest: cli\n  readonly property alias renderTest: settingsCard.card")
        patch(plugin / "components/SchemaForm.qml", "EngineModelCard {", "EngineModelCard {\n          id: cardProbe")
        patch(plugin / "components/SchemaForm.qml", "id: root", "id: root\n  readonly property alias cardTest: cardProbe")

        state = root / "state.json"
        log = root / "commands.jsonl"
        initial = {"model": "parakeet-tdt-0.6b-v3", "cleanup": None, "accept_script": [], "omarchy": None,
                   # The CLI lists Ultra second on purpose: the banner must sort it first.
                   "offers": [
                       {"id": "format-gguf", "kind": "format", "engine": "parakeet",
                        "model": "parakeet-tdt-0.6b-v3-Q8_0.gguf",
                        "title": "Your model format has been updated",
                        "detail": "Redownload parakeet-tdt-0.6b-v3 in the new format for better performance.",
                        "download_bytes": 739508576, "recommended": False, "declined": False},
                       {"id": "parakeet-ultra", "kind": "model", "engine": "parakeet",
                        "model": "parakeet-ultra-Q8_0.gguf",
                        "title": "Parakeet Ultra is now available",
                        "detail": "Parakeet Ultra covers the same 25 European languages as Parakeet v3.",
                        "download_bytes": 740363168, "recommended": True, "declined": False},
                   ]}
        initial.update(SCENARIOS[scenario])
        state.write_text(json.dumps(initial))
        log.write_text("")
        fake = root / "voxtype-fake"
        fake.write_text("#!/bin/sh\nexec " + sys.executable + " -I " + str(HERE / "fake-voxtype-migrate.py") + ' "$@"\n')
        fake.chmod(0o755)
        (plugin / "dev.json").write_text(json.dumps({"voxtypeBin": str(fake)}))
        # Anything the panel would open a terminal for is logged instead.
        terminal_log = root / "terminal.jsonl"
        terminal_log.write_text("")
        terminal = root / "terminal-fake"
        terminal.write_text("#!/bin/sh\nexec " + sys.executable + " -I -c 'import json,sys; "
                            "open(sys.argv[1], \"a\").write(json.dumps(sys.argv[2:]) + \"\\n\")' "
                            + str(terminal_log) + ' "$@"\n')
        terminal.chmod(0o755)
        patch(plugin / "components/VoxtypeCli.qml", '"omarchy-launch-floating-terminal-with-presentation"',
              json.dumps(str(terminal)))

        qml = (HERE / "upgrade-offers.qml").read_text()
        qml = qml.replace("__PLUGIN__", json.dumps(str(plugin))).replace("__SCENARIO__", json.dumps(scenario))
        (root / "shell.qml").write_text(qml)

        env = dict(os.environ, QT_QPA_PLATFORM="offscreen", QT_QUICK_BACKEND="software",
                   QT_QPA_PLATFORMTHEME="", FAKE_VOXTYPE_STATE=str(state), FAKE_VOXTYPE_LOG=str(log))
        try:
            result = subprocess.run(["qs", "-p", str(root / "shell.qml")], env=env, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
            output = result.stdout
            code = result.returncode
        except subprocess.TimeoutExpired as error:
            output = (error.stdout or b"").decode() if isinstance(error.stdout, bytes) else (error.stdout or "")
            output += "\nTIMEOUT"
            code = -1
        calls = [json.loads(line) for line in log.read_text().splitlines() if line]
        launched = [json.loads(line) for line in terminal_log.read_text().splitlines() if line]
        final = json.loads(state.read_text())
        if evidence:
            out = evidence / scenario
            out.mkdir(parents=True, exist_ok=True)
            (out / "qml.log").write_text(output)
            (out / "commands.jsonl").write_text(log.read_text())
            (out / "final-state.json").write_text(json.dumps(final, indent=2))

    problems = []
    if code != 0 or ("UPGRADE_OFFERS_PASS " + scenario) not in output:
        problems.append("QML driver did not pass")
    for marker in ("UPGRADE_OFFERS_FAIL", "TypeError:", "ReferenceError:", "Unable to assign", "is not a type"):
        if marker in output:
            problems.append("log contains " + marker)
    migrate = [c for c in calls if c[:2] == ["setup", "migrate"] and "--json" not in c]
    # The panel must never write config or download through any other route.
    stray = [c for c in calls if c[:2] in (["config", "set"], ["config", "select-model"], ["config", "unset"])
             or (c[:1] == ["setup"] and c[:2] != ["setup", "migrate"]
                 and c[:2] != ["setup", "omarchy"])]
    # Of `setup omarchy`, only the status read, plus the dismissal where Not now
    # is clicked. Switch goes to the terminal, never through here.
    omarchy = [c for c in calls if c[:2] == ["setup", "omarchy"] and c != ["setup", "omarchy", "--status", "--json"]]
    expected_omarchy = [["setup", "omarchy", "--dismiss-copy"]] if scenario.startswith("linknotnow") else []
    if omarchy != expected_omarchy:
        problems.append("setup omarchy calls " + json.dumps(omarchy))
    if stray:
        problems.append("unexpected writes/downloads: " + json.dumps(stray))
    if scenario == "main":
        expected = [["setup", "migrate", "--decline"]] + [
            ["setup", "migrate", "--accept", "parakeet-ultra", "--yes", "--progress-format", "json"]] * 3 + [
            ["setup", "migrate", "--cleanup"]]
        if migrate != expected:
            problems.append("migrate calls " + json.dumps(migrate))
        if final["model"] != "parakeet-ultra-Q8_0.gguf" or final["cleanup"] is not None or final["offers"]:
            problems.append("final fake state " + json.dumps(final))
    elif scenario == "noflag":
        expected = [["setup", "migrate", "--accept", "format-gguf", "--yes", "--progress-format", "json"],
                    ["setup", "migrate", "--accept", "format-gguf", "--yes"]]
        if migrate != expected:
            problems.append("migrate calls " + json.dumps(migrate))
    elif scenario == "latecancel":
        if final["model"] != "parakeet-ultra-Q8_0.gguf":
            problems.append("late cancel must not undo a finished switch: " + json.dumps(final))
    elif scenario == "nomigrate":
        if migrate:
            problems.append("no migrate action expected: " + json.dumps(migrate))
    if scenario.startswith("link"):
        if ["setup", "omarchy", "--status", "--json"] not in calls:
            problems.append("setup omarchy --status --json was never asked")
        # VoxtypeCli.quote() always single-quotes.
        expected = [["'" + str(fake) + "' setup omarchy --replace --yes --restart-shell"]]
        if scenario != "linkcopy":
            expected = []
        if scenario == "linknotnow" and final.get("copy_dismissed") is not True:
            problems.append("dismissal not recorded: " + json.dumps(final))
        if scenario == "linknotnowfail" and "could not record Not now" not in output:
            problems.append("failed dismissal was not logged")
        if scenario.startswith("linknotnow") and calls.count(["setup", "omarchy", "--status", "--json"]) < 2:
            problems.append("link status not re-read after Not now")
        if launched != expected:
            problems.append("terminal launches " + json.dumps(launched))
    elif launched:
        problems.append("unexpected terminal launches " + json.dumps(launched))
    return output, problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--shell", type=Path, default=Path(os.environ.get("OMARCHY_PATH", "/usr/share/omarchy")) / "shell")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--scenario", choices=sorted(SCENARIOS), action="append")
    args = parser.parse_args()
    if os.environ.get("WAYLAND_DISPLAY") or os.environ.get("DISPLAY"):
        parser.error("refusing to run with a display in the environment; use the isolated sandbox")
    if not shutil.which("qs") or not (args.shell / "Commons").is_dir():
        parser.error("Quickshell and an Omarchy shell checkout are required (--shell PATH)")
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
