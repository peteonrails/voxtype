#!/bin/bash
#
# Notarize a macOS binary with Apple
#
# Requires:
#   - Binary signed with Developer ID certificate
#   - App-specific password for notarization
#
# Environment variables (required):
#   APPLE_ID          - Apple Developer account email
#   APPLE_ID_PASSWORD - App-specific password (NOT your account password)
#   APPLE_TEAM_ID     - Team ID from Apple Developer account
#
# Usage:
#   ./scripts/notarize-macos.sh releases/0.5.0/voxtype-0.5.0-macos-universal

set -euo pipefail

BINARY="${1:-}"

if [[ -z "$BINARY" || ! -f "$BINARY" ]]; then
    echo "Usage: $0 BINARY_PATH"
    echo "Example: $0 releases/0.5.0/voxtype-0.5.0-macos-universal"
    exit 1
fi

# Check required environment variables
if [[ -z "${APPLE_ID:-}" ]]; then
    echo "Error: APPLE_ID environment variable not set"
    echo "Set to your Apple Developer account email"
    exit 1
fi

if [[ -z "${APPLE_ID_PASSWORD:-}" ]]; then
    echo "Error: APPLE_ID_PASSWORD environment variable not set"
    echo "Create an app-specific password at https://appleid.apple.com/account/manage"
    exit 1
fi

if [[ -z "${APPLE_TEAM_ID:-}" ]]; then
    echo "Error: APPLE_TEAM_ID environment variable not set"
    echo "Find at https://developer.apple.com/account/#/membership"
    exit 1
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}Notarizing macOS binary...${NC}"
echo "Binary: $BINARY"
echo

# Create ZIP for notarization (Apple requires a container format)
ZIP_PATH="${BINARY}.zip"
echo -e "${YELLOW}Creating ZIP for submission...${NC}"
ditto -c -k "$BINARY" "$ZIP_PATH"
echo "Created: $ZIP_PATH"
echo

# Submit for notarization
echo -e "${YELLOW}Submitting to Apple for notarization...${NC}"
echo "This may take several minutes..."
echo

xcrun notarytool submit "$ZIP_PATH" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_ID_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait

# Clean up ZIP
rm -f "$ZIP_PATH"

# Staple the notarization ticket
echo
echo -e "${YELLOW}Stapling notarization ticket...${NC}"
xcrun stapler staple "$BINARY"

# Verify
echo
echo -e "${YELLOW}Verifying notarization...${NC}"
spctl -a -v "$BINARY"

echo
echo -e "${GREEN}Notarization complete!${NC}"
echo
echo "The binary is now notarized and can be distributed."
echo "Users will not see Gatekeeper warnings when running it."
echo
echo "Next steps:"
echo "  1. Create DMG: ./scripts/build-macos-dmg.sh VERSION"
