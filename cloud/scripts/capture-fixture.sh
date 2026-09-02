#!/usr/bin/env bash
# Capture a real diarized Nova-3 response and pin it as the parser fixture.
# Run once (and again whenever Cloudflare bumps the model), review the diff
# against test/fixtures/nova3-diarized.json, and re-run the tests — all
# Deepgram shape assumptions live in src/deepgram.ts.
#
# Goes through the deployed Worker's auth-protected raw=true debug field, so
# the only credential needed is the pilot API key (cloud/.dev.vars).
#
# Usage:
#   VOXTYPE_CLOUD_URL=https://voxtype-cloud.<account>.workers.dev \
#   VOXTYPE_API_KEY=... ./scripts/capture-fixture.sh path/to/two-speaker.wav
set -euo pipefail

AUDIO="${1:?usage: capture-fixture.sh <two-speaker wav/mp3>}"
: "${VOXTYPE_CLOUD_URL:?set VOXTYPE_CLOUD_URL to the deployed Worker URL}"

# Default the key from cloud/.dev.vars when not set in the environment.
if [ -z "${VOXTYPE_API_KEY:-}" ] && [ -f "$(dirname "$0")/../.dev.vars" ]; then
  VOXTYPE_API_KEY="$(sed -n 's/^VOXTYPE_API_KEY=//p' "$(dirname "$0")/../.dev.vars")"
fi
: "${VOXTYPE_API_KEY:?set VOXTYPE_API_KEY to the pilot key}"

OUT="$(dirname "$0")/../test/fixtures/nova3-diarized.json"

curl --fail-with-body -sS -X POST "${VOXTYPE_CLOUD_URL}/v1/audio/transcriptions" \
  -H "Authorization: Bearer ${VOXTYPE_API_KEY}" \
  -F "file=@${AUDIO}" \
  -F "diarize=true" \
  -F "raw=true" |
  jq . >"$OUT"

echo "Wrote $OUT"
jq '{duration: .metadata?.duration, words: (.results?.channels?[0]?.alternatives?[0]?.words | length), speakers: ([.results?.channels?[0]?.alternatives?[0]?.words[]?.speaker] | unique)}' "$OUT"
