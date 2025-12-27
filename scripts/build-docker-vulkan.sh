#!/bin/bash
# Build voxtype Vulkan binary in Docker for clean toolchain (no AVX-512/GFNI)
#
# This builds on a remote Docker context (TrueNAS with i9-9900KF) to ensure
# no AVX-512 or GFNI instructions leak into the binary. The resulting binary
# is safe for all users, including those with Zen 3 or older CPUs.
#
# This is separate from build-docker.sh because:
#   1. Vulkan build takes much longer (30+ minutes)
#   2. Kompute shader compilation can hang with some glslang versions
#   3. Most users only need AVX2; Vulkan is optional
#
# Usage:
#   ./scripts/build-docker-vulkan.sh              # Build Vulkan binary
#   ./scripts/build-docker-vulkan.sh --no-cache   # Force full rebuild
#   ./scripts/build-docker-vulkan.sh --local      # Build locally (for testing)
#
# Output:
#   releases/X.Y.Z/voxtype-X.Y.Z-linux-x86_64-vulkan

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

VERSION=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
RELEASE_DIR="releases/${VERSION}"

# Default to TrueNAS context (i9-9900KF, no AVX-512/GFNI)
DOCKER_CONTEXT="truenas"
DOCKER_OPTS=""

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
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Build context flag
CONTEXT_FLAG=""
if [[ -n "$DOCKER_CONTEXT" ]]; then
    CONTEXT_FLAG="--context $DOCKER_CONTEXT"
fi

echo "=== Building voxtype v${VERSION} Vulkan binary in Docker ==="
echo ""
if [[ -n "$DOCKER_CONTEXT" ]]; then
    echo "Using Docker context: $DOCKER_CONTEXT (remote build for clean binary)"
else
    echo "Using local Docker (WARNING: may contain AVX-512/GFNI if host has them)"
fi
echo "NOTE: This can take 30+ minutes due to Kompute shader compilation."
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
echo "Building Docker image..."
docker $CONTEXT_FLAG build $DOCKER_OPTS \
    --build-arg VERSION="$VERSION" \
    -f Dockerfile.vulkan \
    -t voxtype-vulkan-builder .

echo ""
echo "Extracting binary to ${RELEASE_DIR}/..."
mkdir -p "$RELEASE_DIR"

# Run container to extract binary
# For remote context, we need to copy the file differently
if [[ -n "$DOCKER_CONTEXT" ]]; then
    # Create container, copy file out, remove container
    CONTAINER_ID=$(docker $CONTEXT_FLAG create voxtype-vulkan-builder)
    docker $CONTEXT_FLAG cp "$CONTAINER_ID:/tmp/voxtype-vulkan" "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-vulkan"
    docker $CONTEXT_FLAG rm "$CONTAINER_ID" > /dev/null
else
    docker run --rm -v "$(pwd)/${RELEASE_DIR}:/output" voxtype-vulkan-builder
fi

echo ""
echo "=== Verifying extracted binary ==="

BINARY="${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-vulkan"

if [[ ! -f "$BINARY" ]]; then
    echo "ERROR: Binary not found at $BINARY"
    exit 1
fi

vpternlog=$(objdump -d "$BINARY" | grep -c vpternlog) || vpternlog=0
broadcast=$(objdump -d "$BINARY" | grep -c '{1to') || broadcast=0
gfni=$(objdump -d "$BINARY" | grep -cE 'vgf2p8|gf2p8') || gfni=0
zmm=$(objdump -d "$BINARY" | grep -c zmm) || zmm=0

echo "  vpternlog: $vpternlog"
echo "  broadcast: $broadcast"
echo "  gfni: $gfni"
echo "  zmm: $zmm"

if [[ "$vpternlog" -gt 0 ]] || [[ "$broadcast" -gt 0 ]] || [[ "$gfni" -gt 0 ]] || [[ "$zmm" -gt 0 ]]; then
    echo ""
    echo "ERROR: Vulkan binary has forbidden instructions!"
    exit 1
fi

echo ""
echo "=== Vulkan build complete ==="
echo ""
ls -lh "$BINARY"
