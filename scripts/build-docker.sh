#!/bin/bash
# Build voxtype AVX2 binary in Docker for clean toolchain (no AVX-512/GFNI contamination)
#
# WHY THIS EXISTS:
# Building on a Zen 4 (or other modern CPU) machine can leak AVX-512 and GFNI
# instructions into binaries via system libstdc++, even with RUSTFLAGS set.
# This causes SIGILL crashes on older CPUs like Zen 3.
#
# This script builds on a remote Docker context (TrueNAS with i9-9900KF) to ensure
# no AVX-512 or GFNI instructions leak into the binary.
#
# Usage:
#   ./scripts/build-docker.sh              # Build on TrueNAS (default)
#   ./scripts/build-docker.sh --no-cache   # Force full rebuild
#   ./scripts/build-docker.sh --local      # Build locally (for testing)
#
# Output:
#   releases/X.Y.Z/voxtype-X.Y.Z-linux-x86_64-avx2
#
# The Dockerfile includes verification that fails if forbidden instructions
# are detected.

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

echo "=== Building voxtype v${VERSION} AVX2 binary in Docker ==="
echo ""
if [[ -n "$DOCKER_CONTEXT" ]]; then
    echo "Using Docker context: $DOCKER_CONTEXT (remote build for clean binary)"
else
    echo "Using local Docker (WARNING: may contain AVX-512/GFNI if host has them)"
fi
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
    -f Dockerfile.build \
    -t voxtype-builder .

echo ""
echo "Extracting binaries to ${RELEASE_DIR}/..."
mkdir -p "$RELEASE_DIR"

# Run container to extract binaries
# For remote context, we need to copy the file differently
if [[ -n "$DOCKER_CONTEXT" ]]; then
    # Create container, copy file out, remove container
    CONTAINER_ID=$(docker $CONTEXT_FLAG create voxtype-builder)
    docker $CONTEXT_FLAG cp "$CONTAINER_ID:/tmp/voxtype-avx2" "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2"
    docker $CONTEXT_FLAG rm "$CONTAINER_ID" > /dev/null
else
    docker run --rm -v "$(pwd)/${RELEASE_DIR}:/output" voxtype-builder
fi

echo ""
echo "=== Verifying extracted binaries ==="

verify_clean() {
    local binary="$1"
    local name="$2"

    if [[ ! -f "$binary" ]]; then
        echo "  ERROR: $name not found!"
        return 1
    fi

    local vpternlog broadcast gfni zmm
    vpternlog=$(objdump -d "$binary" | grep -c vpternlog) || vpternlog=0
    broadcast=$(objdump -d "$binary" | grep -c '{1to') || broadcast=0
    gfni=$(objdump -d "$binary" | grep -cE 'vgf2p8|gf2p8') || gfni=0
    zmm=$(objdump -d "$binary" | grep -c zmm) || zmm=0

    if [[ "$vpternlog" -gt 0 ]] || [[ "$broadcast" -gt 0 ]] || [[ "$gfni" -gt 0 ]] || [[ "$zmm" -gt 0 ]]; then
        echo "  ERROR: $name has forbidden instructions!"
        echo "    vpternlog: $vpternlog, broadcast: $broadcast, gfni: $gfni, zmm: $zmm"
        return 1
    fi

    echo "  $name: clean (no AVX-512/GFNI)"
    return 0
}

if ! verify_clean "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2" "voxtype-avx2"; then
    echo ""
    echo "ERROR: Binary verification failed!"
    echo "The Docker build should have caught this - check Dockerfile.build"
    exit 1
fi

echo ""
echo "=== Docker build complete ==="
echo ""
ls -lh "${RELEASE_DIR}"/voxtype-${VERSION}-linux-x86_64-avx2
echo ""
echo "Next steps:"
echo "  1. Build AVX-512 binary locally (needs native AVX-512):"
echo "     cargo clean && cargo build --release"
echo "     cp target/release/voxtype ${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512"
echo ""
echo "  2. Build Vulkan binary locally:"
echo "     cargo clean"
echo "     RUSTFLAGS=\"-C target-cpu=haswell -C target-feature=-avx512f,-avx512bw,-avx512cd,-avx512dq,-avx512vl,-gfni\" \\"
echo "     cargo build --release --features gpu-vulkan"
echo "     cp target/release/voxtype ${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-vulkan"
echo ""
echo "  3. Create packages:"
echo "     ./scripts/package.sh --skip-build ${VERSION}"
