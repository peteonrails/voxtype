#!/bin/sh
# Voxtype multi-engine wrapper for AppImage
# Dispatches to the correct binary based on configured engine and CPU features.
# Bundled ONNX AppImages include both ONNX engine binaries and the Vulkan
# Whisper binary, so users can switch engines without changing AppImages.

VOXTYPE_LIB="${VOXTYPE_LIB:-/usr/lib/voxtype}"

# Detect which engine the user wants.
# Priority: CLI --engine flag > VOXTYPE_ENGINE env var > config file > default
detect_engine() {
    # Check CLI args for --engine
    for arg in "$@"; do
        case "$prev" in
            --engine) echo "$arg"; return ;;
        esac
        prev="$arg"
    done

    # Check environment variable
    if [ -n "$VOXTYPE_ENGINE" ]; then
        echo "$VOXTYPE_ENGINE"
        return
    fi

    # Check config file
    config="${XDG_CONFIG_HOME:-$HOME/.config}/voxtype/config.toml"
    if [ -f "$config" ]; then
        # Match uncommented engine = "..." line
        engine=$(grep -E '^\s*engine\s*=' "$config" 2>/dev/null | head -1 | sed 's/.*=\s*"\([^"]*\)".*/\1/')
        if [ -n "$engine" ]; then
            echo "$engine"
            return
        fi
    fi

    echo "default"
}

ENGINE=$(detect_engine "$@")

# Whisper engine: use the Vulkan binary if available, otherwise this
# AppImage doesn't include a Whisper CPU binary so fall through to error
case "$ENGINE" in
    whisper)
        if [ -x "$VOXTYPE_LIB/voxtype-vulkan" ]; then
            exec "$VOXTYPE_LIB/voxtype-vulkan" "$@"
        fi
        echo "Error: Whisper engine requested but no Whisper binary found in this AppImage." >&2
        echo "Use the Whisper AppImage for CPU-only Whisper, or set engine to an ONNX engine." >&2
        exit 1
        ;;
esac

# ONNX engines (parakeet, moonshine, sensevoice, paraformer, dolphin, omnilingual)
# or default: use ONNX binaries with CPU dispatch

# Detect AVX-512 support
if [ -f /proc/cpuinfo ] && grep -q avx512f /proc/cpuinfo 2>/dev/null; then
    if [ -x "$VOXTYPE_LIB/voxtype-onnx-avx512" ]; then
        exec "$VOXTYPE_LIB/voxtype-onnx-avx512" "$@"
    fi
fi

# Fall back to AVX2
if [ -x "$VOXTYPE_LIB/voxtype-onnx-avx2" ]; then
    exec "$VOXTYPE_LIB/voxtype-onnx-avx2" "$@"
fi

# Single ONNX binary (CUDA or ROCm AppImage)
if [ -x "$VOXTYPE_LIB/voxtype-onnx-cuda" ]; then
    exec "$VOXTYPE_LIB/voxtype-onnx-cuda" "$@"
fi

echo "Error: No voxtype binary found in $VOXTYPE_LIB" >&2
exit 1
