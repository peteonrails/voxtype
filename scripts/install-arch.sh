#!/bin/bash
# Build and install voxtype Arch package from local source
# Usage: ./scripts/install-arch.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="$PROJECT_ROOT/packaging/arch/build"

echo "==> Building voxtype Arch package..."

# Create clean build directory
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

# Copy PKGBUILD and substitute the project root path
sed "s|_srcdir=\".*\"|_srcdir=\"$PROJECT_ROOT\"|" \
    "$PROJECT_ROOT/packaging/arch/PKGBUILD-local" > "$BUILD_DIR/PKGBUILD"

# Build and install
cd "$BUILD_DIR"
makepkg -si --noconfirm

echo ""
echo "==> voxtype installed successfully!"
echo ""
echo "Post-install steps (if first time):"
echo "  1. Bind push-to-talk in your compositor: voxtype setup compositor hyprland"
echo "     (or sway, river), then set enabled = false under [hotkey]"
echo "  2. systemctl --user enable --now ydotool"
echo "  3. voxtype setup --download"
echo "  4. systemctl --user enable --now voxtype"
echo ""
echo "Without compositor bindings, the built-in hotkey needs the 'input' group"
echo "(sudo usermod -aG input \$USER), which lets every program read every keystroke."
echo ""
echo "To rebuild after changes: ./scripts/install-arch.sh"
