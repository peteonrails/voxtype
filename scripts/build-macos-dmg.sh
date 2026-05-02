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

# If build-macos.sh built with ONNX engines, libonnxruntime.<ver>.dylib will
# be sitting next to the binary in releases/. Pick it up so we can bundle it
# into the .app/Contents/Frameworks/ and make the binary's rpath point at it.
ORT_DYLIB="$(ls "${RELEASES_DIR}"/libonnxruntime.*.dylib 2>/dev/null | head -1 || true)"

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

# Copy the main voxtype binary (named voxtype-bin to match CFBundleExecutable)
cp "$BINARY" "$APP_DIR/Contents/MacOS/voxtype-bin"
chmod +x "$APP_DIR/Contents/MacOS/voxtype-bin"

# If we have an ONNX Runtime dylib alongside the binary, bundle it into
# Contents/Frameworks/ and patch the binary so it can find it at runtime.
# The dylib's install_name is `@rpath/libonnxruntime.<ver>.dylib`, so we
# add `@executable_path/../Frameworks` to the binary's rpath list.
if [[ -n "$ORT_DYLIB" ]]; then
    echo -e "${YELLOW}Bundling $(basename "$ORT_DYLIB") into Frameworks/...${NC}"
    mkdir -p "$APP_DIR/Contents/Frameworks"
    cp "$ORT_DYLIB" "$APP_DIR/Contents/Frameworks/"

    # Drop any existing rpath entry first so re-runs are idempotent. The
    # delete fails harmlessly if the rpath wasn't already there.
    install_name_tool -delete_rpath "@executable_path/../Frameworks" \
        "$APP_DIR/Contents/MacOS/voxtype-bin" 2>/dev/null || true
    install_name_tool -add_rpath "@executable_path/../Frameworks" \
        "$APP_DIR/Contents/MacOS/voxtype-bin"

    # install_name_tool invalidates the existing linker signature; re-sign
    # adhoc so Gatekeeper sees a valid (if untrusted) signature. The
    # outer .app gets signed with Developer ID by sign-macos.sh later, if
    # available.
    codesign --force --sign - "$APP_DIR/Contents/MacOS/voxtype-bin"
fi

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
    <string>voxtype-bin</string>
    <key>CFBundleIdentifier</key>
    <string>io.voxtype.daemon</string>
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
    <key>NSInputMonitoringUsageDescription</key>
    <string>Voxtype monitors keyboard input to detect your push-to-talk hotkey.</string>
</dict>
</plist>
EOF

echo -e "${GREEN}App bundle created:${NC}"
echo "  $APP_DIR"
du -sh "$APP_DIR"
echo

# Create DMG with Applications symlink for drag-to-install
echo -e "${YELLOW}Creating DMG...${NC}"
rm -f "$DMG_PATH"

# Create a staging directory with the app and an Applications symlink
DMG_STAGING="${RELEASES_DIR}/dmg-staging"
rm -rf "$DMG_STAGING"
mkdir -p "$DMG_STAGING"
cp -R "$APP_DIR" "$DMG_STAGING/"
ln -s /Applications "$DMG_STAGING/Applications"

hdiutil create -volname "Voxtype ${VERSION}" \
    -srcfolder "$DMG_STAGING" \
    -ov -format UDZO \
    "$DMG_PATH"

rm -rf "$DMG_STAGING"

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
