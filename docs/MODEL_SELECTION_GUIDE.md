# Model Selection Guide

This guide helps you choose the right transcription engine and model for voxtype v0.6.0. The choice depends on your language, hardware, and how you use dictation.

Voxtype has seven transcription engines. Two are bundled with the standard binary (Whisper and Remote Whisper). The other five require the ONNX binary variant.

---

## Quick Comparison

| Engine | Languages | Architecture | Model Size | Speed (CPU) | Punctuation | Binary |
|--------|-----------|-------------|------------|-------------|-------------|--------|
| **Whisper** | 99+ | Encoder-decoder | 75 MB - 3.1 GB | Moderate | No (use post-process) | Standard |
| **Parakeet** | 25 European | TDT/CTC | 670 MB - 2.6 GB | Fast | Yes (TDT) | ONNX |
| **Moonshine** | en, ja, zh, ko, ar | Encoder-decoder | 100 - 237 MB | Fast | No | ONNX |
| **SenseVoice** | zh, en, ja, ko, yue | Encoder-only CTC | 239 - 938 MB | Fast | Yes (ITN) | ONNX |
| **Paraformer** | zh, en | Encoder-predictor-decoder | 220 - 487 MB | Fast | No | ONNX |
| **Dolphin** | 40+ langs, 22 Chinese dialects | CTC E-Branchformer | 198 MB | Fast | No | ONNX |
| **Omnilingual** | 1600+ | CTC wav2vec2 | 3.9 GB | Moderate | No | ONNX |

---

## Which Engine Should I Use?

Start here and follow the path that matches your situation.

```
What language(s) do you speak?
│
├─ English only
│   ├─ Want best accuracy + punctuation? → Parakeet TDT (ONNX binary)
│   ├─ Want smallest/fastest model?      → Moonshine tiny (ONNX binary)
│   ├─ Want simplest setup?              → Whisper small.en (standard binary)
│   └─ On a laptop / saving battery?     → Whisper small.en + on_demand_loading
│
├─ Chinese (Mandarin or Cantonese)
│   ├─ Chinese + English mixed?          → Paraformer zh or SenseVoice
│   ├─ Chinese + Japanese/Korean?        → SenseVoice
│   ├─ Cantonese specifically?           → SenseVoice (supports yue)
│   └─ Chinese dialects?                 → Dolphin
│
├─ Japanese or Korean
│   ├─ Want a small, fast model?         → Moonshine (tiny-ja/tiny-ko) or SenseVoice
│   └─ Want best quality?                → Whisper large-v3-turbo or SenseVoice
│
├─ European languages (German, French, Spanish, etc.)
│   ├─ Want punctuation + fast?          → Parakeet TDT v3
│   └─ Want broadest coverage?           → Whisper
│
├─ Arabic, Hindi, Thai, Vietnamese, other Asian languages
│   ├─ One of the 40 Dolphin languages?  → Dolphin (small, fast)
│   └─ Otherwise                         → Whisper large-v3-turbo
│
└─ Rare or uncommon language
    └─ → Omnilingual (1600+ languages) or Whisper
```

---

## Standard vs ONNX Binary

Voxtype ships two binary variants:

- **Standard binary** -- includes Whisper (whisper.cpp). This is the default.
- **ONNX binary** -- includes Whisper plus all ONNX-based engines (Parakeet, Moonshine, SenseVoice, Paraformer, Dolphin, Omnilingual).

The ONNX binary is larger because it bundles ONNX Runtime. If you only use Whisper, the standard binary is all you need.

To switch to the ONNX binary:

```bash
sudo voxtype setup onnx --enable
```

To switch back to the standard binary:

```bash
sudo voxtype setup onnx --disable
```

The `setup onnx` command swaps the `/usr/local/bin/voxtype` symlink between the two binaries. Both binaries are installed by the `voxtype-bin` AUR package and the deb/rpm packages.

---

## Engine Details

### 1. Whisper

OpenAI's Whisper via whisper.cpp. The default engine, included in every voxtype binary.

**Languages:** 99+ (the widest coverage of any engine)

**Available models:**

| Model | Size | Speed | Languages |
|-------|------|-------|-----------|
| tiny | 75 MB | ~10x | 99+ |
| tiny.en | 39 MB | ~10x | English only |
| base | 142 MB | ~7x | 99+ |
| base.en | 142 MB | ~7x | English only |
| small | 466 MB | ~4x | 99+ |
| small.en | 466 MB | ~4x | English only |
| medium | 1.5 GB | ~2x | 99+ |
| medium.en | 1.5 GB | ~2x | English only |
| large-v3 | 3.1 GB | 1x | 99+ |
| large-v3-turbo | 1.6 GB | ~8x | 99+ |

