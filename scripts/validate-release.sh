#!/bin/bash
# Validate release binaries before publishing
#
# Checks:
#   1. All expected binaries exist
#   2. All binaries report the correct version
#   3. All binaries have unique SHA256 hashes
#   4. Baseline/AVX2/Vulkan binaries have no AVX-512 contamination
#   5. AVX-512 binaries have AVX-512 instructions
#   6. Every release binary has its matching GGUF worker
#   7. Main binaries and workers stay within their glibc caps
#
# Usage:
#   ./scripts/validate-release.sh [version]
#   ./scripts/validate-release.sh 0.5.0

set -e

VERSION="${1:-$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)}"
RELEASE_DIR="releases/${VERSION}"

echo "=== Validating Release v${VERSION} ==="
echo ""

# Check release directory exists
if [[ ! -d "$RELEASE_DIR" ]]; then
    echo "ERROR: Release directory not found: $RELEASE_DIR"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Define expected binaries. This list is the release manifest: anything
# named here that is missing fails validation (see #351, #436, and the
# v1.0.0-rc1 post-mortem).
WHISPER_BINARIES=(
    "voxtype-${VERSION}-linux-x86_64-baseline"
    "voxtype-${VERSION}-linux-x86_64-avx2"
    "voxtype-${VERSION}-linux-x86_64-avx512"
    "voxtype-${VERSION}-linux-x86_64-vulkan"
)

# The single onnx-cuda binary split into major-version-specific builds in
# v0.7.0, and MIGraphX replaced ROCm in the same release. This list had
# never been updated, so it expected a name nothing produces.
ONNX_BINARIES=(
    "voxtype-${VERSION}-linux-x86_64-onnx-avx2"
    "voxtype-${VERSION}-linux-x86_64-onnx-avx512"
    "voxtype-${VERSION}-linux-x86_64-onnx-cuda-12"
    "voxtype-${VERSION}-linux-x86_64-onnx-cuda-13"
    "voxtype-${VERSION}-linux-x86_64-onnx-migraphx"
)

# Every release binary has a separately built GGUF worker. The suffix keeps
# workers for different ISA floors and GPU capabilities distinct while the
# release assets share one directory.
WORKER_BINARIES=()
for binary in "${WHISPER_BINARIES[@]}" "${ONNX_BINARIES[@]}"; do
    WORKER_BINARIES+=("${binary}.gguf-worker")
done

# OSD frontends and the audio-bridge sidecar, built by Dockerfile.onnx.
# Shipped as their own release assets and consumed by the voxtype-bin AUR
# package, so a release missing them is incomplete (#488).
COMPANION_BINARIES=(
    "voxtype-${VERSION}-linux-x86_64-osd"
    "voxtype-${VERSION}-linux-x86_64-osd-gtk4"
    "voxtype-${VERSION}-linux-x86_64-osd-quickshell"
    "voxtype-${VERSION}-linux-x86_64-audio-bridge"
)

# Binaries that must NOT have AVX-512 instructions
MUST_BE_CLEAN=(
    "voxtype-${VERSION}-linux-x86_64-baseline"
    "voxtype-${VERSION}-linux-x86_64-avx2"
    "voxtype-${VERSION}-linux-x86_64-vulkan"
    "voxtype-${VERSION}-linux-x86_64-baseline.gguf-worker"
    "voxtype-${VERSION}-linux-x86_64-avx2.gguf-worker"
    "voxtype-${VERSION}-linux-x86_64-vulkan.gguf-worker"
    "voxtype-${VERSION}-linux-x86_64-onnx-avx2.gguf-worker"
    "voxtype-${VERSION}-linux-x86_64-onnx-cuda-12.gguf-worker"
    "voxtype-${VERSION}-linux-x86_64-onnx-cuda-13.gguf-worker"
    "voxtype-${VERSION}-linux-x86_64-onnx-migraphx.gguf-worker"
)

