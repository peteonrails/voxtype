#!/usr/bin/env python3
"""Exercise real configure keyboard input with a disposable file, never a daemon.

Run: uv run --with pyte python3 testing/test-tui-cleanup.py --bin PATH --evidence DIR
The child is killed rather than using TUI quit, which can restart a daemon.
A disconnected D-Bus address provides an additional guard against service actions.
"""
import argparse
import codecs
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import tempfile
import termios
import time
import tomllib

import pyte


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", required=True, type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    binary = args.bin.resolve()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    screen = pyte.Screen(160, 40)
    stream = pyte.Stream(screen)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    raw = bytearray()
    events = []
    with tempfile.TemporaryDirectory(prefix="voxtype-tui-cleanup-") as temporary:
        config = Path(temporary) / "config.toml"
        config.write_text("# Disposable real TUI persistence test\nengine = 'cohere'\n[cohere]\nmodel = 'explicit-legacy-onnx' # preserve model\n[text]\nfilter_filler_words = false # preserve inline comment\ncollapse_restarts = false\nrepair_sentence_breaks = true\nformat_numbers = true\n[text.replacements]\n" + "".join(f"'word{i:02d}' = 'Value{i:02d}' # keep replacement {i}\n" for i in range(30)))
        original = tomllib.loads(config.read_text())
        pid, fd = pty.fork()
        if pid == 0:
            env = dict(os.environ, TERM="xterm-256color", DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent-voxtype-tui-test-bus",
                       VOXTYPE_FILTER_FILLERS="true", VOXTYPE_LARD_DISFLUENCY="true", VOXTYPE_LARD_MODEL_PATH="/environment/not-file.vxdf")
            os.execve(str(binary), [str(binary), "--config", str(config), "configure"], env)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 160, 0, 0))

        def drain(seconds=0.35):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                ready, _, _ = select.select([fd], [], [], min(0.05, max(0, deadline - time.monotonic())))
                if ready:
                    chunk = os.read(fd, 65536)
                    if not chunk:
                        raise RuntimeError("TUI exited unexpectedly")
                    raw.extend(chunk)
                    stream.feed(decoder.decode(chunk))

        def send(keys, label):
            events.append(label)
            os.write(fd, keys.encode())
            drain()

        def snapshot(name):
            text = "\n".join(screen.display) + "\n"
            (evidence / name).write_text(text)
            return text

        def saved():
            return tomllib.loads(config.read_text())

        try:
            drain(2)
            send("jjjjj\r", "sidebar: open Text Cleanup")
            text = snapshot("tui-controls.txt")
            assert "Text Cleanup" in text and "LARD disfluency cleanup" in text, text
            assert "Remove filler words" in text and "Collapse restarts" in text, text
            send("j s", "toggle filler and save")
            text_settings = saved()["text"]
            assert text_settings["filter_filler_words"] is True
            assert "lard_disfluency" not in text_settings
            assert "punctuation_casing" not in text_settings
            assert text_settings["collapse_restarts"] is False
            assert "true # preserve inline comment" in config.read_text()
            send("jjjj s", "toggle independent LARD opt-in and save")
            assert saved()["text"]["lard_disfluency"] is True
            assert saved()["text"]["filter_filler_words"] is True
            text = snapshot("tui-lard-opt-in.txt")
            assert "English only" in text and "CC BY-SA 4.0" in text, text
            send("jjj\r/missing/q-model.vxdf\rs", "edit model path containing q; commit and save")
            assert saved()["text"]["lard_model_path"] == "/missing/q-model.vxdf"
            assert not Path("/missing/q-model.vxdf").exists()
            send("\r\x15/missing/cancel.vxdf\x1b", "cancel model path edit")
            send("s", "save after cancel")
            assert saved()["text"]["lard_model_path"] == "/missing/q-model.vxdf"
            send("\r\x15\rs", "clear path to default without disabling LARD")
            assert "lard_model_path" not in saved()["text"]
            assert saved()["text"]["lard_disfluency"] is True
            send("j\r\x15renamed key\r\x15Renamed value\rs", "edit existing replacement key and value")
            replacements = saved()["text"]["replacements"]
            assert "word00" not in replacements and replacements["renamed key"] == "Renamed value"
            send("k" * 9 + "k", "wrap from first control to Add new replacement")
            screen.resize(18, 110)
            fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 18, 110, 0, 0))
            os.kill(pid, signal.SIGWINCH)
            drain()
            text = snapshot("tui-scrolled-add.txt")
            assert "+ Add new replacement" in text, text
            send("\rnew key\rnew value\rs", "add replacement in small scrolled viewport")
            assert saved()["text"]["replacements"]["new key"] == "new value"
            before = len(saved()["text"]["replacements"])
            send("ds", "delete selected replacement and save")
            assert len(saved()["text"]["replacements"]) == before - 1
            send("r", "revert from the explicit disposable config")
            final = saved()
            assert final["cohere"] == original["cohere"]
            for key in ["collapse_restarts", "repair_sentence_breaks", "format_numbers"]:
                assert final["text"][key] == original["text"][key]
            assert "# preserve model" in config.read_text()
            (evidence / "persisted-config.toml").write_text(config.read_text())
            (evidence / "result.json").write_text(json.dumps({"result": "TUI_PTY_PASS", "binary": str(binary), "events": events, "daemon_restart": False, "config_is_disposable": True}, indent=2) + "\n")
            print("TUI_PTY_PASS: navigation, independent switches, omitted keys, comments, model edit/cancel/clear, replacement edit/add/delete, resize/scroll and revert")
        finally:
            (evidence / "terminal.raw").write_bytes(raw)
            snapshot("tui-last.txt")
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
            os.close(fd)


if __name__ == "__main__":
    main()
