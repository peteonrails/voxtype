#!/bin/bash
#
# CI Build Script for voxtype
# Designed to run on TrueNAS SCALE or any Docker-capable Linux host
#
# This script builds voxtype binaries in Docker containers to ensure
# no AVX-512/GFNI instructions leak from the host's toolchain.
#
# Usage:
#   ./scripts/ci-build.sh              # Build all (AVX2 + Vulkan)
#   ./scripts/ci-build.sh avx2         # Build AVX2 only
#   ./scripts/ci-build.sh vulkan       # Build Vulkan only
#   ./scripts/ci-build.sh --version 0.4.2  # Specify version
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Default version from Cargo.toml
VERSION="${VERSION:-$(grep '^version' "$PROJECT_DIR/Cargo.toml" | head -1 | cut -d'"' -f2)}"
TARGETS=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --version|-v)
            VERSION="$2"
            shift 2
            ;;
        avx2|vulkan|avx512|all)
            TARGETS="$TARGETS $1"
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [--version VERSION] [avx2|vulkan|avx512|all]"
            echo ""
            echo "Options:"
            echo "  --version, -v    Specify version (default: from Cargo.toml)"
            echo "  avx2             Build AVX2 binary only"
            echo "  vulkan           Build Vulkan binary only"
            echo "  avx512           Build AVX-512 binary (requires AVX-512 host)"
            echo "  all              Build all binaries (default)"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Default to all if no targets specified
TARGETS="${TARGETS:-all}"

echo "=== Voxtype CI Build ==="
echo "Version: $VERSION"
echo "Targets: $TARGETS"
echo "Project: $PROJECT_DIR"
echo ""

# Create output directory
mkdir -p "$PROJECT_DIR/releases/$VERSION"

cd "$PROJECT_DIR"

# Export version for docker-compose
export VERSION

# Build each target
for target in $TARGETS; do
    echo ""
    echo "=== Building $target ==="
    echo ""

    case $target in
        all)
            # Build AVX2 and Vulkan
            docker compose -f docker-compose.build.yml build avx2 vulkan
            docker compose -f docker-compose.build.yml up --abort-on-container-exit avx2
            docker compose -f docker-compose.build.yml up --abort-on-container-exit vulkan
            ;;
        avx512)
            # AVX-512 requires special profile
            docker compose -f docker-compose.build.yml --profile avx512 build avx512
            docker compose -f docker-compose.build.yml --profile avx512 up --abort-on-container-exit avx512
            ;;
        *)
            docker compose -f docker-compose.build.yml build "$target"
            docker compose -f docker-compose.build.yml up --abort-on-container-exit "$target"
            ;;
    esac
done

echo ""
echo "=== Build Complete ==="
echo "Binaries in: $PROJECT_DIR/releases/$VERSION/"
ls -la "$PROJECT_DIR/releases/$VERSION/"

# Verify binaries
echo ""
echo "=== Verifying Binaries ==="
for binary in "$PROJECT_DIR/releases/$VERSION/"voxtype-*-linux-*; do
    if [[ -f "$binary" ]]; then
        name=$(basename "$binary")

        # Count forbidden instructions
        zmm=$(objdump -d "$binary" 2>/dev/null | grep -c zmm || echo 0)
        avx512=$(objdump -d "$binary" 2>/dev/null | grep -cE 'vpternlog|vpermt2|vpblendm|\{1to' || echo 0)
        gfni=$(objdump -d "$binary" 2>/dev/null | grep -cE 'vgf2p8' || echo 0)

        if [[ "$name" == *"avx512"* ]]; then
            # AVX-512 binary SHOULD have these
            if [[ $avx512 -gt 0 ]]; then
                echo "✓ $name: $avx512 AVX-512 instructions (expected)"
            else
                echo "⚠ $name: No AVX-512 instructions (unexpected)"
            fi
        else
            # Non-AVX512 binaries should NOT have these
            if [[ $zmm -eq 0 && $avx512 -eq 0 && $gfni -eq 0 ]]; then
                echo "✓ $name: Clean (no AVX-512/GFNI)"
            else
                echo "✗ $name: FAILED - zmm=$zmm avx512=$avx512 gfni=$gfni"
                exit 1
            fi
        fi
    fi
done

echo ""
echo "=== All builds verified ==="
