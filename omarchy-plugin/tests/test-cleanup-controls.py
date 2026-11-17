#!/usr/bin/env python3
"""Run actual Panel/SchemaForm controls against a disposable real CLI config."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--old-bin", default="voxtype", help="Executable whose schema lacks LARD (default: voxtype on PATH)")
    parser.add_argument("--shell", type=Path, default=Path("/usr/share/omarchy/shell"))
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    plugin = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="voxtype-cleanup-test-") as directory:
        root = Path(directory)
        config = root / "config.toml"
        config.write_text('# Disposable GUI integration test\nengine = "whisper"\n[text]\nfilter_filler_words = false\ncollapse_restarts = false\nrepair_sentence_breaks = true\nformat_numbers = true\n')
        wrapper = root / "voxtype-test"
        wrapper.write_text("#!/usr/bin/python3\nimport os, sys\nos.execv(" + repr(str(args.bin.resolve())) + ", [" + repr(str(args.bin.resolve())) + ", '--config', " + repr(str(config)) + "] + sys.argv[1:])\n")
        wrapper.chmod(0o755)
        for child in args.shell.iterdir():
            if child.is_dir():
                (root / child.name).symlink_to(child.resolve(), target_is_directory=True)
        shutil.copytree(plugin, root / "plugin", ignore=shutil.ignore_patterns(".git", "__pycache__"))
        # Read-only test access to the real component IDs; no behavior is replaced.
        panel_path = root / "plugin/Panel.qml"
        panel_path.write_text(panel_path.read_text().replace("id: root", "id: root\n  readonly property alias formUnderTest: form\n  readonly property alias cliUnderTest: cli\n  readonly property alias ipcUnderTest: ipcForTest", 1).replace("  IpcHandler {", "  IpcHandler {\n    id: ipcForTest", 1))
        qml = (plugin / "tests/cleanup-controls.qml").read_text().replace("Plugin.Panel { id: panel }", "Plugin.Panel { id: panel; manifest: ({__sourceDir: " + json.dumps(str(root / "plugin")) + "}) }")
        old_schema = subprocess.check_output([args.old_bin, "--config", str(config), "config", "schema", "--json"], text=True)
        (args.evidence / "old-schema.json").write_text(old_schema)
        qml = qml.replace('property int phase: 0', 'property var oldSchema: ' + old_schema + '\n  property int phase: 0')
        (root / "plugin/dev.json").write_text(json.dumps({"voxtypeBin": str(wrapper)}))
        (root / "shell.qml").write_text(qml)
        env = dict(os.environ, QT_QPA_PLATFORM="wayland", QT_QUICK_BACKEND="software")
        result = subprocess.run(["qs", "-p", str(root / "shell.qml")], env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=25)
        print(result.stdout, end="")
        (args.evidence / "qml.log").write_text(result.stdout)
        shutil.copy2(config, args.evidence / "persisted-config.toml")
        ok = (result.returncode == 0 and "CLEANUP_CONTROLS_PASS" in result.stdout
              and not any(marker in result.stdout for marker in
                          ("CLEANUP_CONTROLS_FAIL", "TypeError:", "ReferenceError:", "Unable to assign")))
        if ok:
            saved = tomllib.loads(config.read_text())["text"]
            assert saved == {"filter_filler_words": False, "collapse_restarts": False,
                             "repair_sentence_breaks": True, "format_numbers": True, "lard_disfluency": False}, saved
            assert config.read_text().startswith("# Disposable GUI integration test")
            schema = subprocess.check_output([str(wrapper), "config", "schema", "--json"], text=True)
            (args.evidence / "schema.json").write_text(schema)
            print("PERSISTENCE_PASS: exact keys, sibling values and comments preserved")
        return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
