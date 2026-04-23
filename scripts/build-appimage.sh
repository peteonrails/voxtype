#!/bin/bash
# Build AppImage packages for voxtype
# Uses pre-built release binaries from releases/{version}/
#
# Produces 3 AppImages:
#   voxtype-{ver}-x86_64.AppImage          Whisper (avx2 + avx512 + vulkan)
#   voxtype-{ver}-onnx-x86_64.AppImage     ONNX engines (onnx-avx2 + onnx-avx512 + vulkan)
#   voxtype-{ver}-onnx-cuda-x86_64.AppImage  ONNX CUDA (onnx-cuda + vulkan)
#
# Usage:
#   ./scripts/build-appimage.sh [options] VERSION
#   ./scripts/build-appimage.sh 0.6.5
#   ./scripts/build-appimage.sh --variant whisper 0.6.5
#
# Options:
#   --variant NAME   whisper, onnx, onnx-cuda, all (default: all)
#   --skip-build     Accepted for compatibility (binaries must already exist)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
VARIANT="all"
VERSION=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --variant)
            VARIANT="$2"
            shift 2
            ;;
        --skip-build)
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [--variant NAME] VERSION"
            echo ""
            echo "Variants: whisper, onnx, onnx-cuda, all"
            echo ""
            echo "Requires appimagetool (auto-downloaded if missing)"
            exit 0
            ;;
        *)
            VERSION="$1"
            shift
            ;;
    esac
done

if [[ -z "$VERSION" ]]; then
    echo "Error: VERSION is required" >&2
    echo "Usage: $0 [--variant NAME] VERSION" >&2
    exit 1
fi

RELEASE_DIR="$PROJECT_DIR/releases/$VERSION"
APPIMAGE_DIR="$PROJECT_DIR/packaging/appimage"

if [[ ! -d "$RELEASE_DIR" ]]; then
    echo "Error: Release directory not found: $RELEASE_DIR" >&2
    echo "Build binaries first or check the version number." >&2
    exit 1
fi

# Find or download appimagetool
find_appimagetool() {
    if command -v appimagetool >/dev/null 2>&1; then
        echo "appimagetool"
        return
    fi

    local cached="$HOME/.local/bin/appimagetool"
    if [[ -x "$cached" ]]; then
        echo "$cached"
        return
    fi

    echo "  Downloading appimagetool..." >&2
    mkdir -p "$HOME/.local/bin"
    curl -fsSL -o "$cached" \
        "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
    chmod +x "$cached"
    echo "$cached"
}

APPIMAGETOOL="$(find_appimagetool)"
echo "Using appimagetool: $APPIMAGETOOL"

