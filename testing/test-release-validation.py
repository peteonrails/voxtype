#!/usr/bin/env python3
"""Exercise release validation without compiling or publishing a release."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
manifest_version = re.search(r'^version = "([^"]+)"$', (ROOT / "Cargo.toml").read_text(), re.M)
assert manifest_version is not None, "Cargo.toml must declare the release version"
VERSION = manifest_version.group(1)
WRONG_VERSION = VERSION + "-wrong"


class ReleaseValidationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.release = self.root / "releases" / VERSION
        self.release.mkdir(parents=True)

    def binary(self, suffix="avx2", output=None, status=0, executable=True, stderr=False):
        path = self.release / f"voxtype-{VERSION}-linux-x86_64-{suffix}"
        if output is None:
            output = f"voxtype {VERSION}"
        path.write_text(f"#!/bin/sh\nprintf '%s\\n' '{output}'" +
                        (" >&2" if stderr else "") + f"\nexit {status}\n")
        path.chmod(0o755 if executable else 0o644)
        return path

    def check(self, path):
        source = (ROOT / "scripts/validate-release.sh").read_text()
        start = source.index("# 2. Check version strings")
        end = source.index("# 3. Check all binaries", start)
        # Execute the real version gate; the unrelated ISA/glibc checks need ELF files.
        script = ('set -e\nVERSION="$1"; BASE_VERSION="${VERSION%%-*}"; '
                  'RELEASE_DIR="$2"; SCRIPT_DIR="$3"; FAILED=false\n'
                  'WHISPER_BINARIES=("$4"); ONNX_BINARIES=(); COMPANION_BINARIES=()\n' +
                  source[start:end] + '\n[[ "$FAILED" == false ]]\n')
        return subprocess.run(["bash", "-c", script, "fixture", VERSION,
                               str(self.release), str(ROOT / "scripts"), path.name],
                              capture_output=True, text=True)

    def test_collection_restores_only_binary_exec_bits(self):
        binary = self.binary(executable=False)
        worker = Path(str(binary) + ".gguf-worker")
        untouched = [Path(str(binary) + suffix) for suffix in
                     (".asc", ".libonnxruntime.so", ".version-proof.json", ".tar.gz")]
        for path in [worker] + untouched:
            path.write_text("artifact fixture\n")
            path.chmod(0o644)
        artifacts = self.root / "artifacts" / "matrix-job"
        artifacts.mkdir(parents=True)
        for path in list(self.release.iterdir()):
            path.rename(artifacts / path.name)
        workflow = (ROOT / ".github/workflows/build-linux.yml").read_text()
        collect = workflow.split("      - name: Collect binaries\n", 1)[1].split("\n      - name:", 1)[0]
        script = "\n".join(line[10:] for line in collect.splitlines()[1:])
        script = script.replace("${{ steps.version.outputs.version }}", VERSION)
        # Run the real collection body with only its script location redirected.
        script = script.replace("python3 scripts/", "python3 " + str(ROOT / "scripts") + "/")
        result = subprocess.run(["bash", "-c", script], cwd=self.root, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for path in (binary, worker):
            self.assertTrue(os.access(path, os.X_OK), str(path))
        for path in untouched:
            self.assertEqual(path.stat().st_mode & 0o777, 0o644, str(path))
        self.assertEqual(self.check(binary).returncode, 0)

    def test_cross_and_gtk_builds_produce_version_proofs(self):
        for dockerfile in ("Dockerfile.avx512", "Dockerfile.onnx-avx512"):
            with self.subTest(dockerfile=dockerfile):
                source = (ROOT / dockerfile).read_text()
                self.assertIn("release-artifacts.py record-cross", source)
                self.assertIn('--source /build', source)
        self.assertIn("release-artifacts.py record-runtime", (ROOT / "Dockerfile.onnx").read_text())

    def artifact_command(self, action, path, *extra):
        return subprocess.run(["python3", str(ROOT / "scripts/release-artifacts.py"),
                               action, str(path), VERSION, *extra], capture_output=True, text=True)

    def test_cross_version_proof_is_exact_hash_bound_and_not_execution(self):
        path = self.binary(suffix="avx512", status=99)
        self.assertNotEqual(self.check(path).returncode, 0, "missing proof must fail closed")
        source = self.root / "source"
        source.mkdir()
        (source / "Cargo.toml").write_bytes((ROOT / "Cargo.toml").read_bytes())
        (source / "Cargo.lock").write_bytes((ROOT / "Cargo.lock").read_bytes())
        result = self.artifact_command("record-cross", path, "--source", str(source))
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.check(path)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("NOT executed", result.stdout)
        proof_path = Path(str(path) + ".version-proof.json")
        import json
        proof = json.loads(proof_path.read_text())
        self.assertFalse(proof["executed"])
        original_manifest_hash = proof["cargo_toml_sha256"]
        proof["cargo_toml_sha256"] = "0" * 64
        proof_path.write_text(json.dumps(proof))
        self.assertNotEqual(self.check(path).returncode, 0, "wrong build-source metadata must fail")
        proof["cargo_toml_sha256"] = original_manifest_hash
        proof["version"] = WRONG_VERSION
        proof_path.write_text(json.dumps(proof))
        self.assertNotEqual(self.check(path).returncode, 0)
        self.assertEqual(self.artifact_command("record-cross", path, "--source", str(source)).returncode, 0)
        path.write_text(path.read_text() + "# changed artifact\n")
        self.assertNotEqual(self.check(path).returncode, 0)
        (source / "Cargo.toml").write_text(f'[package]\nversion = "{WRONG_VERSION}"\n')
        self.assertNotEqual(self.artifact_command("record-cross", path, "--source", str(source)).returncode, 0)

    def test_gtk_build_proof_requires_successful_exact_stdout(self):
        path = self.binary(suffix="osd-gtk4", output=f"voxtype-osd-gtk4 {VERSION}", status=1)
        self.assertNotEqual(self.artifact_command("record-runtime", path).returncode, 0)
        self.assertFalse(Path(str(path) + ".version-proof.json").exists())
        self.binary(suffix="osd-gtk4", output=f"voxtype-osd-gtk4 {VERSION}")
        result = self.artifact_command("record-runtime", path)
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.check(path)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("build-stage runtime", result.stdout)
        path.write_text(path.read_text() + "# changed helper\n")
        self.assertNotEqual(self.check(path).returncode, 0)

    def test_gtk_version_probe_suppresses_existing_stdout_logs(self):
        path = self.binary(suffix="osd-gtk4")
        # Mirror the real helper's EnvFilter default INFO log before Args::parse.
        path.write_text('#!/bin/sh\n'
                        'if [ "${RUST_LOG:-info}" != off ]; then\n'
                        '  printf "GTK 4.14 predates the gtk#8366 fix\\n"\n'
                        'fi\n'
                        f'printf "voxtype-osd-gtk4 {VERSION}\\n"\n')
        for action in ("check", "record-runtime"):
            with self.subTest(action=action):
                result = self.artifact_command(action, path)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        # Do not weaken exact stdout matching by discarding arbitrary lines.
        self.binary(suffix="osd-gtk4", output=f"unconditional noise\nvoxtype-osd-gtk4 {VERSION}")
        self.assertNotEqual(self.artifact_command("record-runtime", path).returncode, 0)

    def test_aarch64_cpu_version_check_is_preserved(self):
        path = self.binary(suffix="cpu")
        path = path.rename(Path(str(path).replace("x86_64", "aarch64")))
        result = self.check(path)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_nonexecutable_versioned_path_is_rejected(self):
        result = self.check(self.binary(executable=False))
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_failed_command_cannot_supply_version_on_stderr(self):
        result = self.check(self.binary(output=f"cannot load releases/{VERSION}/voxtype", status=127, stderr=True))
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_failed_command_cannot_supply_valid_stdout(self):
        result = self.check(self.binary(status=1))
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_wrong_prerelease_and_stable_are_rejected(self):
        versions = [WRONG_VERSION, VERSION + "+gstale"]
        if "-" in VERSION:
            versions.append(VERSION.split("-", 1)[0])
        for version in versions:
            with self.subTest(version=version):
                result = self.check(self.binary(output=f"voxtype {version}"))
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_successful_exact_version(self):
        result = self.check(self.binary())
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_helper_stdout_shape(self):
        for suffix in ("osd", "osd-gtk4", "osd-quickshell", "audio-bridge"):
            with self.subTest(suffix=suffix):
                result = self.check(self.binary(suffix=suffix, output=f"voxtype-{suffix} {VERSION}"))
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                result = self.check(self.binary(suffix=suffix, output=f"wrong-program {VERSION}"))
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
