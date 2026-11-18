#!/usr/bin/env python3
"""Release Dockerfiles may only download build inputs that are pinned.

On 2026-11-18 github.com/microsoft/onnxruntime returned 404 and two release
jobs had nothing else to fetch from. Anything a release Dockerfile downloads
(other than distribution packages and Cargo dependencies) now comes from the
project's mirror through scripts/ci/fetch-build-input.sh with a sha256 fixed
in the Dockerfile. This fails when a Dockerfile grows a download that is not.
"""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import hashlib
import os
import re
import subprocess
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "scripts/ci/fetch-build-input.sh"
MIRROR_ARG = "VOXTYPE_BUILD_INPUTS_URL"
MIRROR_DEFAULT = "https://models.voxtype.io/build-inputs"

# Downloads that predate this check, with why each is tolerated and which
# Dockerfiles may use it (None: any). This is not the place for a new release
# input: mirror it with a script like scripts/mirror-onnxruntime-inputs.sh
# and fetch it through the helper instead.
KNOWN_UNPINNED = {
    "https://sh.rustup.rs": (None, "rustup bootstrap over TLS; the toolchain it installs is not pinned either"),
    "http://azure.archive.ubuntu.com": (None, "alternate apt mirror; apt verifies package signatures"),
    "https://github.com/wmww/gtk4-layer-shell.git": (
        {"Dockerfile.onnx", "Dockerfile.aarch64-onnx"},
        "tag clone for the GTK4 OSD; not mirrored yet",
    ),
}

URL = re.compile(r"""(?:(?:https?|git|ssh|ftp)://|\bgit@)[^\s"'\;|)]+""")
PINNED_FETCH = re.compile(
    r'fetch-build-input\.sh\s+"\$\{%s\}[^"\s]*"\s+"\$\{(\w+)\}"\s' % MIRROR_ARG)


def instructions(text):
    """Dockerfile instructions: comment lines dropped, continuations joined."""
    lines = [line for line in text.splitlines() if not line.lstrip().startswith("#")]
    return [ins.strip() for ins in re.sub(r"\\\n", " ", "\n".join(lines)).splitlines() if ins.strip()]


def problems(name, text):
    found = []
    steps = instructions(text)
    pins = dict(re.findall(r"^ENV (\w+)=([0-9a-f]{64})$", "\n".join(steps), re.M))
    for step in steps:
        for url in URL.findall(step):
            if step == f"ARG {MIRROR_ARG}={MIRROR_DEFAULT}":
                continue
            allowed = next((files for known, (files, _) in KNOWN_UNPINNED.items() if url.startswith(known)), False)
            if allowed is False or (allowed is not None and name not in allowed):
                found.append(f"{name}: downloads {url} from outside the mirror without a sha256 pin")
        uses = len(re.findall(r"\$\{?%s\b" % MIRROR_ARG, step))
        if not uses or step.startswith("ARG "):
            continue
        fetches = PINNED_FETCH.findall(step + " ")
        if len(fetches) != uses:
            found.append(f"{name}: uses ${{{MIRROR_ARG}}} outside fetch-build-input.sh URL SHA256 DESTINATION")
        for variable in fetches:
            if variable not in pins:
                found.append(f"{name}: ${{{variable}}} is not an ENV holding a sha256 "
                             "(an ARG could be overridden at build time)")
    return found


@contextmanager
def serve(status, payload):
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
        yield f"http://127.0.0.1:{server.server_port}/input.tar.gz"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