# Populate shared files (docs, completions, config) into an AppDir
populate_shared_files() {
    local appdir="$1"

    mkdir -p "$appdir/usr/share/doc/voxtype"
    cp "$PROJECT_DIR/README.md" "$appdir/usr/share/doc/voxtype/"
    cp "$PROJECT_DIR/LICENSE" "$appdir/usr/share/doc/voxtype/"

    # Default config
    mkdir -p "$appdir/etc/voxtype"
    cp "$PROJECT_DIR/config/default.toml" "$appdir/etc/voxtype/config.toml"

    # Shell completions
    mkdir -p "$appdir/usr/share/bash-completion/completions"
    mkdir -p "$appdir/usr/share/zsh/site-functions"
    mkdir -p "$appdir/usr/share/fish/vendor_completions.d"
    cp "$PROJECT_DIR/packaging/completions/voxtype.bash" "$appdir/usr/share/bash-completion/completions/voxtype"
    cp "$PROJECT_DIR/packaging/completions/voxtype.zsh" "$appdir/usr/share/zsh/site-functions/_voxtype"
    cp "$PROJECT_DIR/packaging/completions/voxtype.fish" "$appdir/usr/share/fish/vendor_completions.d/voxtype.fish"

    # Man pages (if available from a prior cargo build --release)
    local man_dir
    man_dir=$(find "$PROJECT_DIR/target/release/build" -name "man" -type d -path "*/voxtype-*/out/man" 2>/dev/null | head -1)
    if [[ -n "$man_dir" && -d "$man_dir" ]]; then
        mkdir -p "$appdir/usr/share/man/man1"
        cp "$man_dir"/*.1 "$appdir/usr/share/man/man1/"
    fi

    # Desktop entry and icon at AppDir root (AppImage spec)
    cp "$APPIMAGE_DIR/voxtype.desktop" "$appdir/"
    cp "$APPIMAGE_DIR/voxtype.svg" "$appdir/"
}

# Copy a binary into the AppDir if it exists
copy_binary() {
    local src="$1"
    local dst="$2"
    if [[ -f "$src" ]]; then
        cp "$src" "$dst"
        chmod 755 "$dst"
        return 0
    fi
    return 1
}

# Build a single AppImage from a prepared AppDir
build_appimage() {
    local appdir="$1"
    local output_name="$2"
    local output_path="$RELEASE_DIR/$output_name"

    echo "  Building $output_name..."
    ARCH=x86_64 "$APPIMAGETOOL" "$appdir" "$output_path" 2>&1 | tail -1
    chmod +x "$output_path"
    echo "  Created: $output_path ($(du -h "$output_path" | cut -f1))"
}

# Whisper AppImage: avx2 + avx512 + vulkan
# Use VOXTYPE_GPU=1 to select the Vulkan binary at runtime
build_whisper() {
    echo ""
    echo "Building Whisper AppImage (avx2 + avx512 + vulkan)..."

    local avx2="$RELEASE_DIR/voxtype-${VERSION}-linux-x86_64-avx2"
    if [[ ! -f "$avx2" ]]; then
        echo "  Skipping: $avx2 not found" >&2
        return 1
    fi

    local appdir
    appdir="$(mktemp -d "${TMPDIR:-/tmp}/voxtype-appimage.XXXXXX")"
    trap 'rm -rf "$appdir"' RETURN

    mkdir -p "$appdir/usr/bin" "$appdir/usr/lib/voxtype"

    # CPU-adaptive wrapper (handles GPU dispatch via VOXTYPE_GPU=1)
    cp "$SCRIPT_DIR/voxtype-wrapper.sh" "$appdir/usr/bin/voxtype"
    chmod 755 "$appdir/usr/bin/voxtype"

    # Whisper CPU binaries
    copy_binary "$avx2" "$appdir/usr/lib/voxtype/voxtype-avx2"
    copy_binary "$RELEASE_DIR/voxtype-${VERSION}-linux-x86_64-avx512" \
        "$appdir/usr/lib/voxtype/voxtype-avx512" || true

    # Whisper Vulkan GPU binary
    copy_binary "$RELEASE_DIR/voxtype-${VERSION}-linux-x86_64-vulkan" \
        "$appdir/usr/lib/voxtype/voxtype-vulkan" || true

    cp "$APPIMAGE_DIR/AppRun" "$appdir/"
    chmod 755 "$appdir/AppRun"

    populate_shared_files "$appdir"
    build_appimage "$appdir" "voxtype-${VERSION}-x86_64.AppImage"
}

# ONNX AppImage: onnx-avx2 + onnx-avx512 + vulkan
# Wrapper detects engine from config/CLI/env and dispatches accordingly
build_onnx() {
    echo ""
    echo "Building ONNX AppImage (onnx-avx2 + onnx-avx512 + vulkan)..."

    local onnx_avx2="$RELEASE_DIR/voxtype-${VERSION}-linux-x86_64-onnx-avx2"
    if [[ ! -f "$onnx_avx2" ]]; then
        echo "  Skipping: $onnx_avx2 not found" >&2
        return 1
    fi

    local appdir
    appdir="$(mktemp -d "${TMPDIR:-/tmp}/voxtype-appimage.XXXXXX")"
    trap 'rm -rf "$appdir"' RETURN

    mkdir -p "$appdir/usr/bin" "$appdir/usr/lib/voxtype"

    # Multi-engine wrapper (dispatches between ONNX and Vulkan)
    cp "$APPIMAGE_DIR/voxtype-onnx-wrapper.sh" "$appdir/usr/bin/voxtype"
    chmod 755 "$appdir/usr/bin/voxtype"

    # ONNX CPU binaries
    copy_binary "$onnx_avx2" "$appdir/usr/lib/voxtype/voxtype-onnx-avx2"
    copy_binary "$RELEASE_DIR/voxtype-${VERSION}-linux-x86_64-onnx-avx512" \
        "$appdir/usr/lib/voxtype/voxtype-onnx-avx512" || true

    # Vulkan binary for whisper engine fallback
    copy_binary "$RELEASE_DIR/voxtype-${VERSION}-linux-x86_64-vulkan" \
        "$appdir/usr/lib/voxtype/voxtype-vulkan" || true

    cp "$APPIMAGE_DIR/AppRun" "$appdir/"
    chmod 755 "$appdir/AppRun"

    populate_shared_files "$appdir"
    build_appimage "$appdir" "voxtype-${VERSION}-onnx-x86_64.AppImage"
}

# ONNX CUDA AppImage: onnx-cuda + vulkan
build_onnx_cuda() {
    echo ""
    echo "Building ONNX CUDA AppImage (onnx-cuda + vulkan)..."

    local onnx_cuda="$RELEASE_DIR/voxtype-${VERSION}-linux-x86_64-onnx-cuda"
    if [[ ! -f "$onnx_cuda" ]]; then
        echo "  Skipping: $onnx_cuda not found" >&2
        return 1
    fi

    local appdir
    appdir="$(mktemp -d "${TMPDIR:-/tmp}/voxtype-appimage.XXXXXX")"
    trap 'rm -rf "$appdir"' RETURN

    mkdir -p "$appdir/usr/bin" "$appdir/usr/lib/voxtype"

    # Multi-engine wrapper
    cp "$APPIMAGE_DIR/voxtype-onnx-wrapper.sh" "$appdir/usr/bin/voxtype"
    chmod 755 "$appdir/usr/bin/voxtype"

    # ONNX CUDA binary
    copy_binary "$onnx_cuda" "$appdir/usr/lib/voxtype/voxtype-onnx-cuda"

    # Vulkan binary for whisper engine fallback
    copy_binary "$RELEASE_DIR/voxtype-${VERSION}-linux-x86_64-vulkan" \
        "$appdir/usr/lib/voxtype/voxtype-vulkan" || true

    cp "$APPIMAGE_DIR/AppRun" "$appdir/"
    chmod 755 "$appdir/AppRun"

    populate_shared_files "$appdir"
    build_appimage "$appdir" "voxtype-${VERSION}-onnx-cuda-x86_64.AppImage"
}

# Main
echo "Building voxtype AppImage packages v${VERSION}"
echo "Release dir: $RELEASE_DIR"

failed=0

case "$VARIANT" in
    whisper)
        build_whisper || failed=1
        ;;
    onnx)
        build_onnx || failed=1
        ;;
    onnx-cuda)
        build_onnx_cuda || failed=1
        ;;
    all)
        build_whisper || failed=1
        build_onnx || failed=1
        build_onnx_cuda || failed=1
        ;;
    *)
        echo "Error: Unknown variant '$VARIANT'" >&2
        echo "Valid variants: whisper, onnx, onnx-cuda, all" >&2
        exit 1
        ;;
esac

echo ""
if [[ "$failed" -eq 0 ]]; then
    echo "AppImage builds complete."
else
    echo "Some AppImage builds were skipped (missing binaries)."
fi

# List generated AppImages
echo ""
echo "Generated AppImages:"
ls -lh "$RELEASE_DIR"/*.AppImage 2>/dev/null || echo "  (none)"
