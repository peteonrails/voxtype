#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
AUDIO_FILE="${AUDIO_FILE:-$ROOT_DIR/harvard.wav}"
ENDPOINT="${ENDPOINT:-http://localhost:8000}"
MODEL="${MODEL:-gemini-2.5-flash}"
SYSTEM_PROMPT="${SYSTEM_PROMPT:-Translate whatever I say into pirate-speech. Return ONLY the translated text. No preamble, no explanations, no labels, no quotes, no extra lines.}"
API_KEY="${API_KEY:-${VOXTYPE_WHISPER_API_KEY:-}}"

if [[ ! -f "$AUDIO_FILE" ]]; then
    echo "Audio file not found: $AUDIO_FILE" >&2
    exit 1
fi

if [[ -z "$API_KEY" ]]; then
    echo "Missing API key. Set API_KEY or VOXTYPE_WHISPER_API_KEY." >&2
    exit 1
fi

export AUDIO_FILE MODEL SYSTEM_PROMPT

python - <<'PY' | curl -s "${ENDPOINT%/}/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer ${API_KEY}" \
    -d @-
import base64
import json
import os

audio_file = os.environ["AUDIO_FILE"]
model = os.environ["MODEL"]
system_prompt = os.environ["SYSTEM_PROMPT"]

with open(audio_file, "rb") as fh:
    audio_b64 = base64.b64encode(fh.read()).decode("utf-8")

payload = {
    "model": model,
    "messages": [
        {"role": "system", "content": system_prompt},
        {
            "role": "user",
            "content": [
                {"type": "text", "text": "Process this audio."},
                {
                    "type": "image_url",
                    "image_url": {"url": f"data:audio/wav;base64,{audio_b64}"},
                },
            ],
        },
    ],
}

print(json.dumps(payload))
PY
