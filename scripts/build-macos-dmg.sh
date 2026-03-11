#!/bin/bash
#
# Create a DMG installer for macOS
#
# This script builds a complete Voxtype.app bundle containing:
#   - voxtype CLI binary
#   - VoxtypeMenubar.app (menu bar status icon)
#   - VoxtypeSetup.app (settings UI)
#   - Engine notification icons
#
# Requires:
#   - voxtype binary already built (arm64 or universal)
#   - Swift apps will be built automatically
#
# Usage:
#   ./scripts/build-macos-dmg.sh 0.6.0-rc1

set -euo pipefail

VERSION="${1:-}"

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 VERSION"
    echo "Example: $0 0.6.0-rc1"
    exit 1
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
RELEASES_DIR="${PROJECT_DIR}/releases/${VERSION}"
APP_DIR="${RELEASES_DIR}/Voxtype.app"

# Find the binary (try arm64 first, then universal)
if [[ -f "${RELEASES_DIR}/voxtype-${VERSION}-macos-arm64" ]]; then
    BINARY="${RELEASES_DIR}/voxtype-${VERSION}-macos-arm64"
    DMG_PATH="${RELEASES_DIR}/Voxtype-${VERSION}-macos-arm64.dmg"
elif [[ -f "${RELEASES_DIR}/voxtype-${VERSION}-macos-universal" ]]; then
    BINARY="${RELEASES_DIR}/voxtype-${VERSION}-macos-universal"
    DMG_PATH="${RELEASES_DIR}/Voxtype-${VERSION}-macos-universal.dmg"
else
    echo -e "${RED}Error: No binary found in ${RELEASES_DIR}${NC}"
    echo "Expected: voxtype-${VERSION}-macos-arm64 or voxtype-${VERSION}-macos-universal"
    exit 1
fi

echo -e "${GREEN}Building Voxtype.app for ${VERSION}...${NC}"
echo "Binary: $BINARY"
echo

# Build Swift apps
echo -e "${YELLOW}Building VoxtypeMenubar...${NC}"
cd "${PROJECT_DIR}/macos/VoxtypeMenubar"
./build-app.sh > /dev/null 2>&1
MENUBAR_APP="${PROJECT_DIR}/macos/VoxtypeMenubar/.build/VoxtypeMenubar.app"

echo -e "${YELLOW}Building VoxtypeSetup...${NC}"
cd "${PROJECT_DIR}/macos/VoxtypeSetup"
./build-app.sh > /dev/null 2>&1
SETUP_APP="${PROJECT_DIR}/macos/VoxtypeSetup/.build/VoxtypeSetup.app"

# Verify Swift apps exist
if [[ ! -d "$MENUBAR_APP" ]]; then
    echo -e "${RED}Error: VoxtypeMenubar.app not found${NC}"
    exit 1
fi

if [[ ! -d "$SETUP_APP" ]]; then
    echo -e "${RED}Error: VoxtypeSetup.app not found${NC}"
    exit 1
fi

# Create app bundle structure
echo -e "${YELLOW}Creating Voxtype.app bundle...${NC}"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy the main voxtype binary
cp "$BINARY" "$APP_DIR/Contents/MacOS/voxtype"
chmod +x "$APP_DIR/Contents/MacOS/voxtype"

# Copy VoxtypeMenubar.app
cp -R "$MENUBAR_APP" "$APP_DIR/Contents/MacOS/"

# Copy VoxtypeSetup.app
cp -R "$SETUP_APP" "$APP_DIR/Contents/MacOS/"

# Copy engine icons for notifications
if [[ -f "${PROJECT_DIR}/assets/engines/parakeet.png" ]]; then
    cp "${PROJECT_DIR}/assets/engines/parakeet.png" "$APP_DIR/Contents/Resources/"
fi
if [[ -f "${PROJECT_DIR}/assets/engines/whisper.png" ]]; then
    cp "${PROJECT_DIR}/assets/engines/whisper.png" "$APP_DIR/Contents/Resources/"
fi

# Copy app icon if it exists
if [[ -f "${PROJECT_DIR}/assets/icon.icns" ]]; then
    cp "${PROJECT_DIR}/assets/icon.icns" "$APP_DIR/Contents/Resources/AppIcon.icns"
fi

# Create Info.plist
cat > "$APP_DIR/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>voxtype</string>
    <key>CFBundleIdentifier</key>
    <string>io.voxtype.app</string>
    <key>CFBundleName</key>
    <string>Voxtype</string>
    <key>CFBundleDisplayName</key>
    <string>Voxtype</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSUIElement</key>
    <true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>Voxtype needs microphone access to record your voice for transcription.</string>
    <key>NSAppleEventsUsageDescription</key>
    <string>Voxtype uses AppleScript to type transcribed text into applications.</string>
</dict>
</plist>
EOF

echo -e "${GREEN}App bundle created:${NC}"
echo "  $APP_DIR"
du -sh "$APP_DIR"
echo

# Create DMG
echo -e "${YELLOW}Creating DMG...${NC}"
rm -f "$DMG_PATH"

hdiutil create -volname "Voxtype ${VERSION}" \
    -srcfolder "$APP_DIR" \
    -ov -format UDZO \
    "$DMG_PATH"

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

# Update the checksum file
CHECKSUM=$(shasum -a 256 "$DMG_PATH" | cut -d' ' -f1)
echo "${CHECKSUM}  $(basename "$DMG_PATH")" > "${RELEASES_DIR}/macos-SHA256SUMS.txt"

echo
echo "Next steps:"
echo "  1. Test the DMG: open '$DMG_PATH'"
echo "  2. Update Homebrew cask with new SHA256: $CHECKSUM"
echo "  3. Upload to GitHub release"