# Binaries that MUST have AVX-512 instructions
MUST_HAVE_AVX512=(
    "voxtype-${VERSION}-linux-x86_64-avx512"
    "voxtype-${VERSION}-linux-x86_64-onnx-avx512"
    "voxtype-${VERSION}-linux-x86_64-avx512.gguf-worker"
    "voxtype-${VERSION}-linux-x86_64-onnx-avx512.gguf-worker"
)

FAILED=false

# 1. Check all binaries exist
#
# A missing binary used to print an X and carry on, so a release with three
# of eight binaries validated clean. That is how #351 and #436 shipped with
# assets missing. Absence is now a hard failure.
echo "Checking binary existence..."
ALL_BINARIES=(
    "${WHISPER_BINARIES[@]}"
    "${ONNX_BINARIES[@]}"
    "${COMPANION_BINARIES[@]}"
    "${WORKER_BINARIES[@]}"
)
FOUND_BINARIES=()
MISSING_BINARIES=()

for binary in "${ALL_BINARIES[@]}"; do
    if [[ -f "${RELEASE_DIR}/${binary}" ]]; then
        echo "  ✓ ${binary}"
        FOUND_BINARIES+=("$binary")
    else
        echo "  ✗ ${binary} (NOT FOUND)"
        MISSING_BINARIES+=("$binary")
        FAILED=true
    fi
done
echo ""

