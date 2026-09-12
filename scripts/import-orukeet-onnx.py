#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = ["onnx==1.22.0", "onnxruntime==1.30.0", "numpy==2.5.3"]
# ///
"""Import the pinned Orukeet r3 INT8 export for Voxtype's Parakeet backend.

Conversion is offline. No training, requantization, or runtime Python dependency
is introduced. See docs/PARAKEET.md for the archive URL and model configuration.
"""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import tarfile
import tempfile

import numpy as np
import onnx
from onnx import compose
import onnxruntime as ort


ARCHIVE_SHA256 = "f9191f30178cc9122ce2f023bf9fefafc822028307b0efa4caff645ba3fe8d0a"
REVISION = "55a984d46f68323301837194ce647c702f55facc"
ARCHIVE_ROOT = "sherpa-onnx-orukeet-v0.1.0-int8"
ARCHIVE_URL = (
    f"https://huggingface.co/oruk/orukeet/resolve/{REVISION}/onnx/"
    f"{ARCHIVE_ROOT}.tar.bz2"
)
SOURCE_FILES = {
    "encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx",
    "tokens.txt", "bpe.vocab", "LICENSE-WEIGHTS", "NOTICE.md",
}
VOCAB_SIZE = 8193  # 8192 SentencePiece tokens plus blank; five duration logits follow.


