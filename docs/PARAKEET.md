# Parakeet Backend

Voxtype supports NVIDIA's Parakeet ASR models as an alternative to Whisper. Parakeet uses ONNX Runtime and offers excellent CPU performance without requiring a GPU.

## What is Parakeet?

Parakeet is NVIDIA's FastConformer-based speech recognition model. The TDT (Token-and-Duration Transducer) variant provides:

- Fast CPU inference with AVX-512 optimization
- Proper punctuation and capitalization
- Good accuracy for English dictation
- No GPU required (though CUDA acceleration is available)

## Requirements

- An ONNX-enabled voxtype binary (see below)
- ~600MB disk space for the model
- CPU with AVX2 or AVX-512 (AVX-512 recommended for best performance)

## Getting a Parakeet Binary

Parakeet support requires an ONNX-enabled binary. Download from the releases page:

| Binary | Use Case |
|--------|----------|
| `voxtype-*-onnx-avx2` | Most CPUs (Intel Haswell+, AMD Zen+) |
| `voxtype-*-onnx-avx512` | Modern CPUs with AVX-512 (Intel Ice Lake+, AMD Zen 4+) |
| `voxtype-*-onnx-cuda` | NVIDIA GPU acceleration with CPU fallback |

The AVX2 binary works on most modern x86_64 CPUs. Use AVX-512 if your CPU supports it for better performance.

## Downloading the Model

Download the Parakeet TDT 0.6B model:

```bash
# Create models directory
mkdir -p ~/.local/share/voxtype/models

# Download and extract the model
cd ~/.local/share/voxtype/models
curl -L https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/encoder-model.onnx -o encoder-model.onnx
curl -L https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/encoder-model.onnx.data -o encoder-model.onnx.data
curl -L https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/decoder_joint-model.onnx -o decoder_joint-model.onnx
curl -L https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/vocab.txt -o vocab.txt
curl -L https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/config.json -o config.json

# Or download the full directory structure
# The model should be at: ~/.local/share/voxtype/models/parakeet-tdt-0.6b-v2/
```

Alternatively, use the v3 model (https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx):

```bash
mkdir -p ~/.local/share/voxtype/models/parakeet-tdt-0.6b-v3
cd ~/.local/share/voxtype/models/parakeet-tdt-0.6b-v3
# Download encoder-model.onnx, encoder-model.onnx.data, decoder_joint-model.onnx, vocab.txt, config.json
```

## Switching to a Parakeet Binary

The standard voxtype binary does not include Parakeet support. You must switch to an ONNX-enabled binary.

**Manual switching (until `voxtype setup engine` is implemented):**

```bash
# Download the Parakeet binary for your CPU
# Example: AVX-512 capable CPU
curl -L https://github.com/peteonrails/voxtype/releases/download/v0.6.3/voxtype-0.6.3-linux-x86_64-onnx-avx512 \
  -o /tmp/voxtype-onnx

# Make executable and install
chmod +x /tmp/voxtype-onnx
sudo mv /tmp/voxtype-onnx /usr/local/bin/voxtype

# Restart the daemon
systemctl --user restart voxtype

# Verify
voxtype --version
```

To switch back to Whisper, download and install the standard binary (avx2, avx512, or vulkan).

## Configuration

Edit `~/.config/voxtype/config.toml`:

```toml
# Select Parakeet as the transcription engine
engine = "parakeet"

[parakeet]
# Model name (looked up in ~/.local/share/voxtype/models/)
model = "parakeet-tdt-0.6b-v3"

# Or use an absolute path
# model_path = "/path/to/parakeet-tdt-0.6b-v3"
```

Restart the daemon:

```bash
systemctl --user restart voxtype
```

Verify Parakeet is active:

```bash
journalctl --user -u voxtype --since "1 minute ago" | grep -i parakeet
# Should show: "Loading Parakeet Tdt model from..."
```

## Performance

Tested on Ryzen 9 9900X3D (AVX-512):

| Audio Length | Transcription Time | Real-time Factor |
|--------------|-------------------|------------------|
| 1-2s | 0.06-0.09s | ~20x |
| 3-4s | 0.11-0.13s | ~30x |
| 5s | 0.15s | ~33x |

Model load time: ~1.2 seconds (one-time at daemon startup)

### Comparison with Whisper

| Engine | Backend | Typical Speed | GPU Required |
|--------|---------|---------------|--------------|
| Whisper small | CPU | ~3x real-time | No |
| Whisper small | Vulkan | ~60x real-time | Yes |
| Parakeet TDT | CPU (AVX-512) | ~30x real-time | No |
| Parakeet TDT | CUDA | ~80x real-time | Yes (NVIDIA) |

Parakeet on CPU is significantly faster than Whisper on CPU, and competitive with Whisper on GPU.

## Known Limitations

### Repetition Hallucination

Parakeet can hallucinate extra repetitions when you speak repeated words. For example, saying "no no no no no" might transcribe as many more "no"s than you actually said. This is a known issue with many ASR models.

### Proper Noun Handling

Uncommon names and technical terms may be substituted with phonetically similar common words. For example:
- "Krzyzewski" → "Krasiewski"
- "Nguyen" → "Gwen"

### English Only

Parakeet TDT models are English-only. For multilingual support, use Whisper.

### Model Size

The Parakeet TDT 0.6B model is ~600MB, compared to Whisper small at ~500MB. Larger Parakeet models are available but not yet tested with voxtype.

## Switching Back to Whisper

To switch back to Whisper, edit your config:

```toml
engine = "whisper"

[whisper]
model = "small"
```

Or simply remove the `engine` line (Whisper is the default).

## Troubleshooting

### "Parakeet engine requested but voxtype was not compiled with --features parakeet"

