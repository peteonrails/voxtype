#!/bin/bash
#
# Build universal binary for macOS (x86_64 + arm64)
#
# Usage:
#   ./scripts/build-macos.sh 0.5.0
#
# Outputs to releases/${VERSION}/voxtype-${VERSION}-macos-universal

set -euo pipefail

VERSION="${1:-}"

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 VERSION"
    echo "Example: $0 0.5.0"
    exit 1
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}Building voxtype ${VERSION} for macOS...${NC}"
echo

# Check we're on macOS
if [[ "$(uname)" != "Darwin" ]]; then
    echo -e "${RED}Error: This script must be run on macOS${NC}"
    exit 1
fi

# Ensure output directory exists
OUTPUT_DIR="releases/${VERSION}"
mkdir -p "$OUTPUT_DIR"

# Add targets if not already installed
echo "Checking Rust targets..."
rustup target add x86_64-apple-darwin 2>/dev/null || true
rustup target add aarch64-apple-darwin 2>/dev/null || true

# Build for x86_64
echo
echo -e "${YELLOW}Building for x86_64-apple-darwin...${NC}"
cargo build --release --target x86_64-apple-darwin --features gpu-metal

# Build for aarch64
echo
echo -e "${YELLOW}Building for aarch64-apple-darwin...${NC}"
cargo build --release --target aarch64-apple-darwin --features gpu-metal

# Create universal binary
echo
echo -e "${YELLOW}Creating universal binary...${NC}"
UNIVERSAL_PATH="${OUTPUT_DIR}/voxtype-${VERSION}-macos-universal"
lipo -create \
    target/x86_64-apple-darwin/release/voxtype \
    target/aarch64-apple-darwin/release/voxtype \
    -output "$UNIVERSAL_PATH"

# Verify the binary
echo
echo "Verifying universal binary..."
lipo -info "$UNIVERSAL_PATH"

# Make executable
chmod +x "$UNIVERSAL_PATH"

# Get file size
SIZE=$(du -h "$UNIVERSAL_PATH" | cut -f1)

echo
echo -e "${GREEN}Build complete!${NC}"
echo "  Binary: $UNIVERSAL_PATH"
echo "  Size:   $SIZE"
echo "  Architectures: $(lipo -archs "$UNIVERSAL_PATH")"

# Verify version
echo
echo "Verifying version..."
"$UNIVERSAL_PATH" --version

echo
echo "Next steps:"
echo "  1. Sign the binary:     ./scripts/sign-macos.sh $UNIVERSAL_PATH"
echo "  2. Notarize:            ./scripts/notarize-macos.sh $UNIVERSAL_PATH"
echo "  3. Create DMG:          ./scripts/build-macos-dmg.sh ${VERSION}"
