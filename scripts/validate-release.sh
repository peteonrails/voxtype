#!/bin/bash
# Validate release binaries before publishing
#
# Checks:
#   1. All expected binaries exist
#   2. All binaries report the correct version
#   3. All binaries have unique SHA256 hashes
#   4. AVX2/Vulkan binaries have no AVX-512 contamination
#   5. AVX-512 binaries have AVX-512 instructions
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

# Define expected binaries
WHISPER_BINARIES=(
    "voxtype-${VERSION}-linux-x86_64-avx2"
    "voxtype-${VERSION}-linux-x86_64-avx512"
    "voxtype-${VERSION}-linux-x86_64-vulkan"
)

PARAKEET_BINARIES=(
    "voxtype-${VERSION}-linux-x86_64-parakeet-avx2"
    "voxtype-${VERSION}-linux-x86_64-parakeet-avx512"
    "voxtype-${VERSION}-linux-x86_64-parakeet-cuda"
)

# Binaries that must NOT have AVX-512 instructions
MUST_BE_CLEAN=(
    "voxtype-${VERSION}-linux-x86_64-avx2"
    "voxtype-${VERSION}-linux-x86_64-vulkan"
)

# Binaries that MUST have AVX-512 instructions
MUST_HAVE_AVX512=(
    "voxtype-${VERSION}-linux-x86_64-avx512"
    "voxtype-${VERSION}-linux-x86_64-parakeet-avx512"
)

FAILED=false

# 1. Check all binaries exist
echo "Checking binary existence..."
ALL_BINARIES=("${WHISPER_BINARIES[@]}" "${PARAKEET_BINARIES[@]}")
FOUND_BINARIES=()

for binary in "${ALL_BINARIES[@]}"; do
    if [[ -f "${RELEASE_DIR}/${binary}" ]]; then
        echo "  ✓ ${binary}"
        FOUND_BINARIES+=("$binary")
    else
        echo "  ✗ ${binary} (NOT FOUND)"
    fi
done
echo ""

if [[ ${#FOUND_BINARIES[@]} -eq 0 ]]; then
    echo "ERROR: No binaries found!"
    exit 1
fi

# 2. Check version strings
echo "Checking version strings..."
for binary in "${FOUND_BINARIES[@]}"; do
    REPORTED_VERSION=$("${RELEASE_DIR}/${binary}" --version 2>&1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
    if [[ "$REPORTED_VERSION" == "$VERSION" ]]; then
        echo "  ✓ ${binary}: v${REPORTED_VERSION}"
    else
        echo "  ✗ ${binary}: reports v${REPORTED_VERSION}, expected v${VERSION}"
        FAILED=true
    fi
done
echo ""

# 3. Check all binaries have unique hashes
echo "Checking hash uniqueness..."
declare -A HASH_TO_BINARY
HASH_DUPLICATES=false

for binary in "${FOUND_BINARIES[@]}"; do
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
    for binary in "${FOUND_BINARIES[@]}"; do
        sha256sum "${RELEASE_DIR}/${binary}"
    done
fi
