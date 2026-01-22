# Parakeet Backend (Experimental)

> **WARNING: Experimental Feature**
>
> Parakeet support is experimental and not yet fully integrated into voxtype's setup system. Configuration requires manual editing of config files. The API and configuration options may change in future releases. Use at your own risk.

Voxtype 0.5.0+ includes experimental support for NVIDIA's Parakeet ASR models as an alternative to Whisper. Parakeet uses ONNX Runtime and offers excellent CPU performance without requiring a GPU.

## What is Parakeet?

Parakeet is NVIDIA's FastConformer-based speech recognition model. The TDT (Token-and-Duration Transducer) variant provides:

- Fast CPU inference with AVX-512 optimization
- Proper punctuation and capitalization
- Good accuracy for English dictation
- No GPU required (though CUDA acceleration is available)

## Requirements

- A Parakeet-enabled voxtype binary (see below)
- ~600MB disk space for the model
- CPU with AVX2 or AVX-512 (AVX-512 recommended for best performance)

## Getting a Parakeet Binary

Parakeet support requires a specially compiled binary. Download from the releases page:

| Binary | Use Case |
|--------|----------|
| `voxtype-*-parakeet-avx2` | Most CPUs (Intel Haswell+, AMD Zen+) |
| `voxtype-*-parakeet-avx512` | Modern CPUs with AVX-512 (Intel Ice Lake+, AMD Zen 4+) |
| `voxtype-*-parakeet-cuda` | NVIDIA GPU acceleration with CPU fallback |

The AVX2 binary works on most modern x86_64 CPUs. Use AVX-512 if your CPU supports it for better performance.

## Downloading the Model

Download the Parakeet TDT 0.6B model:

```bash
# Create models directory
mkdir -p ~/.local/share/voxtype/models

# Download and extract the model
cd ~/.local/share/voxtype/models
curl -L https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2/resolve/main/onnx/encoder-model.onnx -o encoder-model.onnx
curl -L https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2/resolve/main/onnx/encoder-model.onnx.data -o encoder-model.onnx.data
curl -L https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2/resolve/main/onnx/decoder_joint-model.onnx -o decoder_joint-model.onnx

# Or download the full directory structure
# The model should be at: ~/.local/share/voxtype/models/parakeet-tdt-0.6b-v2/
```

Alternatively, use a v3 model if available:

```bash
mkdir -p ~/.local/share/voxtype/models/parakeet-tdt-0.6b-v3
cd ~/.local/share/voxtype/models/parakeet-tdt-0.6b-v3
# Download encoder-model.onnx, encoder-model.onnx.data, decoder_joint-model.onnx
```

## Switching to a Parakeet Binary

The standard voxtype binary does not include Parakeet support. You must switch to a Parakeet-enabled binary.

**Manual switching (until `voxtype setup engine` is implemented):**

```bash
# Download the Parakeet binary for your CPU
# Example: AVX-512 capable CPU
curl -L https://github.com/peteonrails/voxtype/releases/download/v0.5.0/voxtype-0.5.0-linux-x86_64-parakeet-avx512 \
  -o /tmp/voxtype-parakeet

# Make executable and install
chmod +x /tmp/voxtype-parakeet
sudo mv /tmp/voxtype-parakeet /usr/local/bin/voxtype

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

You're using a standard voxtype binary without Parakeet support. Download a `parakeet-*` binary from the releases page.

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
# Should show: encoder-model.onnx, encoder-model.onnx.data, decoder_joint-model.onnx
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

Parakeet support is experimental. Please report issues at:
https://github.com/peteonrails/voxtype/issues

Include:
- Your CPU model
- Which binary you're using (avx2/avx512/cuda)
- The Parakeet model version
- Sample audio if possible (for accuracy issues)
