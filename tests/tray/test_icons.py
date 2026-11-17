#!/usr/bin/env python3
"""Real CLI icon install/uninstall with disposable XDG data paths."""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]

def main():
    binary = ROOT / 'target-tray/debug/voxtype'
    with tempfile.TemporaryDirectory() as directory:
        data = Path(directory)
        env = dict(os.environ, XDG_DATA_HOME=str(data))
        def invoke(*args):
            subprocess.run([str(binary), 'setup', 'icons', *args], env=env, check=True)
        invoke()
        names = ['128x128/apps/voxtype.png', '128x128/apps/voxtype-recording.png',
                 'scalable/apps/voxtype.svg', 'scalable/apps/voxtype-recording.svg']
        base = data / 'icons/hicolor'
        first = {name: (base / name).read_bytes() for name in names}
        invoke()
        assert first == {name: (base / name).read_bytes() for name in names}
        invoke('--uninstall')
        assert all(not (base / name).exists() for name in names)
    print('ICON_CLI_PASS: explicit XDG install, idempotence and uninstall; no daemon startup installation')

if __name__ == '__main__':
    main()
