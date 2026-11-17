#!/usr/bin/env python3
"""Test the real Voxtype executable's worker routing and init diagnostics.

Build with the release GGUF client (--features gguf,cohere-gguf,parakeet-gguf,
plus the ONNX engines for the ONNX-vs-GGUF model-name checks), then run:
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
        # Catalog names resolve against $XDG_DATA_HOME/voxtype/models; keep
        # that inside the temp tree too.
        self.models = self.root / "xdg-data/voxtype/models"
        self.models.mkdir(parents=True)
        self.env["XDG_DATA_HOME"] = str(self.root / "xdg-data")
        self.env["XDG_CONFIG_HOME"] = str(self.root / "xdg-config")

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

    def transcribe(self, executable, config_text):
        self.config.write_text(config_text)
        result = subprocess.run(
            [str(executable), "--config", str(self.config), "transcribe",
             str(ROOT / "tests/fixtures/vad/speech_hello.wav")],
            env=self.env, capture_output=True, text=True, timeout=20, check=False,
        )
        return result.returncode, result.stdout + result.stderr

    def test_every_release_variant_prefers_its_own_worker(self):
        # Top-level package layout: each variant's worker sits beside it.
        for variant in ("avx2", "avx512", "vulkan", "onnx-avx2", "onnx-avx512"):
            with self.subTest(variant):
                executable = self.executable("voxtype-" + variant)
                self.worker("voxtype-gguf-worker")
                self.check_routing(executable, self.worker("voxtype-gguf-worker-" + variant))

    def test_baseline_uses_generic_worker(self):
        executable = self.executable("voxtype-baseline")
        self.worker("voxtype-gguf-worker-avx2")
        self.check_routing(executable, self.worker("voxtype-gguf-worker"))

    def test_gpu_subdirectory_variants_use_nested_variant_worker(self):
        assert BINARY is not None
        generic = self.worker("voxtype-gguf-worker")
        for subdir, alias in (("cuda-12", "voxtype-onnx-cuda"), ("cuda-13", None),
                              ("migraphx", "voxtype-onnx-rocm")):
            with self.subTest(subdir):
                nested = self.lib / subdir
                nested.mkdir()
                executable = nested / ("voxtype-onnx-" + subdir)
                shutil.copy2(BINARY, executable)
                (nested / "voxtype-gguf-worker").symlink_to("../" + generic.name)
                worker = nested / ("voxtype-gguf-worker-onnx-" + subdir)
                worker.write_text(
                    '#!/usr/bin/env python3\nimport json\nprint(json.dumps('
                    '{"ok": False, "kind": "init_failed", "error": "routing-probe:'
                    + worker.name + '"}))\n')
                worker.chmod(0o755)
                top = self.lib / ("voxtype-onnx-" + subdir)
                top.symlink_to(subdir + "/voxtype-onnx-" + subdir)
                entries = [executable, top]
                if alias:
                    (self.lib / alias).symlink_to(top.name)
                    entries.append(self.lib / alias)
                link = self.bin / ("voxtype-" + subdir)
                link.symlink_to(top)
                entries.append(link)
                for entry in entries:
                    self.check_routing(entry, worker)

    def test_model_names_route_between_onnx_and_gguf(self):
        executable = self.executable("voxtype-onnx-avx2")
        self.worker("voxtype-gguf-worker")
        worker = self.worker("voxtype-gguf-worker-onnx-avx2")
        for engine, gguf, onnx in (
            ("parakeet", "parakeet-ultra-Q8_0.gguf", "parakeet-tdt-0.6b-v3"),
            ("cohere", "cohere-transcribe-03-2026-Q4_K_M.gguf", "cohere-transcribe-q4f16"),
        ):
            with self.subTest(engine=engine, model=gguf):
                (self.models / gguf).touch()
                code, output = self.transcribe(
                    executable, f'engine = "{engine}"\n[{engine}]\nmodel = "{gguf}"\n')
                self.assertNotEqual(code, 0, output)
                self.assertIn("routing-probe:" + worker.name, output)
            with self.subTest(engine=engine, model=onnx):
                # An existing explicit ONNX config keeps running ONNX: the
                # worker is never started, whatever the build's GGUF default.
                code, output = self.transcribe(
                    executable, f'engine = "{engine}"\n[{engine}]\nmodel = "{onnx}"\n')
                self.assertNotEqual(code, 0, output)
                self.assertNotIn("routing-probe:", output)
                self.assertNotIn("gguf-worker", output)
                # ONNX builds name the missing model; GGUF-only builds name
                # the ONNX feature they lack. Neither reaches the worker.
                self.assertTrue(onnx in output or "--features" in output, output)

    def test_explicit_onnx_model_survives_gguf_default(self):
        executable = self.executable("voxtype-onnx-avx2")
        for engine, onnx in (("parakeet", "parakeet-tdt-0.6b-v3"),
                             ("cohere", "cohere-transcribe-q4f16")):
            with self.subTest(engine):
                self.config.write_text(f'engine = "{engine}"\n[{engine}]\nmodel = "{onnx}"\n')
                result = subprocess.run(
                    [str(executable), "--config", str(self.config), "config", "get",
                     engine + ".model"],
                    env=self.env, capture_output=True, text=True, timeout=20, check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), onnx)

    def test_info_engines_reports_gguf_compiled(self):
        result = subprocess.run(
            [str(BINARY), "info", "engines", "--json"],
            env=self.env, capture_output=True, text=True, timeout=20, check=True,
        )
        compiled = {e["name"]: e["compiled"] for e in json.loads(result.stdout)}
        for engine in ("gguf", "parakeet", "cohere"):
            self.assertTrue(compiled.get(engine), (engine, compiled))

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