class DockerfileDownloadTests(unittest.TestCase):
    def dockerfiles(self):
        files = sorted(ROOT.glob("Dockerfile*"))
        self.assertGreaterEqual(len(files), 12, "release Dockerfiles not found")
        return files

    def test_release_dockerfiles_download_only_pinned_or_known_inputs(self):
        found = [p for f in self.dockerfiles() for p in problems(f.name, f.read_text())]
        self.assertEqual(found, [], "\n" + "\n".join(found))

    def test_the_downloads_that_broke_the_release_are_rejected(self):
        curl = ('ENV ORT_VERSION=1.24.4\nRUN set -eux; \\\n    curl -fsSL -o /tmp/ort.tgz \\\n      "https://github.com/'
                'microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-gpu_cuda13-${ORT_VERSION}.tgz"; \\\n'
                "    tar -xzf /tmp/ort.tgz\n")
        clone = ("RUN git clone --depth 1 --branch v${ORT_VERSION} --recurse-submodules \\\n"
                 "        https://github.com/microsoft/onnxruntime.git /build/onnxruntime \\\n    && ./build.sh\n")
        for name, text in (("curl", curl), ("clone", clone), ("add", "ADD https://example.org/x.tar.gz /x\n"),
                           ("ssh", "RUN git clone git@github.com:someone/thing.git\n")):
            self.assertEqual(len(problems("Dockerfile.x", text)), 1, name)
        # A known exception does not carry over to other Dockerfiles.
        gtk = "RUN git clone --branch v1.3.0 https://github.com/wmww/gtk4-layer-shell.git /tmp/g\n"
        self.assertEqual(problems("Dockerfile.onnx", gtk), [])
        self.assertEqual(len(problems("Dockerfile.vulkan", gtk)), 1)

    def test_mirror_downloads_need_the_helper_and_a_fixed_pin(self):
        arg = f"ARG {MIRROR_ARG}={MIRROR_DEFAULT}\n"
        pin = "ENV X_SHA256=" + "a" * 64 + "\n"
        fetch = 'RUN bash scripts/ci/fetch-build-input.sh "${%s}/x/x.tar.gz" "${X_SHA256}" /tmp/x.tar.gz; \\\n    tar -xzf /tmp/x.tar.gz\n' % MIRROR_ARG
        self.assertEqual(problems("Dockerfile.x", arg + pin + fetch), [])
        cases = {
            "pin missing": arg + fetch,
            "pin overridable": arg + "ARG X_SHA256=" + "a" * 64 + "\n" + fetch,
            "pin not a sha256": arg + "ENV X_SHA256=latest\n" + fetch,
            "plain curl": arg + pin + 'RUN curl -fsSL -o /tmp/x "${%s}/x/x.tar.gz"\n' % MIRROR_ARG,
            "another default host": f"ARG {MIRROR_ARG}=https://example.org/build-inputs\n" + pin + fetch,
        }
        for name, text in cases.items():
            self.assertTrue(problems("Dockerfile.x", text), name)

    def test_known_exceptions_are_still_in_use(self):
        text = "\n".join("\n".join(instructions(f.read_text())) for f in self.dockerfiles())
        for url in KNOWN_UNPINNED:
            self.assertIn(url, text, f"{url} is no longer downloaded; drop it from KNOWN_UNPINNED")

    def test_dockerfile_pins_are_the_mirror_script_pins(self):
        script = (ROOT / "scripts/mirror-onnxruntime-inputs.sh").read_text()
        value = lambda name, text: re.search(r'^(?:ENV )?%s="?([0-9a-f]+)"?$' % name, text, re.M).group(1)
        cuda = (ROOT / "Dockerfile.onnx-cuda-13").read_text()
        migraphx = (ROOT / "Dockerfile.onnx-migraphx").read_text()
        self.assertEqual(value("ORT_CUDA13_SHA256", cuda), value("CUDA13_SHA256", script))
        self.assertEqual(value("ORT_SOURCE_SHA256", migraphx), value("SOURCE_SHA256", script))
        self.assertEqual(value("ORT_SOURCE_COMMIT", migraphx), value("SOURCE_COMMIT", script))
        for text in (cuda, migraphx):
            self.assertIn(f"ARG {MIRROR_ARG}={MIRROR_DEFAULT}\n", text)


class FetchBuildInputTests(unittest.TestCase):
    GOOD = b"\x1f\x8b" + bytes(range(256)) * 4

    def fetch(self, status, payload, pin=None):
        pin = hashlib.sha256(self.GOOD).hexdigest() if pin is None else pin
        with tempfile.TemporaryDirectory(prefix="build-input-", dir=os.environ.get("TMPDIR")) as tmp:
            destination = Path(tmp) / "in/input.tar.gz"
            with serve(status, payload) as url:
                result = subprocess.run(["bash", str(HELPER), url, pin, str(destination)],
                                        capture_output=True, text=True, timeout=60)
            kept = destination.read_bytes() if destination.exists() else None
            leftovers = [p.name for p in Path(tmp).rglob("*") if p.is_file() and p != destination]
            return result, kept, leftovers

    def test_keeps_the_pinned_bytes(self):
        result, kept, leftovers = self.fetch(200, self.GOOD)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(kept, self.GOOD)
        self.assertEqual(leftovers, [])

    def test_rejects_other_bytes_of_the_same_size(self):
        result, kept, leftovers = self.fetch(200, self.GOOD[:-1] + b"\x00")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("is not the pinned file", result.stderr)
        self.assertIn(hashlib.sha256(self.GOOD).hexdigest(), result.stderr)
        self.assertIsNone(kept)
        self.assertEqual(leftovers, [])

    def test_rejects_an_http_error_even_with_the_right_body(self):
        result, kept, _ = self.fetch(404, self.GOOD)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("could not download build input", result.stderr)
        self.assertIsNone(kept)

    def test_refuses_to_download_without_a_sha256(self):
        for pin in ("", "latest", "A" * 64, "a" * 63):
            result, kept, _ = self.fetch(200, self.GOOD, pin=pin)
            self.assertNotEqual(result.returncode, 0, pin)
            self.assertIsNone(kept)


if __name__ == "__main__":
    unittest.main(verbosity=2)
