# Deepgram Batch Transcription

Voxtype can send a completed recording to Deepgram's pre-recorded speech-to-text API. Deepgram is available in every binary and does not run a model locally, which makes it useful on low-power computers. Recording, VAD, text processing, notifications, and cursor output remain local Voxtype behavior; only the recorded WAV is sent to Deepgram after the recording stops.

## Configuration

```toml
engine = "deepgram"

[deepgram]
model = "nova-3"
language = "en"
smart_format = true
mip_opt_out = true
timeout_secs = 30
endpoint = "https://api.deepgram.com/v1/listen"
```

`language` accepts a BCP-47 language code such as `en`, `en-US`, or `fr`; `auto` enables language detection; and `multi` enables multilingual recognition. `mip_opt_out = true` asks Deepgram to exclude the request from its Model Improvement Program.

## Credentials

Set `DEEPGRAM_API_KEY` in the daemon environment. It takes precedence over the optional `[deepgram] api_key` fallback:

```bash
export DEEPGRAM_API_KEY="..."
voxtype daemon
```

Avoid putting the key in shell history or a world-readable config. On Linux, a user service can retrieve it from Secret Service immediately before launching Voxtype:

```bash
key="$(secret-tool lookup application voxtype provider deepgram)"
export DEEPGRAM_API_KEY="$key"
exec voxtype daemon
```

The configuration TUI reports whether a credential came from the environment or config, but never displays its value. Voxtype does not log the credential or audio.

## Request behavior

Voxtype encodes the completed 16 kHz mono recording as PCM WAV and sends one HTTPS `POST` with `Content-Type: audio/wav` and `Authorization: Token ...`. This is batch mode: it does not keep streaming state or type partial results. The final transcript is processed and inserted at the cursor through the same output path as local engines.

Authentication, insufficient-credit, rate-limit, network, timeout, empty-result, and malformed-response failures are surfaced as transcription errors. No automatic retry is performed, so a repeated hotkey press cannot accidentally duplicate text.

## Live smoke test

With a key present, record a short sample using the normal hotkey and confirm that the transcript appears at the cursor. For an isolated WAV test, use:

```bash
DEEPGRAM_LIVE_TEST_WAV=/absolute/path/to/short.wav cargo test deepgram_live -- --ignored
```

The live test is ignored in normal CI and also requires `DEEPGRAM_API_KEY`.