Speed is relative to large-v3 (1x baseline). Higher is faster.

The `.en` models are English-only but faster and more accurate for English. Models without `.en` support 99+ languages. There are no `.en` variants for large-v3 or large-v3-turbo.

**Config example:**

```toml
# English-only, good CPU performance
[whisper]
model = "small.en"
language = "en"
```

```toml
# Multilingual with GPU
[whisper]
model = "large-v3-turbo"
language = "auto"    # or "ja", "zh", "ko", "ar", etc.
```

```toml
# Battery-saving laptop mode
[whisper]
model = "small.en"
language = "en"
on_demand_loading = true
gpu_isolation = true
```

**Pros:**
- Widest language support (99+ languages)
- Works with every voxtype binary (no ONNX needed)
- Many model sizes to fit different hardware
- GPU support via Vulkan, CUDA, or ROCm
- Well-tested, mature codebase

**Cons:**
- Slower than Parakeet for equivalent accuracy
- No built-in punctuation (use `[output.post_process]` or `[text] spoken_punctuation = true`)
- Larger models need significant VRAM

**GPU options:** Build with `--features gpu-vulkan` (AMD/Intel), `--features gpu-cuda` (NVIDIA), or `--features gpu-hipblas` (AMD ROCm).

---

### 2. Parakeet

NVIDIA's FastConformer model via the parakeet-rs crate. State-of-the-art accuracy for English and European languages, with built-in punctuation from the TDT decoder.

**Languages:** 25 European languages -- English, German, French, Spanish, Italian, Dutch, Polish, Portuguese, Romanian, Czech, Hungarian, Slovak, Slovenian, Danish, Norwegian, Swedish, Finnish, Greek, Turkish, Ukrainian, Russian, Catalan, Galician, Basque.

**If your language is not in this list, Parakeet will not work for you.**

**Available models:**

| Model | Size | Description |
|-------|------|-------------|
| parakeet-tdt-0.6b-v3 | 2.6 GB | TDT with punctuation (recommended) |
| parakeet-tdt-0.6b-v3-int8 | 670 MB | Quantized, smaller and faster |

**TDT vs CTC:** The TDT model includes punctuation and capitalization. The CTC variant is slightly faster but outputs raw text without punctuation. Use TDT unless you have a specific reason not to.

**Config example:**

```toml
engine = "parakeet"

[parakeet]
model = "parakeet-tdt-0.6b-v3"
```

```toml
# Quantized for lower memory usage
engine = "parakeet"

[parakeet]
model = "parakeet-tdt-0.6b-v3-int8"
```

**Pros:**
- Best accuracy for English (~6% WER, top of HuggingFace ASR leaderboard)
- Built-in punctuation and capitalization (TDT)
- Fast even on CPU thanks to efficient FastConformer architecture
- GPU acceleration via CUDA, ROCm, or TensorRT

**Cons:**
- Limited to 25 European languages (no CJK, Arabic, Hindi, etc.)
- Requires ONNX binary
- Only one model size (0.6B parameters)

**GPU builds:** The ONNX binary variants include GPU support. `onnx-cuda` for NVIDIA, `onnx-rocm` for AMD.

---

### 3. Moonshine

Encoder-decoder transformer optimized for edge devices. Processes variable-length audio without the 30-second padding that Whisper requires.

**Languages:** English (MIT license), plus Japanese, Chinese, Korean, Arabic (Moonshine Community License, non-commercial only).

**Available models:**

| Model | Size | Language | License |
|-------|------|----------|---------|
| base | 237 MB | English | MIT |
| tiny | 100 MB | English | MIT |
| base-ja | 237 MB | Japanese | Community (non-commercial) |
| base-zh | 237 MB | Chinese | Community (non-commercial) |
| tiny-ja | 100 MB | Japanese | Community (non-commercial) |
| tiny-zh | 100 MB | Chinese | Community (non-commercial) |
| tiny-ko | 100 MB | Korean | Community (non-commercial) |
| tiny-ar | 100 MB | Arabic | Community (non-commercial) |

**Config example:**

```toml
engine = "moonshine"

[moonshine]
model = "base"         # English, 237 MB
# quantized = true     # Use quantized variant if available (default: true)
# threads = 4          # CPU threads for ONNX Runtime
```

```toml
# Japanese
engine = "moonshine"

[moonshine]
model = "base-ja"
```

**Pros:**
- Small model sizes (100-237 MB)
- Fast inference, designed for edge devices
- No 30-second padding overhead
- English models are MIT licensed

