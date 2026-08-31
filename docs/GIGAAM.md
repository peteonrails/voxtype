# GigaAM Engine (Russian, offline)

GigaAM adds SberDevices' [GigaAM-v3 e2e
RNN-T](https://github.com/salute-developers/GigaAM) as a local Russian ASR
engine. The `e2e` (end-to-end) variant emits punctuated, normalized Russian
text directly — no post-processing needed — and won ~70:30 in side-by-side
comparisons against Whisper-large-v3 in GigaAM's evaluation.

## Build

```bash
cargo build --release --features gigaam
```

## Model setup

The upstream release ships PyTorch checkpoints, not ONNX, so export the
graphs once with the helper script (needs Python ≥ 3.10 and
[pip install gigaam[torch]]):

```bash
python3 scripts/export_gigaam_onnx.py \
    --out ~/.local/share/voxtype/models/gigaam-v3-e2e-rnnt
```

This produces:

| File | Role |
|------|------|
| `encoder.onnx` | log-mel features → encoded frames (768-dim) |
| `decoder.onnx` | prediction network (embedding + 1-layer LSTM, stateful) |
| `joint.onnx` | fuses encoder frame + prediction → vocab logits (1024 + blank) |
| `tokens.txt` | SentencePiece pieces, one per line (`▁` = word boundary) |

~900 MB on disk, fp32. Runs on CPU (ONNX Runtime); ~7× realtime on a
typical laptop CPU for dictation-length clips.

The log-mel front-end (64 bins, n_fft 320, hop 160, HTK mel, center=False,
`ln(clamp(x, 1e-9, 1e9))`) is implemented in Rust
(`src/transcribe/gigaam_mel.rs`) because `torch.stft` does not export to
ONNX. `cargo test -p voxtype gigaam` pins it against a Python-generated
reference (`tests/gigaam_ref_feats.txt`); regenerate the reference with
`scripts/export_gigaam_onnx.py --emit-mel-reference` after any upstream
preprocessor change.

## Configuration

```toml
engine = "gigaam"

[gigaam]
model = "gigaam-v3-e2e-rnnt"  # or an absolute path to a model dir
# threads = 4                  # ONNX Runtime intra-op threads
# on_demand_loading = false
```

## Usage

```bash
voxtype --engine gigaam transcribe russian-speech.wav
```

## Notes

- `transcribe` processes the whole clip through the encoder at once; for
  best accuracy keep single dictations under ~30 s (chunk longer audio
  externally or use meeting mode).
- Language auto-detection is not applicable: the model is Russian-only and
  reports `ru` for `last_detected_language`.
