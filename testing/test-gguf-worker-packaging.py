#!/usr/bin/env python3
"""Exercise the install-test worker gate in isolated package layouts.

No root, installed Voxtype, model downloads, or native compilation required.
Run: python3 testing/test-gguf-worker-packaging.py
"""

from pathlib import Path
import os
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class WorkerPackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def install(self, relative, executable=True):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/bin/sh\nexit 0\n")
        path.chmod(0o755 if executable else 0o644)

    def check_workers(self):
        source = (ROOT / "testing/test-install.sh").read_text()
        start = source.index("test_gguf_worker_exists() {")
        end = source.index("\n}\n", start) + 3
        # Redirect only the filesystem boundary; execute the real shell gate.
        function = source[start:end].replace("/usr/", f"{self.root}/usr/")
        result = subprocess.run(
            ["bash", "-c", "log_pass() { :; }; log_fail() { :; };\n"
             + function + "\ntest_gguf_worker_exists"],
            capture_output=True, text=True, env=os.environ.copy(), check=False,
        )
        return result.returncode

    def test_legacy_gpu_subdirectories_keep_workers_and_provider_libraries(self):
        source = (ROOT / "scripts/package.sh").read_text()
        start = source.index("    install_onnx_gpu_variant() {")
        end = source.index("\n    }\n", start) + 7
        function = source[start:end]
        release = self.root / "releases"
        stage = self.root / "stage"
        release.mkdir()
        lib = stage / "usr/lib/voxtype"
        lib.mkdir(parents=True)
        generic = lib / "voxtype-gguf-worker"
        generic.write_text("#!/bin/sh\nexit 0\n")
        generic.chmod(0o755)
        for variant, provider, runtime in (
            ("cuda-12", "cuda", ""),
            ("cuda-13", "cuda", "libonnxruntime.so.1.24.4"),
            ("migraphx", "migraphx", ""),
        ):
            prefix = release / ("voxtype-probe-linux-x86_64-onnx-" + variant)
            # Explicit staging fixtures, not compiled or release artifacts.
            prefix.write_bytes(b"staging executable fixture")
            for suffix in (".libonnxruntime_providers_" + provider + ".so",
                           ".libonnxruntime_providers_shared.so"):
                Path(str(prefix) + suffix).write_bytes(suffix.encode())
            if runtime:
                Path(str(prefix) + "." + runtime).write_bytes(b"runtime fixture")
            result = subprocess.run(
                ["bash", "-ec", function + '\ninstall_onnx_gpu_variant "$1" "$2" "$3"',
                 "probe", variant, provider, runtime],
                env=dict(os.environ, STAGING=str(stage), RELEASE_DIR=str(release), VERSION="probe"),
                capture_output=True, text=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            nested = lib / variant
            self.assertEqual((lib / ("voxtype-onnx-" + variant)).resolve(),
                             nested / ("voxtype-onnx-" + variant))
            self.assertTrue((nested / "voxtype-gguf-worker").is_file(), variant)
            self.assertTrue(os.access(nested / "voxtype-gguf-worker", os.X_OK), variant)
            self.assertEqual((nested / "voxtype-gguf-worker").resolve(), generic)
            self.assertEqual((nested / ("libonnxruntime_providers_" + provider + ".so")).read_bytes(),
                             (".libonnxruntime_providers_" + provider + ".so").encode())
            self.assertTrue((nested / "libonnxruntime_providers_shared.so").is_file())
            if runtime:
                self.assertEqual((nested / "libonnxruntime.so").resolve(), nested / runtime)

    def test_cpu_only_lib_layout(self):
        self.install("usr/lib/voxtype/voxtype-gguf-worker")
        self.assertEqual(self.check_workers(), 0)

    def test_single_binary_bin_layout(self):
        self.install("usr/bin/voxtype-gguf-worker")
        self.assertEqual(self.check_workers(), 0)

    def test_missing_generic_worker_is_rejected(self):
        self.assertNotEqual(self.check_workers(), 0)

    def test_vulkan_binary_requires_its_companion(self):
        self.install("usr/lib/voxtype/voxtype-gguf-worker")
        self.install("usr/lib/voxtype/voxtype-vulkan")
        self.assertNotEqual(self.check_workers(), 0)

    def test_vulkan_layout_with_both_workers(self):
        self.install("usr/lib/voxtype/voxtype-gguf-worker")
        self.install("usr/lib/voxtype/voxtype-vulkan")
        self.install("usr/lib/voxtype/voxtype-gguf-worker-vulkan")
        self.assertEqual(self.check_workers(), 0)

    def test_nonexecutable_vulkan_worker_is_rejected(self):
        self.install("usr/lib/voxtype/voxtype-gguf-worker")
        self.install("usr/lib/voxtype/voxtype-vulkan")
        self.install("usr/lib/voxtype/voxtype-gguf-worker-vulkan", executable=False)
        self.assertNotEqual(self.check_workers(), 0)

    def test_vulkan_bin_layout_requires_its_companion(self):
        self.install("usr/bin/voxtype-gguf-worker")
        self.install("usr/bin/voxtype-vulkan")
        self.assertNotEqual(self.check_workers(), 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