**Cons:**
- No built-in punctuation
- Non-English models are non-commercial only
- Fewer language options than Whisper or SenseVoice
- Requires ONNX binary

---

### 4. SenseVoice

Alibaba's FunAudioLLM encoder-only CTC model. Single forward pass (no autoregressive decoder loop), so inference is fast.

**Languages:** Chinese (zh), English (en), Japanese (ja), Korean (ko), Cantonese (yue). Automatic language detection or explicit selection.

**Available models:**

| Model | Size | Description |
|-------|------|-------------|
| small | 239 MB | Quantized int8 (recommended) |
| small-fp32 | 938 MB | Full precision, slightly better accuracy |

**Config example:**

```toml
engine = "sensevoice"

[sensevoice]
model = "sensevoice-small"
language = "auto"      # or "zh", "en", "ja", "ko", "yue"
use_itn = true         # Inverse text normalization (adds punctuation)
# threads = 4
```

**Pros:**
- Good CJK support (Chinese, Japanese, Korean, Cantonese)
- Fast single-pass inference (encoder-only, no decoder loop)
- Inverse text normalization adds punctuation
- Small quantized model (239 MB)
- Automatic language detection

**Cons:**
- Limited to 5 languages
- CJK output is character-based with no spaces between words (see note below)
- Requires ONNX binary

---

### 5. Paraformer

Alibaba's non-autoregressive encoder-predictor-decoder model. Generates all output tokens in a single pass.

**Languages:** Chinese + English (bilingual).

**Available models:**

| Model | Size | Languages | Description |
|-------|------|-----------|-------------|
| zh | 487 MB | Chinese + English | Bilingual, recommended |
| en | 220 MB | English | English only |

**Config example:**

```toml
engine = "paraformer"

[paraformer]
model = "paraformer-zh"    # Chinese + English
# threads = 4
```

**Pros:**
- Good Chinese + English bilingual support
- Non-autoregressive: fast, single-pass decoding
- Moderate model size

**Cons:**
- Only Chinese and English
- No built-in punctuation
- Chinese output is character-based with no spaces (see note below)
- Requires ONNX binary

---

### 6. Dolphin

DataoceanAI's CTC E-Branchformer model optimized for Eastern languages. Covers 40 languages plus 22 Chinese dialects.

**Languages:** Chinese (Mandarin + 22 dialects), Japanese, Korean, Thai, Vietnamese, Indonesian, Malay, Arabic, Hindi, Urdu, Bengali, Tamil, and more.

**Available models:**

| Model | Size | Languages | Description |
|-------|------|-----------|-------------|
| base | 198 MB | 40+ languages | Dictation-optimized, int8 quantized |

**Config example:**

```toml
engine = "dolphin"

[dolphin]
model = "dolphin-base"
# threads = 4
```

**Pros:**
- Broad Eastern language coverage (40 languages + 22 Chinese dialects)
- Small model size (198 MB)
- Optimized for dictation use cases

**Cons:**
- CJK output is character-based with no spaces (see note below)
- No built-in punctuation
- Single model option
- Requires ONNX binary

---

### 7. Omnilingual

Meta's Massively Multilingual Speech (MMS) wav2vec2 model. A single model that covers 1600+ languages with a character-level CTC tokenizer.

**Languages:** 1600+ (language-agnostic, no language selection needed).

**Available models:**

| Model | Size | Parameters | Description |
|-------|------|------------|-------------|
| 300m | 3.9 GB | 300M | 1600+ languages |

**Config example:**

```toml
engine = "omnilingual"

[omnilingual]
model = "omnilingual-large"
# threads = 4
```

**Pros:**
- Widest language coverage of any engine (1600+ languages)
- Language-agnostic: no language selection needed, just speak
- Single model covers everything

**Cons:**
- Large model (3.9 GB)
- Character-level output (no word boundaries for many languages)
- No built-in punctuation
- Accuracy varies by language; less accurate than specialized models for common languages
- Requires ONNX binary

---

## Notes

### Character-Based CJK Output

SenseVoice, Paraformer, and Dolphin produce character-based output for Chinese, Japanese, and Korean. This means the transcribed text has no spaces between words:

```
English: "I went to the store"
Chinese: "我去了商店"          (no spaces, which is correct for Chinese)
Japanese: "お店に行きました"    (no spaces)
```

This is standard for CJK text and generally what you want. If your workflow requires word-segmented output, you would need external post-processing.

### Downloading Models

Use the interactive model selector to browse and download models for any engine:

```bash
voxtype setup model
```

This shows all available models across all engines, marks installed models, and handles downloads from HuggingFace.

### On-Demand Loading

