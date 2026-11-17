#!/usr/bin/env python3
"""Exercise the install-test worker gate in isolated package layouts.

No root, installed Voxtype, model downloads, or native compilation required.
Run: python3 testing/test-gguf-worker-packaging.py
"""

from pathlib import Path
import os
import re
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

    def package_function(self, signature):
        source = (ROOT / "scripts/package.sh").read_text()
        start = source.index("    " + signature + "() {")
        end = source.index("\n    }\n", start) + 7
        return source[start:end]

    def stage(self, script, *args):
        functions = (self.package_function("install_gguf_worker") + "\n"
                     + self.package_function("install_onnx_gpu_variant"))
        return subprocess.run(
            ["bash", "-ec", functions + "\n" + script, "probe", *args],
            env=dict(os.environ, STAGING=str(self.root / "stage"),
                     RELEASE_DIR=str(self.root / "releases"), VERSION="probe"),
            capture_output=True, text=True, check=False,
        )

    def release_worker(self, variant):
        release = self.root / "releases"
        release.mkdir(exist_ok=True)
        worker = release / ("voxtype-probe-linux-x86_64-" + variant + ".gguf-worker")
        # Explicit staging fixture, not a compiled or release artifact.
        worker.write_bytes(("worker fixture " + variant).encode())
        return worker

    def test_top_level_variants_install_their_own_workers(self):
        lib = self.root / "stage/usr/lib/voxtype"
        for variant in ("avx2", "avx512", "vulkan", "onnx-avx2", "onnx-avx512"):
            self.release_worker(variant)
            result = self.stage('install_gguf_worker "$1" "$STAGING/usr/lib/voxtype"', variant)
            self.assertEqual(result.returncode, 0, result.stderr)
            installed = lib / ("voxtype-gguf-worker-" + variant)
            self.assertEqual(installed.read_bytes(), ("worker fixture " + variant).encode())
            self.assertTrue(os.access(installed, os.X_OK), variant)

    def test_gpu_subdirectories_keep_workers_and_provider_libraries(self):
        release = self.root / "releases"
        release.mkdir()
        lib = self.root / "stage/usr/lib/voxtype"
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
            self.release_worker("onnx-" + variant)
            for suffix in (".libonnxruntime_providers_" + provider + ".so",
                           ".libonnxruntime_providers_shared.so"):
                Path(str(prefix) + suffix).write_bytes(suffix.encode())
            if runtime:
                Path(str(prefix) + "." + runtime).write_bytes(b"runtime fixture")
            result = self.stage('install_onnx_gpu_variant "$1" "$2" "$3"', variant, provider, runtime)
            self.assertEqual(result.returncode, 0, result.stderr)
            nested = lib / variant
            self.assertEqual((lib / ("voxtype-onnx-" + variant)).resolve(),
                             nested / ("voxtype-onnx-" + variant))
            # The variant's own worker sits beside the real binary, where the
            # application looks first; the generic CPU worker stays reachable.
            own = nested / ("voxtype-gguf-worker-onnx-" + variant)
            self.assertEqual(own.read_bytes(), ("worker fixture onnx-" + variant).encode())
            self.assertTrue(os.access(own, os.X_OK), variant)
            self.assertTrue((nested / "voxtype-gguf-worker").is_file(), variant)
            self.assertTrue(os.access(nested / "voxtype-gguf-worker", os.X_OK), variant)
            self.assertEqual((nested / "voxtype-gguf-worker").resolve(), generic)
            self.assertEqual((nested / ("libonnxruntime_providers_" + provider + ".so")).read_bytes(),
                             (".libonnxruntime_providers_" + provider + ".so").encode())
            self.assertTrue((nested / "libonnxruntime_providers_shared.so").is_file())
            if runtime:
                self.assertEqual((nested / "libonnxruntime.so").resolve(), nested / runtime)

    def test_every_dockerfile_ships_the_gguf_client_and_matching_worker(self):
        # Vulkan-capable workers on x86_64 vulkan and ONNX images; CPU workers
        # on the 22.04 Whisper CPU tiers (no glslc, CMake < 3.24) and aarch64.
        vulkan_workers = {"Dockerfile.vulkan", "Dockerfile.onnx", "Dockerfile.onnx-avx512",
                          "Dockerfile.onnx-cuda-12", "Dockerfile.onnx-cuda-13",
                          "Dockerfile.onnx-migraphx"}
        cpu_workers = {"Dockerfile.build", "Dockerfile.avx512", "Dockerfile.baseline",
                       "Dockerfile.aarch64-cpu", "Dockerfile.aarch64-onnx"}
        for name in sorted(vulkan_workers | cpu_workers):
            with self.subTest(name):
                text = (ROOT / name).read_text().replace("\\\n", " ")
                # One cargo invocation per segment, up to its `&&` or line end.
                builds = [part.split("&&")[0].split("\n")[0]
                          for part in text.split("cargo build --release")[1:]]
                builds = [b for b in builds if "--bin voxtype-osd" not in b]
                app = [line for line in builds if "-p voxtype-gguf-worker" not in line]
                worker = [line for line in builds if "-p voxtype-gguf-worker" in line]
                self.assertTrue(app and worker, name)
                features = app[0].split("--features", 1)[1].split()[0] if "--features" in app[0] else ""
                features = set(features.split(","))
                self.assertTrue({"gguf", "cohere-gguf", "parakeet-gguf"} <= features
                                or "gpu-vulkan" in features, (name, features))
                wants_vulkan = name in vulkan_workers
                self.assertEqual("--features vulkan" in worker[0], wants_vulkan, name)
                self.assertEqual("glslc" in text and "libvulkan-dev" in text, wants_vulkan, name)
                gate = re.search(r"verify-gguf-companion\.sh((?:\s+--[\w-]+)+)", text)
                self.assertIsNotNone(gate, name)
                self.assertIn("--vulkan" if wants_vulkan else "--cpu-only", gate.group(1).split(), name)

    def test_verify_script_rejects_a_missing_worker(self):
        result = subprocess.run(
            ["bash", str(ROOT / "scripts/verify-gguf-companion.sh"), "--cpu-only",
             str(self.root / "voxtype"), str(self.root / "voxtype.gguf-worker")],
            capture_output=True, text=True, check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing", result.stderr)

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

    def test_onnx_binaries_require_their_own_workers(self):
        self.install("usr/lib/voxtype/voxtype-gguf-worker")
        self.install("usr/lib/voxtype/voxtype-onnx-avx2")
        self.assertNotEqual(self.check_workers(), 0)
        self.install("usr/lib/voxtype/voxtype-gguf-worker-onnx-avx2")
        self.assertEqual(self.check_workers(), 0)

    def test_gpu_subdirectory_binary_requires_nested_worker(self):
        self.install("usr/lib/voxtype/voxtype-gguf-worker")
        self.install("usr/lib/voxtype/cuda-12/voxtype-onnx-cuda-12")
        top = self.root / "usr/lib/voxtype/voxtype-onnx-cuda-12"
        top.symlink_to("cuda-12/voxtype-onnx-cuda-12")
        self.assertNotEqual(self.check_workers(), 0)
        self.install("usr/lib/voxtype/cuda-12/voxtype-gguf-worker-onnx-cuda-12")
        self.assertEqual(self.check_workers(), 0)

    def test_cpu_tier_binaries_use_the_generic_worker(self):
        self.install("usr/lib/voxtype/voxtype-gguf-worker")
        for name in ("baseline", "avx2", "avx512"):
            self.install("usr/lib/voxtype/voxtype-" + name)
        self.assertEqual(self.check_workers(), 0)

    def test_vulkan_bin_layout_requires_its_companion(self):
        self.install("usr/bin/voxtype-gguf-worker")
        self.install("usr/bin/voxtype-vulkan")
        self.assertNotEqual(self.check_workers(), 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
