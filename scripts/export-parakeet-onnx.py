#!/usr/bin/env python3
"""Export NeMo Parakeet models to ONNX format compatible with parakeet-rs.

Supports two model architectures:
  - TDT (Token-and-Duration Transducer): Japanese model
  - CTC (Connectionist Temporal Classification): Vietnamese model

Output layouts match what parakeet-rs expects:
  TDT: encoder-model.onnx, encoder-model.onnx.data, decoder_joint-model.onnx, vocab.txt
  CTC: model.onnx, model.onnx_data, tokenizer.json

GPU export is attempted first. If VRAM is insufficient (GTX 1660 = 6GB, tight for
0.6B models), the script falls back to CPU export automatically.
"""

import argparse
import json
import os
import shutil
import sys
from pathlib import Path

import onnx
import torch
from onnx.external_data_helper import convert_model_to_external_data


MODELS = {
    "ja": {
        "hf_id": "nvidia/parakeet-tdt_ctc-0.6b-ja",
        "arch": "tdt",
        "output_dir": "parakeet-tdt-0.6b-ja",
    },
    "vi": {
        "hf_id": "nvidia/parakeet-ctc-0.6b-Vietnamese",
        "arch": "ctc",
        "output_dir": "parakeet-ctc-0.6b-vi",
    },
}


def export_tdt_model(model, output_dir: Path):
    """Export a TDT model to ONNX with external data format.

    Creates:
      encoder-model.onnx         (small, references external data)
      encoder-model.onnx.data    (large, ~2.6GB of encoder weights)
      decoder_joint-model.onnx   (decoder + joiner network)
      vocab.txt                  (token vocabulary with indices)
      config.json                (model metadata for parakeet-rs)
    """
    temp_dir = output_dir / "temp"
    temp_dir.mkdir(parents=True, exist_ok=True)

    # NeMo export creates encoder-model.onnx and decoder_joint-model.onnx
    print("  Exporting model to ONNX...")
    model.export(str(temp_dir / "model.onnx"))

    # Convert encoder to external data format (avoids 2GB protobuf limit)
    print("  Converting encoder to external data format...")
    encoder_file = temp_dir / "encoder-model.onnx"
    if not encoder_file.exists():
        print(f"  ERROR: Expected {encoder_file} but it doesn't exist")
        print(f"  Files in temp dir: {list(temp_dir.iterdir())}")
        sys.exit(1)

    data_filename = "encoder-model.onnx.data"
    onnx_model = onnx.load(str(encoder_file))
    convert_model_to_external_data(
        onnx_model,
        all_tensors_to_one_file=True,
        location=data_filename,
        size_threshold=0,
        convert_attribute=False,
    )
    onnx.save_model(
        onnx_model,
        str(output_dir / "encoder-model.onnx"),
        save_as_external_data=True,
        all_tensors_to_one_file=True,
        location=data_filename,
        size_threshold=0,
    )

    # Move decoder/joiner to final location
    decoder_file = temp_dir / "decoder_joint-model.onnx"
    if not decoder_file.exists():
        print(f"  ERROR: Expected {decoder_file} but it doesn't exist")
        print(f"  Files in temp dir: {list(temp_dir.iterdir())}")
        sys.exit(1)
    shutil.move(str(decoder_file), str(output_dir / "decoder_joint-model.onnx"))

    # Clean up temp directory
    shutil.rmtree(temp_dir)

    # Generate vocab.txt from model tokenizer
    print("  Generating vocab.txt...")
    with (output_dir / "vocab.txt").open("w") as f:
        for i, token in enumerate([*model.tokenizer.vocab, "<blk>"]):
            f.write(f"{token} {i}\n")

    # Write config.json
    config = {
        "model_type": "nemo-conformer-tdt",
        "features_size": 128,
        "subsampling_factor": 8,
        "enable_local_attn": True,
        "conv_chunking_factor": -1,
    }
    with (output_dir / "config.json").open("w") as f:
        json.dump(config, f, indent=2)


