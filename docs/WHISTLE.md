# Whistle Backend

Voxtype supports [Whistle](https://huggingface.co/Cactus-Compute/whistle), Cactus Compute's 16.9 MB on-device speech recognition model, as an alternative to Whisper and the other local engines. Unlike the ONNX engines, Whistle runs as an external process: voxtype drives Cactus's `needle` CLI runner as a subprocess and reads the transcript from its JSON output, the same pattern as the whisper-cli backend.

## What is Whistle?

Whistle is a small on-device ASR model from Cactus Compute:

- **16.9 MB** — the smallest model voxtype can drive; it fits anywhere, from a phone-class machine to a desktop
- **Seven languages** — English, German, French, Spanish, Italian, Dutch, Polish
- **Apache-2.0** — both the weights and the runner engine are permissively licensed
- **CPU only** — no GPU and no ONNX Runtime; voxtype adds no native dependency
- **One pass up to 30 seconds** of audio, windowed beyond that
- **Keyword biasing and word timestamps** are available through the runner

## Why Use Whistle?

Whistle is a good fit if you want:

- **A tiny footprint**: 16.9 MB of weights, no GPU, no ONNX Runtime. Whisper small is ~466 MB on disk and Parakeet is ~600 MB.
- **Seven-language dictation without a large model**: English, German, French, Spanish, Italian, Dutch, and Polish, with automatic language detection.
- **A vocabulary nudge**: a plain text file of names and phrases (`keywords_file`) biases the model toward the words you actually use.
- **Per-word timing**: `word_timestamps = true` adds start/end times and per-word probabilities to the runner's output.

Whistle is not the best choice if you need:

- **Maximum accuracy or broad language coverage**: Whisper (99+ languages) and Parakeet are stronger on longer or unusual audio.
- **Live streaming partials**: voxtype transcribes each recording in a single pass through the runner. There are no partials at the cursor while you speak.

## Privacy

Whistle is fully local — audio is written to a temporary WAV file and handed to the runner on your machine, and nothing leaves the box.

One caveat: the Cactus `needle` runner ships with **telemetry on by default**. Voxtype always launches it with `NEEDLE_TELEMETRY=0` and `DO_NOT_TRACK=1`, so no usage data is sent. This project is local-only by design; if you invoke `needle` yourself outside voxtype, set those two environment variables yourself. See [Telemetry](#telemetry).

## Requirements

- A voxtype binary with Whistle support (the engine is built into every release binary)
- The `needle` runner binary (~a few MB; not bundled with voxtype)
- The `whistle.cact` weights (16.9 MB; not bundled with voxtype)
- CPU only — no GPU, no ONNX Runtime

## Installing the Runner and Weights

Neither artifact is bundled with voxtype, and `voxtype setup model` does not fetch them. Download both yourself from HuggingFace — the runner and the weights are both Apache-2.0.

### 1. The runner engine (`needle`)

Download page: https://huggingface.co/Cactus-Compute/needle3

Pick the folder for your platform, for example `linux-x86_64/needle` or `macos-arm64/needle`. The same folder also carries `needle.h` and `libneedle.a` if you are building against the engine yourself; voxtype only needs the `needle` executable.

```bash
mkdir -p ~/.local/bin
curl -L https://huggingface.co/Cactus-Compute/needle3/resolve/main/linux-x86_64/needle \
  -o ~/.local/bin/needle
chmod +x ~/.local/bin/needle
```

### 2. The weights (`whistle.cact`)

Download page: https://huggingface.co/Cactus-Compute/whistle

Put `whistle.cact` in voxtype's models directory so the default `model = "whistle.cact"` resolves without an absolute path:

```bash
mkdir -p ~/.local/share/voxtype/models
curl -L https://huggingface.co/Cactus-Compute/whistle/resolve/main/whistle.cact \
  -o ~/.local/share/voxtype/models/whistle.cact
```

When `runner_path` is unset, voxtype looks for `needle` in these places, in order: on your `PATH`, then `./needle`, `/usr/local/bin/needle`, `/usr/bin/needle`, and `~/.local/bin/needle`. If your runner lives anywhere else, set `runner_path` in the `[whistle]` section.

## Configuration

Edit `~/.config/voxtype/config.toml`:

```toml
# Select Whistle as the transcription engine
engine = "whistle"

[whistle]
# Weights file: a short name is resolved in ~/.local/share/voxtype/models/,
# an absolute path is used as is.
model = "whistle.cact"

# Runner binary. Omit to search PATH and the common locations above.
# runner_path = "/usr/local/bin/needle"

# Force a language, or leave empty for auto-detect.
# language = "en"

# File of words/phrases to favour, one per line.
# keywords_file = "/home/me/voxtype-keywords.txt"

# word_timestamps = false
# threads = 4
# on_demand_loading = false
```

Restart the daemon:

```bash
systemctl --user restart voxtype
```

You can also switch the engine from the CLI, the same as any other engine:

```bash
voxtype config set engine whistle     # updates config.toml in place
voxtype --engine whistle daemon       # one-off override
```

## Configuration Reference

See [CONFIGURATION.md → [whistle]](CONFIGURATION.md#whistle) for the full field-by-field reference. Key settings:

| Field | Type | Default | Notes |
|---|---|---|---|
| `model` | String | `"whistle.cact"` | Path to `whistle.cact`, or a short filename resolved in voxtype's models directory |
| `runner_path` | Path (optional) | unset | Path to the `needle` runner; unset searches `PATH` and common locations |
| `language` | String | `""` (auto-detect) | One of `en`, `de`, `fr`, `es`, `it`, `nl`, `pl`; empty = auto-detect |
| `keywords_file` | Path (optional) | unset | Newline-separated words/phrases to favour |
| `word_timestamps` | Boolean | `false` | Include per-word timings |
| `threads` | Integer (optional) | unset (runner picks) | Runner worker threads |
| `on_demand_loading` | Boolean | `false` | Load only when recording starts |

## Available Languages

Whistle supports seven languages. Set `language` to one of these codes, or leave it empty to let the model detect the language.

| Code | Language |
|------|----------|
| `en` | English |
| `de` | German |
| `fr` | French |
| `es` | Spanish |
| `it` | Italian |
| `nl` | Dutch |
| `pl` | Polish |

## How Voxtype Calls the Runner

For each recording, voxtype writes the audio to a temporary 16 kHz mono WAV file and runs the runner, for example:

```bash
needle --model ~/.local/share/voxtype/models/whistle.cact --audio clip.wav
```

It adds the runner flags that your config implies:

| Config | Flag added |
|---|---|
| `language = "en"` (anything other than empty/`auto`) | `--audio-language en` |
| `keywords_file = "..."` | `--audio-keywords <file>` |
| `word_timestamps = true` | `--audio-word-timestamps` |
| `threads = 4` | `--threads 4` |

The runner prints a JSON object that voxtype parses:

```json
{"text": "hello world", "language": "en", "ttft_ms": 12.0, "decode_tps": 120.0}
```

With `word_timestamps = true`, the JSON also carries a `words` array whose entries have `word`, `start`, `end`, and `probability`:

```json
{"text": "hello world", "language": "en", "ttft_ms": 12.0, "decode_tps": 120.0,
 "words": [{"word": "hello", "start": 0.02, "end": 0.42, "probability": 0.98}]}
```

The runner itself exposes more switches than voxtype currently drives — `--audio-depth N` and `--audio-stream` among them. Use those directly if you run `needle` by hand.

## Telemetry

The `needle` runner has telemetry **on by default**. Voxtype always starts the runner with:

```bash
NEEDLE_TELEMETRY=0
DO_NOT_TRACK=1
```

so no usage data is reported. This is deliberate: voxtype is a local, on-device dictation tool and does not phone home. If you run `needle` yourself — for example to try a clip before wiring it into voxtype — export both variables first to keep it local.

## Known Limitations

- **One pass, no streaming.** Voxtype sends each recording to the runner once. Recordings up to 30 seconds go through in a single pass; longer audio is windowed. There are no live partials.
- **Seven languages only.** English, German, French, Spanish, Italian, Dutch, and Polish. For other languages use Whisper or a specialized engine.
- **Two external artifacts.** Neither the runner nor the weights ship with voxtype, and `voxtype setup model` does not download them — you install both yourself. If either is missing, the daemon reports it at startup and the engine fails to initialize.
- **Accuracy is model-class-appropriate.** A 16.9 MB model trades accuracy for size; expect more errors on unusual vocabulary and noisy audio than Whisper large or Parakeet.

## Switching Back to Whisper

Edit your config:

```toml
engine = "whisper"

[whisper]
model = "large-v3-turbo"
```

Or remove the `engine` line entirely, since Whisper is the default.

## Troubleshooting

### "Whistle engine selected but [whistle] config section is missing"

Add the `[whistle]` section to your config:

```toml
[whistle]
model = "whistle.cact"
```

### "needle runner not found"

Voxtype searched `PATH`, `./needle`, `/usr/local/bin/needle`, `/usr/bin/needle`, and `~/.local/bin/needle` and found nothing. Download the runner from https://huggingface.co/Cactus-Compute/needle3 (pick your platform's folder) and either put it on `PATH` / in one of those locations, or point at it explicitly:

```toml
[whistle]
runner_path = "/opt/whistle/needle"
```

### "Whistle model '...' not found"

Voxtype looks for the weights in `~/.local/share/voxtype/models/` (appending `.cact` if your `model` value has no extension) and then in the current directory. Download `whistle.cact` from https://huggingface.co/Cactus-Compute/whistle:

```bash
curl -L https://huggingface.co/Cactus-Compute/whistle/resolve/main/whistle.cact \
  -o ~/.local/share/voxtype/models/whistle.cact
```

Or set `model` to an absolute path.

### "needle failed: ..."

The runner exited non-zero and voxtype surfaced its stderr. Run the same command by hand to see the full error:

```bash
needle --model ~/.local/share/voxtype/models/whistle.cact --audio clip.wav
```

### Empty transcription output

Whistle expects 16 kHz mono audio, the same format as the other engines. Check the daemon logs:

```bash
journalctl --user -u voxtype --since "1 minute ago" | grep -i whistle
```

## Feedback

Please report issues at:
https://github.com/peteonrails/voxtype/issues

Include:
- Your CPU model and OS
- Which `needle` runner build you installed
- Whether `whistle.cact` loads when you run `needle` by hand
- Sample audio if possible (for accuracy issues)
