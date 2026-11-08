#!/usr/bin/env python3
"""Compare installed Voxtype with the native Cohere CPU/Intel GPU benchmark.

Only reads model/audio files. Writes isolated configs, logs and JSON to --output;
never restarts the daemon or changes the user's configuration. See
 docs/COHERE_GPU_BENCHMARK.md. Python is benchmark tooling, not a runtime backend.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import time
import unicodedata
import wave


def run_logged(command, directory, name, timeout):
    start = time.perf_counter()
    # An ambient model-directory or backend override would invalidate the A/B
    # comparison even though each generated TOML file looks correct.
    env = {key: value for key, value in os.environ.items() if not key.startswith("VOXTYPE_")}
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout, env=env)
    except subprocess.TimeoutExpired as error:
        # TimeoutExpired may hold bytes even when text=True. Keep partial logs
        # so a stalled GPU compile can be distinguished from stalled inference.
        def text(value):
            return value.decode("utf-8", errors="replace") if isinstance(value, bytes) else value or ""
        (directory / f"{name}.stdout").write_text(text(error.stdout))
        (directory / f"{name}.stderr").write_text(text(error.stderr) + f"\nTimed out after {timeout}s\n")
        raise RuntimeError(f"{name} timed out; see {directory}/{name}.stderr") from error
    seconds = time.perf_counter() - start
    (directory / f"{name}.stdout").write_text(result.stdout)
    (directory / f"{name}.stderr").write_text(result.stderr)
    if result.returncode:
        raise RuntimeError(f"{name} failed ({result.returncode}); see {directory}/{name}.stderr")
    return result, seconds


def word_error_rate(reference, hypothesis):
    # Case/punctuation-insensitive WER; preserve accents and word boundaries.
    reference = re.findall(r"\w+", unicodedata.normalize("NFC", reference.casefold()))
    hypothesis = re.findall(r"\w+", unicodedata.normalize("NFC", hypothesis.casefold()))
    previous = list(range(len(hypothesis) + 1))
    for i, expected in enumerate(reference, 1):
        current = [i]
        for j, actual in enumerate(hypothesis, 1):
            current.append(min(current[-1] + 1, previous[j] + 1,
                               previous[j - 1] + (expected != actual)))
        previous = current
    return {"word_errors": previous[-1], "reference_words": len(reference),
            "wer": previous[-1] / len(reference) if reference else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument("--sample", action="append", required=True, metavar="LANG:WAV")
    parser.add_argument("--installed", default="/usr/bin/voxtype")
    parser.add_argument("--benchmark", type=Path, help="Native benchmark_cohere executable; omit for baseline only")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--references", type=Path, help="Fixture metadata JSON with samples[].sha256 and original_transcript")
    parser.add_argument("--installed-runs", type=int, default=3)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--gpu-first", action="store_true", help="Reverse native backend order for a counterbalanced repeat")
    parser.add_argument("--timeout", type=int, default=300)
    args = parser.parse_args()
    if args.installed_runs < 1 or args.runs < 1 or args.warmups < 0 or args.timeout < 1:
        parser.error("runs and timeout must be positive and warmups nonnegative")
    args.output.mkdir(parents=True, exist_ok=True)
    report = {"model_dir": str(args.model_dir.resolve()), "samples": [],
              "method": "installed fresh processes vs matched-source CPU/GPU; first inference separate from warm medians",
              "environment": {k: os.environ[k] for k in ("ORT_DYLIB_PATH", "LD_LIBRARY_PATH", "XDG_CACHE_HOME") if k in os.environ}}
    # These libraries and drivers affect results; record rather than assume them.
    report["installed_version"] = subprocess.check_output([args.installed, "--version"], text=True).strip()
    for name, filename in (("installed_binary", shutil.which(args.installed)),
                           ("benchmark_binary", args.benchmark)):
        if filename:
            binary = Path(filename).resolve()
            report[name] = {"path": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    references = {}
    if args.references:
        references = {s["sha256"]: s["original_transcript"] for s in json.loads(args.references.read_text())["samples"]}
    for index, sample in enumerate(args.sample):
        lang, filename = sample.split(":", 1)
        audio = Path(filename).resolve()
        with wave.open(str(audio)) as wav:
            if (wav.getnchannels(), wav.getframerate(), wav.getsampwidth()) != (1, 16000, 2):
                parser.error(f"{audio}: expected mono 16 kHz PCM16")
            duration = wav.getnframes() / wav.getframerate()
        directory = args.output / f"{index}-{lang}"
        directory.mkdir(exist_ok=True)
        configs = {}
        for backend in ("onnx", "openvino_gpu"):
            cfg = directory / f"{backend}.toml"
            cfg.write_text('engine = "cohere"\n[cohere]\n'
                           f'model = {json.dumps(str(args.model_dir.resolve()))}\n'
                           f'language = {json.dumps(lang)}\nthreads = 4\non_demand_loading = false\n'
                           f'encoder_backend = "{backend}"\n')
            configs[backend] = cfg
        row = {"audio": str(audio), "language": lang, "audio_seconds": duration,
               "sha256": hashlib.sha256(audio.read_bytes()).hexdigest(), "installed": []}
        for i in range(args.installed_runs):
            result, wall = run_logged([args.installed, "--config", str(configs["onnx"]), "transcribe", str(audio)], directory, f"installed-{i}", args.timeout)
            logs = re.sub(r"\x1b\[[0-9;]*m", "", result.stdout + result.stderr)
            load = re.search(r"Cohere model loaded in ([\d.]+)s", logs)
            inference = re.search(r"Cohere transcription completed in ([\d.]+)s", logs)
            if not load or not inference:
                raise RuntimeError("Installed binary did not report Cohere timings; inspect logs")
            clean_stdout = re.sub(r"\x1b\[[0-9;]*m", "", result.stdout)
            if "\n\n" not in clean_stdout:
                raise RuntimeError("Cannot locate installed CLI transcript; inspect stdout")
            text = clean_stdout.rsplit("\n\n", 1)[-1].strip()
            row["installed"].append({"wall_seconds": wall, "load_seconds": float(load[1]),
                                     "inference_seconds": float(inference[1]), "text": text})
        row["installed_inference_median_seconds"] = statistics.median(r["inference_seconds"] for r in row["installed"])
        row["installed_wall_median_seconds"] = statistics.median(r["wall_seconds"] for r in row["installed"])
        if args.benchmark:
            row["native_backend_order"] = ["openvino_gpu", "onnx"] if args.gpu_first else ["onnx", "openvino_gpu"]
            for backend in row["native_backend_order"]:
                result, wall = run_logged([str(args.benchmark.resolve()), "--config", str(configs[backend]),
                                          "--audio", str(audio), "--runs", str(args.runs), "--warmups", str(args.warmups)],
                                         directory, backend, args.timeout)
                data = json.loads(result.stdout)
                if data["config"]["encoder_backend"] != backend:
                    raise RuntimeError(f"Requested {backend} but benchmark used {data['config']['encoder_backend']}")
                if data["config"]["language"] != lang or data["config"]["threads"] != 4:
                    raise RuntimeError("Benchmark configuration differs from the requested language/thread count")
                data["process_wall_seconds"] = wall
                row[backend] = data
            row["matched_source_warm_speedup"] = row["onnx"]["warm_median_seconds"] / row["openvino_gpu"]["warm_median_seconds"]
            row["installed_vs_gpu_warm_speedup"] = row["installed_inference_median_seconds"] / row["openvino_gpu"]["warm_median_seconds"]
        reference = references.get(row["sha256"])
        if reference is not None:
            row["reference"] = reference
            for result in row["installed"]:
                result["accuracy"] = word_error_rate(reference, result["text"])
            for backend in ("onnx", "openvino_gpu"):
                if backend in row:
                    for result in row[backend]["measurements"]:
                        result["accuracy"] = word_error_rate(reference, result["text"])
        if args.benchmark:
            cpu_texts = {m["text"] for m in row["onnx"]["measurements"]}
            gpu_texts = {m["text"] for m in row["openvino_gpu"]["measurements"]}
            row["cpu_gpu_transcripts_identical"] = cpu_texts == gpu_texts and len(cpu_texts) == 1
        report["samples"].append(row)
        (args.output / "results.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
        print(json.dumps({"sample": str(audio), "installed_median_seconds": row["installed_inference_median_seconds"],
                          "matched_source_warm_speedup": row.get("matched_source_warm_speedup")}), flush=True)


if __name__ == "__main__":
    main()