def export_ctc_model(model, output_dir: Path):
    """Export a CTC model to ONNX with external data format.

    Creates:
      model.onnx           (small, references external data)
      model.onnx_data      (large, encoder weights)
      tokenizer.json       (HuggingFace-format tokenizer)
      config.json          (model metadata for parakeet-rs)
    """
    temp_dir = output_dir / "temp"
    temp_dir.mkdir(parents=True, exist_ok=True)

    # NeMo export for CTC creates a single model.onnx (encoder + CTC head)
    print("  Exporting model to ONNX...")
    model.export(str(temp_dir / "model.onnx"))

    # Find the exported ONNX file - CTC models produce a single file
    # NeMo may name it model.onnx or encoder-model.onnx depending on version
    exported_file = None
    for candidate in ["model.onnx", "encoder-model.onnx"]:
        if (temp_dir / candidate).exists():
            exported_file = temp_dir / candidate
            break

    if exported_file is None:
        print(f"  ERROR: No ONNX file found in {temp_dir}")
        print(f"  Files: {list(temp_dir.iterdir())}")
        sys.exit(1)

    # Convert to external data format
    print("  Converting to external data format...")
    data_filename = "model.onnx_data"
    onnx_model = onnx.load(str(exported_file))
    convert_model_to_external_data(
        onnx_model,
        all_tensors_to_one_file=True,
        location=data_filename,
        size_threshold=0,
        convert_attribute=False,
    )
    onnx.save_model(
        onnx_model,
        str(output_dir / "model.onnx"),
        save_as_external_data=True,
        all_tensors_to_one_file=True,
        location=data_filename,
        size_threshold=0,
    )

    # Clean up temp directory
    shutil.rmtree(temp_dir)

    # Generate tokenizer.json from model tokenizer
    print("  Generating tokenizer.json...")
    tokenizer = model.tokenizer
    # Build a HuggingFace-compatible tokenizer.json
    vocab = {}
    if hasattr(tokenizer, "vocab"):
        for i, token in enumerate(tokenizer.vocab):
            vocab[token] = i
    elif hasattr(tokenizer, "tokenizer") and hasattr(tokenizer.tokenizer, "get_vocab"):
        vocab = tokenizer.tokenizer.get_vocab()
    else:
        # Fallback: iterate through token IDs
        for i in range(tokenizer.vocab_size):
            token = tokenizer.ids_to_tokens([i])
            if token:
                vocab[token[0]] = i

    tokenizer_data = {
        "version": "1.0",
        "model": {
            "type": "BPE",
            "vocab": vocab,
        },
    }
    with (output_dir / "tokenizer.json").open("w") as f:
        json.dump(tokenizer_data, f, indent=2)

    # Write config.json
    config = {
        "model_type": "nemo-conformer-ctc",
        "features_size": 128,
        "subsampling_factor": 8,
        "enable_local_attn": True,
        "conv_chunking_factor": -1,
    }
    with (output_dir / "config.json").open("w") as f:
        json.dump(config, f, indent=2)


def export_model(model_key: str, output_base: Path, device: str):
    """Download and export a single model."""
    import nemo.collections.asr as nemo_asr

    info = MODELS[model_key]
    output_dir = output_base / info["output_dir"]

    if output_dir.exists():
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True)

    print(f"\nExporting {info['hf_id']} ({info['arch'].upper()})...")
    print(f"  Output: {output_dir}")

    # Load model from HuggingFace
    print(f"  Loading model (device={device})...")
    if device == "cuda":
        map_location = torch.device("cuda")
    else:
        map_location = torch.device("cpu")

    model = nemo_asr.models.ASRModel.from_pretrained(
        info["hf_id"], map_location=map_location
    )

    if device == "cuda":
        model = model.cuda()
    else:
        model = model.cpu()

    # Enable local attention for long audio support
    print("  Enabling local attention (window=[128, 128])...")
    model.change_attention_model("rel_pos_local_attn", [128, 128])
    model.change_subsampling_conv_chunking_factor(-1)

    # Export based on architecture
    if info["arch"] == "tdt":
        export_tdt_model(model, output_dir)
    else:
        export_ctc_model(model, output_dir)

    # Print results
    print(f"\n  Export complete. Files:")
    for f in sorted(output_dir.iterdir()):
        size_mb = f.stat().st_size / (1024 * 1024)
        print(f"    {f.name:40s} {size_mb:8.1f} MB")


def main():
    parser = argparse.ArgumentParser(
        description="Export NeMo Parakeet models to ONNX for parakeet-rs"
    )
    parser.add_argument(
        "--model",
        choices=["ja", "vi", "all"],
        default="all",
        help="Which model to export (default: all)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("/output"),
        help="Output base directory (default: /output)",
    )
    parser.add_argument(
        "--cpu-only",
        action="store_true",
        help="Force CPU export (skip GPU attempt)",
    )
    args = parser.parse_args()

    print(f"PyTorch version: {torch.__version__}")
    print(f"CUDA available: {torch.cuda.is_available()}")
    if torch.cuda.is_available():
        print(f"GPU: {torch.cuda.get_device_name(0)}")
        vram_gb = torch.cuda.get_device_properties(0).total_mem / (1024**3)
        print(f"VRAM: {vram_gb:.1f} GB")

    models_to_export = list(MODELS.keys()) if args.model == "all" else [args.model]

    for model_key in models_to_export:
        device = "cpu"

        if not args.cpu_only and torch.cuda.is_available():
            # Try GPU first, fall back to CPU on OOM
            try:
                print(f"\nAttempting GPU export for {model_key}...")
                export_model(model_key, args.output, device="cuda")
                continue
            except (torch.cuda.OutOfMemoryError, RuntimeError) as e:
                if "out of memory" in str(e).lower() or isinstance(
                    e, torch.cuda.OutOfMemoryError
                ):
                    print(f"\n  GPU OOM, falling back to CPU export...")
                    torch.cuda.empty_cache()
                    device = "cpu"
                else:
                    raise

        export_model(model_key, args.output, device=device)

    print("\n=== All exports complete ===")
    print(f"Output directory: {args.output}")
    for model_key in models_to_export:
        info = MODELS[model_key]
        model_dir = args.output / info["output_dir"]
        if model_dir.exists():
            total_mb = sum(
                f.stat().st_size for f in model_dir.iterdir()
            ) / (1024 * 1024)
            print(f"  {info['output_dir']}: {total_mb:.0f} MB total")


if __name__ == "__main__":
    main()