All engines support `on_demand_loading = true`, which loads the model only when you start recording and unloads it after transcription. This saves memory and battery on laptops at the cost of a short delay before the first transcription.

### Common Config Options

Every ONNX engine supports these options:

| Option | Default | Description |
|--------|---------|-------------|
| `model` | varies | Model name or path to model directory |
| `threads` | auto | Number of CPU threads for ONNX Runtime |
| `on_demand_loading` | false | Load model on demand to save memory |

Whisper has additional options (`language`, `gpu_isolation`, `mode`, etc.) documented in the [Configuration Guide](CONFIGURATION.md).

---

## Hardware Recommendations

### VRAM Requirements

| Engine / Model | Minimum VRAM |
|----------------|-------------|
| Whisper tiny/base | 1 GB |
| Whisper small | 2 GB |
| Whisper medium | 5 GB |
| Whisper large-v3-turbo | 6 GB |
| Whisper large-v3 | 10 GB |
| Parakeet TDT 0.6B | 4 GB (GPU), works on CPU |
| Parakeet TDT 0.6B int8 | 2 GB (GPU), works on CPU |
| Moonshine / SenseVoice / Paraformer / Dolphin | CPU-friendly, no GPU needed |
| Omnilingual 300M | 4+ GB (GPU), works on CPU |

### CPU Performance (10 seconds of audio, modern CPU)

| Engine / Model | Approx Time |
|----------------|-------------|
| Whisper tiny | 1-2s |
| Whisper base | 2-3s |
| Whisper small | 4-6s |
| Whisper medium | 10-15s |
| Parakeet TDT (CPU) | 2-4s |
| Parakeet TDT (CUDA) | <1s |
| Moonshine base | 1-2s |
| SenseVoice small | 1-2s |
| Paraformer zh | 1-3s |
| Dolphin base | 1-2s |

### Desktop with GPU

Use the largest model your GPU can hold. For English, Parakeet TDT with CUDA is the fastest option. For multilingual, Whisper large-v3-turbo with Vulkan or CUDA.

### Laptop on Battery

```toml
[whisper]
model = "small.en"
language = "en"
on_demand_loading = true
gpu_isolation = true
```

Or for an ONNX engine:

```toml
engine = "moonshine"

[moonshine]
model = "tiny"
on_demand_loading = true
```

---

## Troubleshooting

### "Engine requested but voxtype was not compiled with --features ..."

You are running the standard (Whisper-only) binary and selected an ONNX engine. Switch to the ONNX binary:

```bash
sudo voxtype setup onnx --enable
```

### "My transcription is slow"

1. Try a smaller model (base instead of small, tiny instead of base).
2. Enable GPU acceleration if available.
3. For English, Parakeet on CPU is often faster than Whisper at similar accuracy.
4. For CJK, SenseVoice and Dolphin are fast single-pass models.

### "My transcription has errors"

1. Try a larger model.
2. Use an engine specialized for your language (SenseVoice for CJK, Parakeet for European).
3. For Whisper, use the `.en` model if you only speak English.
4. Check that `language` is set correctly.
5. Try `[output.post_process]` for LLM-based cleanup.

### "My language isn't supported by [engine]"

Switch to an engine that supports your language. Whisper (99+ languages) and Omnilingual (1600+ languages) have the broadest coverage.

```toml
# Switch back to Whisper for unsupported languages
engine = "whisper"

[whisper]
model = "large-v3-turbo"
language = "auto"
```

### "I need punctuation"

1. Use Parakeet TDT (European languages) or SenseVoice with `use_itn = true` (CJK + English).
2. Enable spoken punctuation: `[text] spoken_punctuation = true`
3. Configure LLM post-processing: `[output.post_process]`

---

## Further Reading

- [Whisper Model Card](https://github.com/openai/whisper)
- [Whisper Large V3 Turbo](https://huggingface.co/openai/whisper-large-v3-turbo)
- [Parakeet TDT 0.6B v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3)
- [Moonshine](https://huggingface.co/onnx-community/moonshine-base-ONNX)
- [SenseVoice](https://github.com/FunAudioLLM/SenseVoice)
- [Paraformer](https://github.com/modelscope/FunASR)
- [Dolphin (DataoceanAI)](https://huggingface.co/csukuangfj/sherpa-onnx-dolphin-base-ctc-multi-lang-int8-2025-04-02)
- [Meta MMS / Omnilingual](https://huggingface.co/facebook/mms-1b-all)
- [HuggingFace Open ASR Leaderboard](https://huggingface.co/spaces/hf-audio/open_asr_leaderboard)
- [Configuration Guide](CONFIGURATION.md)
