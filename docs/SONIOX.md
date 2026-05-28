# Soniox Backend

Voxtype supports [Soniox](https://soniox.com) as a cloud streaming speech-to-text backend. Unlike voxtype's other engines (Whisper, Parakeet, Moonshine, etc.), nothing runs on your machine — audio streams to Soniox's servers over WebSocket and tokens stream back.

## What is Soniox?

Soniox is a paid cloud STT provider offering:

- **60+ languages** including strong Hungarian, English, German, French, Spanish, etc.
- **Per-token finality** — server marks each token as `is_final: true` or `false`, with stable-final guarantees
- **Sub-second latency** for partial tokens
- **Server-side endpoint detection** — automatic finalization at utterance boundaries
- **Two API modes** — realtime WebSocket (`stt-rt-v4`) and async REST (`stt-async-v4`)
- **Domain context** — bias the model toward your vocabulary

## Privacy

Audio is sent to a third-party service over TLS. Soniox processes the audio server-side and discards it according to their [data retention policy](https://soniox.com). **Use a local engine (Whisper, Parakeet) instead if your dictation contains anything you cannot send off-device.**

## Cost

Soniox is paid SaaS. Sign up at [console.soniox.com](https://console.soniox.com) for an API key. New accounts get free credits to evaluate; pay-as-you-go pricing kicks in after that. Check the current [pricing page](https://soniox.com/pricing) before relying on it for high-volume dictation.

## Requirements

- voxtype (Soniox ships in every release binary; no feature flag needed)
- A Soniox API key (free tier credits available)
- Outbound HTTPS / WebSocket (wss://) access

No local model files. No GPU.

## Quick Start

1. Get an API key at [console.soniox.com](https://console.soniox.com).
2. Export it:
   ```bash
   export SONIOX_API_KEY="your-key-here"
   ```
3. Minimal config in `~/.config/voxtype/config.toml`:
   ```toml
   engine = "soniox"

   [hotkey]
   mode = "toggle"           # required when streaming (default)
   key = "SCROLLLOCK"

   [soniox]
   language_hints = ["en"]   # adjust for your languages
   ```
4. Run voxtype, press the hotkey, dictate, press again to stop.

## Two API Modes

Soniox exposes two distinct backends. Voxtype supports both via `[soniox] async_api`.

### Realtime (default — `async_api = false`)

WebSocket-based. Tokens stream back as you speak.

- **Latency:** partials appear within ~100ms of speech, finals at utterance boundaries.
- **Model:** `stt-rt-v4`.
- **Activation:** **toggle only.** Push-to-talk is auto-promoted to toggle for the running session, because typing characters at the cursor while the PTT key is still held breaks libinput's held-key state on Hyprland/Sway/River.
- **Live typing:** non-final tokens are typed at the cursor as they arrive (`type_partials = true`). Soniox occasionally revises the tail when finalizing; voxtype emits a backspace+retype primitive (`StreamingEvent::Replace`) to patch up the cursor.

```toml
[hotkey]
mode = "toggle"

[soniox]
streaming = true            # default
type_partials = true        # default; live cursor feedback
language_hints = ["en"]
```

To use realtime without live partial typing (only commits at finals):

```toml
[soniox]
type_partials = false       # cursor stays still until finals arrive
```

To use realtime without streaming (one-shot WebSocket on key release):

```toml
[hotkey]
mode = "push_to_talk"       # streaming=false makes PTT safe again

[soniox]
streaming = false           # buffer locally, single WS round trip on release
```

### Async (`async_api = true`)

REST-based file upload + poll. Higher accuracy claim, but slower roundtrip.

- **Latency:** for a 15s recording, ~1s upload + 2-5s server processing = 3-6s wait after release.
- **Model:** `stt-async-v4`.
- **Activation:** push-to-talk compatible. No live typing, no compositor-state clobbering.
- **Quality:** marketed as more accurate than realtime. In practice quality varies by language and content — benchmark both before committing.

```toml
[hotkey]
mode = "push_to_talk"

[soniox]
async_api = true
language_hints = ["en"]
# model defaults to stt-async-v4 when async_api = true
```

### Dictation vs Meeting Mode (automatic)

The two Soniox APIs target different use cases and voxtype routes them automatically:

| Mode | API used | Why |
|---|---|---|
| Dictation (hotkey) | follows your `async_api` setting (default `false` → realtime WS) | Live partials, sub-second latency |
| Meeting (`voxtype meeting start`) | **always** async REST, regardless of `async_api` | Fixed-chunk batch is what async is designed for; diarization-friendly; audio-second billing instead of WS-duration; survives network hiccups |

Concretely: if your config has the default `async_api = false`, dictation keeps live-partial WebSocket typing while meetings transparently switch to `stt-async-v4` for each 30-second chunk. No knob to turn off — meetings on the realtime WS would open one fresh socket per chunk, pay connect latency, and bill by session-duration not audio-duration.

If your config has `async_api = true` (explicit), both paths use async — your call.

You can see the meeting-mode switch in the daemon log: `Soniox meeting mode: routing to async API (stt-async-v4); dictation path unchanged`.

## Language Hints

Soniox supports 60+ languages and can auto-detect, but providing `language_hints` improves accuracy and reduces latency for short utterances. For bilingual dictation:

```toml
[soniox]
language_hints = ["hu", "en"]   # Hungarian + English, in priority order
language_hints_strict = true    # default — restrict output to hinted languages
```

Empty array `[]` means full auto-detect across all supported languages. When `language_hints_strict = true` (the default), the model is strongly biased to produce output only in the listed languages, avoiding mid-stream drift to a third language in partials. Set to `false` to allow the model to occasionally produce other languages it detects with high confidence. The strict flag is ignored when `language_hints` is empty. See https://soniox.com/docs/stt/concepts/language-restrictions.

## Configuration Reference

See [CONFIGURATION.md → [soniox]](CONFIGURATION.md#soniox) for the full field-by-field reference. Key settings:

| Field | Default | Notes |
|---|---|---|
| `api_key` | env: `SONIOX_API_KEY` | Required |
| `model` | `stt-rt-v4` (or `stt-async-v4` if `async_api`) | Advanced override |
| `language_hints` | `["hu", "en"]` | Empty = auto-detect |
| `language_hints_strict` | `true` | Restrict output to hinted languages (no-op if hints empty) |
| `streaming` | `true` | Realtime live; ignored if `async_api` |
| `type_partials` | `true` | Live cursor feedback; realtime only |
| `context` | none | Free-form domain prose (`context.text`) |
| `terms` | none | Inline boost-term array (`context.terms`) |
| `terms_file` | none | JSON file with boost terms |
| `async_api` | `false` | Use REST instead of WebSocket |
| `async_max_wait_secs` | `120` | Async timeout |

## Performance Notes

Streaming output calls the typing driver many times per session (~60 partials for typical dictation). With direct `dotool` invocations each call spawns a fresh dotool process that pays ~700ms of uinput device setup — stacking to 40+ seconds of typing latency per session.

**Strongly recommended:** run `dotoold` so voxtype can route calls without
per-call XKB hints through `dotoolc` (sub-10ms per call). `dotoolc` does not
work with variants and cannot receive voxtype's per-call XKB hints, so hinted
layout/variant calls use direct `dotool` instead. See
[Streaming performance: dotoold fast path](CONFIGURATION.md#streaming-performance-dotoold-fast-path)
for the systemd user unit template.

If you rely on direct dotool layout or variant hints for non-English text,
switch the active desktop layout to the matching layout before dictating.
dotool sends key events; it does not change the focused app's active layout.

If you're on KDE Plasma and see system-tray flicker during streaming, that's the `eitype` driver triggering the RemoteDesktop portal security indicator on each invocation. Prefer `dotool` (via dotoold) in your `[output] driver_order`.

## Troubleshooting

See [TROUBLESHOOTING.md → Soniox Backend Issues](TROUBLESHOOTING.md#soniox-backend-issues) for:
- Auth errors (401/403)
- Connect failures and 408 timeouts
- Tail-revision divergence (typed text doesn't match final)
- Flicker on KDE
- Slow typing without dotoold
- Wrong layout when dotoold is running
- Stuck async jobs

## Comparing to Local Engines

| | Soniox realtime | Soniox async | Whisper local | Parakeet local |
|---|---|---|---|---|
| Quality (HU) | Good | Slightly better | Excellent | N/A |
| Quality (EN) | Excellent | Excellent | Excellent | Excellent |
| Latency | ~100ms partials | 3-6s on release | 500ms-3s on release | ~100ms partials |
| Privacy | Cloud | Cloud | Local | Local |
| Cost | Paid | Paid | Free | Free |
| GPU needed | No | No | Optional | Optional |
| RAM | ~50 MB | ~50 MB | 1-5 GB | ~1.5 GB |
| Offline | No | No | Yes | Yes |
| Multilingual | 60+ languages | 60+ languages | 99 languages | English only |

## Limitations

- **No on-prem option.** Soniox is SaaS only.
- **Internet dependency.** No fallback to a local engine if the network drops mid-session — voxtype surfaces a `Streaming Error` notification and returns to idle.
- **Stream sessions can't be replayed.** Once tokens arrive, they're processed once. The async API supports re-fetching transcripts, but realtime tokens are not stored.
- **Tail revisions cause occasional cursor artifacts.** When Soniox revises a non-final token that voxtype already typed (e.g. `,` → `.`), voxtype emits a backspace+retype. In rare cases where the revision is bigger than my LCP-reconciler can handle, you may see transient duplication. Set `type_partials = false` to avoid entirely.
- **Async API doesn't stream.** Use realtime for live cursor feedback.
