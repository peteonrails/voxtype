#!/usr/bin/env python3
"""Restore artifact modes and validate exact release versions, without inference."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

VARIANTS = ("baseline", "cpu", "avx2", "avx512", "vulkan", "onnx", "onnx-avx2",
            "onnx-avx512", "onnx-cuda-12", "onnx-cuda-13", "onnx-migraphx")
HELPERS = ("osd", "osd-gtk4", "osd-quickshell", "audio-bridge")
CROSS = ("avx512", "onnx-avx512")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.sha256(stream.read()).hexdigest()


def identity(path, version):
    match = re.fullmatch(r"voxtype-" + re.escape(version) +
                         r"-linux-(x86_64|aarch64)-(.+)", path.name)
    if not match:
        raise ValueError("not a release binary: " + path.name)
    suffix = match.group(2)
    if suffix not in VARIANTS + HELPERS:
        raise ValueError("unknown release binary: " + path.name)
    return suffix, "voxtype-" + suffix if suffix in HELPERS else "voxtype"


def runtime_stdout(path, program, version):
    if not os.access(path, os.X_OK):
        raise ValueError("binary is not executable: " + str(path))
    # Never parse stderr or accept output from a failed process.
    result = subprocess.run([str(path.resolve()), "--version"], capture_output=True,
                            text=True, check=True, timeout=30)
    expected = program + " " + version + "\n"
    if result.stdout != expected:
        raise ValueError("unexpected --version stdout: " + repr(result.stdout) +
                         "; expected " + repr(expected))
    return result.stdout


def proof_path(path):
    return Path(str(path) + ".version-proof.json")


def record(path, version, kind, source):
    suffix, program = identity(path, version)
    proof = {"version": version, "binary": path.name, "sha256": digest(path), "kind": kind}
    if kind == "cross-compiled-manifest":
        if suffix not in CROSS or source is None:
            raise ValueError("compile metadata is only allowed for AVX-512 binaries")
        # Docker copies a git-free source context. build.rs therefore falls back
        # to CARGO_PKG_VERSION, not the developer's untagged +g build stamp.
        if (source / ".git").exists():
            raise ValueError("cross-version proof requires a git-free build context")
        manifest = (source / "Cargo.toml").read_text()
        lock = (source / "Cargo.lock").read_text()
        compiled = re.search(r'^version = "([^"]+)"$', manifest, re.M)
        locked = re.search(r'\[\[package\]\]\nname = "voxtype"\nversion = "([^"]+)"', lock)
        if not compiled or not locked or compiled.group(1) != version or locked.group(1) != version:
            raise ValueError("build manifest/lock version does not match release")
        if version.encode() not in path.read_bytes():
            raise ValueError("exact compiled version is absent from binary")
        proof.update(cargo_toml_sha256=digest(source / "Cargo.toml"),
                     cargo_lock_sha256=digest(source / "Cargo.lock"), executed=False)
    else:
        if suffix != "osd-gtk4":
            raise ValueError("runtime proof is only needed for the GTK-linked helper")
        proof.update(stdout=runtime_stdout(path, program, version), executed=True)
    proof_path(path).write_text(json.dumps(proof, indent=2) + "\n")


def check(path, version):
    suffix, program = identity(path, version)
    if not os.access(path, os.X_OK):
        raise ValueError("binary is not executable: " + str(path))
    if suffix in CROSS or suffix == "osd-gtk4" and proof_path(path).is_file():
        proof = json.loads(proof_path(path).read_text())
        expected_kind = "cross-compiled-manifest" if suffix in CROSS else "runtime"
        if any(proof.get(k) != v for k, v in {
            "version": version, "binary": path.name, "sha256": digest(path),
            "kind": expected_kind, "executed": suffix not in CROSS,
        }.items()):
            raise ValueError("stale or invalid build-stage version proof")
        if suffix not in CROSS and proof.get("stdout") != program + " " + version + "\n":
            raise ValueError("build-stage stdout does not match release")
        if suffix in CROSS:
            source = Path(__file__).resolve().parents[1]
            if (proof.get("cargo_toml_sha256") != digest(source / "Cargo.toml") or
                    proof.get("cargo_lock_sha256") != digest(source / "Cargo.lock")):
                raise ValueError("build manifest hashes do not match release source")
        print(path.name + ": exact " + version + " via build-stage " + expected_kind +
              (" (NOT executed; ISA checks are separate)" if suffix in CROSS else ""))
    else:
        runtime_stdout(path, program, version)
        print(path.name + ": exact " + version + " (executed)")


def restore(directory, version):
    for path in directory.iterdir():
        if not path.is_file() or path.is_symlink():
            continue
        candidate = Path(str(path).removesuffix(".gguf-worker"))
        try:
            suffix, _ = identity(candidate, version)
        except ValueError:
            continue
        if path.name.endswith(".gguf-worker") and suffix not in VARIANTS:
            continue
        path.chmod(path.stat().st_mode | 0o111)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "restore", "record-cross", "record-runtime"))
    parser.add_argument("path", type=Path)
    parser.add_argument("version")
    parser.add_argument("--source", type=Path)
    args = parser.parse_args()
    try:
        if args.action == "restore":
            restore(args.path, args.version)
        elif args.action == "check":
            check(args.path, args.version)
        else:
            record(args.path, args.version,
                   "cross-compiled-manifest" if args.action == "record-cross" else "runtime", args.source)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print("Release version validation failed: " + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
