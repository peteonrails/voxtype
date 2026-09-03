# Volcengine Seed-ASR Integration

This document describes voxtype's native Volcengine Seed-ASR engine. It covers
configuration, authentication, streaming behavior, and the WebSocket protocol
boundary implemented by `transcribe::seedasr`.

## Overview

Seed-ASR is a cloud speech-recognition engine. Voxtype sends 16 kHz mono PCM
audio to Volcengine over WebSocket and maps cumulative recognition results to
its existing streaming events.

The initial implementation supports:

- Seed-ASR 2.0 bidirectional streaming
- New-console API key authentication
- Legacy-console App ID and access token authentication
- Native live streaming and buffered one-shot transcription
- Stable final results, optional partial typing, and tail correction
- A configurable resource ID and WebSocket URL

OSD transcript snapshots are deliberately outside this implementation. The
engine only emits the existing `StreamingEvent` variants consumed by the daemon.

Seed-ASR is a third-party paid service. Audio leaves the local machine and is
subject to the account's Volcengine billing, retention, and regional policies.
Use a local engine when dictated audio cannot be sent off-device.

## Requirements

1. A Volcengine account with Seed-ASR enabled.
2. Either a new-console API key or legacy-console credentials.
3. Network access to the configured WebSocket endpoint.

Seed-ASR is compiled into every voxtype binary and does not require a Cargo
feature.

## Quick start

Run `voxtype configure`, choose `seedasr` in the Engine section, select an
authentication mode, and enter its credentials. The form masks API keys and
access tokens.

The same setup can be performed non-interactively:

```bash
voxtype config set engine seedasr
voxtype config set seedasr.api_key "$SEEDASR_API_KEY"
voxtype config set seedasr.resource_id volc.seedasr.sauc.duration
```

### New-console credentials

```toml
engine = "seedasr"

[hotkey]
mode = "toggle"

[seedasr]
api_key = "your-api-key"
resource_id = "volc.seedasr.sauc.duration"
```

The API key can be kept out of the configuration file:

```bash
export SEEDASR_API_KEY="your-api-key"
```

### Legacy-console credentials

```toml
engine = "seedasr"

[hotkey]
mode = "toggle"

[seedasr]
app_id = "your-app-id"
access_token = "your-access-token"
resource_id = "volc.seedasr.sauc.duration"
```

Equivalent environment variables are `SEEDASR_APP_ID` and
`SEEDASR_ACCESS_TOKEN`.

## Authentication

Voxtype accepts exactly one authentication mode per configuration.

| Mode | Configuration | WebSocket headers |
|---|---|---|
| New console | `api_key` | `X-Api-Key` |
| Legacy console | `app_id` and `access_token` | `X-Api-App-Key` and `X-Api-Access-Key` |

Both modes also send a generated UUID in `X-Api-Request-Id` and the configured
value in `X-Api-Resource-Id`.

Configuring both `api_key` and any legacy credential is an error. Legacy mode
requires both `app_id` and `access_token`.

## Streaming and buffered modes

### Native streaming

`streaming = true` is the default. Voxtype opens the bidirectional WebSocket at
recording start and sends audio in 200 ms gzip-compressed PCM16 frames. The
daemon automatically treats this as a streaming engine and promotes an
incompatible push-to-talk hotkey to toggle mode through the existing streaming
gate.

The request enables `enable_nonstream` so the service can return its second-pass
stable result for each finalized utterance. This is distinct from voxtype's
`streaming` setting: the former is a Seed-ASR two-pass recognition option, while
the latter selects voxtype's live pipeline.

### Buffered one-shot mode

Set `streaming = false` to keep push-to-talk behavior. Voxtype buffers the
recording, opens the configured WebSocket after release, sends the complete
audio stream, and returns the last cumulative transcript.

Both modes use `url`. The default endpoint is:

```text
wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async
```

## Result reconciliation

Seed-ASR returns cumulative transcript snapshots. Text in the current utterance
can change until its `definite` flag becomes true. Voxtype converts those
snapshots to cursor-oriented deltas:

