#!/bin/bash
# Build VoxtypeSetup.app bundle

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
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

# Sign the app
codesign --force --deep --sign - "$APP_BUNDLE"

echo "Built: $APP_BUNDLE"
echo ""
echo "To install:"
echo "  cp -r $APP_BUNDLE /Applications/"
echo ""
echo "To run:"
echo "  open $APP_BUNDLE"
