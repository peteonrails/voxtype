#!/usr/bin/env python3
"""Train Voxtype's compact token tagger from the LARD CSV files.

The script uses only the Python standard library. It emits the stable VXDF
binary format consumed by src/text/disfluency.rs and prints validation metrics
to stderr. The input CSV files are not copied into the repository or model.
"""

from __future__ import annotations

import argparse
import ast
import csv
import hashlib
import math
import random
import struct
import sys
from array import array
from pathlib import Path

MAGIC = b"VXDFL001"
EXPECTED_MD5 = {
    "train.csv": "ada1e11d565c04779940100cb9323ba8",
    "validation.csv": "af36afcd23712d3ebc01064a5e65ede0",
    "test.csv": "ccba0fd9f1e3fb973961527be6c58762",
}


def fnv1a(value: str) -> int:
    result = 0xCBF29CE484222325
    for byte in value.encode("utf-8"):
        result ^= byte
        result = (result * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return result


def normalize(token: str) -> str:
    token = token.lower().replace("’", "'")
    return "#" if token.isdigit() else token


def shape(token: str) -> str:
    result = []
    for char in token:
        kind = "A" if char.isalpha() else "9" if char.isdigit() else char
        if not result or result[-1] != kind:
            result.append(kind)
    return "".join(result)


def features(tokens: list[str], index: int, dimensions: int) -> list[tuple[int, float]]:
    words = [normalize(token) for token in tokens]
    word = words[index]
    prev = words[index - 1] if index else "<s>"
    nxt = words[index + 1] if index + 1 < len(words) else "</s>"
    raw = [
        "bias",
        f"w={word}",
        f"p={prev}",
        f"n={nxt}",
        f"pw={prev}|{word}",
        f"wn={word}|{nxt}",
        f"shape={shape(tokens[index])}",
        f"pre={word[:3]}",
        f"suf={word[-3:]}",
        f"pos={'start' if index < 2 else 'end' if index + 2 >= len(words) else 'mid'}",
    ]
    for distance in range(1, 9):
        if index >= distance and words[index - distance] == word:
            raw.append(f"same-prev={distance}")
        if index + distance < len(words) and words[index + distance] == word:
            raw.append(f"same-next={distance}")
    result = []
    for item in raw:
        hashed = fnv1a(item)
        sign = -1.0 if hashed & (1 << 63) else 1.0
        result.append((hashed & (dimensions - 1), sign))
    return result


def rows(path: Path) -> list[tuple[list[str], list[int]]]:
    digest = hashlib.md5(path.read_bytes()).hexdigest()  # nosec: published identity
    expected = EXPECTED_MD5.get(path.name)
    if expected and digest != expected:
        raise SystemExit(f"{path}: md5 {digest}, expected published LARD file {expected}")
    parsed = []
    with path.open(newline="", encoding="utf-8") as handle:
        for row in csv.DictReader(handle):
            tokens = ast.literal_eval(row["tokenized_disfluent_text"])
            tags = [1 if tag == "D" else 0 for tag in ast.literal_eval(row["token_tags"])]
            if len(tokens) != len(tags):
                raise SystemExit(f"{path}: token/tag length mismatch")
            parsed.append((tokens, tags))
    return parsed


def score(weights: array, values: list[tuple[int, float]]) -> float:
    return sum(weights[index] * sign for index, sign in values)


def train(examples: list[tuple[list[str], list[int]]], dimensions: int, epochs: int) -> array:
    weights = array("f", [0.0]) * dimensions
    order = list(range(len(examples)))
    rng = random.Random(696)
    step = 0
    for epoch in range(epochs):
        rng.shuffle(order)
        for row_index in order:
            tokens, tags = examples[row_index]
            for index, label in enumerate(tags):
                values = features(tokens, index, dimensions)
                prediction = score(weights, values)
                probability = 1.0 / (1.0 + math.exp(-max(-20.0, min(20.0, prediction))))
                # Disfluent tokens are the minority. Weighting them avoids a
                # superficially accurate model that never removes anything.
                error = (label - probability) * (3.0 if label else 1.0)
                rate = 0.08 / (1.0 + step / 2_000_000)
                for bucket, sign in values:
                    weights[bucket] += rate * error * sign
                step += 1
        print(f"finished epoch {epoch + 1}/{epochs}", file=sys.stderr)
    return weights


def tune_threshold(weights: array, examples: list[tuple[list[str], list[int]]], dimensions: int) -> tuple[float, float, float, float, float]:
    scored = []
    for tokens, tags in examples:
        scored.append(([score(weights, features(tokens, i, dimensions)) for i in range(len(tags))], tags))
    flat = [item for sequence, tags in scored for item in zip(sequence, tags)]
    candidates = []
    for tenth in range(-10, 81):
        threshold = tenth / 10.0
        tp = fp = fn = 0
        for value, label in flat:
            predicted = value >= threshold
            tp += predicted and label == 1
            fp += predicted and label == 0
            fn += (not predicted) and label == 1
        f1 = 2 * tp / max(1, 2 * tp + fp + fn)
        precision = tp / max(1, tp + fp)
        recall = tp / max(1, tp + fn)
        candidates.append((threshold, f1, precision, recall))
    anchored = []
    for continuation_tenth in range(10, 41, 5):
        continuation = continuation_tenth / 10.0
        for threshold_tenth in range(continuation_tenth + 1, 81, 2):
            threshold = threshold_tenth / 10.0
            tp = fp = fn = 0
            for values, tags in scored:
                predicted = [value >= threshold for value in values]
                for anchor in [i for i, value in enumerate(predicted) if value]:
                    left = anchor - 1
                    while left >= 0 and values[left] >= continuation:
                        predicted[left] = True
                        left -= 1
                    right = anchor + 1
                    while right < len(values) and values[right] >= continuation:
                        predicted[right] = True
                        right += 1
                for value, label in zip(predicted, tags):
                    tp += value and label == 1
                    fp += value and label == 0
                    fn += (not value) and label == 1
            precision = tp / max(1, tp + fp)
            recall = tp / max(1, tp + fn)
            f1 = 2 * tp / max(1, 2 * tp + fp + fn)
            anchored.append((threshold, continuation, f1, precision, recall))
    # This stage deletes words. Prefer a missed disfluency to a fluent-word
    # deletion, even though that is not the threshold with the highest F1.
    safe = [candidate for candidate in anchored if candidate[3] >= 0.95]
    if safe:
        return max(safe, key=lambda candidate: (candidate[4], candidate[2]))
    return max(anchored, key=lambda candidate: (candidate[3], candidate[4]))


def write_model(path: Path, weights: array, threshold: float, continuation: float) -> None:
    maximum = max(abs(value) for value in weights)
    scale = maximum / 32767.0 if maximum else 1.0
    quantized = [max(-32767, min(32767, round(value / scale))) for value in weights]
    payload = MAGIC + struct.pack("<Ifff", len(weights), threshold, continuation, scale)
    payload += struct.pack(f"<{len(quantized)}h", *quantized)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(payload)
    print(f"wrote {path} ({len(payload)} bytes, sha256 {hashlib.sha256(payload).hexdigest()})", file=sys.stderr)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--train", type=Path, required=True)
    parser.add_argument("--validation", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--dimensions", type=int, default=32768)
    parser.add_argument("--epochs", type=int, default=3)
    args = parser.parse_args()
    if args.dimensions <= 0 or args.dimensions & (args.dimensions - 1):
        parser.error("--dimensions must be a power of two")
    weights = train(rows(args.train), args.dimensions, args.epochs)
    threshold, continuation, f1, precision, recall = tune_threshold(weights, rows(args.validation), args.dimensions)
    print(
        f"validation token precision={precision:.4f}, recall={recall:.4f}, "
        f"F1={f1:.4f}, thresholds={threshold:.1f}/{continuation:.1f}",
        file=sys.stderr,
    )
    write_model(args.output, weights, threshold, continuation)


if __name__ == "__main__":
    main()
