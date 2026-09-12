# Orukeet ONNX: local CPU comparison, September 12, 2026

This is contributor-provided evidence for the optional Orukeet importer, not a
recommendation to replace VoxType's default model. It compares Parakeet TDT
0.6B v3 INT8 with Orukeet r3 INT8 on an AMD Ryzen 9 9950X3D with 64 GB RAM,
running Arch Linux. All inference was CPU-only.

See [import instructions](../PARAKEET.md) and the
[sanitized accuracy measurements](orukeet-onnx-20260912.json). The measurements
contain public sample IDs, hashes, error counts, timings, and model provenance;
they contain no audio, reference text, model transcripts, or desktop logs.

## Public, reference-scored accuracy

Both models processed the same 55 public clips (4,035 reference words;
1,808.402 seconds of audio) using the installed VoxType 0.7.5 `transcribe` CLI.
Each model/clip pair ran once, with alternating model order. Temporary configs
differed only in the model directory. The original WAVs used the same VoxType
mono/resampling path, without post-processing, prompts, reference hints, or
text delivery to applications. Processes used CPUs 0-7 and nice +15.

| Source | Clips | Reference words | Parakeet errors / WER | Orukeet errors / WER |
| --- | ---: | ---: | ---: | ---: |
| Technical English | 15 | 720 | 24 / 3.33% | 29 / 4.03% |
| General English | 20 | 505 | 19 / 3.76% | 20 / 3.96% |
| Structured dictation | 20 | 2,810 | 360 / 12.81% | 237 / 8.43% |
| **Combined** | **55** | **4,035** | **403 / 9.99%** | **286 / 7.09%** |

Orukeet made 117 fewer word errors: a **29.03% relative reduction** or
**2.90 percentage-point reduction in WER** on this sample. It improved 20
clips, regressed on 12, and tied on 23. Neither model produced an empty output.
The improvement was driven by structured dictation; technical and general
English were slightly worse. The pooled result weights longer clips more
heavily and is not a universal accuracy claim.

### Actual datasets and reference fields

All selections use the first rows in Hugging Face datasets-server order,
not random or hidden evaluation samples. Dataset revisions were checked before
and after collection; the JSON records observed revisions and sample hashes.
The reference transcripts are dataset-provided ground truth, not another
model's output used as a reference. They were not independently audited by
listening during this comparison.

