#!/usr/bin/env python3
"""Export GigaAM-v3 e2e RNN-T to the ONNX layout voxtype's gigaam engine uses.

voxtype consumes four files from the model directory
(~/.local/share/voxtype/models/gigaam-v3-e2e-rnnt by default):

    encoder.onnx   log-mel features -> encoded frames
    decoder.onnx   prediction network (embedding + LSTM, stateful)
    joint.onnx     fused encoder/prediction projection -> vocab logits
    tokens.txt     one SentencePiece piece per line ('▁' marks word starts)

The encoder ONNX from `model.to_onnx()` takes 64-dim log-mel features as
input (torch.stft does not export to ONNX), so voxtype computes the
features itself in Rust (src/transcribe/gigaam_mel.rs). Use
--emit-mel-reference to regenerate tests/gigaam_ref_feats.txt after any
change to the GigaAM preprocessor.

Usage:
    pip install -e "git+https://github.com/salute-developers/GigaAM#egg=gigaam[torch]"
    python3 scripts/export_gigaam_onnx.py --out ~/.local/share/voxtype/models/gigaam-v3-e2e-rnnt
"""

import argparse
import sys
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--model", default="v3_e2e_rnnt", help="gigaam model name (default: v3_e2e_rnnt)"
    )
    parser.add_argument("--out", required=True, help="output model directory")
    parser.add_argument(
        "--emit-mel-reference",
        action="store_true",
        help="also write tests/gigaam_ref_feats.txt (64x50 mel features of a chirp)",
    )
    args = parser.parse_args()

    import torch

    import gigaam

    model = gigaam.load_model(args.model)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    model.to_onnx(dir_path=str(out), dtype=torch.float32)

    renames = {
        f"{args.model}_encoder.onnx": "encoder.onnx",
        f"{args.model}_decoder.onnx": "decoder.onnx",
        f"{args.model}_joint.onnx": "joint.onnx",
    }
    for src, dst in renames.items():
        src_path = out / src
        if src_path.exists():
            src_path.rename(out / dst)
            print(f"{src} -> {dst}")

    tokenizer = model.decoding.tokenizer.model
    tokens_path = out / "tokens.txt"
    with open(tokens_path, "w") as f:
        for i in range(tokenizer.get_piece_size()):
            f.write(tokenizer.IdToPiece(i) + "\n")
    print(f"tokens.txt: {tokenizer.get_piece_size()} pieces")

    if args.emit_mel_reference:
        import numpy as np

        n = 32000
        t = torch.arange(n) / 16000.0
        chirp = torch.sin(2 * np.pi * (200.0 + 600.0 * t) * t) * 0.5
        feats = model.preprocessor.featurizer(chirp.unsqueeze(0))
        ref = Path(__file__).resolve().parent.parent / "tests" / "gigaam_ref_feats.txt"
        np.savetxt(ref, feats[0, :, :50].numpy(), fmt="%.8f")
        print(f"mel reference: {ref}")

    print("done:", out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
