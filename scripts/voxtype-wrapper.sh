#!/bin/sh
# Voxtype CPU-adaptive wrapper script
# Detects CPU capabilities and executes the appropriate binary variant.
#
# `voxtype setup variant`, `setup gpu` and `setup onnx` replace this script
# with a link (or a one-line wrapper) to the build the user chose, so an
# explicit choice is never second-guessed here.

VOXTYPE_LIB="${VOXTYPE_LIB:-/usr/lib/voxtype}"

# Whole-word CPU flag test, the same one the AUR package's install hook uses.
cpu_has() {
    grep -qw -- "$1" /proc/cpuinfo 2>/dev/null
}

# A CPU without AVX2 (Ivy Bridge and older) can run only the baseline
# build: every other x86_64 build, Vulkan included, is compiled for Haswell
# and dies with SIGILL on its first AVX2 instruction. Only x86_64 packages
# ship a baseline build, so other architectures never take this branch.
if [ -x "$VOXTYPE_LIB/voxtype-baseline" ] && ! cpu_has avx2; then
    exec "$VOXTYPE_LIB/voxtype-baseline" "$@"
fi

# GPU mode: use Vulkan binary if available and requested
# Set VOXTYPE_GPU=1 to prefer the Vulkan binary
if [ "$VOXTYPE_GPU" = "1" ] && [ -x "$VOXTYPE_LIB/voxtype-vulkan" ]; then
    if [ -z "${VOXTYPE_GGUF_WORKER:-}" ] && [ -x "$VOXTYPE_LIB/voxtype-gguf-worker-vulkan" ]; then
        VOXTYPE_GGUF_WORKER="$VOXTYPE_LIB/voxtype-gguf-worker-vulkan"
        export VOXTYPE_GGUF_WORKER
    fi
    exec "$VOXTYPE_LIB/voxtype-vulkan" "$@"
fi

# Prefer the AVX-512 binary when the CPU has it (Linux-specific)
if cpu_has avx512f && [ -x "$VOXTYPE_LIB/voxtype-avx512" ]; then
    exec "$VOXTYPE_LIB/voxtype-avx512" "$@"
fi

# AVX2 build: the default for x86_64 CPUs from 2013 on
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
