# Deepgram Transcription

Voxtype can transcribe with Deepgram's cloud speech-to-text API. Deepgram is available in every binary and does not run a model locally, which makes it useful on low-power computers. Recording, VAD, text processing, notifications, and cursor output remain local Voxtype behavior; only audio is sent to Deepgram.

Dictation streams to Deepgram's realtime API while you speak, so the transcript is ready when you stop. A finished recording (`voxtype transcribe file.wav`, meeting chunks, and dictation with `streaming = false`) goes to Deepgram's pre-recorded API in one request.

## Configuration

```toml
engine = "deepgram"

[deepgram]
model = "nova-3"
language = "en"
smart_format = true
mip_opt_out = true
streaming = true
# endpointing_ms = 300
timeout_secs = 30
endpoint = "https://api.deepgram.com/v1/listen"
```

| Option | Default | Description |
|--------|---------|-------------|
| `model` | `nova-3` | Deepgram model identifier |
| `language` | `en` | BCP-47 code such as `en`, `en-US` or `fr`; `auto` for language detection; `multi` for multilingual recognition |
| `smart_format` | `true` | Punctuation, numerals, dates and related formatting |
| `mip_opt_out` | `true` | Exclude the audio from Deepgram's Model Improvement Program |
| `streaming` | `true` | Stream audio while recording. `false` sends the finished recording instead |
| `endpointing_ms` | unset | Streaming only: milliseconds of silence that end a segment. Unset uses Deepgram's default |
| `timeout_secs` | `30` | How long to wait for Deepgram: the whole request for a finished recording, and the connection and the last transcript after recording stops when streaming |
| `endpoint` | `https://api.deepgram.com/v1/listen` | The `/v1/listen` endpoint. The realtime API uses the same URL over `wss://`, so either scheme works. HTTPS is required except for localhost testing |

Deepgram detects the language of a finished recording only. With `language = "auto"`, a streamed dictation is transcribed as English; set the language you speak, or `multi`.

With `streaming = true`, the built-in push-to-talk hotkey switches to toggle mode, as it does for Soniox: text is typed while you speak, so the key does not have to be held. Compositor bindings that call `voxtype record start` and `voxtype record stop` work unchanged.

## Credentials

Set `DEEPGRAM_API_KEY` (or `VOXTYPE_DEEPGRAM_API_KEY`) in the daemon environment. Either takes precedence over the optional `[deepgram] api_key` fallback:

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

## Vocabulary

Profile vocabulary (`[profiles.<name>.vocabulary]`) reaches Deepgram as `keyterm` on nova-3 models and as `keywords` on older models. Deepgram accepts up to 100 terms and 500 tokens of `keyterm`, or 100 `keywords`, and refuses a request over the limit. Deepgram does not publish how it counts tokens, so Voxtype counts about three characters per token, which is more than most words take, and drops the terms past the limit with a warning in the log.

## Request behavior

**Streaming.** Voxtype opens a WebSocket to the realtime API when recording starts, authenticated with an `Authorization: Token ...` header, and sends 16 kHz mono 16-bit PCM as it is captured. Deepgram returns a final transcript for each segment, at a pause in speech, and Voxtype types each one as it arrives (or holds them, in file output). When recording stops, Voxtype asks Deepgram to finalize the last segment and close the stream; with audio arriving at microphone speed, that answer takes a fraction of a second, and Voxtype types it after the text already typed. If the stream fails before it produces any text, Voxtype transcribes the recording through the pre-recorded API instead.

**Finished recordings.** Voxtype encodes the 16 kHz mono recording as PCM WAV and sends one HTTPS `POST` with `Content-Type: audio/wav`. The final transcript is processed and inserted at the cursor through the same output path as local engines.

Authentication, insufficient-credit, rate-limit, network, timeout, empty-result, and malformed-response failures are surfaced as transcription errors. No automatic retry is performed, so a repeated hotkey press cannot accidentally duplicate text.

## Live smoke test

With a key present, record a short sample using the normal hotkey and confirm that the transcript appears at the cursor. For an isolated WAV test of the pre-recorded path, use:

```bash
DEEPGRAM_LIVE_TEST_WAV=/absolute/path/to/short.wav cargo test deepgram_live -- --ignored
```

The live test is ignored in normal CI and also requires `DEEPGRAM_API_KEY`.
