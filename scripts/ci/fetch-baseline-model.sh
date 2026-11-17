#!/usr/bin/env bash
# tiny.en is a fixed CI input, not an unchecked HTTP response passed to Whisper.
# Pin from the mirror's manifest, verified against the actual model bytes.
set -euo pipefail

destination=${1:?usage: fetch-baseline-model.sh DESTINATION}
mkdir -p "$(dirname -- "$destination")"
partial=$(mktemp "${destination}.XXXXXX")
trap 'rm -f -- "$partial"' EXIT

curl --fail --silent --show-error --location --retry 3 \
    --connect-timeout 15 --max-time 180 \
    -o "$partial" \
    https://models.voxtype.io/whisper/tiny.en/ggml-tiny.en.bin

python3 - "$partial" <<'PY'
import hashlib
from pathlib import Path
import sys

path = Path(sys.argv[1])
expected_size = 77704715
expected_sha256 = "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f"
if path.stat().st_size != expected_size:
    raise SystemExit(f"tiny.en size mismatch: expected {expected_size}, got {path.stat().st_size}")
digest = hashlib.sha256()
with path.open("rb") as source:
    for chunk in iter(lambda: source.read(1024 * 1024), b""):
        digest.update(chunk)
if digest.hexdigest() != expected_sha256:
    raise SystemExit(f"tiny.en sha256 mismatch: expected {expected_sha256}, got {digest.hexdigest()}")
print(f"Verified tiny.en: {expected_size} bytes, sha256 {expected_sha256}")
PY

mv -f -- "$partial" "$destination"
