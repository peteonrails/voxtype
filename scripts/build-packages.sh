#!/bin/bash
# Build distribution packages for Voxtype
# Usage: ./scripts/build-packages.sh [arch|debian|rpm|all]

set -e

VERSION="0.1.2"
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_DIR="$PROJECT_ROOT/dist"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Build the release binary
build_release() {
    log_info "Building release binary..."
    cd "$PROJECT_ROOT"
    cargo build --release
    log_info "Binary built: target/release/voxtype"
}

# Create source tarball
create_tarball() {
    log_info "Creating source tarball..."
    mkdir -p "$BUILD_DIR"

    cd "$PROJECT_ROOT"
    git archive --format=tar.gz --prefix="voxtype-$VERSION/" HEAD > "$BUILD_DIR/voxtype-$VERSION.tar.gz"

    log_info "Tarball created: $BUILD_DIR/voxtype-$VERSION.tar.gz"
}

# Build Arch package
build_arch() {
    log_info "Building Arch Linux package..."

    if ! command -v makepkg &> /dev/null; then
        log_error "makepkg not found. Are you on Arch Linux?"
        return 1
    fi

    mkdir -p "$BUILD_DIR/arch"
    cp "$PROJECT_ROOT/packaging/arch/PKGBUILD" "$BUILD_DIR/arch/"

    cd "$BUILD_DIR/arch"

    # Update source to use local tarball
    sed -i "s|source=.*|source=(\"$BUILD_DIR/voxtype-$VERSION.tar.gz\")|" PKGBUILD
    sed -i "s|sha256sums=.*|sha256sums=('SKIP')|" PKGBUILD

    makepkg -sf

    log_info "Arch package built in $BUILD_DIR/arch/"
}

# Build Debian package
build_debian() {
    log_info "Building Debian package..."

    if ! command -v dpkg-buildpackage &> /dev/null; then
        log_error "dpkg-buildpackage not found. Install devscripts package."
        return 1
    fi

    mkdir -p "$BUILD_DIR/debian-build"

    # Extract source
    cd "$BUILD_DIR/debian-build"
    tar xzf "$BUILD_DIR/voxtype-$VERSION.tar.gz"
    cd "voxtype-$VERSION"

    # Copy debian directory
    cp -r "$PROJECT_ROOT/packaging/debian" .

    # Build
    dpkg-buildpackage -us -uc -b

    mv "$BUILD_DIR/debian-build"/*.deb "$BUILD_DIR/"

    log_info "Debian package built in $BUILD_DIR/"
}

# Build RPM package
build_rpm() {
    log_info "Building RPM package..."

    if ! command -v rpmbuild &> /dev/null; then
        log_error "rpmbuild not found. Install rpm-build package."
        return 1
    fi

    # Setup rpmbuild directories
    mkdir -p ~/rpmbuild/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

    # Copy source and spec
    cp "$BUILD_DIR/voxtype-$VERSION.tar.gz" ~/rpmbuild/SOURCES/
    cp "$PROJECT_ROOT/packaging/rpm/voxtype.spec" ~/rpmbuild/SPECS/

    # Build
    rpmbuild -ba ~/rpmbuild/SPECS/voxtype.spec

    cp ~/rpmbuild/RPMS/*/*.rpm "$BUILD_DIR/"

    log_info "RPM package built in $BUILD_DIR/"
}

# Clean build artifacts
clean() {
    log_info "Cleaning build artifacts..."
    rm -rf "$BUILD_DIR"
    cargo clean
    log_info "Clean complete"
}

# Show usage
usage() {
    echo "Usage: $0 [command]"
    echo ""
    echo "Commands:"
    echo "  all      Build all packages (default)"
    echo "  arch     Build Arch Linux package only"
    echo "  debian   Build Debian package only"
    echo "  rpm      Build RPM package only"
    echo "  tarball  Create source tarball only"
    echo "  clean    Clean build artifacts"
    echo "  help     Show this help"
}

# Main
main() {
    local cmd="${1:-all}"

    case "$cmd" in
        all)
            build_release
            create_tarball
            build_arch || log_warn "Arch build skipped"
            build_debian || log_warn "Debian build skipped"
            build_rpm || log_warn "RPM build skipped"
            log_info "All packages built in $BUILD_DIR/"
            ;;
        arch)
            build_release
            create_tarball
            build_arch
            ;;
        debian)
            build_release
            create_tarball
            build_debian
            ;;
        rpm)
            build_release
            create_tarball
            build_rpm
            ;;
        tarball)
            create_tarball
            ;;
        clean)
            clean
            ;;
        help|--help|-h)
            usage
            ;;
        *)
            log_error "Unknown command: $cmd"
            usage
            exit 1
            ;;
    esac
}

main "$@"
