#!/usr/bin/env bash
# Re-render the built-in recording announcement,
# assets/sounds/meeting-announcement.wav.
#
# The words come from ANNOUNCEMENT_TEXT in src/meeting/announce.rs, the one
# place they live, so change them there first. The voice is ElevenLabs'
# "Jessica" on eleven_multilingual_v2, written as 24 kHz mono 16-bit WAV.
#
# This runs only when a maintainer changes the clip. Voxtype itself never
# calls ElevenLabs: the WAV ships in the binary and plays offline.
#
# Usage:
#   ELEVENLABS_API_KEY=... scripts/render-meeting-announcement.sh [out.wav]
#
# Overrides: VOICE_ID (default Jessica, cgSgspJ2msm6clMCkdW9) and MODEL_ID
# (default eleven_multilingual_v2).
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$repo/assets/sounds/meeting-announcement.wav}
voice=${VOICE_ID:-cgSgspJ2msm6clMCkdW9}
model=${MODEL_ID:-eleven_multilingual_v2}

if [[ -z ${ELEVENLABS_API_KEY:-} ]]; then
    echo "Set ELEVENLABS_API_KEY (needs the text_to_speech permission)." >&2
    exit 1
fi

text=$(sed -n 's/^pub const ANNOUNCEMENT_TEXT: &str = "\(.*\)";$/\1/p' "$repo/src/meeting/announce.rs")
if [[ -z $text ]]; then
    echo "Cannot find ANNOUNCEMENT_TEXT in src/meeting/announce.rs." >&2
    exit 1
fi
echo "Rendering: $text"

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
