#!/usr/bin/env python3
"""Real PTY persistence smoke, never normal quit/restart the user's daemon."""
import fcntl
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import termios
import time
import tomllib

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = ROOT / 'tray-evidence/tui'
EVIDENCE.mkdir(parents=True, exist_ok=True)
BIN = ROOT / 'target-tray/debug/voxtype'

def exercise(name, initial, keys):
    config = EVIDENCE / (name + '.toml')
    config.write_text(initial)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 140, 0, 0))
    env = dict(os.environ, TERM='xterm-256color')
    proc = subprocess.Popen([str(BIN), '--config', str(config), 'configure'], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
    os.close(slave)
    raw = bytearray()
    def drain(seconds=0.2):
        until = time.monotonic() + seconds
        while time.monotonic() < until:
            if select.select([master], [], [], 0.05)[0]:
                try: raw.extend(os.read(master, 65536))
                except OSError: break
    try:
        drain(0.5)
        # General -> Status, then open the form.
        for _ in range(10):
            os.write(master, b'j'); drain(0.03)
        os.write(master, b'\r'); drain(0.3)
        # Differential terminal repainting splits labels across escape sequences.
        # The persisted canonical tray key below is the navigation assertion.
        for key in keys:
            os.write(master, key); drain(0.2)
        data = tomllib.loads(config.read_text())
    finally:
        proc.kill(); proc.wait(timeout=3)
        os.close(master)
        (EVIDENCE / (name + '.pty')).write_bytes(raw)
    assert '# keep comment' in config.read_text()
    assert data['hotkey']['key'] == 'F13'
    return data

def main():
    base = "# keep comment\n[hotkey]\nkey = 'F13'\n"
    assert exercise('enable', base, [b' ', b's'])['tray']['enabled'] is True
    assert 'tray' not in exercise('unchanged', base, [b's'])
    assert exercise('disable', base + '\n[tray]\nenabled = true\n', [b' ', b's'])['tray']['enabled'] is False
    print('TUI_TRAY_PASS: actual configure navigation, enable/disable, unchanged omission, sibling values/comments retained')

if __name__ == '__main__':
    main()
