#!/usr/bin/env python3
"""The packaged plugin layout loads in Omarchy's plugin host.

Runs scripts/stage-omarchy-plugin.sh into a temp dir, checks the tree is
exactly the packaged layout (manifest, LICENSE, <version>/Panel.qml,
<version>/components/*.qml and nothing else), then links it into a temp
plugins directory the way `voxtype setup omarchy` does and loads it the way
Omarchy does:

- Omarchy's own PluginRegistry.qml (copied, with only its startup
  `mkdir -p ~/.config/omarchy/plugins` call removed) scans the temp plugins
  directory, validates the manifest and resolves the entry point URL;
- a Loader loads that URL, which proves `<version>/Panel.qml` finds its
  `components` import from inside the versioned directory;
- the panel is opened against a fake `voxtype` named by dev.json at the
  plugin root, which proves the dev.json lookup at manifest.__sourceDir still
  works when the QML sits one level below it.

The panel's layer-shell PanelWindow needs a Wayland compositor, so the staged
copy swaps it for a FloatingWindow of the same contents, after the tree check.
No config, daemon or network is touched.

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
REPO = PLUGIN.parent
STAGE = REPO / "scripts" / "stage-omarchy-plugin.sh"
PLUGIN_ID = "io.voxtype.settings"

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

REGISTRY_STARTUP = "Component.onCompleted: ensureUserDir()"

FAKE_CLI = r'''
import json, sys
args = sys.argv[1:]
if args[:3] == ["config", "schema", "--json"]:
    print(json.dumps({
        "schema_version": 1, "voxtype_version": "staged-layout-test", "config_path": "",
        "engine": "parakeet", "replacements": {},
        "keys": [
            {"key": "engine", "type": "enum", "choices": ["whisper", "parakeet"], "section": "Engine",
             "label": "Transcription engine", "description": "", "value": "parakeet",
             "file_value": "parakeet", "engine": None, "compiled": True, "restart_required": True},
            {"key": "audio.device", "type": "string", "section": "Audio", "label": "Device",
             "description": "", "value": "default", "file_value": None, "engine": None,
             "compiled": True, "restart_required": True},
        ],
    }))
elif args[:3] == ["info", "engines", "--json"]:
    print(json.dumps([{"name": "whisper", "compiled": True}, {"name": "parakeet", "compiled": True}]))
elif args[:3] == ["status", "--format", "json"]:
    print(json.dumps({"class": "idle"}))
else:
    sys.exit(2)
'''

DRIVER = r"""
import QtQuick
import Quickshell
import "registry" as Omarchy

