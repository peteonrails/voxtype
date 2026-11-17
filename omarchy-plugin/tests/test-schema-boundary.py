#!/usr/bin/env python3
"""Exercise schema delivery in the real QML engine and Omarchy panel.

Requires Quickshell and an Omarchy checkout. Never opens the panel, writes
settings, or restarts the daemon. TMPDIR controls the temporary config root.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--shell", type=Path, default=Path(os.environ.get("OMARCHY_PATH", "/usr/share/omarchy")) / "shell")
    parser.add_argument("--plugin", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    if not shutil.which("qs") or not (args.shell / "Commons").is_dir():
        parser.error("Quickshell and an Omarchy shell checkout are required (--shell PATH)")
    with tempfile.TemporaryDirectory(prefix="voxtype-schema-test-") as directory:
        root = Path(directory)
        # Real shell types keep this test honest about QML property/signal
        # conversion. No service or full shell is started.
        for child in args.shell.iterdir():
            if child.is_dir():
                (root / child.name).symlink_to(child.resolve(), target_is_directory=True)
        shutil.copytree(args.plugin, root / "plugin", ignore=shutil.ignore_patterns(".git", "__pycache__"))
        source = Path(__file__).with_name("schema-boundary.qml").read_text()
        (root / "shell.qml").write_text(source.replace('import "../" as Plugin', 'import "plugin" as Plugin'))
        # PanelWindow needs the Wayland backend even while invisible. Run in
        # the existing session (or under a headless compositor in CI).
        env = dict(os.environ, QT_QPA_PLATFORM="wayland", QT_QUICK_BACKEND="software")
        try:
            result = subprocess.run(["qs", "-p", str(root / "shell.qml")], env=env,
                                    text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=20)
        except subprocess.TimeoutExpired as error:
            print(error.stdout or "QML test timed out", file=sys.stderr)
            return 1
        print(result.stdout, end="")
        return 0 if result.returncode == 0 and "SCHEMA_BOUNDARY_PASS" in result.stdout and "SCHEMA_BOUNDARY_FAIL" not in result.stdout else 1


if __name__ == "__main__":
    sys.exit(main())
