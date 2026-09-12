"""Small CPU tests for the importer; no downloaded model or audio required."""

import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock

import numpy as np
import onnx
from onnx import TensorProto as T, helper as H, numpy_helper as N

spec = importlib.util.spec_from_file_location(
    "import_orukeet_onnx", Path(__file__).resolve().parents[1] / "import-orukeet-onnx.py"
)
importer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(importer)


def value(name, dtype, shape):
    return H.make_tensor_value_info(name, dtype, shape)


def models():
    # Deliberately reuse initializer/node names between graphs to exercise
    # namespace isolation, and distinguish hidden-state and cell-state outputs.
    decoder = H.make_model(H.make_graph([
        H.make_node("ReduceSum", ["states.1", "axes"], ["pooled"], keepdims=0),
        H.make_node("Cast", ["targets"], ["float_targets"], to=T.FLOAT),
        H.make_node("Add", ["pooled", "float_targets"], ["prediction"]),
        H.make_node("Unsqueeze", ["prediction", "output_axes"], ["outputs"]),
        H.make_node("Identity", ["target_length"], ["prednet_lengths"]),
        H.make_node("Add", ["states.1", "one"], ["states"]),
        H.make_node("Add", ["onnx::Slice_3", "two"], ["162"]),
    ], "decoder", [
        value("targets", T.INT32, [1, 1]), value("target_length", T.INT32, [1]),
        value("states.1", T.FLOAT, [2, 1, 640]), value("onnx::Slice_3", T.FLOAT, [2, 1, 640]),
    ], [
        value("outputs", T.FLOAT, [1, 640, 1]), value("prednet_lengths", T.INT32, [1]),
        value("states", T.FLOAT, [2, 1, 640]), value("162", T.FLOAT, [2, 1, 640]),
    ], [
        N.from_array(np.array([0], dtype=np.int64), "axes"),
        N.from_array(np.array([2], dtype=np.int64), "output_axes"),
        N.from_array(np.array(1, dtype=np.float32), "one"),
        N.from_array(np.array(2, dtype=np.float32), "two"),
    ]), opset_imports=[H.make_opsetid("", 17)], ir_version=8)
    joiner = H.make_model(H.make_graph([
        H.make_node("ReduceSum", ["encoder_outputs", "axes"], ["pooled"], keepdims=0),
        H.make_node("ReduceSum", ["decoder_outputs", "axes"], ["decoded"], keepdims=0),
        H.make_node("Add", ["pooled", "decoded"], ["sum"]),
        H.make_node("Unsqueeze", ["sum", "output_axes"], ["expanded"]),
        H.make_node("Add", ["expanded", "bias"], ["outputs"]),
    ], "joiner", [
        value("encoder_outputs", T.FLOAT, [1, 1024, 1]),
        value("decoder_outputs", T.FLOAT, [1, 640, 1]),
    ], [value("outputs", T.FLOAT, [1, 1, 1, importer.VOCAB_SIZE + 5])], [
        N.from_array(np.array([1], dtype=np.int64), "axes"),
        N.from_array(np.array([2, 3], dtype=np.int64), "output_axes"),
        N.from_array(np.linspace(-1, 1, importer.VOCAB_SIZE + 5, dtype=np.float32)
                     .reshape(1, 1, 1, -1), "bias"),
    ]), opset_imports=[H.make_opsetid("", 17)], ir_version=8)
    return decoder, joiner


def write_archive(path, members):
    """Build the release layout from named byte strings, including duplicates."""
    with tarfile.open(path, "w:bz2") as bundle:
        root = tarfile.TarInfo(importer.ARCHIVE_ROOT)
        root.type = tarfile.DIRTYPE
        bundle.addfile(root)
        for name, content in members:
            member = tarfile.TarInfo(f"{importer.ARCHIVE_ROOT}/{name}")
            member.size = len(content)
            bundle.addfile(member, io.BytesIO(content))