| Dataset | Revision | Selection | Reference |
| --- | --- | --- | --- |
| [Tech-Sentences-For-ASR-Training](https://huggingface.co/datasets/danielrosehill/Tech-Sentences-For-ASR-Training/tree/e75d9ea8ec89d3fc26b8ea55c10a635f70806e2f) | `e75d9ea8ec89d3fc26b8ea55c10a635f70806e2f` | default/test, all 15 rows | `text`; audio in `audio_filepath` |
| [Small-STT-Eval-Audio-Dataset](https://huggingface.co/datasets/danielrosehill/Small-STT-Eval-Audio-Dataset/tree/d395fcce66e8843d8ec6ec1036f009ade9329b23) | `d395fcce66e8843d8ec6ec1036f009ade9329b23` | default/train, first 20 of 92 rows | `transcription`; audio in `audio` |
| [VoiceCodeBench](https://huggingface.co/datasets/besimple-ai/voice-code-bench/tree/bef2824f83ef1c796f3e79731a3b0741708730df) | `bef2824f83ef1c796f3e79731a3b0741708730df` | default/test, first 20 of 300 rows: `contact_routing_001` through `contact_routing_020` | `transcripts.acoustic`, `transcripts.canonical`, `transcripts.template`, and `entities`; audio in `data/` + `file_name` |

### Scoring and limitations

Word tokens are lowercase ASCII alphanumeric runs; punctuation is ignored.
WER is total substitutions, deletions, and insertions divided by total reference
words, not the average of per-clip WERs.

For VoiceCodeBench, each annotated entity can independently match either its
acoustic or canonical rendering. The scorer finds minimum word edit distance
through that reference lattice, using the acoustic transcript word count as the
denominator. This follows the approach in the pinned
[VoiceCodeBench metrics implementation](https://huggingface.co/datasets/besimple-ai/voice-code-bench/blob/bef2824f83ef1c796f3e79731a3b0741708730df/scripts/voice_code_bench/metrics.py).
It is **not exact identifier/command correctness**: CTEM and TSR were not
measured. Historical canonical-only WER scores are not directly comparable.

A stratified paired clip bootstrap (2,000 resamples, seed 20260912) gives a
95% interval of [-5.2671, -0.9692] percentage points for Orukeet minus Parakeet
WER. This describes variability within the convenience sample, not uncertainty
across arbitrary speakers or workloads. It does not address training-data
overlap, source annotation errors, or generalization to the contributor's voice.

The original orchestration/scoring harness is local and not included in this
importer-only change. The pinned datasets, selection, scoring method, model
hashes, and per-clip measurements are provided for independent replication and
arithmetic audit; this is not a packaged one-command benchmark. To repeat the
comparison, import the pinned model, transcribe these same public WAVs with
identical CPU-only settings for each model, and score against the indicated
references. References must never be passed to inference as hints.

### CLI wall time on the public clips

These medians include a fresh process and model load for every invocation.
They are **not warm dictation latency** and are not interchangeable with the
separate warmed-worker measurements below.

| Source | Parakeet median | Orukeet median |
| --- | ---: | ---: |
| Technical English | 1,298.01 ms | 1,275.06 ms |
| General English | 1,080.40 ms | 1,110.21 ms |
| Structured dictation | 2,163.73 ms | 1,956.70 ms |
| Combined | 1,298.01 ms | 1,275.06 ms |

## Separate warmed CPU benchmark

A `parakeet-rs 0.3.5` worker decoded 20 private, unlabeled recordings in three
paired rounds: 120 measured decodes across the two models. It used eight
intra-op threads, one inter-op thread, CPUs 0-7, and nice +15. Six actual VoxType
CLI smoke checks were also completed. These are local observations; the
private recordings are not published and cannot serve as a reproducible public
accuracy benchmark.

| Metric | Parakeet v3 INT8 | Orukeet r3 INT8 |
| --- | ---: | ---: |
| Warm decode median | 81.94 ms | 54.77 ms |
| Warm decode p95 | 175.44 ms | 115.30 ms |
| Weighted real-time factor | 0.01419 | 0.00924 |
| Model-load median | 926.56 ms | 1,111.37 ms |
| Peak worker RSS | 1,162.86 MiB | 1,278.69 MiB |

The warm median decode time was about 33% lower with Orukeet, but model startup
was slower and peak memory was higher. Decode timing includes feature
extraction and excludes recording, personalized post-processing, and typing.
Real-time factor is total decode time divided by total audio duration. These
results do not establish performance on lower-end CPUs or streaming behavior.

## Import provenance and validation

The source is the publisher's
[pinned INT8 ONNX archive](https://huggingface.co/oruk/orukeet/resolve/55a984d46f68323301837194ce647c702f55facc/onnx/sherpa-onnx-orukeet-v0.1.0-int8.tar.bz2),
SHA-256 `f9191f30178cc9122ce2f023bf9fefafc822028307b0efa4caff645ba3fe8d0a`.
See the [model card](https://huggingface.co/oruk/orukeet) and
[publisher's ONNX exporter](https://github.com/Oruk-AI/orukeet/tree/main/export/onnx).
The importer retains `LICENSE-WEIGHTS` and `NOTICE.md` for the CC BY-SA 4.0
weights; it does not bundle weights into VoxType.

The real pinned conversion passed eight recurrent CPU parity steps with a
maximum absolute difference of 0.0, including logits, recurrent states,
lengths, token choices, and duration choices. This is a sampled numerical
compatibility check, not a proof of equivalence for all inputs.

Twelve importer unit tests cover successful synthetic import, invalid graph
interfaces/states and archive contents, dangling output symlinks, and cleanup
when parity fails. Fourteen local accuracy-scorer tests also pass, including
acoustic/canonical alternatives. The scorer tests are separate local harness
tests, not part of the upstream importer CI job.

The new importer CI job runs on Python 3.12 with `onnx==1.22.0`,
`onnxruntime==1.30.0`, and `numpy==2.5.3`. No Rust source or runtime dependency
changes are part of this contribution. Broad Rust checks were not rerun locally;
upstream CI status is tracked on the pull request.
