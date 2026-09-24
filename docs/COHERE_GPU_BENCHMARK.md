# Experimental Cohere Intel GPU benchmark

Measured results: [initial feasibility](benchmarks/cohere-intel-arc-b390.md) and
[expanded English/Spanish hardening](benchmarks/cohere-intel-arc-b390-hardening.md).

This experiment keeps the existing `cohere-transcribe-q4f16` model files, Rust
feature extraction, tokenizer, generation loop, and ONNX Runtime CPU decoder.
Only the encoder changes: native OpenVINO compiles the existing ONNX graph for
`GPU`. It does not use the ONNX Runtime OpenVINO execution provider, a browser,
a Python subprocess, or converted model weights.

The backend is opt-in (`cohere.encoder_backend = "openvino_gpu"`) and requires
`--features cohere-openvino`. The default `onnx` backend is unchanged. Explicit
GPU selection fails on initialization/inference errors rather than silently
reporting CPU execution as a successful GPU benchmark. The hardened backend
explicitly requests `ACCURACY` execution mode and `LATENCY` scheduling. These
are different hints: latency scheduling alone does not preserve precision.
The original exploratory results used OpenVINO's default `PERFORMANCE` mode.

## Build and runtime

Use a new target directory when changing Cargo features; never replace your
installed binary merely to benchmark. These are local test builds, not portable
release artifacts (release distribution builds require the project's Docker
build process).

```bash
CARGO_TARGET_DIR=target/cohere-openvino \
  cargo build --release --features cohere-openvino,onnx-load-dynamic \
  --bin voxtype --example benchmark_cohere
```

Supply an ONNX Runtime shared library compatible with the project's `ort`
dependency (API 24), plus native OpenVINO and Intel GPU compute drivers. The
first tested combination was ONNX Runtime 1.24.4, OpenVINO 2026.4, and Intel
compute-runtime 26.31.39395.13. The Rust binary loads shared libraries; Python is
not required for inference. An isolated Python wheel installation can supply
the OpenVINO libraries for development.

Set `ORT_DYLIB_PATH` to the ONNX Runtime library file and put OpenVINO's library
directory on `LD_LIBRARY_PATH`. The Rust library finder looks for
`libopenvino_c.so`; wheels containing only a versioned filename need an
unversioned symlink in a private library directory, not a system installation.
Do not put the older OpenVINO libraries bundled with `onnxruntime-openvino` on
this path.

OpenVINO caches compiled models under
`$XDG_CACHE_HOME/voxtype/cohere-openvino` (normally `~/.cache/voxtype/...`). Use a
private `XDG_CACHE_HOME` to measure compilation without touching existing caches.
First compilation and first inference can be much slower than warmed inference.
A changed audio length can also trigger shape-specific kernel compilation.

## Audio fixtures

Use genuine human speech, not synthesized text. The initial fixtures were:

- English: `whisper.cpp`'s `samples/jfk.wav`, 11.00 seconds, JFK inaugural speech.
  Source: <https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/samples/jfk.wav>.
  Reference: “And so my fellow Americans, ask not what your country can do for
  you, ask what you can do for your country.” The repository is MIT licensed;
  a separate license for the historical audio was not verified.
- Spanish: FLEURS `es_419`, sentence 2001, 12.84 seconds. Audio delivered by
  `Lingeng/fleurs_es`, test row 0; reference checked against Google's official
  `data/es_419/test.tsv`, not the mirror's machine-generated transcript.
  Source: <https://huggingface.co/datasets/Lingeng/fleurs_es/viewer/default/test?row=0>.
  Attribution: Google FLEURS and contributors, CC-BY-4.0.
  Reference: “Se recomienda enfáticamente a los viajeros que se informen sobre
  cualquier riesgo de clima extremo en el área que visitan, dado que ello puede
  afectar sus planes de viaje.”

Convert each with `ffmpeg -i input -ac 1 -ar 16000 -c:a pcm_s16le output.wav`.
No trimming, normalization or denoising was applied. SHA256 after conversion:

```text
en-jfk.wav             064ae533b488033184aeda697d8128791f2d934a53a11157a617e802797af4a5
es-fleurs-test-0.wav   0726361482d8eceac9aa6a0b180a5ecb0f9037c31c566455e8f43ac7a3e0bc01
```

## Run

```bash
python3 scripts/benchmark_cohere_gpu.py \
  --model-dir "$HOME/.local/share/voxtype/models/cohere-transcribe-q4f16" \
  --sample en:/path/to/en-jfk.wav \
  --sample es:/path/to/es-fleurs-test-0.wav \
  --benchmark target/cohere-openvino/release/examples/benchmark_cohere \
  --output /path/to/benchmark-results
```

The script creates separate TOML files with language, model, four CPU threads,
and backend selection. It does not edit the user's config, restart the daemon,
record the microphone, type text, or use the clipboard. `--installed` defaults to
`/usr/bin/voxtype`; omit `--benchmark` for an installed-binary baseline only.

For each sample, it measures:

