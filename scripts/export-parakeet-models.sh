#!/bin/bash
# Export NeMo Parakeet models to ONNX format for parakeet-rs
#
# Downloads models from HuggingFace, exports to ONNX, and produces files
# compatible with parakeet-rs. Runs in Docker with GPU support.
#
# Usage:
#   ./scripts/export-parakeet-models.sh                  # Export all models on TrueNAS
#   ./scripts/export-parakeet-models.sh --model ja       # Japanese only
#   ./scripts/export-parakeet-models.sh --model vi       # Vietnamese only
#   ./scripts/export-parakeet-models.sh --local          # Build locally
#   ./scripts/export-parakeet-models.sh --no-cache       # Clean rebuild
#
# Output:
#   models/parakeet-onnx/parakeet-tdt-0.6b-ja/
#   models/parakeet-onnx/parakeet-ctc-0.6b-vi/

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

# Defaults
DOCKER_CONTEXT="truenas"
DOCKER_OPTS=""
MODEL="all"
OUTPUT_DIR="models/parakeet-onnx"

while [[ "$1" == --* ]]; do
    case "$1" in
        --no-cache)
            DOCKER_OPTS="--no-cache"
            shift
            ;;
        --local)
            DOCKER_CONTEXT=""
            shift
            ;;
        --model)
            MODEL="$2"
            if [[ "$MODEL" != "ja" && "$MODEL" != "vi" && "$MODEL" != "all" ]]; then
                echo "Error: --model must be ja, vi, or all"
                exit 1
            fi
            shift 2
            ;;
        --output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo ""
            echo "Usage: $0 [--model ja|vi|all] [--local] [--no-cache] [--output DIR]"
            exit 1
            ;;
    esac
done

# Build context flag
CONTEXT_FLAG=""
if [[ -n "$DOCKER_CONTEXT" ]]; then
    CONTEXT_FLAG="--context $DOCKER_CONTEXT"
fi

echo "=== Exporting Parakeet models to ONNX ==="
echo ""
if [[ -n "$DOCKER_CONTEXT" ]]; then
    echo "Docker context: $DOCKER_CONTEXT (remote, GTX 1660)"
else
    echo "Docker context: local"
fi
echo "Model(s): $MODEL"
echo "Output: $OUTPUT_DIR"
echo ""

# Check for Docker
if ! command -v docker &> /dev/null; then
    echo "Error: docker is required but not installed."
    exit 1
fi

# Check if context exists
if [[ -n "$DOCKER_CONTEXT" ]]; then
    if ! docker context inspect "$DOCKER_CONTEXT" &>/dev/null; then
        echo "Error: Docker context '$DOCKER_CONTEXT' not found."
        echo "Available contexts:"
        docker context ls
        echo ""
        echo "Use --local to build on this machine instead."
        exit 1
    fi
fi

# Build Docker image
echo "Building Docker image (this downloads NeMo + PyTorch, may take a while)..."
docker $CONTEXT_FLAG build $DOCKER_OPTS \
    -f Dockerfile.onnx-export \
    -t voxtype-onnx-export .

echo ""
echo "Running ONNX export..."
mkdir -p "$OUTPUT_DIR"

# Run the export container
# Remote context: can't use volume mounts, use docker create + docker cp
# Local context: use volume mount + GPU passthrough
if [[ -n "$DOCKER_CONTEXT" ]]; then
    # Remote: create container, run it, copy output, remove it
    CONTAINER_ID=$(docker $CONTEXT_FLAG create \
        --gpus all \
        voxtype-onnx-export \
        --output /output --model "$MODEL")

    echo "Container: $CONTAINER_ID"
    docker $CONTEXT_FLAG start -a "$CONTAINER_ID"

    echo ""
    echo "Copying output from container..."

    # Copy each model directory based on what was exported
    if [[ "$MODEL" == "ja" || "$MODEL" == "all" ]]; then
        docker $CONTEXT_FLAG cp "$CONTAINER_ID:/output/parakeet-tdt-0.6b-ja" "$OUTPUT_DIR/" 2>/dev/null || true
    fi
    if [[ "$MODEL" == "vi" || "$MODEL" == "all" ]]; then
        docker $CONTEXT_FLAG cp "$CONTAINER_ID:/output/parakeet-ctc-0.6b-vi" "$OUTPUT_DIR/" 2>/dev/null || true
    fi

    docker $CONTEXT_FLAG rm "$CONTAINER_ID" > /dev/null
else
    # Local: volume mount + GPU
    docker run --rm \
        --gpus all \
        -v "$(pwd)/$OUTPUT_DIR:/output" \
        voxtype-onnx-export \
        --output /output --model "$MODEL"
fi

echo ""
echo "=== Export complete ==="
echo ""

# List output files with sizes
if [[ -d "$OUTPUT_DIR" ]]; then
    for model_dir in "$OUTPUT_DIR"/parakeet-*/; do
        if [[ -d "$model_dir" ]]; then
            echo "$(basename "$model_dir"):"
            ls -lh "$model_dir" | tail -n +2
            echo ""
        fi
    done
else
    echo "WARNING: Output directory $OUTPUT_DIR not found"
    exit 1
fi

echo "Next steps:"
echo "  1. Verify vocab.txt / tokenizer.json are well-formed"
echo "  2. Test with parakeet-rs inference"
echo "  3. Upload to HuggingFace or distribute with voxtype"
