# BRIEF: Cohere Transcribe backend — evaluation, not implementation

## Goal

**Evaluate** whether Cohere Transcribe can ship as a first-class Voxtype backend without compromising the static-Rust-binary deployment story. This worktree is a *research spike*. Do not merge code without a follow-up decision from Pete.

## Why

Cohere Transcribe is hyprwhspr's strongest accuracy claim: #1 on the Open ASR Leaderboard, 5.42 avg WER, 14 languages. It's the only local engine they have that Voxtype doesn't. Adding it would close their last meaningful local-backend advantage.

The catch: it's a gated HuggingFace model, and the reference inference is PyTorch. Voxtype's deployment story assumes statically-linked binaries with bundled ONNX Runtime — see CLAUDE.md "Building Release Binaries" for why this matters (SIGILL handler, glibc-capped Docker builds, 7-binary matrix). If the only viable path is a Python sidecar, that retroactively validates hyprwhspr's "Python is the right tool" argument for this backend specifically.

## Investigation tasks (do these in order)

1. **Does an ONNX export exist?** Search HuggingFace, Cohere's model card, and community repos for `cohere-transcribe.onnx` or equivalent. Note license terms — gated models typically restrict redistribution.

2. **If no published ONNX, is conversion feasible?** Pull the PyTorch reference and try `torch.onnx.export` or `optimum-cli export onnx`. Document any architecture-specific blockers (custom ops, dynamic shapes, layers that don't trace cleanly). Time-box this to 4 hours.

3. **License compatibility.** Read the model license. Voxtype is GPL-3.0 (verify against `LICENSE`). Confirm whether bundling weights with our binaries, requiring users to download separately, or requiring HF token at runtime is acceptable. Note that hyprwhspr requires a HF token at setup — that's the user-friction floor.

4. **VRAM footprint.** Their docs say ~4 GB bf16 / ~8 GB CPU RAM. Check whether ONNX Runtime can hold this with int8/fp16 quantization at acceptable quality. Compare to current Voxtype ONNX engines.

5. **Backend architecture if ONNX path works.** Sketch how it slots into `src/transcribe/` alongside the existing ONNX engines (Parakeet, Moonshine, SenseVoice, Paraformer, Dolphin, Omnilingual). Reuse the existing ONNX dispatch — don't invent a new abstraction layer.

6. **Backend architecture if ONNX path does *not* work.** Document the cost honestly: PyO3 + bundled libtorch, or sidecar Python process, or skip. Each option's impact on binary size, glibc requirement, AVX-512 contamination risk, and the existing release matrix. This is the deliverable that lets Pete make an informed call.

## Files to read

- `src/transcribe/mod.rs` — `Transcriber` trait and factory
- `src/transcribe/whisper.rs` — example local backend
- The ONNX engine implementations (whichever file holds Parakeet/Moonshine etc.) — for the integration pattern
- `Cargo.toml` — current ONNX-related deps (`ort` crate version)
- `docker/Dockerfile.onnx*` — how ONNX builds are containerized

## Deliverable

A `FINDINGS.md` in this worktree covering:

1. ONNX export availability (yes / no / convertible-with-effort)
2. License compatibility verdict
3. Estimated integration effort if ONNX-path
4. Cost analysis if non-ONNX-path (PyO3, sidecar, or skip — with concrete numbers on binary size, deps, glibc)
5. Recommendation: **ship**, **wait**, or **decline**, with reasoning

If the recommendation is "ship" *and* the ONNX path works, also produce a minimal proof-of-concept commit that loads the model and transcribes a test WAV. Don't wire it through CLI/config — that's for the implementation pass after the decision.

## Out of scope

- Don't ship a Python sidecar in this branch. Document the cost, don't pay it.
- Don't add the backend to release builds.
- Don't update user-facing docs.
- Don't bump the version.

## Open questions to flag in FINDINGS.md

- Is HF token UX acceptable, or does it violate principle 1 ("Dead simple user experience")?
- If ONNX export exists but the file is huge (multi-GB), how does that interact with the existing model-download flow in `src/setup/model.rs`?
- Does Cohere's license permit Voxtype to redistribute or only direct-from-HF download?
