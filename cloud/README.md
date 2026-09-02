# Voxtype Cloud (pilot)

An OpenAI-compatible speech-to-text API on Cloudflare Workers, backed by
Workers AI. Transcription runs on `@cf/deepgram/nova-3` (with optional speaker
diarization) or `@cf/openai/whisper-large-v3-turbo`. This Worker is the pilot
for a hosted Voxtype Cloud offering and the seed of the planned local
OpenAI-compatible STT API (#244) — voxtype's existing remote Whisper client
speaks this protocol without modification.

## Endpoints

All endpoints require `Authorization: Bearer <key>`.

### `POST /v1/audio/transcriptions`

OpenAI-compatible multipart form:

| Field | Notes |
|---|---|
| `file` | required; the audio (WAV/MP3/OGG — content type taken from the part) |
| `model` | `nova-3` (default) or any `whisper*` name → whisper-large-v3-turbo |
| `language` | Nova-3 set only (en/es/fr/de/hi/ru/pt/ja/it/nl + regional variants, or `multi`); anything else is omitted |
| `response_format` | `json` (default) or `verbose_json` |
| `prompt` | accepted, ignored (keyterm mapping is a follow-up) |
| `diarize` | `true` → per-speaker-turn segments (voxtype extension; nova-3 only) |

Responses:

```jsonc
// response_format=json
{ "text": "full transcript" }

// response_format=verbose_json (speaker/confidence appear when diarize=true)
{
  "task": "transcribe", "language": "en", "duration": 8.42,
  "text": "full transcript",
  "segments": [ { "id": 0, "start": 0.32, "end": 1.98, "text": "So the deploy failed twice.", "speaker": 0, "confidence": 0.98 } ],
  "words": [ { "word": "so", "punctuated_word": "So", "start": 0.32, "end": 0.48, "confidence": 0.99, "speaker": 0 } ]
}
```

Errors use the OpenAI envelope `{"error":{"message","type","code"}}` with
401 / 400 / 413 (body > 50 MB) / 502.

### `GET /v1/models`

Static OpenAI-style list. `/v1/chat/completions` and `/v1/realtime` are
reserved (404 with a "not yet available" message).

## Deploy

```bash
cd cloud
npm install
npx wrangler types        # generates worker-configuration.d.ts (gitignored)
npm run check && npm test
npx wrangler secret put VOXTYPE_API_KEY   # the single pilot key
npm run deploy
```

The Worker serves on its workers.dev URL until the voxtype.io zone is on
Cloudflare DNS; then uncomment the `routes` block in `wrangler.jsonc` to claim
`api.voxtype.io` as a custom domain and redeploy.

## Fixture

All knowledge of the Nova-3 response shape lives in `src/deepgram.ts`, pinned
by `test/fixtures/nova3-diarized.json`. The committed fixture starts as a
hand-written placeholder matching Deepgram's documented schema — replace it
with a live capture before trusting field names:

```bash
CLOUDFLARE_ACCOUNT_ID=... CLOUDFLARE_API_TOKEN=... \
  ./scripts/capture-fixture.sh two-speaker-sample.wav
npm test
```

## Try it

```bash
URL=https://voxtype-cloud.<account>.workers.dev
curl -sS "$URL/v1/audio/transcriptions" \
  -H "Authorization: Bearer $VOXTYPE_API_KEY" \
  -F file=@test.wav -F model=nova-3 \
  -F response_format=verbose_json -F diarize=true | jq .
```

Voxtype client config for the pilot:

```toml
engine = "whisper"

[whisper]
mode = "remote"
remote_endpoint = "https://api.voxtype.io"   # or the workers.dev URL
remote_model = "nova-3"
remote_api_key = "<pilot key>"               # or VOXTYPE_WHISPER_API_KEY
remote_timeout_secs = 60

[meeting]
chunk_duration_secs = 120

[meeting.diarization]
enabled = true
backend = "remote"
```

## Privacy and logging

Audio and transcripts are never logged — only durations, sizes, status, and
model names. Audio sent here transits Cloudflare and is processed by
Deepgram's model under Deepgram's terms; this is an explicitly opt-in
departure from voxtype's local-first default.

## Cost

Nova-3 bills $0.0052 per audio minute (HTTP). A meeting with mic + loopback
both active costs ≈ $0.62/hour, less whatever silence the client-side VAD
skips. whisper-large-v3-turbo is $0.0005/min with no diarization.
