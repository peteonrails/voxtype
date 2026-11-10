#!/bin/sh
# Voxtype CPU-adaptive wrapper script
# Detects CPU capabilities and executes the appropriate binary variant

VOXTYPE_LIB="${VOXTYPE_LIB:-/usr/lib/voxtype}"

# GPU mode: use Vulkan binary if available and requested
# Set VOXTYPE_GPU=1 to prefer the Vulkan binary
if [ "$VOXTYPE_GPU" = "1" ] && [ -x "$VOXTYPE_LIB/voxtype-vulkan" ]; then
    if [ -z "${VOXTYPE_GGUF_WORKER:-}" ] && [ -x "$VOXTYPE_LIB/voxtype-gguf-worker-vulkan" ]; then
        VOXTYPE_GGUF_WORKER="$VOXTYPE_LIB/voxtype-gguf-worker-vulkan"
        export VOXTYPE_GGUF_WORKER
    fi
    exec "$VOXTYPE_LIB/voxtype-vulkan" "$@"
fi

# Detect AVX-512 support (Linux-specific)
if [ -f /proc/cpuinfo ] && grep -q avx512f /proc/cpuinfo 2>/dev/null; then
    # Prefer AVX-512 binary if available
    if [ -x "$VOXTYPE_LIB/voxtype-avx512" ]; then
        exec "$VOXTYPE_LIB/voxtype-avx512" "$@"
    fi
fi

# Fall back to AVX2 (baseline for x86_64)
if [ -x "$VOXTYPE_LIB/voxtype-avx2" ]; then
    exec "$VOXTYPE_LIB/voxtype-avx2" "$@"
fi

# Final fallback for aarch64 or single-binary installs
if [ -x "$VOXTYPE_LIB/voxtype" ]; then
    exec "$VOXTYPE_LIB/voxtype" "$@"
fi

# If we get here, no binary was found
echo "Error: No voxtype binary found in $VOXTYPE_LIB" >&2
echo "Please reinstall the package." >&2
exit 1
