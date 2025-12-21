#!/bin/bash
# Build voxtype Vulkan binary in Docker for clean toolchain
#
# This is separate from build-docker.sh because:
#   1. Vulkan build takes much longer (30+ minutes)
#   2. Kompute shader compilation can hang with some glslang versions
#   3. Most users only need AVX2; Vulkan is optional
#
# Usage:
#   ./scripts/build-docker-vulkan.sh              # Build Vulkan binary
#   ./scripts/build-docker-vulkan.sh --no-cache   # Force full rebuild

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

VERSION=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
RELEASE_DIR="releases/${VERSION}"

# Parse options
DOCKER_OPTS=""
while [[ "$1" == --* ]]; do
    case "$1" in
        --no-cache)
            DOCKER_OPTS="--no-cache"
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo "=== Building voxtype v${VERSION} Vulkan binary in Docker ==="
echo ""
echo "This builds the Vulkan GPU binary using Ubuntu 22.04's toolchain."
echo "NOTE: This can take 30+ minutes due to Kompute shader compilation."
echo ""

# Check for Docker
if ! command -v docker &> /dev/null; then
    echo "Error: docker is required but not installed."
    exit 1
fi

# Build Docker image
echo "Building Docker image..."
docker build $DOCKER_OPTS -f Dockerfile.vulkan -t voxtype-vulkan-builder .

echo ""
echo "Extracting binary to ${RELEASE_DIR}/..."
mkdir -p "$RELEASE_DIR"

# Run container to extract binary
docker run --rm -v "$(pwd)/${RELEASE_DIR}:/output" voxtype-vulkan-builder

echo ""
echo "=== Verifying extracted binary ==="

BINARY="${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-vulkan"

if [[ ! -f "$BINARY" ]]; then
    echo "ERROR: Binary not found at $BINARY"
    exit 1
fi

vpternlog=$(objdump -d "$BINARY" | grep -c vpternlog || echo 0)
broadcast=$(objdump -d "$BINARY" | grep -c '{1to' || echo 0)
gfni=$(objdump -d "$BINARY" | grep -cE 'vgf2p8|gf2p8' || echo 0)
zmm=$(objdump -d "$BINARY" | grep -c zmm || echo 0)

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
