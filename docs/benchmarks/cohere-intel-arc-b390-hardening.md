# Cohere Intel GPU hardening: English, Spanish, failure paths and reloads

Date: 2026-09-19. Follow-up to the [initial feasibility benchmark](cohere-intel-arc-b390.md).
[Machine-readable measurements and references](cohere-intel-arc-b390-hardening.json).

## Scope

Intel Core Ultra X7 358H / Arc B390 iGPU, native OpenVINO 2026.4.0, ONNX Runtime
1.24.4, unchanged `cohere-transcribe-q4f16` files. GPU encoder, CPU decoder.
The hardened implementation explicitly selects `ACCURACY` execution mode and
`LATENCY` scheduling. The initial feasibility implementation used the runtime's
default `PERFORMANCE` mode.

All tests used private configurations, caches and executables. The installed
binary, configuration, daemon and language hotkeys were not changed.

## Human speech and accuracy

Eight additional FLEURS test recordings: four Spanish and four English, totaling
129.50 seconds. All are human speech, converted to mono 16 kHz PCM16 without
trimming, normalization or denoising. References were checked against Google's
pinned FLEURS test metadata, not another recognizer's output.

Attribution: Google FLEURS and contributors, CC-BY-4.0. Spanish audio was delivered
by `Lingeng/fleurs_es`; English by `lmms-lab-audio/fleurs`. Source revisions,
viewer/reference URLs, converted WAV hashes, sentence IDs and transcripts are in
the JSON. Spanish recording identity is inferred from unique transcript and
sample-count matches; English additionally matches original filenames. Neither
archive-byte comparison nor listening verification was performed. Male/female
metadata is represented, but distinct speakers cannot be established without
speaker IDs.

### Matched-source warm inference

Each sample/backend used a separate process, four CPU threads, a first inference,
one additional warmup, then three measured calls. These timing runs had no
concurrent builds or inference jobs. The table predates the shared-core reload
fix below; each process constructed only one model. All eight GPU transcripts
were subsequently rechecked, twice each, with the final shared-core build.
Those final checks ran alongside builds and are **not** timing benchmarks.

| Fixture | Audio seconds | CPU seconds | GPU seconds | Warm speedup |
|---|---:|---:|---:|---:|
| es-fleurs-test-1 | 21.36 | 6.281 | 1.816 | 3.46× |
| es-fleurs-test-103 | 18.12 | 4.685 | 1.491 | 3.14× |
| es-fleurs-test-104 | 16.20 | 4.261 | 1.259 | 3.38× |
| es-fleurs-test-105 | 29.46 | 9.013 | 3.669 | 2.46× |
| en-fleurs-test-0 | 10.56 | 2.611 | 0.394 | 6.63× |
| en-fleurs-test-4 | 4.32 | 0.887 | 0.213 | 4.17× |
| en-fleurs-test-15 | 15.56 | 3.787 | 0.664 | 5.71× |
| en-fleurs-test-16 | 13.92 | 3.420 | 0.603 | 5.67× |

Seven recordings produced identical CPU/GPU transcripts across all calls. On
Spanish test-105, GPU output used correct Spanish **estable** where CPU output
used **estável**, and quotation punctuation differed. The difference also
occurred under the earlier performance-default policy; it is not evidence that
the execution-mode change itself improved recognition.

Case/punctuation-insensitive, NFC-normalized WER, retaining accents and without
special number/abbreviation normalization:

| Language | CPU errors / reference words | GPU errors / reference words |
|---|---:|---:|
| English | 4 / 97 (4.12%) | 4 / 97 (4.12%) |
| Spanish | 7 / 172 (4.07%) | 6 / 172 (3.49%) |

This small set shows no observed WER regression, not universal accuracy parity.
The CPU decoder limits gains on longer/token-heavy utterances. These are warmed
inference measurements, not cold startup or hotkey-to-text latency.

## Structural and failure tests

- Interleaved empty, 10/25/50 ms, 1/4/11/22/30-second inputs, silence and deterministic
  low noise: 99 GPU-selected and 33 CPU transcribe calls, no errors or transcript
  differences. Empty and too-short inputs return before encoder inference.
  Sliced/repeated speech is structural stress, not additional independent speech.
- Silence/noise produced `؟` on **both** backends. This existing model behavior
  is not fixed by GPU acceleration. Repeated 22-second speech also collapsed to
  one copy on both backends; successful execution is not an accuracy assertion.
