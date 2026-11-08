#!/usr/bin/env python3
"""Test the real Voxtype executable's worker routing and init diagnostics.

Build with --features cohere-gguf,parakeet-gguf, then run:
  python3 testing/test-gguf-worker-routing.py --binary target/debug/voxtype

Protocol-sentinel workers isolate routing from native backend/model availability.
This is not an inference or GPU-acceleration test. All files stay in a temp tree.
"""

import argparse
import errno
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
BINARY = None


class WorkerRoutingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.lib = self.root / "usr/lib/voxtype"
        self.bin = self.root / "usr/bin"
        self.lib.mkdir(parents=True)
        self.bin.mkdir(parents=True)
        self.model = self.root / "probe.gguf"
        self.model.touch()
        self.config = self.root / "config.toml"
        self.config.write_text(
            'engine = "cohere"\n[cohere]\nmodel = '
            + json.dumps(str(self.model)) + '\ngguf_backend = "cpu"\nthreads = 2\n'
        )
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("VOXTYPE_")}

    def executable(self, name):
        assert BINARY is not None
        path = self.lib / name
        try:
            os.link(BINARY, path)
        except OSError as error:
            if error.errno != errno.EXDEV:
                raise
            shutil.copy2(BINARY, path)
        return path

    def worker(self, name):
        path = self.lib / name
        path.write_text(
            '#!/usr/bin/env python3\nimport json\nprint(json.dumps('
            '{"ok": False, "kind": "init_failed", "error": "routing-probe:'
            + name + '"}))\n'
        )
        path.chmod(0o755)
        return path

    def check_routing(self, executable, worker):
        result = subprocess.run(
            [str(executable), "--config", str(self.config), "transcribe",
             str(ROOT / "tests/fixtures/vad/speech_hello.wav")],
            env=self.env, capture_output=True, text=True, timeout=20, check=False,
        )
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        self.assertIn("routing-probe:" + worker.name, output)
        self.assertIn(str(worker), output, "Initialization error must identify the selected worker")

    def test_nested_legacy_gpu_layout_uses_packaged_cpu_worker(self):
        executable = self.lib / "migraphx/voxtype-onnx-migraphx"
        executable.parent.mkdir()
        assert BINARY is not None
        shutil.copy2(BINARY, executable)
        link = self.bin / "voxtype"
        link.symlink_to(executable)
        generic = self.worker("voxtype-gguf-worker")
        worker = executable.parent / "voxtype-gguf-worker"
        worker.symlink_to("../voxtype-gguf-worker")
        # The sentinel label comes from the generic target; errors name the
        # selected companion symlink beside the resolved application binary.
        result = subprocess.run(
            [str(link), "--config", str(self.config), "transcribe",
             str(ROOT / "tests/fixtures/vad/speech_hello.wav")],
            env=self.env, capture_output=True, text=True, timeout=20, check=False,
        )
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        self.assertIn("routing-probe:" + generic.name, output)
        self.assertIn(str(worker), output)

    def test_direct_vulkan_prefers_variant(self):
        executable = self.executable("voxtype-vulkan")
        self.worker("voxtype-gguf-worker")
        self.check_routing(executable, self.worker("voxtype-gguf-worker-vulkan"))

    def test_setup_gpu_symlink_prefers_variant(self):
        executable = self.executable("voxtype-vulkan")
        link = self.bin / "voxtype"
        link.symlink_to(executable)
        self.worker("voxtype-gguf-worker")
        self.check_routing(link, self.worker("voxtype-gguf-worker-vulkan"))

    def test_plain_cpu_uses_generic(self):
        executable = self.executable("voxtype")
        self.worker("voxtype-gguf-worker-vulkan")
        self.check_routing(executable, self.worker("voxtype-gguf-worker"))

    def test_cpu_variant_uses_generic(self):
        executable = self.executable("voxtype-avx2")
        self.worker("voxtype-gguf-worker-vulkan")
        self.check_routing(executable, self.worker("voxtype-gguf-worker"))

    def test_vulkan_falls_back_to_generic(self):
        executable = self.executable("voxtype-vulkan")
        self.check_routing(executable, self.worker("voxtype-gguf-worker"))

    def test_override_beats_variant(self):
        executable = self.executable("voxtype-vulkan")
        self.worker("voxtype-gguf-worker-vulkan")
        worker = self.worker("custom-worker")
        self.env["VOXTYPE_GGUF_WORKER"] = str(worker)
        self.check_routing(executable, worker)

    def test_future_variant_name(self):
        executable = self.executable("voxtype-onnx-cuda-13")
        self.worker("voxtype-gguf-worker")
        self.check_routing(executable, self.worker("voxtype-gguf-worker-onnx-cuda-13"))

    def test_early_exit_names_worker(self):
        executable = self.executable("voxtype-vulkan")
        worker = self.worker("voxtype-gguf-worker-vulkan")
        worker.write_text("#!/bin/sh\nexit 1\n")
        result = subprocess.run(
            [str(executable), "--config", str(self.config), "transcribe",
             str(ROOT / "tests/fixtures/vad/speech_hello.wav")],
            env=self.env, capture_output=True, text=True, timeout=20, check=False,
        )
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        self.assertIn(str(worker), output)
        self.assertIn("exited before loading the model", output)

    def test_missing_override_names_worker(self):
        executable = self.executable("voxtype-vulkan")
        worker = self.lib / "missing-worker"
        self.env["VOXTYPE_GGUF_WORKER"] = str(worker)
        self.worker("voxtype-gguf-worker-vulkan")
        result = subprocess.run(
            [str(executable), "--config", str(self.config), "transcribe",
             str(ROOT / "tests/fixtures/vad/speech_hello.wav")],
            env=self.env, capture_output=True, text=True, timeout=20, check=False,
        )
        output = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0, output)
        self.assertIn("Cannot start " + str(worker), output)
        self.assertNotIn("routing-probe:", output)

    def test_gpu_wrapper_and_override(self):
        self.executable("voxtype-vulkan")
        self.worker("voxtype-gguf-worker")
        variant = self.worker("voxtype-gguf-worker-vulkan")
        self.env.update(VOXTYPE_GPU="1", VOXTYPE_LIB=str(self.lib))
        wrapper = self.bin / "voxtype"
        wrapper.write_bytes((ROOT / "scripts/voxtype-wrapper.sh").read_bytes())
        wrapper.chmod(0o755)
        self.check_routing(wrapper, variant)
        worker = self.worker("custom-worker")
        self.env["VOXTYPE_GGUF_WORKER"] = str(worker)
        self.check_routing(wrapper, worker)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args, rest = parser.parse_known_args()
    BINARY = args.binary.resolve(strict=True)
    unittest.main(argv=[__file__] + rest, verbosity=2)