Item {
  id: host
  property string failure: ""
  property bool done: false

  function finish(message) {
    if (host.done) return
    host.done = true
    if (message !== "") host.failure = message
    console.log(host.failure === "" ? "STAGED_LAYOUT_PASS" : "STAGED_LAYOUT_FAIL " + host.failure)
    Qt.callLater(Qt.quit)
  }

  Omarchy.PluginRegistry {
    id: registry
    firstPartyDir: ""
    pluginsDir: __PLUGINS_DIR__
    onScanFinished: {
      var manifest = registry.installedPlugins[__PLUGIN_ID__]
      if (!manifest) return host.finish("registry did not find " + __PLUGIN_ID__)
      if (manifest.version !== __VERSION__) return host.finish("registry read version " + manifest.version)
      var url = registry.entryPointUrl(manifest, "panel")
      if (url !== __EXPECTED_URL__) return host.finish("entry URL " + url + ", expected " + __EXPECTED_URL__)
      loader.manifest = manifest
      loader.source = url
    }
  }

  Loader {
    id: loader
    property var manifest: null
    asynchronous: true
    onStatusChanged: if (status === Loader.Error) host.finish("Loader could not load " + source)
    onLoaded: {
      if (!("manifest" in item)) return host.finish("loaded item has no manifest property")
      item.manifest = loader.manifest
      item.open("{}")
    }
  }

  Timer {
    interval: 100
    repeat: true
    running: loader.status === Loader.Ready && !host.done
    onTriggered: {
      var panel = loader.item
      if (panel.schemaError !== "") return host.finish("schema error: " + panel.schemaError)
      if (!panel.opened || panel.sections.length === 0 || panel.engine !== "parakeet") return
      if (panel.voxtypeVersion !== "staged-layout-test")
        return host.finish("panel did not run the dev.json binary (version " + panel.voxtypeVersion + ")")
      host.finish("")
    }
  }

  Component.onCompleted: registry.rescan()
  Timer { interval: 30000; running: true; onTriggered: host.finish("timeout") }
}
"""


def read_version(manifest_path):
    return json.loads(manifest_path.read_text())["version"]


def check_tree(plugin_dir, version):
    problems = []
    want = {"LICENSE", "manifest.json", version + "/Panel.qml"}
    want |= {version + "/components/" + p.name for p in (PLUGIN / "components").glob("*.qml")}
    have = {str(p.relative_to(plugin_dir)) for p in plugin_dir.rglob("*") if not p.is_dir()}
    if have != want:
        problems.append("tree differs: missing " + json.dumps(sorted(want - have))
                        + ", unexpected " + json.dumps(sorted(have - want)))
    staged = json.loads((plugin_dir / "manifest.json").read_text())
    source = json.loads((PLUGIN / "manifest.json").read_text())
    expected = dict(source, entryPoints=dict(source["entryPoints"], panel=version + "/Panel.qml"))
    if staged != expected:
        problems.append("staged manifest " + json.dumps(staged))
    for path in [plugin_dir] + list(plugin_dir.rglob("*")):
        mode = path.stat().st_mode & 0o777
        if mode != (0o755 if path.is_dir() else 0o644):
            problems.append("%s has mode %o" % (path.relative_to(plugin_dir.parent), mode))
    for name in ("Panel.qml", "components"):
        if (plugin_dir / name).exists():
            problems.append(name + " must not sit at the plugin root")
    return problems


def patch(path, old, new):
    text = path.read_text()
    assert old in text, (path, old[:60])
    path.write_text(text.replace(old, new, 1))


def run(shell, evidence):
    version = read_version(PLUGIN / "manifest.json")
    with tempfile.TemporaryDirectory(prefix="voxtype-staged-layout-") as directory:
        root = Path(directory)
        staged_root = root / "usr-share-voxtype-omarchy-plugin"
        result = subprocess.run(["bash", str(STAGE), str(PLUGIN), str(staged_root)],
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        if result.returncode != 0:
            return result.stdout, ["stage script failed"]
        plugin_dir = staged_root / PLUGIN_ID
        problems = check_tree(plugin_dir, version)
        if problems:
            return result.stdout, problems

        # Link it the way `voxtype setup omarchy` does (contract 2.2).
        plugins = root / "plugins"
        plugins.mkdir()
        link = plugins / PLUGIN_ID
        link.symlink_to(plugin_dir, target_is_directory=True)

        qs_root = root / "shell"
        qs_root.mkdir()
        for child in shell.iterdir():
            if child.is_dir():
                (qs_root / child.name).symlink_to(child.resolve(), target_is_directory=True)
        registry_dir = qs_root / "registry"
        registry_dir.mkdir()
        shutil.copy(shell / "services" / "PluginRegistry.qml", registry_dir / "PluginRegistry.qml")
        # Its only startup action is creating the user's plugin directory and
        # reaping stray watchers; neither belongs in a test.
        patch(registry_dir / "PluginRegistry.qml", REGISTRY_STARTUP, "Component.onCompleted: {}")

        patch(plugin_dir / version / "Panel.qml", WINDOW_OLD, WINDOW_NEW)
        fake = root / "voxtype-fake"
        fake_py = root / "fake-voxtype.py"
        fake_py.write_text(FAKE_CLI)
        fake.write_text("#!/bin/sh\nexec " + sys.executable + " -I " + str(fake_py) + ' "$@"\n')
        fake.chmod(0o755)
        (plugin_dir / "dev.json").write_text(json.dumps({"voxtypeBin": str(fake)}))

        expected_url = "file://" + str(link) + "/" + version + "/Panel.qml"
        qml = (DRIVER.replace("__PLUGINS_DIR__", json.dumps(str(plugins)))
               .replace("__PLUGIN_ID__", json.dumps(PLUGIN_ID))
               .replace("__VERSION__", json.dumps(version))
               .replace("__EXPECTED_URL__", json.dumps(expected_url)))
        (qs_root / "shell.qml").write_text(qml)

        env = dict(os.environ, QT_QPA_PLATFORM="offscreen", QT_QUICK_BACKEND="software",
                   QT_QPA_PLATFORMTHEME="", HOME=str(root))
        try:
            proc = subprocess.run(["qs", "-p", str(qs_root / "shell.qml")], env=env, text=True,
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
            output, code = proc.stdout, proc.returncode
        except subprocess.TimeoutExpired as error:
            output = error.stdout.decode() if isinstance(error.stdout, bytes) else (error.stdout or "")
            output += "\nTIMEOUT"
            code = -1
        if evidence:
            evidence.mkdir(parents=True, exist_ok=True)
            (evidence / "qml.log").write_text(output)

    problems = []
    if code != 0 or "STAGED_LAYOUT_PASS" not in output:
        problems.append("QML driver did not pass")
    for marker in ("STAGED_LAYOUT_FAIL", "TypeError:", "ReferenceError:", "is not a type", "Unable to assign"):
        if marker in output:
            problems.append("log contains " + marker)
    return output, problems


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--shell", type=Path,
                        default=Path(os.environ.get("OMARCHY_PATH", "/usr/share/omarchy")) / "shell")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    if os.environ.get("WAYLAND_DISPLAY") or os.environ.get("DISPLAY"):
        parser.error("refusing to run with a display in the environment; use the isolated sandbox")
    if not shutil.which("qs") or not (args.shell / "services" / "PluginRegistry.qml").is_file():
        parser.error("Quickshell and an Omarchy shell checkout are required (--shell PATH)")
    output, problems = run(args.shell, args.evidence)
    if problems:
        print(output)
        print("FAIL staged-layout: " + "; ".join(problems))
        return 1
    print("PASS staged-layout")
    return 0


if __name__ == "__main__":
    sys.exit(main())
