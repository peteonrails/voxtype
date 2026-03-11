#!/bin/bash
# Build VoxtypeSetup.app bundle

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$SCRIPT_DIR"

# Build release
swift build -c release

# Create app bundle structure
APP_NAME="VoxtypeSetup"
APP_BUNDLE="$SCRIPT_DIR/.build/${APP_NAME}.app"
CONTENTS="$APP_BUNDLE/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"

rm -rf "$APP_BUNDLE"
mkdir -p "$MACOS" "$RESOURCES"

# Copy binary
cp ".build/release/$APP_NAME" "$MACOS/"

# Create icns from source icon
ICON_SOURCE="$REPO_ROOT/assets/icon.png"
if [ -f "$ICON_SOURCE" ]; then
    ICONSET_DIR="$SCRIPT_DIR/.build/AppIcon.iconset"
    rm -rf "$ICONSET_DIR"
    mkdir -p "$ICONSET_DIR"

    # Generate all required sizes for macOS app icons
    sips -z 16 16     "$ICON_SOURCE" --out "$ICONSET_DIR/icon_16x16.png" 2>/dev/null
    sips -z 32 32     "$ICON_SOURCE" --out "$ICONSET_DIR/icon_16x16@2x.png" 2>/dev/null
    sips -z 32 32     "$ICON_SOURCE" --out "$ICONSET_DIR/icon_32x32.png" 2>/dev/null
    sips -z 64 64     "$ICON_SOURCE" --out "$ICONSET_DIR/icon_32x32@2x.png" 2>/dev/null
    sips -z 128 128   "$ICON_SOURCE" --out "$ICONSET_DIR/icon_128x128.png" 2>/dev/null
    sips -z 256 256   "$ICON_SOURCE" --out "$ICONSET_DIR/icon_128x128@2x.png" 2>/dev/null
    sips -z 256 256   "$ICON_SOURCE" --out "$ICONSET_DIR/icon_256x256.png" 2>/dev/null
    sips -z 512 512   "$ICON_SOURCE" --out "$ICONSET_DIR/icon_256x256@2x.png" 2>/dev/null
    sips -z 512 512   "$ICON_SOURCE" --out "$ICONSET_DIR/icon_512x512.png" 2>/dev/null
    sips -z 1024 1024 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_512x512@2x.png" 2>/dev/null

    # Convert iconset to icns
    iconutil -c icns "$ICONSET_DIR" -o "$RESOURCES/AppIcon.icns"
    rm -rf "$ICONSET_DIR"
    echo "Created app icon from $ICON_SOURCE"
fi

# Create Info.plist
cat > "$CONTENTS/Info.plist" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>VoxtypeSetup</string>
    <key>CFBundleIdentifier</key>
    <string>io.voxtype.setup</string>
    <key>CFBundleName</key>
    <string>Voxtype Setup</string>
    <key>CFBundleDisplayName</key>
    <string>Voxtype Setup</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>Voxtype needs microphone access for voice-to-text transcription.</string>
    <key>NSAppleEventsUsageDescription</key>
    <string>Voxtype needs to control other applications to type transcribed text.</string>
</dict>
</plist>
EOF

# Sign the app with entitlements
ENTITLEMENTS="$SCRIPT_DIR/VoxtypeSetup.entitlements"
codesign --force --deep --sign - --entitlements "$ENTITLEMENTS" "$APP_BUNDLE"

echo "Built: $APP_BUNDLE"
echo ""
echo "To install:"
echo "  cp -r $APP_BUNDLE /Applications/"
echo ""
echo "To run:"
echo "  open $APP_BUNDLE"
