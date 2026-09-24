# Cohere q4f16 on Intel Arc B390: initial experiment

Measured locally on 2026-09-19. This is a small feasibility benchmark, not a
representative ASR evaluation or a portable release qualification.

These historical measurements precede explicit `ACCURACY` mode and shared-core
reload handling. See the [expanded hardening report](cohere-intel-arc-b390-hardening.md)
for additional human speech, failure tests, the memory-retention fix and remaining
limitations.

## Setup

- CPU: Intel Core Ultra X7 358H; GPU: integrated Intel Arc B390.
- Installed Voxtype 1.0.1, `voxtype-onnx-avx2`, Cohere q4f16, four CPU threads.
- Experimental native Rust Voxtype build: `cohere-openvino,onnx-load-dynamic`.
- Native OpenVINO 2026.4; ONNX Runtime 1.24.4 CPU decoder in both new-build paths.
- Intel compute-runtime 26.31.39395.13, graphics compiler 2.40.13.
- Same original ONNX model files, Rust feature extractor, decoder and tokenizer.
  No conversion, alternate model download, or speech synthesis.
- Local optimized build with LTO disabled and 16 codegen units to shorten the
  development build. It is not a distributable release build.
- The existing optional Whisper OpenVINO fork was resolved from its local
  checkout at `8109638cfc78eb16953316627bf194d59ca521ab` to avoid fetching its
  large unused C++ submodules. The source revision matches Cargo.lock.
- Ordinary desktop load, no CPU affinity or power-policy changes. Builds and
  other test inference jobs were stopped during each benchmark pass.

The running daemon, installed binary, hotkeys, and user configuration were not
changed. The benchmark does not measure capture, VAD, post-processing, or paste.

## Method

Two public human recordings, described and hashed in
[the benchmark guide](../COHERE_GPU_BENCHMARK.md#audio-fixtures): English JFK
(11.00 s) and Spanish FLEURS (12.84 s).

Two passes reversed both native backend order and language order. Each pass
included three fresh installed-CLI invocations per recording. Each new-build
backend had its own process: model load, first inference, two further warmups,
then five timed inferences. The table pools the two passes: six installed
inferences and ten warm measurements per native backend per recording.

## Results

All times are seconds; medians, not best runs. Installed inference timings come
from logs rounded to 0.01 s. Native timings use a monotonic high-resolution clock.

| Human speech | Audio length | Installed CPU inference, fresh session | Matched-source CPU, warm | Intel GPU encoder + CPU decoder, warm | Matched-source warm speedup |
|---|---:|---:|---:|---:|---:|
| English | 11.00 | 2.465 | 2.937 | 0.575 | 5.10x |
| Spanish | 12.84 | 3.290 | 3.712 | 0.838 | 4.43x |

Relative to the installed CPU's first inference, the warmed GPU path was 4.28x
and 3.93x faster. This is deliberately labeled separately from the matched warm
comparison: first inference and warm inference are not identical conditions.
Warm real-time factors were approximately 0.052 (English) and 0.065 (Spanish).

Individual-pass medians varied: GPU English 0.425 / 0.620 s, Spanish 0.977 /
0.759 s. Matched CPU English 2.893 / 2.969 s, Spanish 3.595 / 3.841 s. These are
small laptop benchmarks with ordinary scheduling/thermal variability; do not
interpret the pooled figures as guaranteed latency.

### Startup costs

- First observed GPU load with an empty application model cache: **10.349 s**,
  plus **1.730 s** for its first English inference. Driver/compiler caches had
  already been exercised by the preliminary synthetic-input probe; this is not
  a completely cold machine measurement.
- Subsequent GPU model loads: **1.737–2.476 s**.
- First inference after those loads: **0.890–1.671 s**.
- Matched CPU model loads: **1.817–1.929 s**.

GPU acceleration is clearly useful once warm, but the first-ever dictation can
be slower. On-demand unloading and language-switch daemon restarts can pay some
startup cost again. Keeping a model loaded could avoid that cost, but was not
applied to the user's configuration. GPU memory and battery consumption were
not benchmarked.

### Transcripts

Installed CPU, matched-source CPU and GPU produced the same transcript in every
run of each clip. Punctuation/case-insensitive word errors against the published
references were also unchanged:

- English: **0 / 22 words**.
- Spanish: **2 / 29 words** (6.9% WER). Both paths said “del clima” instead of
  “de clima”, and “pueda afectar” instead of “puede afectar”.

The Spanish result is not perfect; acceleration did not introduce additional
errors on these clips. Two recordings cannot establish general accuracy parity.

### Evidence of GPU execution

The native backend compiles explicitly for `GPU`, not `AUTO` or `HETERO`, and
reported `Intel(R) Arc(TM) B390 GPU (iGPU)` with execution device `GPU.0`. A
separate native-runtime profiling probe of the same unmodified encoder recorded
GPU GEMM, convolution, and attention kernels. Some shape bookkeeping remains on
CPU, and the entire decoder intentionally stays on CPU. Provider-registration
logs alone were not used as proof of acceleration.

## Validation

- 1,090 library tests passed; three hardware/model-dependent tests ignored in
  the normal run.
- The new ignored Intel GPU test was also explicitly run and passed, exercising
  changing recording lengths and output-buffer ownership.
- CLI/backend override tests passed; CLI > environment > file precedence and
  invalid environment rejection were also checked in separate processes.
- CPU-only Cohere build check passed.
- Formatting and Clippy with `-D warnings` passed for the GPU library, CLI, and
  benchmark example. Small pre-existing ONNX/CTC lint issues exposed by this
  feature set were cleaned up without changing inference behavior.

## Reproduction and artifacts

Use `scripts/benchmark_cohere_gpu.py` and
`examples/benchmark_cohere.rs`, following [the guide](../COHERE_GPU_BENCHMARK.md).
For the second pass add `--gpu-first` and reverse the `--sample` order.
`--references` accepts the fixture metadata to compute the reported WER.

Raw JSON, per-process stdout/stderr, isolated configurations, audio provenance,
and test runtime are retained locally under:

```text
~/.cache/voxtype-gpu-poc/audio/metadata.json
~/.cache/voxtype-gpu-poc/comparison/results.json
~/.cache/voxtype-gpu-poc/comparison-repeat/results.json
~/.cache/voxtype-gpu-poc/build/target/release/voxtype
~/.cache/voxtype-gpu-poc/build/target/release/examples/benchmark_cohere
```

Model graph SHA256:

```text
encoder_model.onnx          81c3369197348c87f28048fcd0cc3fa286b4f63cd950fa52c9756f2ea1c6eefa
decoder_model_merged.onnx    4c5c1a993dc715920fb3d46bfeb35350e6edf58ce474ecd6bb150ff58a8055c6
```
