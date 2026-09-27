"""Offline tests for benchmark accounting and failure diagnostics.

Run: python3 -B -m unittest discover -s scripts/tests -p 'test_benchmark_cohere_gpu.py'
"""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "benchmark_cohere_gpu", Path(__file__).resolve().parents[1] / "benchmark_cohere_gpu.py"
)
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


class WordErrorRateTests(unittest.TestCase):
    def test_case_punctuation_and_canonical_unicode(self):
        self.assertEqual(benchmark.word_error_rate("¡ÁREA, sí!", "a\u0301rea SI\u0301")["word_errors"], 0)
        self.assertEqual(benchmark.word_error_rate("área", "area")["word_errors"], 1)

    def test_insert_delete_and_substitute(self):
        for hypothesis in ("a x b c", "a c", "a x c"):
            result = benchmark.word_error_rate("a b c", hypothesis)
            self.assertEqual(result, {"word_errors": 1, "reference_words": 3, "wer": 1 / 3})

    def test_empty_reference_and_hypothesis(self):
        self.assertIsNone(benchmark.word_error_rate("", "hello")["wer"])
        self.assertEqual(benchmark.word_error_rate("a b", "")["wer"], 1.0)


class SubprocessTests(unittest.TestCase):
    def test_environment_overrides_are_removed_and_logs_are_saved(self):
        with tempfile.TemporaryDirectory() as directory, \
             patch.dict("os.environ", {"VOXTYPE_COHERE_MODEL_DIR": "wrong-model", "ORT_DYLIB_PATH": "runtime.so"}), \
             patch.object(benchmark.subprocess, "run", return_value=subprocess.CompletedProcess(["test"], 0, "ok", "diagnostic")) as run:
            path = Path(directory)
            result, elapsed = benchmark.run_logged(["test"], path, "run", 3)
            self.assertEqual(result.returncode, 0)
            self.assertGreaterEqual(elapsed, 0)
            env = run.call_args.kwargs["env"]
            self.assertFalse(any(key.startswith("VOXTYPE_") for key in env))
            self.assertEqual(env["ORT_DYLIB_PATH"], "runtime.so")
            self.assertEqual((path / "run.stdout").read_text(), "ok")
            self.assertEqual((path / "run.stderr").read_text(), "diagnostic")

    def test_aborted_process_keeps_logs(self):
        with tempfile.TemporaryDirectory() as directory, \
             patch.object(benchmark.subprocess, "run", return_value=subprocess.CompletedProcess(["test"], -6, "partial", "aborted")):
            path = Path(directory)
            with self.assertRaisesRegex(RuntimeError, "failed.*-6"):
                benchmark.run_logged(["test"], path, "run", 3)
            self.assertEqual((path / "run.stderr").read_text(), "aborted")

    def test_timeout_keeps_partial_byte_logs(self):
        error = subprocess.TimeoutExpired(["test"], 3, output=b"loaded", stderr=b"compiling")
        with tempfile.TemporaryDirectory() as directory, patch.object(benchmark.subprocess, "run", side_effect=error):
            path = Path(directory)
            with self.assertRaisesRegex(RuntimeError, "timed out"):
                benchmark.run_logged(["test"], path, "run", 3)
            self.assertEqual((path / "run.stdout").read_text(), "loaded")
            self.assertIn("compiling", (path / "run.stderr").read_text())
            self.assertIn("Timed out after 3s", (path / "run.stderr").read_text())


if __name__ == "__main__":
    unittest.main()
