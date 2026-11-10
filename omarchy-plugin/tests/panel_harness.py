"""Shared plumbing for the offscreen panel tests (test-cli-missing.py,
test-section-refresh.py).

Same approach as test-upgrade-offers.py: copy the real plugin, swap the
layer-shell PanelWindow for a FloatingWindow (offscreen has no compositor), add
test-only aliases, point dev.json at fake-voxtype-migrate.py, and route the
floating-terminal launcher to a logger so no terminal is ever opened. The QML
driver prints "<MARKER>_PASS <scenario>" or "<MARKER>_FAIL <scenario>: why".
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
PLUGIN = HERE.parent
FAKE = HERE / "fake-voxtype-migrate.py"

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

LOG_MARKERS = ("TypeError:", "ReferenceError:", "Unable to assign", "is not a type")


def patch(path, old, new):
    text = path.read_text()
    assert old in text, (path, old[:60])
    path.write_text(text.replace(old, new, 1))


def base_state(**overrides):
    state = {"model": "parakeet-tdt-0.6b-v3", "cleanup": None, "accept_script": [], "offers": [],
             "omarchy": None}
    state.update(overrides)
    return state


def guard_environment(parser, shell):
    if os.environ.get("WAYLAND_DISPLAY") or os.environ.get("DISPLAY"):
        parser.error("refusing to run with a display in the environment; use the isolated sandbox")
    if not shutil.which("qs") or not (shell / "Commons").is_dir():
        parser.error("Quickshell and an Omarchy shell checkout are required (--shell PATH)")


class Run:
    """One scenario's sandbox directory. Use as a context manager."""

    def __init__(self, shell, state):
        self._tmp = tempfile.TemporaryDirectory(prefix="voxtype-panel-test-")
        self.root = Path(self._tmp.name)
        for child in shell.iterdir():
            if child.is_dir():
                (self.root / child.name).symlink_to(child.resolve(), target_is_directory=True)
        self.plugin = self.root / "plugin"
        shutil.copytree(PLUGIN, self.plugin, ignore=shutil.ignore_patterns(".git", "__pycache__", "tests", "dev.json"))
        patch(self.plugin / "Panel.qml", WINDOW_OLD, WINDOW_NEW)
        patch(self.plugin / "Panel.qml", "id: root", "id: root\n  readonly property alias formTest: form\n"
              "  readonly property alias cliTest: cli\n  readonly property alias ipcTest: ipcProbe\n"
              "  readonly property alias renderTest: settingsCard.card")
        patch(self.plugin / "Panel.qml", "  IpcHandler {\n", "  IpcHandler {\n    id: ipcProbe\n")

        self.state = self.root / "state.json"
        self.state.write_text(json.dumps(state))
        self.log = self.root / "commands.jsonl"
        self.log.write_text("")
        self.fake = self.root / "voxtype-fake"
        self.fake.write_text("#!/bin/sh\nexec " + sys.executable + " -I " + str(FAKE) + ' "$@"\n')
        self.fake.chmod(0o755)

        self.terminal_log = self.root / "terminal.jsonl"
        self.terminal_log.write_text("")
        terminal = self.root / "terminal-fake"
        terminal.write_text("#!/bin/sh\nexec " + sys.executable + " -I -c 'import json,sys; "
                            "open(sys.argv[1], \"a\").write(json.dumps(sys.argv[2:]) + \"\\n\")' "
                            + str(self.terminal_log) + ' "$@"\n')
        terminal.chmod(0o755)
        patch(self.plugin / "components/VoxtypeCli.qml", '"omarchy-launch-floating-terminal-with-presentation"',
              json.dumps(str(terminal)))

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self._tmp.cleanup()

    def dev_json(self, bin_path):
        (self.plugin / "dev.json").write_text(json.dumps({"voxtypeBin": str(bin_path)}))

    def run(self, driver, substitutions, env=None, timeout=60):
        qml = driver.read_text().replace("__PLUGIN__", json.dumps(str(self.plugin)))
        for key, value in substitutions.items():
            qml = qml.replace(key, json.dumps(value))
        (self.root / "shell.qml").write_text(qml)
        full_env = dict(os.environ, QT_QPA_PLATFORM="offscreen", QT_QUICK_BACKEND="software",
                        QT_QPA_PLATFORMTHEME="", FAKE_VOXTYPE_STATE=str(self.state),
                        FAKE_VOXTYPE_LOG=str(self.log))
        full_env.update(env or {})
        qs = shutil.which("qs")
        try:
            result = subprocess.run([qs, "-p", str(self.root / "shell.qml")], env=full_env, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout)
            return result.stdout, result.returncode
        except subprocess.TimeoutExpired as error:
            out = error.stdout or ""
            if isinstance(out, bytes):
                out = out.decode()
            return out + "\nTIMEOUT", -1

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines() if line]

    def launched(self):
        return [json.loads(line) for line in self.terminal_log.read_text().splitlines() if line]


def driver_problems(output, code, marker, scenario):
    problems = []
    if code != 0 or (marker + "_PASS " + scenario) not in output:
        problems.append("QML driver did not pass")
    for text in (marker + "_FAIL",) + LOG_MARKERS:
        if text in output:
            problems.append("log contains " + text)
    return problems
