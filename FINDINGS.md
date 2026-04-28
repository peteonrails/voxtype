# FINDINGS: Cohere Transcribe backend evaluation

Authored by research-agent v1 (no commits made; agent was blocked from `git`/`cargo`).

## Recommendation: SHIP (ONNX path, gated behind a `cohere` Cargo feature)

**Single most important reason:** the existing `src/transcribe/moonshine.rs` is a ~90% template for Cohere. The architecture (Conformer encoder + Transformer decoder + KV cache + tokenizer.json), dependency graph (`ort` 2.0.0-rc.11, `tokenizers` 0.20), Docker build matrix (rides existing `onnx-avx2`/`onnx-avx512`/`onnx-cuda`/`onnx-rocm` binaries), and download UX (mirrors Whisper-large precedent) are all solved problems in this codebase.

## What was committed

**Nothing.** `Bash` permissions for `git`, `cargo fmt`, `cargo clippy`, and `git commit -S` were denied throughout the session. All research was completed via Read + WebSearch + WebFetch. The PoC commit step is straightforward to produce in a re-spawned session — the integration shape below is fully specified.

---

## 1. ONNX export availability — YES, multiple public exports

- `onnx-community/cohere-transcribe-03-2026-ONNX` — 18 quantized variants (fp32, fp16, q8, int8, int4, q4, uint8). Powers Cohere's official WebGPU Space via Transformers.js + onnxruntime-web, which proves the export uses stock ops and loads in `ort` 2.x natively.
- `cstr/cohere-transcribe-onnx-int4`, `cstr/cohere-transcribe-onnx-int8` — single-package int4/int8. Layout: `cohere-encoder.{int4,int8}.onnx` (+ `.onnx.data`), `cohere-decoder.{int4,int8}.onnx` (+ `.onnx.data`), `tokens.txt`. Used by `transcribe-rs` 0.3 on crates.io.
- `cstr/cohere-transcribe-03-2026-GGUF` — Q5_0 1.74 GB up to Q8_0 2.42 GB (size reference; not relevant to ONNX path).

Architecture per Cohere's blog: log-mel spectrogram → Fast-Conformer encoder (~90% of 2B params) → lightweight Transformer decoder. Same shape as Moonshine, larger.

## 2. License — Apache 2.0, GPL-3.0 compatible

- Original `CohereLabs/cohere-transcribe-03-2026` is **gated** (must accept terms via HF account/token). Apache 2.0 weights.
- Community ONNX export is Apache 2.0; onnx-community's other exports (Moonshine, Whisper) are not gated, so this one almost certainly is not, but verify at integration time.
- **Apache 2.0 permits redistribution.** Voxtype could mirror the ONNX weights to GitHub Releases, attaching the Apache LICENSE + NOTICE files via `setup/model.rs`. This eliminates HF-token UX friction at the cost of ~2.4 GB of release-asset storage per Cohere version.

## 3. VRAM / disk footprint

| Quant | Disk | Notes |
|---|---|---|
| fp32 | ~8 GB | Reference only |
| fp16/bf16 | ~4 GB | Cohere's stated VRAM baseline |
| int8 | ~2.4 GB | Sane default; runs on any 4 GB+ GPU and on CPU |
| int4 | ~1.2-1.4 GB | Laptop default candidate |

Cohere quotes ~4 GB VRAM bf16, ~5 GB during chunked long-file inference, ~8 GB CPU RAM at fp32. int8 ONNX is **larger than every current Voxtype ONNX engine** (Parakeet TDT-0.6B ~600 MB, Moonshine-base ~120 MB) but smaller than Whisper-large GGUF (~3 GB) which the project already ships, so the precedent is set. ONNX Runtime handles this comfortably; the WebGPU demo runs int4 in a browser tab.

## 4. Integration sketch (ONNX path)

### New file: `src/transcribe/cohere.rs`

Clone of `moonshine.rs` with three deltas:

- **Preprocessing:** Cohere consumes log-mel, not raw audio. Use the existing `src/transcribe/fbank.rs` (already shared by SenseVoice/Paraformer/Dolphin/Omnilingual). Add `cohere` to the cfg-any blocks gating `fbank` and `ctc` modules in `mod.rs`.
- **Decoder loop:** autoregressive with KV cache, identical to Moonshine's `run_inference()`. Use the same dynamic head/dim detection from `past_key_values` input metadata that Moonshine already does — don't hardcode dims, which avoids breakage across int4/int8/fp16 variants.
- **Tokenizer:** SentencePiece via tokenizer.json, handled by existing `tokenizers` 0.20 dep.