You're using a standard voxtype binary without Parakeet support. Download an `onnx-*` binary from the releases page.

### "Parakeet engine selected but [parakeet] config section is missing"

Add the `[parakeet]` section to your config:

```toml
[parakeet]
model = "parakeet-tdt-0.6b-v3"
```

### Model not found

Ensure the model is in the correct location:

```bash
ls ~/.local/share/voxtype/models/parakeet-tdt-0.6b-v3/
# Should show: encoder-model.onnx, encoder-model.onnx.data, decoder_joint-model.onnx, vocab.txt, config.json
```

### SIGILL crash on older CPUs

Parakeet binaries include ONNX Runtime, which contains AVX-512 optimized code paths. ONNX Runtime performs CPU feature detection at runtime and should only execute instructions your CPU supports.

If you experience a SIGILL (illegal instruction) crash, this is likely a bug in ONNX Runtime's CPU detection rather than a fundamental incompatibility. As a workaround, switch to a Whisper binary:

- `voxtype-*-avx2` - Works on Intel Haswell+ and AMD Zen+
- `voxtype-*-vulkan` - GPU acceleration for AMD/Intel GPUs

Please report the issue at https://github.com/peteonrails/voxtype/issues with:
- Your CPU model (`cat /proc/cpuinfo | grep "model name" | head -1`)
- Which Parakeet binary you were using
- The full error output

## Feedback

Please report issues at:
https://github.com/peteonrails/voxtype/issues

Include:
- Your CPU model
- Which binary you're using (avx2/avx512/cuda)
- The Parakeet model version
- Sample audio if possible (for accuracy issues)

## Orukeet r3 INT8 (local ONNX import)

Orukeet is a Parakeet TDT v3 adaptation. Its published Sherpa-ONNX archive
contains a compatible encoder but separate decoder and joiner graphs. Renaming
those files alone does not make the archive loadable by `parakeet-rs`.

The offline importer composes the decoder and joiner, maps their tensor names to
the existing Parakeet interface, and copies the encoder and vocabulary unchanged.
It does not retrain or requantize the model. Python is needed only for this import;
normal transcription uses Voxtype's existing CPU/ONNX runtime.

Download the pinned publisher manifest and r3 archive (about 487 MB):

```bash
curl --fail --location --connect-timeout 10 --max-time 30 \
  'https://huggingface.co/oruk/orukeet/resolve/55a984d46f68323301837194ce647c702f55facc/onnx/manifest.json' \
  --output orukeet-release-manifest.json
curl --fail --location \
  'https://huggingface.co/oruk/orukeet/resolve/55a984d46f68323301837194ce647c702f55facc/onnx/sherpa-onnx-orukeet-v0.1.0-int8.tar.bz2' \
  --output sherpa-onnx-orukeet-v0.1.0-int8.tar.bz2
```

From the Voxtype source checkout, import it with `uv` (Python 3.12 or newer):

```bash
uv run --script scripts/import-orukeet-onnx.py \
  --archive sherpa-onnx-orukeet-v0.1.0-int8.tar.bz2 \
  --manifest orukeet-release-manifest.json \
  --output "$HOME/.local/share/voxtype/models/orukeet-r3-int8"
```

The importer verifies the release manifest's pinned byte count and SHA256, then
uses its archive size and checksum to validate the local archive. The publisher
requests this manifest download for Hugging Face download accounting. Conversion
and transcription stay offline. Save the manifest with the archive for later
offline imports.

The pinned archive SHA256 is
`f9191f30178cc9122ce2f023bf9fefafc822028307b0efa4caff645ba3fe8d0a`.
The importer refuses existing output directories and checks eight decoder/joiner
steps on CPU before publishing the converted directory. Logits and recurrent states must
be finite in both the source and combined graphs, even when they otherwise match.
Allow about 2 GB of temporary free disk space during import, in addition to the
downloaded archive. A
`VOXTYPE-CONVERSION.json` file records provenance, tool versions, file hashes,
and the numerical check results. This check is not an ASR accuracy benchmark.

Use an ONNX-enabled Voxtype binary with a separate trial config:

```toml
engine = "parakeet"

[parakeet]
model = "/absolute/path/to/orukeet-r3-int8"
model_type = "tdt"
streaming = false
```

Smoke-test saved audio without changing the running daemon:

```bash
voxtype --config /path/to/orukeet-trial.toml transcribe /path/to/mono-16khz.wav
```

The one-shot command prints the transcript: do not capture private audio or
transcripts in public issue/PR artifacts. Measure warmed inference separately
from model startup when comparing resident dictation latency.

Limitations and licensing:

- This is a local custom-model import, not a new engine or a download-picker entry.
  Voxtype's model catalog uses maintainer-controlled, checksum-verified hosting;
  a catalog entry requires a separately published converted artifact and manifest.
- The import targets this exact archive. Other Orukeet exports need their own
  compatibility checks. GPU execution and cache-aware streaming are not validated.
- Orukeet's weights and fitted kernels are CC BY-SA 4.0. The importer preserves
  `LICENSE-WEIGHTS` and `NOTICE.md`; retain those notices with redistributed models.
- Keep the previous Parakeet model/config until a paired benchmark with reviewed
  reference transcripts establishes an accuracy benefit for your own speech.

Run the converter's small, model-free CPU tests in an environment containing the
script's pinned dependencies:

```bash
python -m unittest discover -s scripts/tests -p 'test_import_orukeet_onnx.py' -v
```

### Orukeet CPU benchmark evidence

See the [September 12, 2026 comparison](benchmarks/orukeet-onnx-20260912.md)
for public reference-scored accuracy, pinned dataset links, per-clip evidence,
and separate warm CPU and whole-CLI timings. The small convenience sample is
not a recommendation to change the default model.
