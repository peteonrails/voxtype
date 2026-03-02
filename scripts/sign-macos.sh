#!/bin/bash
#
# Sign a macOS binary for distribution
#
# Requires:
#   - Apple Developer ID Application certificate
#   - Certificate installed in keychain
#
# Environment variables:
#   CODESIGN_IDENTITY - Developer ID (default: auto-detect from keychain)
#
# Usage:
#   ./scripts/sign-macos.sh releases/0.5.0/voxtype-0.5.0-macos-universal

set -euo pipefail

BINARY="${1:-}"

if [[ -z "$BINARY" || ! -f "$BINARY" ]]; then
    echo "Usage: $0 BINARY_PATH"
    echo "Example: $0 releases/0.5.0/voxtype-0.5.0-macos-universal"
    exit 1
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}Signing macOS binary...${NC}"
echo "Binary: $BINARY"
echo

# Find signing identity
if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
    IDENTITY="$CODESIGN_IDENTITY"
else
    # Try to find a Developer ID certificate
    IDENTITY=$(security find-identity -v -p codesigning | grep "Developer ID Application" | head -1 | sed 's/.*"\(.*\)".*/\1/' || true)

    if [[ -z "$IDENTITY" ]]; then
        echo -e "${RED}Error: No Developer ID Application certificate found in keychain${NC}"
        echo
        echo "To sign binaries for distribution outside the Mac App Store, you need:"
        echo "  1. Apple Developer Program membership"
        echo "  2. Developer ID Application certificate"
        echo
        echo "Install certificate: Xcode > Preferences > Accounts > Manage Certificates"
        echo "Or set CODESIGN_IDENTITY environment variable"
        exit 1
    fi
fi

echo "Using identity: $IDENTITY"
echo

# Sign the binary
echo -e "${YELLOW}Signing...${NC}"
codesign --deep --force --verify --verbose \
    --sign "$IDENTITY" \
    --timestamp \
    --options runtime \
    "$BINARY"

echo
echo -e "${YELLOW}Verifying signature...${NC}"
codesign -dv --verbose=4 "$BINARY" 2>&1 | head -20

echo
echo -e "${GREEN}Signature verification:${NC}"
codesign --verify --strict --verbose=2 "$BINARY"

echo
echo -e "${GREEN}Binary signed successfully!${NC}"
echo
echo "Next steps:"
echo "  1. Notarize: ./scripts/notarize-macos.sh $BINARY"