- Ignored hardware regression explicitly run: output shapes matched ORT around
  subsampling boundaries; outputs were finite and independently owned across
  request reuse and model reload. Invalid Rust input and a rejected native
  tensor name did not prevent subsequent inference. Relative encoder-output L2
  differences were 0.12–2.31% on the tested feature prefixes, below the test's 5%
  guard. Different floating-point runtimes are not bit-identical.
- Nine startup checks passed: CPU without OpenVINO and GPU initialization from an
  unrelated working directory succeeded. Missing runtime, missing GPU plugin,
  unavailable OpenCL devices, unwritable cache, absent model, missing external
  weights and truncated external weights each exited normally with an error.

## Memory investigation and fix

The reload soak found a real problem hidden by the original two-clip benchmark:
creating a fresh OpenVINO core for each model grew DRM resident allocations by
about **26 MiB per reload**. After model drop, the 15-load Rust run increased from
1,425,892 to 1,798,800 KiB. A native Python-only encoder reproducer showed the same
pattern, without Rust bindings or the ORT decoder. This localizes the behavior to
the native runtime/driver lifecycle, not a Rust output-buffer ownership leak;
it does not identify the responsible upstream component.

The implementation now shares one process-wide core, serializing initialization
and compilation but not inference. Requests still drop before compiled models;
the core outlives both. In the final **15 reload / 60 transcription** Rust soak,
post-drop DRM resident allocation settled at **1,446,376 KiB by reload 3 and
remained there through reload 15**. Post-drop RSS ended around 274 MiB, rather
than the earlier 599 MiB. This is a bounded observed soak, not proof against every
possible long-running leak. CPU-only reloads also retained allocator memory.

Runtime/device caches remain resident until process exit. Model unload, including
on-demand unloading, does not guarantee zero GPU allocation. RSS and DRM counts
can overlap on this iGPU and must not be added together. The memory probes are
not performance measurements. An attempted internal `GPU_ENABLE_MEMORY_POOL`
workaround was rejected by the runtime and is **not** used by the implementation.

## Review fixes and automated checks

- Avoid leaking native property-getter APIs; log the runtime version instead.
- Reject invalid/overflowing shapes before native allocation; reject non-finite
  encoder output and copy results into independent ORT storage.
- Preserve ONNX as default, validate actual file/environment/CLI layering, reject
  unavailable GPU choices before `config set` writes, expose per-choice schema
  availability, and include the capability in `info variants`.
- `info accel` recognizes successful native GPU inference, not mere preparation.
- Correct the earlier TUI documentation claim: there is no backend picker yet;
  existing manually configured values are preserved.
- Preserve benchmark timeout/abort logs, isolate environment overrides, and
  report process and DRM memory separately.

The full test suite passed 1,154 tests, including 1,095 library tests. Three
hardware-dependent tests remained ignored in that run. The GPU hardware test
passed when enabled separately. Three process-isolated CLI/config tests, the
benchmark example's median test and six offline Python tests passed. Clippy with
`-D warnings` passed for Cohere-only, native Cohere GPU, and combined
Cohere/OpenVINO-Whisper builds. Default CPU-only all-target compilation passed.
CI jobs now exercise the optional feature combinations without model/GPU access;
hosted CI itself has not been run for this uncommitted work.

Local builds used path overrides to the already-fetched, locked OpenVINO Rust
fork revision `8109638cfc78eb16953316627bf194d59ca521ab`, avoiding unused C++
submodule downloads. Repository dependencies remain Git-based. These host-built
executables are test artifacts, not Docker-built portable release binaries.

## Remaining limits before broad production claims

- Only this model variant, GPU/runtime combination, and two languages were tested.
- No all-day daemon soak, suspend/resume, device-loss recovery, power measurement,
  packaging validation, or live hotkey-to-text benchmark was performed.
- Cohere runs in-process; Whisper's `gpu_isolation` does not apply.
- The pinned native binding still leaks allocated error-message strings on error
  paths and may panic on malformed/missing-symbol runtime libraries. These need
  upstream binding fixes; avoiding success-path property queries does not fix them.

This supports an **explicitly experimental PR**, not a claim that every Intel GPU,
runtime version, or native failure is supported. See the
[reproduction guide](../COHERE_GPU_BENCHMARK.md) for commands and runtime setup.
