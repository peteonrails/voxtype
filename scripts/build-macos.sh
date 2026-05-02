#!/bin/bash
#
# Build voxtype for macOS (arm64) with all transcription engines.
#
# This script:
#   1. Downloads Microsoft's official ONNX Runtime prebuilt (cached)
#   2. Builds with --features "gpu-metal,parakeet-coreml,moonshine,sensevoice,
#      paraformer,dolphin,omnilingual,cohere"
#   3. Copies the binary and libonnxruntime.dylib into releases/${VERSION}/
#
# The DMG packaging step (build-macos-dmg.sh) bundles the dylib into
# Voxtype.app/Contents/Frameworks/ and patches the binary's rpath.
#
# Pyke's CDN (cdn.pyke.io) is unreliable from some build environments, so
# we use Microsoft's GitHub release directly with ORT_STRATEGY=system. The
# ONNX Runtime version is pinned to whatever ort 2.0.0-rc.12 targets.
#
# Usage:
#   ./scripts/build-macos.sh 0.7.0-rc1
#
# Outputs:
#   releases/${VERSION}/voxtype-${VERSION}-macos-arm64
#   releases/${VERSION}/libonnxruntime.${ORT_VERSION}.dylib

set -euo pipefail

VERSION="${1:-}"

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 VERSION"
    echo "Example: $0 0.7.0-rc1"
    exit 1
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# ONNX Runtime version that ort 2.0.0-rc.12 binds against.
# Bump this when upgrading the ort dep in Cargo.toml.
ORT_VERSION="1.24.2"

# Engines to build with. Whisper is always on; the rest are opt-in features.
# parakeet-coreml gives Parakeet CoreML acceleration on Apple Silicon.
ENGINE_FEATURES="gpu-metal,parakeet-coreml,moonshine,sensevoice,paraformer,dolphin,omnilingual,cohere"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
CACHE_DIR="${PROJECT_DIR}/.cache/onnxruntime"
RELEASES_DIR="${PROJECT_DIR}/releases/${VERSION}"

echo -e "${GREEN}Building voxtype ${VERSION} for macOS (arm64)...${NC}"
echo

if [[ "$(uname)" != "Darwin" ]]; then
    echo -e "${RED}Error: This script must be run on macOS${NC}"
    exit 1
fi

if [[ "$(uname -m)" != "arm64" ]]; then
    echo -e "${RED}Error: This script requires Apple Silicon (arm64)${NC}"
    echo "x86_64 macOS support is not yet wired up; build on an arm64 host."
    exit 1
fi

mkdir -p "$RELEASES_DIR" "$CACHE_DIR"

# ---- Fetch Microsoft ONNX Runtime prebuilt ---------------------------------

ORT_TARBALL="onnxruntime-osx-arm64-${ORT_VERSION}.tgz"
ORT_DIR="${CACHE_DIR}/onnxruntime-osx-arm64-${ORT_VERSION}"
ORT_URL="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/${ORT_TARBALL}"

if [[ ! -d "$ORT_DIR" ]]; then
    echo -e "${YELLOW}Downloading ONNX Runtime ${ORT_VERSION} from Microsoft...${NC}"
    curl -fL --max-time 300 -o "${CACHE_DIR}/${ORT_TARBALL}" "$ORT_URL"

    echo "Extracting..."
    tar -xzf "${CACHE_DIR}/${ORT_TARBALL}" -C "$CACHE_DIR"
    rm -f "${CACHE_DIR}/${ORT_TARBALL}"
else
    echo "Using cached ONNX Runtime: ${ORT_DIR}"
fi

ORT_LIB_DIR="${ORT_DIR}/lib"
ORT_DYLIB_NAME="libonnxruntime.${ORT_VERSION}.dylib"
if [[ ! -f "${ORT_LIB_DIR}/${ORT_DYLIB_NAME}" ]]; then
    echo -e "${RED}Error: expected ${ORT_DYLIB_NAME} not found in ${ORT_LIB_DIR}${NC}"
    ls -la "$ORT_LIB_DIR"
    exit 1
fi

# ---- Build voxtype with all engine features --------------------------------

echo
echo -e "${YELLOW}Building voxtype with all engines (${ENGINE_FEATURES})...${NC}"
rustup target add aarch64-apple-darwin >/dev/null 2>&1 || true

(
    cd "$PROJECT_DIR"
    ORT_STRATEGY=system \
    ORT_LIB_LOCATION="$ORT_LIB_DIR" \
    ORT_PREFER_DYNAMIC_LINK=1 \
        cargo build --release \
            --target aarch64-apple-darwin \
            --features "$ENGINE_FEATURES"
)

BUILT_BINARY="${PROJECT_DIR}/target/aarch64-apple-darwin/release/voxtype"

if [[ ! -f "$BUILT_BINARY" ]]; then
    echo -e "${RED}Error: build did not produce ${BUILT_BINARY}${NC}"
    exit 1
fi

# ---- Stage outputs into releases/ -----------------------------------------

OUTPUT_BINARY="${RELEASES_DIR}/voxtype-${VERSION}-macos-arm64"
OUTPUT_DYLIB="${RELEASES_DIR}/${ORT_DYLIB_NAME}"

cp "$BUILT_BINARY" "$OUTPUT_BINARY"
chmod +x "$OUTPUT_BINARY"
cp "${ORT_LIB_DIR}/${ORT_DYLIB_NAME}" "$OUTPUT_DYLIB"

# Smoke-test: verify the binary executes when given access to the dylib.
echo
echo "Verifying binary..."
DYLD_LIBRARY_PATH="$ORT_LIB_DIR" "$OUTPUT_BINARY" --version

BINARY_SIZE=$(du -h "$OUTPUT_BINARY" | cut -f1)
DYLIB_SIZE=$(du -h "$OUTPUT_DYLIB" | cut -f1)

echo
echo -e "${GREEN}Build complete!${NC}"
echo "  Binary:   $OUTPUT_BINARY  ($BINARY_SIZE)"
echo "  Dylib:    $OUTPUT_DYLIB  ($DYLIB_SIZE)"
echo "  Engines:  whisper, parakeet (CoreML), moonshine, sensevoice,"
echo "            paraformer, dolphin, omnilingual, cohere"
echo
echo "Next steps:"
echo "  1. Build DMG:    ./scripts/build-macos-dmg.sh ${VERSION}"
echo "  2. Sign binary:  ./scripts/sign-macos.sh ${OUTPUT_BINARY}  (optional, needs Dev ID)"
echo "  3. Notarize:     ./scripts/notarize-macos.sh <DMG>"
