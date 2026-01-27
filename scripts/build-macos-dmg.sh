#!/bin/bash
#
# Create a DMG installer for macOS
#
# Requires:
#   - Universal binary already built and signed
#   - create-dmg tool (brew install create-dmg)
#
# Usage:
#   ./scripts/build-macos-dmg.sh 0.5.0

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
NC='\033[0m'

BINARY="releases/${VERSION}/voxtype-${VERSION}-macos-universal"
DMG_PATH="releases/${VERSION}/voxtype-${VERSION}-macos-universal.dmg"

if [[ ! -f "$BINARY" ]]; then
    echo -e "${RED}Error: Binary not found: $BINARY${NC}"
    echo "Run ./scripts/build-macos.sh $VERSION first"
    exit 1
fi

echo -e "${GREEN}Creating DMG for voxtype ${VERSION}...${NC}"
echo "Binary: $BINARY"
echo

# Check for create-dmg
if ! command -v create-dmg &> /dev/null; then
    echo -e "${YELLOW}Installing create-dmg...${NC}"
    brew install create-dmg
fi

# Create temporary directory for DMG contents
TEMP_DIR=$(mktemp -d)
trap "rm -rf $TEMP_DIR" EXIT

# Copy binary
cp "$BINARY" "$TEMP_DIR/voxtype"
chmod +x "$TEMP_DIR/voxtype"

# Create README
cat > "$TEMP_DIR/README.txt" << 'EOF'
Voxtype - Push-to-talk voice-to-text for macOS

Installation:
  1. Drag 'voxtype' to /usr/local/bin or your preferred location
  2. Grant Accessibility permissions when prompted
  3. Run: voxtype setup launchd

Quick Start:
  voxtype daemon          - Start the daemon
  voxtype setup launchd   - Install as LaunchAgent (auto-start)
  voxtype setup model     - Download/select Whisper model
  voxtype --help          - Show all options

For more information, visit: https://voxtype.io
EOF

# Remove existing DMG if present
rm -f "$DMG_PATH"

# Create DMG
echo -e "${YELLOW}Creating DMG...${NC}"
create-dmg \
    --volname "Voxtype $VERSION" \
    --volicon "packaging/macos/icon.icns" \
    --background "packaging/macos/dmg-background.png" \
    --window-pos 200 120 \
    --window-size 600 400 \
    --icon-size 100 \
    --icon "voxtype" 175 190 \
    --icon "README.txt" 425 190 \
    --hide-extension "voxtype" \
    --app-drop-link 425 190 \
    "$DMG_PATH" \
    "$TEMP_DIR" 2>/dev/null || {
        # If create-dmg fails (e.g., missing background), create simple DMG
        echo -e "${YELLOW}Creating simple DMG (no custom background)...${NC}"
        hdiutil create -volname "Voxtype $VERSION" \
            -srcfolder "$TEMP_DIR" \
            -ov -format UDZO \
            "$DMG_PATH"
    }

# Get DMG size
SIZE=$(du -h "$DMG_PATH" | cut -f1)

echo
echo -e "${GREEN}DMG created successfully!${NC}"
echo "  DMG:  $DMG_PATH"
echo "  Size: $SIZE"

# Generate checksum
echo
echo "SHA256 checksum:"
shasum -a 256 "$DMG_PATH"

echo
echo "Next steps:"
echo "  1. Test the DMG by mounting it"
echo "  2. Upload to GitHub release"
echo "  3. Update Homebrew cask formula"