1. Three fresh invocations of the installed CLI, including wall time, logged
   model-load time, and logged inference time (the latter rounded to 0.01 s).
2. The same-source Rust ONNX baseline and OpenVINO GPU backend, each in its own
   process. Model load and first inference are separate. Two additional warmups
   precede five timed inferences. The median is reported, not the best run.
3. Full transcripts and input hashes, so speed cannot hide changed input or
   garbled output. Optional `--references metadata.json` computes case/punctuation-
   insensitive word error rates from `samples[].sha256` and `original_transcript`.
   Two clips cannot establish general recognition quality.

Use `--gpu-first` and reverse the sample order for a counterbalanced second pass.
Ambient `VOXTYPE_*` variables are removed from benchmark children so they cannot
silently override the generated configs.

Run sequentially with no concurrent build/inference jobs. Compare matched-source
warm CPU/GPU medians for backend speedup; comparing installed first inference to
warm GPU inference is a different measurement and is labeled separately.
`RUST_LOG=debug` adds encoder/decoder timings to stderr. First CLI wall time is
not the same as hotkey-to-text latency: recording, model preloading, and paste
are deliberately excluded from this controlled test.

## Hardware regression test

With the runtime environment above, this opt-in test reuses a single encoder for
different feature lengths around the encoder's subsampling boundaries, including
long-short-long transitions. It compares every output shape with ONNX Runtime,
checks finite values and numerical drift, verifies ownership across subsequent
calls, and verifies recovery after invalid input and a native request error.

```bash
VOXTYPE_COHERE_MODEL_DIR=/path/to/cohere-transcribe-q4f16 \
VOXTYPE_COHERE_TEST_AUDIO=/path/to/en-jfk.wav \
  cargo test --release --features cohere-openvino,onnx-load-dynamic --lib \
  openvino_encoder_changes_length -- --ignored
```

It is ignored in normal CI because it requires model weights and an Intel GPU.

## Repeated loads and structural stress

The native example also accepts a playlist and repeated model construction:

```bash
target/cohere-openvino/release/examples/benchmark_cohere \
  --config /path/to/isolated-config.toml \
  --audio /path/to/long.wav --audio /path/to/short.wav \
  --reloads 15 --warmups 0 --runs 1 > soak.json
```

Clips interleave every round, exercising shape changes rather than repeatedly
warming one shape. `--allow-empty` permits an empty WAV; `--continue-on-error`
records failures and continues, then exits nonzero after printing the report.
With multiple clips or reloads the JSON uses `sessions[].clips[]`. `first` means
the first call for that clip in the session, not a fresh process for every clip.

Linux reports RSS after each call and before/after model drop. DRM fdinfo reports
per-client GPU memory counters where supported, deduplicating shared fds. RSS
and DRM counters can overlap on integrated GPUs; do not add them or describe RSS
alone as GPU memory. Empty DRM data is unavailable accounting, not proof of zero
GPU allocation. Rising RSS needs a longer soak to distinguish allocator retention
from a leak. Silence, noise, sliced and repeated speech test structure, not WER.

Offline tooling tests: `python3 -B -m unittest discover -s scripts/tests`.
The CI feature matrix checks Cohere-only and combined Cohere/OpenVINO-Whisper
builds, configuration layering and capability validation without hardware.

## Native binding review caveats

The pinned OpenVINO Rust binding's core/compiled-model property getters do not
free returned C strings. This backend avoids those calls; its version query
uses the matching native free function. Tensor output is copied before request
reuse, and requests are dropped before compiled models. One process-wide core
outlives all models: recreating the native core grew DRM allocations by about
26 MiB per reload on the tested runtime, including in a Python-only reproducer.
Reusing the core avoided that growth in the reproducer and the final 15-reload
Rust test plateaued after its initial warmup. Native runtime/device
caches remain resident until process exit; on-demand model unloading is not a
guarantee of complete GPU memory reclamation.

Two dependency-level issues still need upstream work: native error-message
allocations are not freed by the binding, and malformed/missing-symbol runtime
libraries can panic in its loader. Ordinary missing-runtime/plugin/device and
model errors are tested separately; this is not a guarantee that arbitrary
broken libraries or native driver failures are recoverable. Cohere runs in the
daemon process, not Whisper's `gpu_isolation` worker. The binding rejects
pre-2025.1 runtimes, but only native 2026.4/Arc B390 has been validated here.

## Initial compatibility findings

The `onnxruntime-openvino==1.24.1` wheel bundled OpenVINO 2025.4.1. It loaded this
encoder and completed one synthetic-input inference, then aborted during
background GPU kernel compilation on Arc B390. The core stack included the
OpenVINO GPU plugin and Intel graphics compiler. This is an observed runtime
failure, not a diagnosis of which upstream component is responsible.

Native OpenVINO 2026.4 compiled the unmodified q4f16 encoder and completed repeated
inference on `GPU.0`. Profiling reported GPU convolution, GEMM and attention
kernels, with some shape bookkeeping on CPU. Merely registering a provider was
not counted as proof of acceleration.