if [[ ${#FOUND_BINARIES[@]} -eq 0 ]]; then
    echo "ERROR: No binaries found!"
    exit 1
fi

# 2. Check version strings, including the complete prerelease suffix.
# AVX-512 uses hash-bound compile metadata from the Docker build stage;
# the GTK helper can use exact stdout captured where its libraries exist.
# Neither alternative is represented as native execution on this runner.
echo "Checking exact version strings..."
VERSIONED_BINARIES=(
    "${WHISPER_BINARIES[@]}"
    "${ONNX_BINARIES[@]}"
    "${COMPANION_BINARIES[@]}"
)
for binary in "${VERSIONED_BINARIES[@]}"; do
    [[ -f "${RELEASE_DIR}/${binary}" ]] || continue
    if ! python3 "${SCRIPT_DIR}/release-artifacts.py" check "${RELEASE_DIR}/${binary}" "$VERSION"; then
        FAILED=true
    fi
done
echo ""

# 3. Check all binaries have unique hashes
echo "Checking hash uniqueness..."
declare -A HASH_TO_BINARY
HASH_DUPLICATES=false

for binary in "${VERSIONED_BINARIES[@]}"; do
    [[ -f "${RELEASE_DIR}/${binary}" ]] || continue
    HASH=$(sha256sum "${RELEASE_DIR}/${binary}" | cut -d' ' -f1)
    SHORT_HASH="${HASH:0:12}"

    if [[ -n "${HASH_TO_BINARY[$HASH]}" ]]; then
        echo "  ✗ ${binary} has same hash as ${HASH_TO_BINARY[$HASH]}"
        echo "    Hash: ${SHORT_HASH}..."
        HASH_DUPLICATES=true
        FAILED=true
    else
        HASH_TO_BINARY[$HASH]="$binary"
        echo "  ✓ ${binary}: ${SHORT_HASH}..."
    fi
done

if [[ "$HASH_DUPLICATES" == "true" ]]; then
    echo ""
    echo "  ERROR: Duplicate hashes detected!"
    echo "  This usually means one build overwrote another."
fi

# 6. Check glibc symbol requirements. Ubuntu 22.04 variants cap at 2.35;
# Ubuntu 24.04 variants cap at 2.39. Workers are checked against the same cap
# as the main binary they accompany.
if command -v objdump &> /dev/null; then
    echo "Checking glibc compatibility..."
    for binary in "${WHISPER_BINARIES[@]}" "${ONNX_BINARIES[@]}" "${WORKER_BINARIES[@]}"; do
        [[ -f "${RELEASE_DIR}/${binary}" ]] || continue
        case "$binary" in
            *-baseline|*-baseline.gguf-worker|*-avx2|*-avx2.gguf-worker|*-avx512|*-avx512.gguf-worker)
                # ONNX variants use Ubuntu 24.04 despite sharing these suffixes.
                if [[ "$binary" == *-onnx-* ]]; then
                    max_allowed=2.39
                else
                    max_allowed=2.35
                fi
                ;;
            *) max_allowed=2.39 ;;
        esac
        max_glibc=$(objdump -T "${RELEASE_DIR}/${binary}" 2>/dev/null \
            | grep -oE 'GLIBC_[0-9]+\.[0-9]+' \
            | sed 's/GLIBC_//' \
            | sort -V \
            | tail -1 || true)
        if [[ -z "$max_glibc" ]]; then
            echo "  ✗ ${binary}: no GLIBC symbols found"
            FAILED=true
        elif [[ "$(printf '%s\n%s\n' "$max_allowed" "$max_glibc" | sort -V | tail -1)" != "$max_allowed" ]]; then
            echo "  ✗ ${binary}: GLIBC_${max_glibc} exceeds GLIBC_${max_allowed}"
            FAILED=true
        else
            echo "  ✓ ${binary}: GLIBC_${max_glibc} (max GLIBC_${max_allowed})"
        fi
    done
    echo ""
fi
echo ""

# 4. Check AVX-512 contamination (only if objdump is available)
if command -v objdump &> /dev/null; then
    echo "Checking AVX-512 instruction contamination..."

    for binary in "${MUST_BE_CLEAN[@]}"; do
        if [[ ! -f "${RELEASE_DIR}/${binary}" ]]; then
            continue
        fi

        ZMM_COUNT=$(objdump -d "${RELEASE_DIR}/${binary}" 2>/dev/null | grep -c zmm || true)

        if [[ "$ZMM_COUNT" -gt 0 ]]; then
            echo "  ✗ ${binary}: ${ZMM_COUNT} AVX-512 instructions (CONTAMINATED)"
            FAILED=true
        else
            echo "  ✓ ${binary}: clean (no AVX-512)"
        fi
    done
    echo ""

    # 5. Check AVX-512 binaries have optimizations
    echo "Checking AVX-512 optimizations..."
    for binary in "${MUST_HAVE_AVX512[@]}"; do
        if [[ ! -f "${RELEASE_DIR}/${binary}" ]]; then
            continue
        fi

        ZMM_COUNT=$(objdump -d "${RELEASE_DIR}/${binary}" 2>/dev/null | grep -c zmm || true)

        if [[ "$ZMM_COUNT" -eq 0 ]]; then
            echo "  ✗ ${binary}: no AVX-512 instructions (NOT OPTIMIZED)"
            FAILED=true
        else
            echo "  ✓ ${binary}: ${ZMM_COUNT} AVX-512 instructions"
        fi
    done
    echo ""
else
    echo "Warning: objdump not found, skipping instruction checks"
    echo ""
fi

# Summary
echo "=== Validation Summary ==="
echo ""
echo "Binaries found: ${#FOUND_BINARIES[@]} / ${#ALL_BINARIES[@]}"

if [[ ${#MISSING_BINARIES[@]} -gt 0 ]]; then
    echo ""
    echo "Missing from the release manifest:"
    for binary in "${MISSING_BINARIES[@]}"; do
        echo "  - ${binary}"
    done
fi

if [[ "$FAILED" == "true" ]]; then
    echo ""
    echo "VALIDATION FAILED"
    echo "Fix the issues above before releasing."
    exit 1
else
    echo ""
    echo "ALL CHECKS PASSED"
    echo ""
    echo "SHA256 checksums for release:"
    echo ""
    for binary in "${VERSIONED_BINARIES[@]}" "${WORKER_BINARIES[@]}"; do
        [[ -f "${RELEASE_DIR}/${binary}" ]] || continue
        sha256sum "${RELEASE_DIR}/${binary}"
    done
fi