def sha256(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def extract_archive(archive, destination):
    """Copy only the seven regular files in this exact release layout."""
    with tarfile.open(archive, "r:bz2") as bundle:
        files = {}
        for member in bundle.getmembers():
            if member.name.rstrip("/") == ARCHIVE_ROOT and member.isdir():
                continue
            prefix = ARCHIVE_ROOT + "/"
            name = member.name.removeprefix(prefix)
            if (
                not member.name.startswith(prefix)
                or name not in SOURCE_FILES
                or not member.isfile()
                or name in files
            ):
                raise ValueError(f"Unexpected archive member: {member.name}")
            files[name] = member
        if files.keys() != SOURCE_FILES:
            raise ValueError("Archive does not contain the complete Orukeet r3 export")
        destination.mkdir()
        for name, member in files.items():
            with bundle.extractfile(member) as source, (destination / name).open("wb") as target:
                shutil.copyfileobj(source, target)


def validate_vocab(path):
    lines = Path(path).read_text(encoding="utf-8").splitlines()
    if len(lines) != VOCAB_SIZE:
        raise ValueError(f"Expected {VOCAB_SIZE} vocabulary entries including blank")
    for token_id, line in enumerate(lines):
        parts = line.split(" ")
        if len(parts) != 2 or not parts[0] or parts[1] != str(token_id):
            raise ValueError(f"Unexpected vocabulary format at token {token_id}")
    if lines[-1] != f"<blk> {VOCAB_SIZE - 1}":
        raise ValueError("The final vocabulary token must be blank")


def merge_decoder_joiner(decoder, joiner):
    """Compose the published graphs, preserving tensors and quantization."""
    if [v.name for v in decoder.graph.input] != [
        "targets", "target_length", "states.1", "onnx::Slice_3"
    ] or [v.name for v in decoder.graph.output] != [
        "outputs", "prednet_lengths", "states", "162"
    ]:
        raise ValueError("Unexpected Orukeet decoder interface")
    if [v.name for v in joiner.graph.input] != [
        "encoder_outputs", "decoder_outputs"
    ] or [v.name for v in joiner.graph.output] != ["outputs"]:
        raise ValueError("Unexpected Orukeet joiner interface")

    merged = compose.merge_models(
        decoder, joiner,
        io_map=[("outputs", "decoder_outputs")],
        prefix1="decoder/", prefix2="joiner/",
    )
    names = {
        "decoder/targets": "targets",
        "decoder/target_length": "target_length",
        "decoder/states.1": "input_states_1",
        "decoder/onnx::Slice_3": "input_states_2",
        "joiner/encoder_outputs": "encoder_outputs",
        "joiner/outputs": "outputs",
        "decoder/prednet_lengths": "prednet_lengths",
        "decoder/states": "output_states_1",
        "decoder/162": "output_states_2",
    }
    # This pinned export has no control-flow subgraphs. Prefixing above keeps
    # identically named internal tensors in the two source graphs separate.
    for node in merged.graph.node:
        for fields in (node.input, node.output):
            for index, name in enumerate(fields):
                fields[index] = names.get(name, name)
    for fields in (
        merged.graph.input, merged.graph.output,
        merged.graph.value_info, merged.graph.initializer,
    ):
        for value in fields:
            value.name = names.get(value.name, value.name)
    merged.producer_name = "voxtype-orukeet-import"
    merged.producer_version = "1"
    merged.doc_string = (
        "Orukeet r3 INT8 decoder and joiner composed for parakeet-rs. "
        "No weight changes. Retain LICENSE-WEIGHTS and NOTICE.md (CC BY-SA 4.0)."
    )
    onnx.checker.check_model(merged)
    return merged


def verify_combination(decoder_path, joiner_path, combined_path):
    """Check logits, greedy decisions, and recurrent state on CPU (no audio)."""
    options = ort.SessionOptions()
    options.intra_op_num_threads = 1
    options.inter_op_num_threads = 1
    options.log_severity_level = 3
    sessions = [
        ort.InferenceSession(str(path), options, providers=["CPUExecutionProvider"])
        for path in (decoder_path, joiner_path, combined_path)
    ]
    decoder, joiner, combined = sessions
    rng = np.random.default_rng(0)
    h = rng.normal(0, 0.1, (2, 1, 640)).astype(np.float32)
    c = rng.normal(0, 0.1, (2, 1, 640)).astype(np.float32)
    max_abs_error = 0.0
    for step in range(8):
        # Include blank initialization as well as normal prediction steps.
        token = VOCAB_SIZE - 1 if step == 0 else int(rng.integers(VOCAB_SIZE - 1))
        targets = np.array([[token]], dtype=np.int32)
        length = np.array([1], dtype=np.int32)
        frame = rng.normal(0, 0.1, (1, 1024, 1)).astype(np.float32)
        predicted, pred_length, next_h, next_c = decoder.run(
            ["outputs", "prednet_lengths", "states", "162"],
            {"targets": targets, "target_length": length,
             "states.1": h, "onnx::Slice_3": c},
        )
        expected = joiner.run(
            ["outputs"], {"encoder_outputs": frame, "decoder_outputs": predicted}
        )[0]
        logits, actual_length, actual_h, actual_c = combined.run(
            ["outputs", "prednet_lengths", "output_states_1", "output_states_2"],
            {"encoder_outputs": frame, "targets": targets, "target_length": length,
             "input_states_1": h, "input_states_2": c},
        )
        for actual, reference in (
            (logits, expected), (actual_h, next_h), (actual_c, next_c)
        ):
            np.testing.assert_allclose(actual, reference, rtol=1e-5, atol=1e-5)
            max_abs_error = max(max_abs_error, float(np.max(np.abs(actual - reference))))
        np.testing.assert_array_equal(actual_length, pred_length)
        for region in (slice(None, VOCAB_SIZE), slice(VOCAB_SIZE, None)):
            np.testing.assert_array_equal(
                np.argmax(logits[..., region], axis=-1),
                np.argmax(expected[..., region], axis=-1),
            )
        h, c = next_h, next_c
    return {"steps": 8, "max_abs_error": max_abs_error, "provider": "CPUExecutionProvider"}


def import_archive(archive, output):
    archive, output = Path(archive), Path(output)
    if output.exists() or output.is_symlink():
        raise FileExistsError(f"Refusing to overwrite existing output: {output}")
    if sha256(archive) != ARCHIVE_SHA256:
        raise ValueError("Archive SHA256 mismatch; use the pinned release in docs/PARAKEET.md")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".orukeet-import-", dir=output.parent) as temporary:
        root = Path(temporary)
        source, converted = root / "source", root / "converted"
        extract_archive(archive, source)
        validate_vocab(source / "tokens.txt")
        converted.mkdir()
        decoder_path, joiner_path = source / "decoder.int8.onnx", source / "joiner.int8.onnx"
        merged = merge_decoder_joiner(onnx.load(decoder_path), onnx.load(joiner_path))
        combined_path = converted / "decoder_joint-model.int8.onnx"
        onnx.save_model(merged, combined_path)
        parity = verify_combination(decoder_path, joiner_path, combined_path)
        for original, destination in (
            ("encoder.int8.onnx", "encoder-model.int8.onnx"),
            ("tokens.txt", "vocab.txt"),
            ("bpe.vocab", "bpe.vocab"),
            ("LICENSE-WEIGHTS", "LICENSE-WEIGHTS"),
            ("NOTICE.md", "NOTICE.md"),
        ):
            shutil.copyfile(source / original, converted / destination)
        manifest = {
            "schema_version": 1,
            "model": "orukeet-r3-int8",
            "source_url": ARCHIVE_URL,
            "archive_sha256": ARCHIVE_SHA256,
            "weight_license": "CC-BY-SA-4.0",
            "conversion": "Encoder and vocabulary copied; decoder/joiner composed, I/O renamed; no requantization.",
            "tools": {"onnx": onnx.__version__, "onnxruntime": ort.__version__, "numpy": np.__version__},
            "cpu_graph_parity": parity,
            "files": {path.name: sha256(path) for path in sorted(converted.iterdir())},
        }
        (converted / "VOXTYPE-CONVERSION.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        # Staging beside the destination avoids exposing a partially converted
        # model. Do not run concurrent imports to the same output directory.
        if output.exists() or output.is_symlink():
            raise FileExistsError(f"Output appeared during import: {output}")
        converted.rename(output)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True, help="Pinned Orukeet INT8 .tar.bz2")
    parser.add_argument("--output", type=Path, required=True, help="New model directory; never overwritten")
    args = parser.parse_args()
    try:
        manifest = import_archive(args.archive, args.output)
    except (OSError, ValueError, tarfile.TarError, AssertionError) as error:
        parser.exit(1, f"Import failed: {error}\n")
    print(f"Imported Orukeet to {args.output}")
    print(json.dumps(manifest["cpu_graph_parity"], sort_keys=True))


if __name__ == "__main__":
    main()
