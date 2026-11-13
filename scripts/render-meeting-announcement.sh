#!/usr/bin/env bash
# Re-render a built-in recording announcement.
#
# The words come from ANNOUNCEMENT_TEXT in src/meeting/announce.rs, the one
# place they live, so change them there first. Two clips exist:
#
#   default    assets/sounds/meeting-announcement.wav, ElevenLabs' "Jessica"
#              on eleven_multilingual_v2. Embedded by default builds.
#   --kokoro   assets/sounds/meeting-announcement-kokoro.wav, Kokoro-82M's
#              af_heart voice from a local Kokoro server. Embedded by builds
#              with the announcement-kokoro feature (the Nix flake).
#
# Both are written as 24 kHz mono 16-bit WAV. Nothing is played.
#
# This runs only when a maintainer changes a clip. Voxtype itself never calls
# a speech service: the WAV ships in the binary and plays offline.
#
# Afterwards, update the size and sha256 in assets/sounds/README.md, which
# also holds the credit and licensing notes for each clip. Commit the WAV as
# rendered: never strip provenance metadata or watermarks from it.
#
# Usage:
#   ELEVENLABS_API_KEY=... scripts/render-meeting-announcement.sh [out.wav]
#   scripts/render-meeting-announcement.sh --kokoro [out.wav]
#
# Overrides, ElevenLabs: VOICE_ID (default Jessica, cgSgspJ2msm6clMCkdW9) and
# MODEL_ID (default eleven_multilingual_v2).
# Overrides, Kokoro: KOKORO_URL (default http://127.0.0.1:8842, an
# OpenAI-compatible /v1/audio/speech server) and KOKORO_VOICE (default
# af_heart).
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
mode=elevenlabs
if [[ ${1:-} == --kokoro ]]; then
    mode=kokoro
    shift
fi

text=$(sed -n 's/^pub const ANNOUNCEMENT_TEXT: &str = "\(.*\)";$/\1/p' "$repo/src/meeting/announce.rs")
if [[ -z $text ]]; then
    echo "Cannot find ANNOUNCEMENT_TEXT in src/meeting/announce.rs." >&2
    exit 1
fi
echo "Rendering ($mode): $text"

if [[ $mode == kokoro ]]; then
    out=${1:-$repo/assets/sounds/meeting-announcement-kokoro.wav}
    url=${KOKORO_URL:-http://127.0.0.1:8842}
    voice=${KOKORO_VOICE:-af_heart}
    body=$(python3 -c 'import json, sys; print(json.dumps({"input": sys.argv[1], "voice": sys.argv[2], "speed": 1.0}))' "$text" "$voice")
    curl -sS --fail-with-body \
        -H "Content-Type: application/json" \
        -d "$body" \
        -o "$out" \
        "$url/v1/audio/speech"
    python3 - "$out" <<'EOF'
import sys, wave
with wave.open(sys.argv[1]) as w:
    spec = (w.getnchannels(), w.getsampwidth(), w.getframerate())
    if spec != (1, 2, 24000):
        sys.exit(f"{sys.argv[1]}: expected 24 kHz mono 16-bit, got {spec}")
    print(f"Wrote {sys.argv[1]}: {w.getnframes() / 24000:.1f} s")
EOF
else
    out=${1:-$repo/assets/sounds/meeting-announcement.wav}
    voice=${VOICE_ID:-cgSgspJ2msm6clMCkdW9}
    model=${MODEL_ID:-eleven_multilingual_v2}

    if [[ -z ${ELEVENLABS_API_KEY:-} ]]; then
        echo "Set ELEVENLABS_API_KEY (needs the text_to_speech permission)." >&2
        exit 1
    fi

    pcm=$(mktemp)
    trap 'rm -f "$pcm"' EXIT
    body=$(python3 -c 'import json, sys; print(json.dumps({"text": sys.argv[1], "model_id": sys.argv[2]}))' "$text" "$model")
    curl -sS --fail-with-body \
        -H "xi-api-key: $ELEVENLABS_API_KEY" \
        -H "Content-Type: application/json" \
        -H "Accept: audio/pcm" \
        -d "$body" \
        -o "$pcm" \
        "https://api.elevenlabs.io/v1/text-to-speech/$voice?output_format=pcm_24000"

    python3 - "$pcm" "$out" <<'EOF'
import sys, wave
pcm = open(sys.argv[1], "rb").read()
with wave.open(sys.argv[2], "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(24000)
    w.writeframes(pcm)
print(f"Wrote {sys.argv[2]}: {len(pcm) / 48000:.1f} s")
EOF
fi

sha256sum "$out"
echo "Update the size and sha256 in assets/sounds/README.md."