def synthetic_archive(directory):
    """Exercise the full import without downloading weights or using audio."""
    decoder, joiner = models()
    tokens = "".join(f"token{i} {i}\n" for i in range(importer.VOCAB_SIZE - 1))
    files = {
        "encoder.int8.onnx": b"synthetic encoder copied without modification",
        "decoder.int8.onnx": decoder.SerializeToString(),
        "joiner.int8.onnx": joiner.SerializeToString(),
        "tokens.txt": (tokens + f"<blk> {importer.VOCAB_SIZE - 1}\n").encode(),
        "bpe.vocab": b"synthetic vocabulary\n",
        "LICENSE-WEIGHTS": b"synthetic license notice\n",
        "NOTICE.md": b"synthetic attribution notice\n",
    }
    archive = directory / "model.tar.bz2"
    write_archive(archive, files.items())
    return archive, files


class ImportTests(unittest.TestCase):
    def test_composition_preserves_logits_lengths_and_states_on_cpu(self):
        decoder, joiner = models()
        combined = importer.merge_decoder_joiner(decoder, joiner)
        self.assertEqual({x.name for x in combined.graph.input}, {
            "encoder_outputs", "targets", "target_length", "input_states_1", "input_states_2",
        })
        with tempfile.TemporaryDirectory() as temporary:
            paths = [Path(temporary) / name for name in ("decoder.onnx", "joiner.onnx", "combined.onnx")]
            for model, path in zip((decoder, joiner, combined), paths):
                onnx.save_model(model, path)
            result = importer.verify_combination(*paths)
            self.assertEqual(result["steps"], 8)
            self.assertEqual(result["max_abs_error"], 0.0)

    def test_rejects_unknown_decoder_layout(self):
        decoder, joiner = models()
        decoder.graph.input[2].name = "unknown_state"
        with self.assertRaisesRegex(ValueError, "decoder interface"):
            importer.merge_decoder_joiner(decoder, joiner)

    def test_rejects_unknown_joiner_layout(self):
        decoder, joiner = models()
        joiner.graph.input[1].name = "unknown_prediction"
        with self.assertRaisesRegex(ValueError, "joiner interface"):
            importer.merge_decoder_joiner(decoder, joiner)

    def test_parity_rejects_changed_recurrent_state(self):
        decoder, joiner = models()
        combined = importer.merge_decoder_joiner(decoder, joiner)
        state_increment = next(
            value for value in combined.graph.initializer if value.name == "decoder/one"
        )
        state_increment.CopyFrom(N.from_array(np.array(2, dtype=np.float32), "decoder/one"))
        with tempfile.TemporaryDirectory() as temporary:
            paths = [
                Path(temporary) / name for name in ("decoder.onnx", "joiner.onnx", "combined.onnx")
            ]
            for model, path in zip((decoder, joiner, combined), paths):
                onnx.save_model(model, path)
            with self.assertRaises(AssertionError):
                importer.verify_combination(*paths)

    def test_vocab_requires_contiguous_ids_and_final_blank(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "tokens.txt"
            text = "".join(f"token{i} {i}\n" for i in range(importer.VOCAB_SIZE - 1))
            path.write_text(text + f"<blk> {importer.VOCAB_SIZE - 1}\n", encoding="utf-8")
            importer.validate_vocab(path)
            path.write_text(text + f"not-blank {importer.VOCAB_SIZE - 1}\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "final vocabulary token"):
                importer.validate_vocab(path)
            path.write_text("token 1\n" + text.split("\n", 1)[1]
                            + f"<blk> {importer.VOCAB_SIZE - 1}\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "token 0"):
                importer.validate_vocab(path)

    def test_rejects_incorrect_archive_checksum_without_creating_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive, output = Path(temporary) / "archive", Path(temporary) / "output"
            archive.write_bytes(b"not the pinned archive")
            with self.assertRaisesRegex(ValueError, "SHA256 mismatch"):
                importer.import_archive(archive, output)
            self.assertFalse(output.exists())

    def test_never_overwrites_existing_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            with self.assertRaises(FileExistsError):
                importer.import_archive(output / "nonexistent-archive", output)

    def test_never_overwrites_dangling_output_link(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "output"
            output.symlink_to(root / "missing")
            with self.assertRaises(FileExistsError):
                importer.import_archive(root / "nonexistent-archive", output)
            self.assertTrue(output.is_symlink())

    def test_rejects_missing_and_duplicate_archive_members_before_extraction(self):
        complete = [(name, b"synthetic") for name in sorted(importer.SOURCE_FILES)]
        for members in (complete[:-1], complete + [complete[0]]):
            with self.subTest(count=len(members)), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                archive, destination = root / "model.tar.bz2", root / "source"
                write_archive(archive, members)
                with self.assertRaises(ValueError):
                    importer.extract_archive(archive, destination)
                self.assertFalse(destination.exists())

    def test_full_import_preserves_files_and_records_provenance(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive, source_files = synthetic_archive(root)
            output = root / "output"
            archive_hash = importer.sha256(archive)
            with mock.patch.object(importer, "ARCHIVE_SHA256", archive_hash):
                result = importer.import_archive(archive, output)

            recorded = json.loads((output / "VOXTYPE-CONVERSION.json").read_text())
            self.assertEqual(recorded, result)
            self.assertEqual(recorded["archive_sha256"], archive_hash)
            self.assertEqual(recorded["source_url"], importer.ARCHIVE_URL)
            self.assertEqual(recorded["weight_license"], "CC-BY-SA-4.0")
            self.assertEqual(recorded["cpu_graph_parity"]["steps"], 8)
            self.assertEqual(recorded["cpu_graph_parity"]["max_abs_error"], 0.0)
            copied_files = {
                "encoder.int8.onnx": "encoder-model.int8.onnx",
                "tokens.txt": "vocab.txt",
                "bpe.vocab": "bpe.vocab",
                "LICENSE-WEIGHTS": "LICENSE-WEIGHTS",
                "NOTICE.md": "NOTICE.md",
            }
            for source, destination in copied_files.items():
                self.assertEqual((output / destination).read_bytes(), source_files[source])
            self.assertEqual(
                set(recorded["files"]),
                set(copied_files.values()) | {"decoder_joint-model.int8.onnx"},
            )
            for name, checksum in recorded["files"].items():
                self.assertEqual(importer.sha256(output / name), checksum)
            self.assertEqual(list(root.glob(".orukeet-import-*")), [])

    def test_failed_parity_does_not_publish_output_or_leave_staging(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive, _ = synthetic_archive(root)
            output = root / "output"
            with (
                mock.patch.object(importer, "ARCHIVE_SHA256", importer.sha256(archive)),
                mock.patch.object(
                    importer, "verify_combination", side_effect=AssertionError("parity failure")
                ),
                self.assertRaisesRegex(AssertionError, "parity failure"),
            ):
                importer.import_archive(archive, output)
            self.assertFalse(output.exists())
            self.assertEqual(list(root.glob(".orukeet-import-*")), [])

    def test_rejects_archive_traversal_and_links_before_extraction(self):
        for name, kind in (
            (importer.ARCHIVE_ROOT + "/../../outside", tarfile.REGTYPE),
            (importer.ARCHIVE_ROOT + "/tokens.txt", tarfile.SYMTYPE),
        ):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                archive, destination = Path(temporary) / "archive.tar.bz2", Path(temporary) / "source"
                with tarfile.open(archive, "w:bz2") as bundle:
                    member = tarfile.TarInfo(name)
                    member.type = kind
                    member.linkname = "../../outside" if kind == tarfile.SYMTYPE else ""
                    bundle.addfile(member, io.BytesIO(b""))
                with self.assertRaisesRegex(ValueError, "Unexpected archive member"):
                    importer.extract_archive(archive, destination)
                self.assertFalse(destination.exists())


if __name__ == "__main__":
    unittest.main()