| Seed-ASR state | Voxtype event | Purpose |
|---|---|---|
| Stable prefix extends committed text | `Final` | Types and commits only the new stable suffix |
| Optional provisional prefix extends the typed tail | `Partial` | Types only the new provisional suffix |
| Stable text revises a typed provisional tail | `Replace` | Backspaces the divergent Unicode characters and commits the replacement |
| Final response package | `Ended` after reconciliation | Closes the session cleanly |
| Protocol, server, or network failure | `Error`, then `Ended` | Surfaces the failure and resets daemon state |

`type_partials = false` is the default. This emits stable text only and avoids
visible corrections. When enabled, voxtype types only monotonic extensions of a
partial result; it suppresses provisional revisions until Seed-ASR finalizes the
utterance.

If the service revises text already marked definite and committed, voxtype ends
the session with an error. The existing streaming contract can revise the active
partial tail but intentionally cannot rewrite arbitrary older output.

## Wire protocol

The engine implements Volcengine's binary WebSocket framing directly:

1. A full client request (`0x1`) contains gzip-compressed JSON.
2. Audio-only requests (`0x2`) contain gzip-compressed raw PCM16.
3. The last audio request uses the protocol's last-packet flag.
4. Full server responses (`0x9`) are decompressed and parsed as JSON.
5. Server error frames (`0xF`) are converted to `TranscribeError`.

The response parser accepts both the direct recognition payload and the
`payload_msg` wrapper used by newer protocol examples. A negative response
sequence, the binary last-packet flag, or `is_last_package = true` terminates
the recognition stream.

Official protocol references:

- [Seed-ASR bidirectional streaming API](https://docs.volcengine.com/docs/6561/2630027?lang=zh)
- [Seed-ASR binary WebSocket protocol](https://docs.volcengine.com/docs/6561/1354869?lang=zh)
- [Legacy credential FAQ](https://docs.volcengine.com/docs/6561/196768)

## Configuration reference

| Option | Environment variable | Default | Description |
|---|---|---|---|
| `api_key` | `SEEDASR_API_KEY` | unset | New-console API key |
| `app_id` | `SEEDASR_APP_ID` | unset | Legacy-console App ID |
| `access_token` | `SEEDASR_ACCESS_TOKEN` | unset | Legacy-console access token |
| `resource_id` | `SEEDASR_RESOURCE_ID` | `volc.seedasr.sauc.duration` | Service/resource identifier |
| `url` | `SEEDASR_URL` | Seed-ASR 2.0 bidirectional endpoint | WebSocket endpoint |
| `streaming` | - | `true` | Use native live streaming |
| `type_partials` | - | `false` | Type monotonic provisional text |
| `language` | - | unset | Recognition language; unset or `auto` enables detection |
| `enable_itn` | - | `true` | Enable inverse text normalization |
| `enable_punc` | - | `true` | Enable punctuation |
| `enable_ddc` | - | `false` | Enable semantic smoothing and filler-word removal |
| `end_window_ms` | - | `800` | Silence window used to finalize utterances; valid range 300-5000 ms |

Common alternative resource IDs include
`volc.seedasr.sauc.concurrent` for concurrency-based Seed-ASR 2.0 plans. Keep
this value configurable because account entitlements and product generations
can require a different resource ID.

## Security and diagnostics

API keys and access tokens are never included in voxtype logs. Startup logging
records only the authentication mode.
Volcengine's `X-Tt-Logid` response header is logged at debug level to help
support teams trace failed requests.

Use protocol trace logging only when needed:

```bash
RUST_LOG=voxtype::seedasr=debug,voxtype::seedasr::wire=trace voxtype daemon
```

Wire logs include recognized text but not request headers. Treat them as
sensitive if dictated content is private.

## Current limitations

- The engine sends 16 kHz mono PCM16 input only.
- Hotwords, contextual prompts, speaker diarization, and unidirectional
  high-accuracy mode are not exposed in the initial implementation.
- Live service integration tests require user credentials and are not part of
  the offline test suite.
- No OSD transcript-snapshot event is introduced by this change.
