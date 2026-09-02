#!/usr/bin/env bash
# Capture a real diarized Nova-3 response from Workers AI and pin it as the
# parser fixture. Run once (and again whenever Cloudflare bumps the model),
# then review the diff against test/fixtures/nova3-diarized.json and re-run
# the tests — all Deepgram shape assumptions live in src/deepgram.ts.
#
# Usage:
#   CLOUDFLARE_ACCOUNT_ID=... CLOUDFLARE_API_TOKEN=... \
#     ./scripts/capture-fixture.sh path/to/two-speaker.wav
set -euo pipefail

AUDIO="${1:?usage: capture-fixture.sh <two-speaker wav/mp3>}"
: "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID}"
: "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN (needs Workers AI read/run permission)}"

OUT="$(dirname "$0")/../test/fixtures/nova3-diarized.json"

case "$AUDIO" in
  *.wav) CONTENT_TYPE="audio/wav" ;;
  *.mp3) CONTENT_TYPE="audio/mpeg" ;;
  *.ogg) CONTENT_TYPE="audio/ogg" ;;
  *) CONTENT_TYPE="application/octet-stream" ;;
esac

curl --fail-with-body -sS -X POST \
  "https://api.cloudflare.com/client/v4/accounts/${CLOUDFLARE_ACCOUNT_ID}/ai/run/@cf/deepgram/nova-3?diarize=true&punctuate=true&smart_format=true" \
  -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" \
  -H "Content-Type: ${CONTENT_TYPE}" \
  --data-binary "@${AUDIO}" |
  # The REST API wraps the model output in {result, success, errors}; the AI
  # binding returns the bare model output, which is what the fixture pins.
  jq '.result' >"$OUT"

echo "Wrote $OUT"
jq '{duration: .metadata.duration, words: (.results.channels[0].alternatives[0].words | length), speakers: ([.results.channels[0].alternatives[0].words[]?.speaker] | unique)}' "$OUT"
