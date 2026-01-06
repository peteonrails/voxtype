#!/bin/bash
# Package voxtype for distribution
# Creates deb and rpm packages from pre-built binaries
#
# ============================================================================
# BUILD STRATEGY
# ============================================================================
# This script automatically builds three binary variants:
#
#   - AVX2:   Built in Docker (Ubuntu 22.04) for clean toolchain
#             Compatible with most CPUs from 2013+ (Haswell, Zen 1+)
#
#   - AVX512: Built locally with native optimizations
#             For modern CPUs (Zen 4+, Intel Ice Lake+)
#
#   - Vulkan: Built locally with GPU acceleration
#             May contain some AVX-512 from local toolchain (acceptable)
#             GPU users typically have modern CPUs anyway
#
# WHY DOCKER FOR AVX2?
# Building on modern CPUs (Zen 4, etc.) can leak AVX-512/GFNI instructions
# into binaries via system libstdc++, even with RUSTFLAGS set correctly.
# This causes SIGILL crashes on older CPUs. Docker with Ubuntu 22.04
# provides a clean, older toolchain without AVX-512 optimizations.
#
# MANUAL WORKFLOW (if automation fails):
#   ./scripts/build-docker.sh              # AVX2 in Docker
#   cargo build --release                  # AVX-512 locally
#   ./scripts/package.sh --skip-build      # Package existing binaries
# ============================================================================
#
# This script builds tiered CPU binaries to support different CPUs:
#   x86_64:
#     - voxtype-avx2:   AVX2 baseline (compatible with most CPUs from 2013+)
#     - voxtype-avx512: AVX-512 optimized (Zen 4+, some Intel)
#   aarch64:
#     - voxtype:        Single binary (no CPU feature tiers needed)
#
# The packages include a post-install script that detects CPU capabilities
# and creates a symlink to the appropriate binary.
#
# Requirements:
#   - fpm: gem install fpm
#   - rpmbuild (for rpm): sudo dnf install rpm-build
#   - ar, tar (for deb validation)
#
# Usage:
#   ./scripts/package.sh [options] [version]
#   ./scripts/package.sh 0.3.0
#   ./scripts/package.sh --deb-only 0.3.0
#   ./scripts/package.sh --skip-build 0.3.0
#   ./scripts/package.sh --arch aarch64 0.3.0
#
# Options:
#   --deb-only    Build only the Debian package
#   --rpm-only    Build only the RPM package
#   --skip-build  Skip building binaries (use existing)
#   --no-validate Skip package validation
#   --release N   Set package release number (default: 1)
#   --arch ARCH   Target architecture: x86_64 (default) or aarch64

set -e

# Parse options
BUILD_DEB=true
BUILD_RPM=true
SKIP_BUILD=false
VALIDATE=true
RELEASE="1"
TARGET_ARCH="${TARGET_ARCH:-x86_64}"  # Default to x86_64, allow env override

while [[ "$1" == --* ]]; do
    case "$1" in
        --deb-only)
            BUILD_RPM=false
            shift
            ;;
        --rpm-only)
            BUILD_DEB=false
            shift
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        --no-validate)
            VALIDATE=false
            shift
            ;;
        --release)
            RELEASE="$2"
            shift 2
            ;;
        --arch)
            TARGET_ARCH="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate architecture
case "$TARGET_ARCH" in
    x86_64|amd64)
        TARGET_ARCH="x86_64"
        DEB_ARCH="amd64"
        ;;
    aarch64|arm64)
        TARGET_ARCH="aarch64"
        DEB_ARCH="arm64"
        ;;
    *)
        echo "Error: Unsupported architecture: $TARGET_ARCH"
        echo "Supported: x86_64, aarch64"
        exit 1
        ;;
esac

VERSION="${1:-$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)}"
RELEASE_DIR="releases/${VERSION}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

