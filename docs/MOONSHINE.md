# Moonshine Backend (Experimental)

> **WARNING: Experimental Feature**
>
> Moonshine support is experimental. The API and configuration options may change in future releases.

Voxtype includes experimental support for Moonshine AI's speech recognition models as an alternative to Whisper and Parakeet. Moonshine is an encoder-decoder transformer that runs via ONNX Runtime and is optimized for fast inference on CPUs.

## What is Moonshine?

Moonshine is an automatic speech recognition model from Moonshine AI (Useful Sensors). It differs from Whisper in a few important ways:

- **Variable-length input**: Moonshine processes exactly the audio you give it, with no padding to 30 seconds like Whisper. This makes short utterances very fast.
- **Small model size**: The base model is 61M parameters (237MB on disk), compared to Whisper base at 142MB and Whisper large-v3-turbo at 1.6GB.
- **ONNX Runtime**: Same inference backend as Parakeet, so Moonshine shares execution provider support (CPU, CUDA, TensorRT) without adding new native dependencies.

## Why Use Moonshine?

Moonshine is a good fit if you want:

- **Very fast transcription on CPU**: 0.09 seconds for a 4-second recording on a Ryzen 9 9900X3D. For comparison, Whisper large-v3-turbo takes 17.7 seconds for the same audio on CPU.
- **Low memory usage**: The base model uses ~237MB of disk and loads quickly.
- **Multilingual support**: Moonshine has models for Japanese, Mandarin Chinese, Korean, Arabic, and more. Parakeet is English-only, and Whisper multilingual models are much larger.
- **Push-to-talk dictation**: For typical push-to-talk usage (1-10 second recordings), Moonshine's batch mode is fast enough that streaming adds no practical benefit.

Moonshine is not the best choice if you need:

- **Highest possible accuracy**: Whisper large-v3-turbo or Parakeet TDT will be more accurate, especially for longer recordings.
- **Punctuation and capitalization**: Moonshine outputs lowercase text without punctuation. Use voxtype's spoken punctuation feature or a post-processing command for cleanup.

## Requirements

- A Moonshine-enabled voxtype binary (compiled with `--features moonshine`)
- ~240MB disk space for the base model (~100MB for tiny)
- curl (for downloading models)

## Getting a Moonshine Binary

Moonshine support requires a binary compiled with the `moonshine` feature flag. This is not yet included in the standard release binaries.

**Build from source:**

```bash
# CPU-only
cargo build --release --features moonshine

# With CUDA GPU acceleration
cargo build --release --features moonshine-cuda

# With TensorRT GPU acceleration
cargo build --release --features moonshine-tensorrt
```

## Downloading Models

The recommended way to download models is through the setup tool:

```bash
voxtype setup model
```

This shows an interactive menu with all available engines and models. Select a Moonshine model to download and configure it automatically.

**Manual download (alternative):**

```bash
# Create model directory
mkdir -p ~/.local/share/voxtype/models/moonshine-base

# Download the three required files
cd ~/.local/share/voxtype/models/moonshine-base
curl -L https://huggingface.co/onnx-community/moonshine-base-ONNX/resolve/main/onnx/encoder_model.onnx -o encoder_model.onnx
curl -L https://huggingface.co/onnx-community/moonshine-base-ONNX/resolve/main/onnx/decoder_model_merged.onnx -o decoder_model_merged.onnx
curl -L https://huggingface.co/onnx-community/moonshine-base-ONNX/resolve/main/tokenizer.json -o tokenizer.json
```

## Configuration

Edit `~/.config/voxtype/config.toml`:

```toml
# Select Moonshine as the transcription engine
engine = "moonshine"

[moonshine]
# Model name (looked up in ~/.local/share/voxtype/models/moonshine-{name}/)
model = "base"

# Use quantized model files if available (default: true)
# Falls back to full precision if quantized files are not found
quantized = true
```

Restart the daemon:

```bash
systemctl --user restart voxtype
```

You can also override the engine for a single transcription:

```bash
voxtype transcribe recording.wav --engine moonshine
```

## Available Models

### English Models (MIT License)

| Model | Params | Size | Description |
|-------|--------|------|-------------|
| `base` | 61M | 237 MB | Good accuracy, fast inference (recommended) |
| `tiny` | 27M | 100 MB | Fastest, lower accuracy |

