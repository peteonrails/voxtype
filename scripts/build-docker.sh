#!/bin/bash
# Build voxtype AVX2 binary in Docker for clean toolchain (no AVX-512/GFNI contamination)
#
# WHY THIS EXISTS:
# Building on a Zen 4 (or other modern CPU) machine can leak AVX-512 and GFNI
# instructions into binaries via system libstdc++, even with RUSTFLAGS set.
# This causes SIGILL crashes on older CPUs like Zen 3.
#
# This script builds in Ubuntu 22.04 Docker container which has an older
# toolchain without these optimizations.
#
# NOTE: Only builds AVX2 binary. Vulkan is skipped because:
#   1. Kompute shader compilation hangs with Ubuntu 22.04's glslang
#   2. Vulkan users have GPUs and typically modern CPUs
#   3. GPU-bound code is less affected by CPU instruction contamination
#
# Usage:
#   ./scripts/build-docker.sh              # Build and extract binary
#   ./scripts/build-docker.sh --no-cache   # Force full rebuild
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

echo "=== Building voxtype v${VERSION} AVX2 binary in Docker ==="
echo ""
echo "This builds the AVX2 binary using Ubuntu 22.04's toolchain"
echo "to avoid AVX-512/GFNI contamination from modern system libraries."
echo ""

# Check for Docker
if ! command -v docker &> /dev/null; then
    echo "Error: docker is required but not installed."
    exit 1
fi

# Build Docker image
echo "Building Docker image (this takes ~15 minutes on first run)..."
docker build $DOCKER_OPTS -f Dockerfile.build -t voxtype-builder .

echo ""
echo "Extracting binaries to ${RELEASE_DIR}/..."
mkdir -p "$RELEASE_DIR"

# Run container to extract binaries
docker run --rm -v "$(pwd)/${RELEASE_DIR}:/output" voxtype-builder

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
    vpternlog=$(objdump -d "$binary" | grep -c vpternlog || echo 0)
    broadcast=$(objdump -d "$binary" | grep -c '{1to' || echo 0)
    gfni=$(objdump -d "$binary" | grep -cE 'vgf2p8|gf2p8' || echo 0)
    zmm=$(objdump -d "$binary" | grep -c zmm || echo 0)

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
