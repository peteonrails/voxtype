#!/bin/bash
# Package voxtype for distribution
# Creates deb and rpm packages from pre-built binaries
#
# This script builds tiered CPU binaries to support different CPUs:
#   - voxtype-avx2:   AVX2 baseline (compatible with most CPUs from 2013+)
#   - voxtype-avx512: AVX-512 optimized (Zen 4+, some Intel)
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
#
# Options:
#   --deb-only    Build only the Debian package
#   --rpm-only    Build only the RPM package
#   --skip-build  Skip building binaries (use existing)
#   --no-validate Skip package validation
#   --release N   Set package release number (default: 1)

set -e

# Parse options
BUILD_DEB=true
BUILD_RPM=true
SKIP_BUILD=false
VALIDATE=true
RELEASE="1"

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
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

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

echo "=== Packaging voxtype v${VERSION} ==="
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
    # Build AVX2 baseline binary (compatible with most CPUs from 2013+)
    # This disables AVX-512 to prevent SIGILL on older CPUs
    if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2" ]]; then
        echo "Building AVX2 baseline release (broad compatibility)..."
        echo "  Setting WHISPER_NO_AVX512=ON to disable AVX-512 instructions"
        WHISPER_NO_AVX512=ON cargo build --release
        cp target/release/voxtype "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2"
    fi

    # Build AVX-512 optimized binary (for Zen 4+, some Intel)
    if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512" ]]; then
        echo "Building AVX-512 optimized release..."
        # Clean build cache to ensure whisper.cpp recompiles with AVX-512 enabled
        cargo clean
        cargo build --release
        cp target/release/voxtype "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512"
    fi

    # Build Vulkan GPU release (optional)
    if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-vulkan" ]]; then
        echo "Building Vulkan GPU release..."
        WHISPER_NO_AVX512=ON cargo build --release --features gpu-vulkan
        cp target/release/voxtype "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-vulkan"
    fi
else
    echo "Skipping binary build (--skip-build)"
    if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2" ]]; then
        echo "Error: Binary not found: ${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2"
        exit 1
    fi
    if [[ ! -f "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512" ]]; then
        echo "Error: Binary not found: ${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512"
        exit 1
    fi
fi

# Create staging directory
# Note: We don't create /usr/bin here - the postinstall script creates the symlink
STAGING="/tmp/voxtype-package-$$"
rm -rf "$STAGING"
mkdir -p "$STAGING"/{usr/lib/voxtype,etc/voxtype,usr/lib/systemd/user,usr/share/doc/voxtype}
mkdir -p "$STAGING"/usr/share/{bash-completion/completions,zsh/site-functions,fish/vendor_completions.d}

# Copy tiered CPU binaries to /usr/lib/voxtype/
# The post-install script will create a symlink at /usr/bin/voxtype
cp "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx2" "$STAGING/usr/lib/voxtype/voxtype-avx2"
cp "${RELEASE_DIR}/voxtype-${VERSION}-linux-x86_64-avx512" "$STAGING/usr/lib/voxtype/voxtype-avx512"
chmod 755 "$STAGING/usr/lib/voxtype/voxtype-avx2"
chmod 755 "$STAGING/usr/lib/voxtype/voxtype-avx512"
cp config/default.toml "$STAGING/etc/voxtype/config.toml"
cp packaging/systemd/voxtype.service "$STAGING/usr/lib/systemd/user/"
cp README.md "$STAGING/usr/share/doc/voxtype/"
cp LICENSE "$STAGING/usr/share/doc/voxtype/"

# Shell completions
cp packaging/completions/voxtype.bash "$STAGING/usr/share/bash-completion/completions/voxtype"
cp packaging/completions/voxtype.zsh "$STAGING/usr/share/zsh/site-functions/_voxtype"
cp packaging/completions/voxtype.fish "$STAGING/usr/share/fish/vendor_completions.d/voxtype.fish"

# Post-install script - detects CPU and creates symlink to appropriate binary
cat > "$STAGING/postinstall.sh" << 'POSTINST'
#!/bin/bash
# Detect CPU capabilities and symlink the appropriate voxtype binary
#
# Binary variants:
#   voxtype-avx2:   Works on most CPUs from 2013+ (Intel Haswell, AMD Zen)
#   voxtype-avx512: Optimized for newer CPUs (AMD Zen 4+, some Intel)

# Remove existing binary/symlink if present (for upgrades)
rm -f /usr/bin/voxtype

# Detect AVX-512 support
if grep -q avx512f /proc/cpuinfo 2>/dev/null; then
    VARIANT="avx512"
    ln -sf /usr/lib/voxtype/voxtype-avx512 /usr/bin/voxtype
else
    VARIANT="avx2"
    ln -sf /usr/lib/voxtype/voxtype-avx2 /usr/bin/voxtype
fi

# Restore SELinux context if available (for Fedora/RHEL)
if command -v restorecon &> /dev/null; then
    restorecon /usr/bin/voxtype 2>/dev/null || true
fi

echo ""
echo "=== Voxtype Post-Installation ==="
echo ""
echo "CPU detected: $VARIANT (using voxtype-$VARIANT)"
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
chmod +x "$STAGING/postinstall.sh"

# Post-uninstall script - removes the symlink
cat > "$STAGING/postuninstall.sh" << 'POSTRM'
#!/bin/bash
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
    --architecture x86_64
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
DEB_FILE="${RELEASE_DIR}/voxtype_${VERSION}-${RELEASE}_amd64.deb"
if [[ "$BUILD_DEB" == "true" ]]; then
    echo ""
    echo "Building deb package..."
    rm -f "$DEB_FILE"
    fpm -s dir -t deb \
        "${FPM_OPTS[@]}" \
        --depends "libasound2 | libasound2t64" \
        --depends libc6 \
        --deb-recommends wtype \
        --deb-recommends wl-clipboard \
        --deb-suggests ydotool \
        --deb-suggests libnotify-bin \
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
RPM_FILE="${RELEASE_DIR}/voxtype-${VERSION}-${RELEASE}.x86_64.rpm"
if [[ "$BUILD_RPM" == "true" ]]; then
    echo ""
    echo "Building rpm package..."
    rm -f "$RPM_FILE"
    fpm -s dir -t rpm \
        "${FPM_OPTS[@]}" \
        --depends "alsa-lib" \
        --depends "glibc" \
        --rpm-summary "$DESCRIPTION" \
        --package "$RPM_FILE" \
        usr etc

    echo "  Created: $RPM_FILE"
fi

# Cleanup
rm -rf "$STAGING"

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
