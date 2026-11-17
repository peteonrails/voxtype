#!/usr/bin/env python3
"""Check the setup allowlist covers the module; actual CLI load is also gated."""
import pathlib
import re

root = pathlib.Path(__file__).resolve().parents[1]
source = (root / "src/setup/quickshell.rs").read_text()
block = source.split("const SHARED_FILES: &[&str] = &[", 1)[1].split("];", 1)[0]
installed = set(re.findall(r'"([^"\n]+)"', block))
module = root / "quickshell/voxtype-shared"
required = set()
for line in (module / "qmldir").read_text().splitlines():
    tokens = line.split()
    if tokens and tokens[-1].endswith(".qml"):
        required.add(tokens[-1])
missing = sorted(required - installed)
assert not missing, f"setup quickshell omits required module assets: {missing}"
print(f"QUICKSHELL_INSTALL_ASSETS_PASS {len(required)} QML module files")