# Validate a .deb package
# Checks for duplicate control fields and basic structure
validate_deb() {
    local deb_file="$1"
    local tmpdir
    tmpdir=$(mktemp -d)

    # Convert to absolute path if relative
    if [[ ! "$deb_file" = /* ]]; then
        deb_file="$PROJECT_DIR/$deb_file"
    fi

    echo "  Validating $deb_file..."

    # Extract control file
    cd "$tmpdir"
    if ! ar x "$deb_file" 2>/dev/null; then
        echo "  ERROR: Failed to extract deb archive"
        rm -rf "$tmpdir"
        cd "$PROJECT_DIR"
        return 1
    fi

    if ! tar -xf control.tar.* 2>/dev/null; then
        echo "  ERROR: Failed to extract control archive"
        rm -rf "$tmpdir"
        cd "$PROJECT_DIR"
        return 1
    fi

    # Check for duplicate fields
    local duplicates
    duplicates=$(grep -E "^[A-Z][a-zA-Z-]+:" control | cut -d: -f1 | sort | uniq -d)

    if [[ -n "$duplicates" ]]; then
        echo "  ERROR: Duplicate control fields found:"
        echo "$duplicates" | sed 's/^/    - /'
        echo ""
        echo "  Control file contents:"
        cat control | sed 's/^/    /'
        rm -rf "$tmpdir"
        cd "$PROJECT_DIR"
        return 1
    fi

    # Verify required fields exist
    local required_fields=("Package" "Version" "Architecture" "Maintainer" "Description")
    for field in "${required_fields[@]}"; do
        if ! grep -q "^${field}:" control; then
            echo "  ERROR: Missing required field: $field"
            rm -rf "$tmpdir"
            cd "$PROJECT_DIR"
            return 1
        fi
    done

    rm -rf "$tmpdir"
    cd "$PROJECT_DIR"
    echo "  Validation passed"
    return 0
}

echo "=== Packaging voxtype v${VERSION} (${TARGET_ARCH}) ==="
echo ""

# Check for fpm
if ! command -v fpm &> /dev/null; then
    echo "Error: fpm is required but not installed."
    echo "Install with: gem install fpm"
    exit 1
fi

# Ensure release directory exists
mkdir -p "$RELEASE_DIR"

# Build binaries (unless --skip-build was specified)
if [[ "$SKIP_BUILD" == "false" ]]; then
    if [[ "$TARGET_ARCH" == "x86_64" ]]; then
        # =======================================================================
        # BUILD STRATEGY:
        # - AVX2:   Built in Docker (Ubuntu 22.04) to avoid toolchain contamination
        # - AVX512: Built locally with native optimizations
        # - Vulkan: Built in Docker (TrueNAS i9-9900KF) for clean binary
        #
        # All binaries for end-users are built on systems without AVX-512/GFNI
        # to ensure compatibility with Zen 3 and older CPUs.
        # =======================================================================

        # Build AVX2 baseline binary via Docker (clean toolchain, no contamination)
        if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2" ]]; then
            echo "Building AVX2 baseline release via Docker..."
            echo "  Using Ubuntu 22.04 to avoid toolchain contamination"
            ./scripts/build-docker.sh
        fi

        # Build AVX-512 optimized binary locally (for Zen 4+, some Intel)
        if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512" ]]; then
            echo "Building AVX-512 optimized release locally..."
            cargo clean
            cargo build --release
            cp target/release/voxtype "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512"
        fi

        # Build Vulkan GPU release via Docker (clean toolchain, no AVX-512/GFNI)
        # This ensures the Vulkan binary works on all CPUs including Zen 3
        if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-vulkan" ]]; then
            echo "Building Vulkan GPU release via Docker (TrueNAS context)..."
            echo "  This ensures no AVX-512/GFNI instructions leak into the binary."
            echo "  NOTE: This can take 30+ minutes due to shader compilation."
            ./scripts/build-docker-vulkan.sh
        fi
    else
        # aarch64: Single binary, no CPU feature tiers needed
        if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-aarch64" ]]; then
            echo "Building aarch64 release..."
            cargo build --release --target aarch64-unknown-linux-gnu
            cp target/aarch64-unknown-linux-gnu/release/voxtype "${RELEASE_DIR}/voxtype-${VERSION}-linux-aarch64"
        fi
    fi
else
    echo "Skipping binary build (--skip-build)"
    if [[ "$TARGET_ARCH" == "x86_64" ]]; then
        if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2" ]]; then
            echo "Error: Binary not found: ${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2"
            exit 1
        fi
        if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512" ]]; then
            echo "Error: Binary not found: ${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512"
            exit 1
        fi
    else
        if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-aarch64" ]]; then
            echo "Error: Binary not found: ${RELEASE_DIR}/voxtype-${VERSION}-linux-aarch64"
            exit 1
        fi
    fi
fi

# Verify binaries don't have incorrect CPU instructions (x86_64 only)
# This catches build cache issues where AVX-512 instructions leak into AVX2/Vulkan binaries
if [[ "$TARGET_ARCH" == "x86_64" ]]; then
    echo ""
    echo "Verifying binary CPU instructions..."

    verify_no_forbidden_instructions() {
        local binary="$1"
        local name="$2"
        if ! command -v objdump &> /dev/null; then
            echo "  Warning: objdump not found, skipping instruction verification"
            return 0
        fi
        local zmm_count avx512_count gfni_count
        # Check for 512-bit zmm registers
        zmm_count=$(objdump -d "$binary" 2>/dev/null | grep -c zmm) || zmm_count=0
        # Check for AVX-512 specific instructions (can use xmm/ymm with EVEX encoding)
        # Includes: vpternlog, vpermt2, vpblendm, vpmov, vptestm, vpcompres, vpexpand,
        # vrangep, vreduce, vrndscale, vscalef, vfixup, vgetexp, vgetmant, vfpclass,
        # vpcmp (AVX-512 form), vpmull, vpmadd52, vpdp, vpopcnt, vpshld, vpshrd
        # Also check for EVEX broadcast syntax {1to4}, {1to8}, {1to16}, {z}, {k
        avx512_count=$(objdump -d "$binary" 2>/dev/null | grep -cE 'vpternlog|vpermt2|vpblendm|vptestm|vpcompres|vpexpand|vrangep|vreduce|vrndscale|vscalef|vfixup|vgetexp|vgetmant|vfpclass|vpmadd52|vpdp|vpopcnt|vpshld|vpshrd|\{1to[0-9]+\}|\{z\}|\{k[0-7]\}') || avx512_count=0
        # Check for GFNI instructions
        gfni_count=$(objdump -d "$binary" 2>/dev/null | grep -cE 'vgf2p8|gf2p8') || gfni_count=0
        local failed=false
        if [[ "$zmm_count" -gt 0 ]]; then
            echo "  ERROR: $name has $zmm_count AVX-512 (zmm) instructions!"
            echo "         This binary will crash on CPUs without AVX-512."
            failed=true
        fi
        if [[ "$avx512_count" -gt 0 ]]; then
            echo "  ERROR: $name has $avx512_count AVX-512 EVEX instructions!"
            echo "         This binary will crash on CPUs without AVX-512."
            failed=true
        fi
        if [[ "$gfni_count" -gt 0 ]]; then
            echo "  ERROR: $name has $gfni_count GFNI instructions!"
            echo "         This binary will crash on CPUs without GFNI (e.g., Zen 3)."
            failed=true
        fi
        if [[ "$failed" == "true" ]]; then
            echo "         The build cache was likely polluted. Try: cargo clean && re-run"
            return 1
        fi
        echo "  ✓ $name: no AVX-512 or GFNI instructions"
        return 0
    }

    verify_has_avx512() {
        local binary="$1"
        local name="$2"
        if ! command -v objdump &> /dev/null; then
            echo "  Warning: objdump not found, skipping instruction verification"
            return 0
        fi
        local zmm_count
        zmm_count=$(objdump -d "$binary" 2>/dev/null | grep -c zmm) || zmm_count=0
        if [[ "$zmm_count" -eq 0 ]]; then
            echo "  ERROR: $name has no AVX-512 (zmm) instructions!"
            echo "         This binary should be optimized for AVX-512 CPUs."
            return 1
        fi
        echo "  ✓ $name: $zmm_count AVX-512 instructions (expected)"
        return 0
    }

    VERIFY_FAILED=false

    # AVX2 binary MUST NOT have AVX-512 or GFNI instructions (strict)
    if ! verify_no_forbidden_instructions "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2" "voxtype-avx2"; then
        VERIFY_FAILED=true
    fi

    # Vulkan binary MUST NOT have AVX-512 or GFNI instructions (strict)
    # Since we now build Vulkan in Docker on TrueNAS (i9-9900KF), it should be clean
    echo ""
    if ! verify_no_forbidden_instructions "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-vulkan" "voxtype-vulkan"; then
        VERIFY_FAILED=true
    fi

    # AVX512 binary SHOULD have AVX-512 instructions
    if ! verify_has_avx512 "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512" "voxtype-avx512"; then
        VERIFY_FAILED=true
    fi

    if [[ "$VERIFY_FAILED" == "true" ]]; then
        echo ""
        echo "Binary verification FAILED!"
        echo "Remove releases/${VERSION}/ and rebuild from scratch."
        exit 1
    fi
    echo ""
fi

# Create staging directory using mktemp for portability
# Note: We don't create /usr/bin here - the postinstall script creates the symlink
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/voxtype-package.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
mkdir -p "$STAGING"/{usr/lib/voxtype,etc/voxtype,usr/lib/systemd/user,usr/share/doc/voxtype}
mkdir -p "$STAGING"/usr/share/{bash-completion/completions,zsh/site-functions,fish/vendor_completions.d}

# Copy binaries to /usr/lib/voxtype/
# The post-install script will create a symlink at /usr/bin/voxtype
if [[ "$TARGET_ARCH" == "x86_64" ]]; then
    # x86_64: Tiered CPU binaries + Vulkan GPU binary
    cp "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2" "$STAGING/usr/lib/voxtype/voxtype-avx2"
    cp "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512" "$STAGING/usr/lib/voxtype/voxtype-avx512"
    cp "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-vulkan" "$STAGING/usr/lib/voxtype/voxtype-vulkan"
    chmod 755 "$STAGING/usr/lib/voxtype/voxtype-avx2"
    chmod 755 "$STAGING/usr/lib/voxtype/voxtype-avx512"
    chmod 755 "$STAGING/usr/lib/voxtype/voxtype-vulkan"
else
    # aarch64: Single binary
    cp "${RELEASE_DIR}/voxtype-${VERSION}-linux-aarch64" "$STAGING/usr/lib/voxtype/voxtype"
    chmod 755 "$STAGING/usr/lib/voxtype/voxtype"
fi
cp config/default.toml "$STAGING/etc/voxtype/config.toml"
cp packaging/systemd/voxtype.service "$STAGING/usr/lib/systemd/user/"
cp README.md "$STAGING/usr/share/doc/voxtype/"
cp LICENSE "$STAGING/usr/share/doc/voxtype/"

# Shell completions (must be world-readable for non-root users)
cp packaging/completions/voxtype.bash "$STAGING/usr/share/bash-completion/completions/voxtype"
cp packaging/completions/voxtype.zsh "$STAGING/usr/share/zsh/site-functions/_voxtype"
cp packaging/completions/voxtype.fish "$STAGING/usr/share/fish/vendor_completions.d/voxtype.fish"
chmod 644 "$STAGING/usr/share/bash-completion/completions/voxtype"
chmod 644 "$STAGING/usr/share/zsh/site-functions/_voxtype"
chmod 644 "$STAGING/usr/share/fish/vendor_completions.d/voxtype.fish"

# Man pages (generated by build.rs during cargo build --release)
MAN_DIR=$(find target/release/build -name "man" -type d -path "*/voxtype-*/out/man" 2>/dev/null | head -1)
if [[ -n "$MAN_DIR" && -d "$MAN_DIR" ]]; then
    mkdir -p "$STAGING/usr/share/man/man1"
    cp "$MAN_DIR"/*.1 "$STAGING/usr/share/man/man1/"
    chmod 644 "$STAGING/usr/share/man/man1/"*.1
    echo "  Included $(ls "$MAN_DIR"/*.1 | wc -l) man pages"
else
    echo "  Warning: Man pages not found. Run 'cargo build --release' first."
fi

# Post-install script - detects CPU and creates symlink to appropriate binary
# Generate architecture-specific post-install script
if [[ "$TARGET_ARCH" == "x86_64" ]]; then
    cat > "$STAGING/postinstall.sh" << 'POSTINST'
#!/bin/sh
# Detect CPU capabilities and symlink the appropriate voxtype binary
#
# Binary variants (x86_64):
#   voxtype-avx2:   CPU - Works on most CPUs from 2013+ (Intel Haswell, AMD Zen)
#   voxtype-avx512: CPU - Optimized for newer CPUs (AMD Zen 4+, some Intel)
#   voxtype-vulkan: GPU - Vulkan acceleration (NVIDIA, AMD, Intel)

# Remove existing binary/symlink if present (for upgrades)
rm -f /usr/bin/voxtype

# Detect AVX-512 support (Linux-specific, falls back to AVX2 on other systems)
if [ -f /proc/cpuinfo ] && grep -q avx512f /proc/cpuinfo 2>/dev/null; then
    VARIANT="avx512"
    ln -sf /usr/lib/voxtype/voxtype-avx512 /usr/bin/voxtype
else
    VARIANT="avx2"
    ln -sf /usr/lib/voxtype/voxtype-avx2 /usr/bin/voxtype
fi

# Restore SELinux context if available (for Fedora/RHEL)
if command -v restorecon >/dev/null 2>&1; then
    restorecon /usr/bin/voxtype 2>/dev/null || true
fi

# Detect GPU for Vulkan acceleration recommendation
GPU_DETECTED=""
if [ -d /dev/dri ]; then
    # Check for render nodes (indicates GPU with driver)
    if ls /dev/dri/renderD* >/dev/null 2>&1; then
        # Try to identify the GPU
        if command -v lspci >/dev/null 2>&1; then
            GPU_INFO=$(lspci 2>/dev/null | grep -i 'vga\|3d\|display' | head -1 | sed 's/.*: //')
            if [ -n "$GPU_INFO" ]; then
                GPU_DETECTED="$GPU_INFO"
            fi
        fi
        # Fallback if lspci didn't work
        if [ -z "$GPU_DETECTED" ]; then
            GPU_DETECTED="GPU detected (install pciutils for details)"
        fi
    fi
fi

echo ""
echo "=== Voxtype Post-Installation ==="
echo ""
echo "CPU backend: $VARIANT (using voxtype-$VARIANT)"

if [ -n "$GPU_DETECTED" ]; then
    echo ""
    echo "GPU detected: $GPU_DETECTED"
    echo ""
    echo "  For GPU acceleration (faster inference), run:"
    echo "    voxtype setup gpu --enable"
    echo ""
    echo "  Requires: vulkan-icd-loader and GPU drivers"
fi

echo ""
echo "To complete setup:"
echo ""
echo "  1. Add your user to the 'input' group:"
echo "     sudo usermod -aG input \$USER"
echo ""
echo "  2. Log out and back in for group changes to take effect"
echo ""
echo "  3. Download a whisper model:"
echo "     voxtype setup --download"
echo ""
echo "  4. Start voxtype:"
echo "     systemctl --user enable --now voxtype"
echo ""
POSTINST
else
    # aarch64: Single binary, no CPU detection needed
    cat > "$STAGING/postinstall.sh" << 'POSTINST'
#!/bin/sh
# Create symlink for voxtype binary (aarch64)

# Remove existing binary/symlink if present (for upgrades)
rm -f /usr/bin/voxtype

# Create symlink to the binary
ln -sf /usr/lib/voxtype/voxtype /usr/bin/voxtype

# Restore SELinux context if available (for Fedora/RHEL)
if command -v restorecon >/dev/null 2>&1; then
    restorecon /usr/bin/voxtype 2>/dev/null || true
fi

echo ""
echo "=== Voxtype Post-Installation ==="
echo ""
echo "To complete setup:"
echo ""
echo "  1. Add your user to the 'input' group:"
echo "     sudo usermod -aG input \$USER"
echo ""
echo "  2. Log out and back in for group changes to take effect"
echo ""
echo "  3. Download a whisper model:"
echo "     voxtype setup --download"
echo ""
echo "  4. Start voxtype:"
echo "     systemctl --user enable --now voxtype"
echo ""
POSTINST
fi
chmod +x "$STAGING/postinstall.sh"

# Post-uninstall script - removes the symlink
cat > "$STAGING/postuninstall.sh" << 'POSTRM'
#!/bin/sh
# Remove symlink on package removal
rm -f /usr/bin/voxtype
POSTRM
chmod +x "$STAGING/postuninstall.sh"

DESCRIPTION="Push-to-talk voice-to-text for Linux. Optimized for Wayland, works on X11 too."

# Common fpm options
FPM_OPTS=(
    --name voxtype
    --version "$VERSION"
    --iteration "$RELEASE"
    --architecture "$TARGET_ARCH"
    --maintainer "Peter Jackson <pete@peteonrails.com>"
    --url "https://voxtype.io"
    --license "MIT"
    --description "$DESCRIPTION"
    --after-install "$STAGING/postinstall.sh"
    --after-remove "$STAGING/postuninstall.sh"
    --config-files /etc/voxtype/config.toml
    -C "$STAGING"
)

# Build deb
DEB_FILE="${RELEASE_DIR}/voxtype_${VERSION}-${RELEASE}_${DEB_ARCH}.deb"
if [[ "$BUILD_DEB" == "true" ]]; then
    echo ""
    echo "Building deb package..."
    rm -f "$DEB_FILE"
    fpm -s dir -t deb \
        "${FPM_OPTS[@]}" \
        --depends "libasound2 | libasound2t64" \
        --depends libc6 \
        --depends curl \
        --deb-recommends wtype \
        --deb-recommends wl-clipboard \
        --deb-suggests ydotool \
        --deb-suggests libnotify-bin \
        --deb-suggests libvulkan1 \
        --package "$DEB_FILE" \
        usr etc

    echo "  Created: $DEB_FILE"

    # Validate the deb package
    if [[ "$VALIDATE" == "true" ]]; then
        if ! validate_deb "$DEB_FILE"; then
            echo ""
            echo "ERROR: Debian package validation failed!"
            rm -f "$DEB_FILE"
            rm -rf "$STAGING"
            exit 1
        fi
    fi
fi

# Build rpm
RPM_FILE="${RELEASE_DIR}/voxtype-${VERSION}-${RELEASE}.${TARGET_ARCH}.rpm"
if [[ "$BUILD_RPM" == "true" ]]; then
    echo ""
    echo "Building rpm package..."
    rm -f "$RPM_FILE"
    fpm -s dir -t rpm \
        "${FPM_OPTS[@]}" \
        --depends "alsa-lib" \
        --depends "glibc" \
        --depends "curl" \
        --rpm-summary "$DESCRIPTION" \
        --package "$RPM_FILE" \
        usr etc

    echo "  Created: $RPM_FILE"
fi

# Cleanup handled by trap

echo ""
echo "=== Packaging complete ==="
echo ""

# Show summary
if [[ "$BUILD_DEB" == "true" && -f "$DEB_FILE" ]]; then
    echo "Debian package: $DEB_FILE"
    echo "  Install: sudo dpkg -i $DEB_FILE"
    echo "  Test:    dpkg-deb --info $DEB_FILE"
fi

if [[ "$BUILD_RPM" == "true" && -f "$RPM_FILE" ]]; then
    echo ""
    echo "RPM package: $RPM_FILE"
    echo "  Install: sudo rpm -i $RPM_FILE"
fi

echo ""
ls -lh "$RELEASE_DIR"/*.{deb,rpm} 2>/dev/null || true