English models are MIT-licensed and can be used freely for any purpose, including commercial use.

### Multilingual Models (Community License)

| Model | Language | Size | HuggingFace Repo |
|-------|----------|------|------------------|
| `base-ja` | Japanese | 237 MB | onnx-community/moonshine-base-ja-ONNX |
| `base-zh` | Mandarin Chinese | 237 MB | onnx-community/moonshine-base-zh-ONNX |
| `tiny-ja` | Japanese | 100 MB | onnx-community/moonshine-tiny-ja-ONNX |
| `tiny-zh` | Mandarin Chinese | 100 MB | onnx-community/moonshine-tiny-zh-ONNX |
| `tiny-ko` | Korean | 100 MB | onnx-community/moonshine-tiny-ko-ONNX |
| `tiny-ar` | Arabic | 100 MB | onnx-community/moonshine-tiny-ar-ONNX |

Additional languages (Spanish, Vietnamese, Ukrainian) exist as PyTorch models but do not yet have ONNX exports available for direct use with voxtype.

## Licensing

Moonshine models have two different licenses depending on the language:

**English models (tiny, base):** MIT License. You can use these models for any purpose, including commercial use, without restriction.

**Non-English models:** Moonshine Community License. This license allows free use for personal, academic, and non-commercial purposes. Commercial use of non-English models requires a separate license agreement with Moonshine AI. See the model card on HuggingFace for full license terms.

When downloading a non-English model through `voxtype setup model`, voxtype will display a warning about the license and ask for confirmation before proceeding.

## Performance

Tested on Ryzen 9 9900X3D (CPU-only, no GPU):

| Engine | Model | Time (4s audio) | Model Size |
|--------|-------|-----------------|------------|
| Moonshine | base | 0.09s | 237 MB |
| Parakeet | TDT 0.6B int8 | 0.3-0.5s | 670 MB |
| Whisper | large-v3-turbo | 17.7s | 1.6 GB |

Moonshine encoder completes in roughly 10ms, and the decoder runs about 6ms per token. For typical push-to-talk recordings of 1-10 seconds, the total transcription time is well under a second.

## Known Limitations

### No Punctuation or Capitalization

Moonshine outputs lowercase text without punctuation. If you need punctuation, enable voxtype's spoken punctuation feature:

```toml
[text]
spoken_punctuation = true
```

Or use a post-processing command to clean up the output.

### Two Model Sizes Only

Moonshine comes in only two sizes: tiny (27M params) and base (61M params). There are no medium or large variants. The models were designed for edge devices, prioritizing speed over maximum accuracy.

### Streaming Not Yet Available

Moonshine v2 supports streaming inference with sliding-window attention, but the streaming ONNX model exports are not yet available. Batch-mode transcription is fast enough for push-to-talk that this is not a practical limitation.

## Switching Back to Whisper

Edit your config:

```toml
engine = "whisper"

[whisper]
model = "large-v3-turbo"
```

Or remove the `engine` line entirely, since Whisper is the default.

## Troubleshooting

### "Moonshine engine requested but voxtype was not compiled with --features moonshine"

You need a Moonshine-enabled binary. Build from source with `--features moonshine`, or download a Moonshine binary from the releases page once they become available.

### "Moonshine engine selected but [moonshine] config section is missing"

Add the `[moonshine]` section to your config:

```toml
[moonshine]
model = "base"
```

### Model not found

Ensure the model is in the correct location:

```bash
ls ~/.local/share/voxtype/models/moonshine-base/
# Should show: encoder_model.onnx, decoder_model_merged.onnx, tokenizer.json
```

You can also run `voxtype setup model` to download models interactively.

### Empty transcription output

If Moonshine returns empty text, check that your audio is 16kHz mono. Moonshine expects the same audio format as Whisper and Parakeet. Check the daemon logs:

```bash
journalctl --user -u voxtype --since "1 minute ago" | grep -i moonshine
```

## Feedback

Moonshine support is experimental. Please report issues at:
https://github.com/peteonrails/voxtype/issues

Include:
- Your CPU model
- The Moonshine model you're using (tiny/base)
- Sample audio if possible (for accuracy issues)
