#!/usr/bin/env python3
"""Exercise the baseline CI model fetch against private loopback responses."""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import os
import re
import hashlib
import subprocess
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[1]
URL = "https://models.voxtype.io/whisper/tiny.en/ggml-tiny.en.bin"
FIXTURE = b"lmgg" + bytes(range(256)) * 2


@contextmanager
def response(status, payload):
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(status)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, format, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/model"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


class BaselineModelTests(unittest.TestCase):
    def fetch(self, status, payload, destination, expected=None):
        workflow = (ROOT / ".github/workflows/build-linux.yml").read_text()
        step = workflow.split("- name: Run inference on a QEMU-modeled v2 CPU", 1)[1]
        if "bash scripts/ci/fetch-baseline-model.sh" in step:
            script = (ROOT / "scripts/ci/fetch-baseline-model.sh").read_text()
            if expected is not None:
                # Parameterize only the two pins in a private script copy, so
                # ordinary offline CI exercises success and same-size hash
                # corruption without a network model download. Production pins
                # and the production helper remain unchanged and non-overridable.
                script, count = re.subn(r"(?m)^expected_size = \d+$",
                    f"expected_size = {len(expected)}", script)
                self.assertEqual(count, 1)
                script, count = re.subn(r'(?m)^expected_sha256 = "[0-9a-f]{64}"$',
                    f'expected_sha256 = "{hashlib.sha256(expected).hexdigest()}"', script)
                self.assertEqual(count, 1)
        else:
            # Replay the original workflow fetch, not an invented failing command.
            script = step.split('          curl -sSL', 1)[1].split("          printf 'engine", 1)[0]
            script = "curl -sSL" + script
        with response(status, payload) as endpoint:
            env = os.environ.copy()
            env["XDG_DATA_HOME"] = str(destination.parents[2])
            return subprocess.run(
                ["bash", "-eu", "-c", script.replace(URL, endpoint), "--", str(destination)],
                capture_output=True, text=True, env=env, timeout=30,
            )

    def reject(self, status, payload, expected=None):
        with tempfile.TemporaryDirectory(prefix="baseline-model-", dir=os.environ.get("TMPDIR")) as tmp:
            dest = Path(tmp) / "voxtype/models/ggml-tiny.en.bin"
            dest.parent.mkdir(parents=True)
            dest.write_bytes(b"previous verified model")
            result = self.fetch(status, payload, dest, expected=expected)
            self.assertNotEqual(result.returncode, 0, "the workflow accepted an invalid model")
            self.assertEqual(dest.read_bytes(), b"previous verified model")

    def test_rejects_http_error_body(self):
        self.reject(404, b"<html>not a model</html>")

    def test_rejects_successful_html_response(self):
        self.reject(200, b"<html>not a model</html>")

    def test_rejects_truncated_ggml(self):
        self.reject(200, b"lmgg" + bytes(100))

    def test_rejects_same_size_fixture_corruption(self):
        payload = FIXTURE[:-1] + bytes([FIXTURE[-1] ^ 1])
        self.reject(200, payload, expected=FIXTURE)

    def test_accepts_verified_fixture_bytes(self):
        with tempfile.TemporaryDirectory(prefix="baseline-model-", dir=os.environ.get("TMPDIR")) as tmp:
            dest = Path(tmp) / "voxtype/models/ggml-tiny.en.bin"
            dest.parent.mkdir(parents=True)
            payload = FIXTURE
            result = self.fetch(200, payload, dest, expected=FIXTURE)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(dest.read_bytes(), payload)

    def test_production_model_pins_cannot_be_overridden(self):
        script = (ROOT / "scripts/ci/fetch-baseline-model.sh").read_text()
        self.assertIn("expected_size = 77704715\n", script)
        self.assertIn('expected_sha256 = "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f"\n', script)
        self.assertNotIn("os.environ", script)
        self.assertIn('python3 - "$partial"', script)


if __name__ == "__main__":
    unittest.main(verbosity=2)