### Wiring changes

- `src/transcribe/mod.rs`: `#[cfg(feature = "cohere")] pub mod cohere;` + `TranscriptionEngine::Cohere` arm in factory.
- `src/config.rs`: `CohereConfig { model, quantization, language, threads, gpu }`, `pub cohere: Option<CohereConfig>` on `Config`.
- `src/cli.rs`: `--engine cohere` and matching flags.
- `Cargo.toml`: feature `cohere = ["onnx-common", "dep:tokenizers"]` plus `cohere-cuda`, `cohere-tensorrt` mirrors. **No new deps.**
- `src/setup/model.rs`: download flow.

### Build matrix impact: ZERO new binaries

Cohere rides the existing `onnx-avx2/avx512/cuda/rocm` set via the feature flag. The 7-binary matrix stays at 7. Whisper-only binaries (avx2/avx512/vulkan) correctly exclude Cohere.

### Effort estimate: 1-2 days

| Task | Hours |
|---|---|
| `cohere.rs` (port Moonshine, swap to mel input) | 4-6 |
| Config/CLI/factory wiring | 2 |
| `setup/model.rs` download flow | 2-3 |
| Tests | 2 |
| Docker compose + docs | 3 |

### Don't take the `transcribe-rs` shortcut

`transcribe-rs` 0.3 has a working `CohereModel` using `ort`. Pulling it in would shrink the new file by 75% but adds a third-party crate that overlaps Voxtype's hand-rolled ONNX layer. Voxtype's engines are intentionally hand-rolled to share `fbank.rs`/`ctc.rs`, control GPU EP probing (see `parakeet.rs::probe_cuda_runtime`), and keep the dep graph small. Read `transcribe-rs/src/onnx/cohere.rs` as a reference, don't depend on it.

## 5. Cost analysis if non-ONNX path (rejected)

- **PyO3 + libtorch:** +500 MB to 3 GB binary size, libstdc++ ABI risk, AVX-512 from libtorch (same situation as ONNX Runtime today, runtime-dispatched), expands release matrix from 7 to 10. Heavy. Only justified if ONNX broke, which it didn't.
- **Python sidecar:** kills the static-binary doctrine. No.
- **Skip:** defensible but leaves the leaderboard win to hyprwhspr. Remote-API path via OpenAI-compatible Cohere endpoints remains available regardless.

## 6. Open questions for Pete

1. **HF token UX vs. principle 1.** Match hyprwhspr (require token, document it) or mirror weights to GitHub Releases (eats ~2.4 GB per Cohere version, removes friction)? Agent's read: mirror it. Principle 1 wins.
2. **Multi-GB first-run download.** `voxtype setup model` should add a size confirmation gate. Match the existing Whisper-large flow.
3. **Default quantization.** int8 for multilingual (Cohere's selling point); int4 only for English-only laptop default.

## Sources

- [CohereLabs/cohere-transcribe-03-2026 (HuggingFace)](https://huggingface.co/CohereLabs/cohere-transcribe-03-2026)
- [onnx-community/cohere-transcribe-03-2026-ONNX](https://huggingface.co/onnx-community/cohere-transcribe-03-2026-ONNX)
- [cstr/cohere-transcribe-03-2026-GGUF](https://huggingface.co/cstr/cohere-transcribe-03-2026-GGUF)
- [transcribe-rs (cjpais)](https://github.com/cjpais/transcribe-rs)
- [second-state/cohere_transcribe_rs (libtorch path, rejected)](https://github.com/second-state/cohere_transcribe_rs)
- [sherpa-onnx issue #3442](https://github.com/k2-fsa/sherpa-onnx/issues/3442)
- [Cohere blog: Introducing Cohere Transcribe](https://cohere.com/blog/transcribe)
- [HuggingFace blog: Cohere Transcribe release](https://huggingface.co/blog/CohereLabs/cohere-transcribe-03-2026-release)
- [Cohere Transcribe WebGPU Space](https://huggingface.co/spaces/CohereLabs/Cohere-Transcribe-WebGPU)
