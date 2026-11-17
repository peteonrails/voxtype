# Third-Party Licenses

This file lists third-party models, assets, and algorithms distributed with,
downloaded by, or used by voxtype.

## rphonetic

- **Source:** https://crates.io/crates/rphonetic
- **License:** Apache-2.0
- **Origin:** Rust port of phonetic algorithms from Apache Commons Codec

Used for Double Metaphone keys in conservative profile vocabulary correction.

## GTCRN Speech Enhancement Model

- **File:** `gtcrn_simple.onnx` (auto-downloaded on first meeting start)
- **Source:** https://github.com/Xiaobin-Rong/gtcrn
- **License:** MIT
- **Copyright:** Copyright (c) 2024 Rong Xiaobin

Used for removing speaker bleed-through from microphone audio during meeting
transcription. The model is downloaded from the
[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) project's release assets.

## XLM-RoBERTa Punctuation, Full Stop, and True-Case Model

- **File:** `model.onnx` (downloaded by `voxtype setup punctuation`)
- **Source:** https://huggingface.co/1-800-BAD-CODE/xlm-roberta_punctuation_fullstop_truecase
- **Source revision:** `d1769a597ce8dfaa070d436bc67d4ee761f58884`
- **License:** Apache-2.0
- **Changes:** None to the ONNX graph

The pure-Rust runtime uses the byte-compatible `tokenizer.json` from
[`FacebookAI/xlm-roberta-base`](https://huggingface.co/FacebookAI/xlm-roberta-base)
at revision `e73636d4f797dec63c3081bb6ed5c7b0bb3f2089` instead of the model
repository's raw SentencePiece file. The tokenizer vocabulary IDs were
verified against that SentencePiece model. Both files are mirrored through
models.voxtype.io and pinned by the per-file SHA-256 hashes in its manifest.
