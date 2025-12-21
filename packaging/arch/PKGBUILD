# Maintainer: Peter Jackson <pete@peteonrails.com>
pkgname=voxtype
pkgver=0.4.1
pkgrel=8
pkgdesc="Push-to-talk voice-to-text for Linux (optimized for Wayland, works on X11)"
arch=('x86_64' 'aarch64')
url="https://voxtype.io"
license=('MIT')
depends=(
    'alsa-lib'
    'curl'
    'gcc-libs'
    'glibc'
)
makedepends=(
    'cargo'
    'clang'
    'cmake'
    'pkgconf'
    'shaderc'
    'vulkan-headers'
)
optdepends=(
    'wtype: keyboard simulation for Wayland (recommended, best CJK support)'
    'ydotool: keyboard simulation fallback (X11/TTY support)'
    'wl-clipboard: clipboard support'
    'libnotify: desktop notifications'
    'pipewire: audio capture (recommended)'
    'pulseaudio: audio capture (alternative)'
    'vulkan-icd-loader: GPU acceleration via Vulkan (enable with: voxtype setup gpu --enable)'
)
backup=('etc/voxtype/config.toml')
install=voxtype.install
source=("$pkgname-$pkgver.tar.gz::https://github.com/peteonrails/voxtype/archive/refs/tags/v$pkgver-$pkgrel.tar.gz")
sha256sums=('0935b4d987d123574b52a69f74f33d194ea1ce6fdd978e3ae5e17453b6fe28fd')

prepare() {
    cd "$pkgname-$pkgver-$pkgrel"
    export RUSTUP_TOOLCHAIN=stable
    cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    echo "=== BUILD ENVIRONMENT DEBUG ==="
    echo "cargo: $(which cargo)"
    cargo --version
    echo "rustc: $(which rustc)"
    rustc --version
    echo "clang: $(which clang)"
    clang --version | head -1
    echo "cmake: $(which cmake)"
    cmake --version | head -1
    echo "PATH=$PATH"
    echo "RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-unset}"
    echo "CFLAGS=$CFLAGS"
    echo "CXXFLAGS=$CXXFLAGS"
    echo "LDFLAGS=$LDFLAGS"
    env | grep -i rust || true
    echo "==============================="

    cd "$pkgname-$pkgver-$pkgrel"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target

    # Clear flags set by makepkg - they can interfere with whisper-rs/whisper.cpp build
    unset RUSTFLAGS
    unset DEBUG_RUSTFLAGS
    unset CFLAGS
    unset CXXFLAGS
    unset LDFLAGS

    # Limit parallelism to prevent cmake deadlocks during whisper-rs build
    export CMAKE_BUILD_PARALLEL_LEVEL=4
    export MAKEFLAGS="-j4"

    # Build native CPU binary (optimized for the user's machine)
    # Disable LTO to prevent linking hangs (Cargo.toml has lto=true)
    cargo build --frozen --release \
        --config 'profile.release.lto=false' \
        --config 'profile.release.codegen-units=8'
    cp target/release/voxtype voxtype-cpu

    # Build Vulkan GPU binary (for GPU acceleration)
    cargo clean
    cargo build --frozen --release --features gpu-vulkan \
        --config 'profile.release.lto=false' \
        --config 'profile.release.codegen-units=8'
    cp target/release/voxtype voxtype-vulkan
}

check() {
    cd "$pkgname-$pkgver-$pkgrel"
    export RUSTUP_TOOLCHAIN=stable
    cargo test --frozen
}

package() {
    cd "$pkgname-$pkgver-$pkgrel"

    # Install native CPU binary directly to /usr/bin/
    install -Dm755 "voxtype-cpu" "$pkgdir/usr/bin/voxtype"

    # Install Vulkan GPU binary to /usr/lib/voxtype/ for optional GPU acceleration
    install -Dm755 "voxtype-vulkan" "$pkgdir/usr/lib/voxtype/voxtype-vulkan"

    # Install default configuration
    install -Dm644 "config/default.toml" "$pkgdir/etc/voxtype/config.toml"

    # Install systemd user service
    install -Dm644 "packaging/systemd/voxtype.service" \
        "$pkgdir/usr/lib/systemd/user/voxtype.service"

    # Install documentation
    install -Dm644 "README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
    install -Dm644 "docs/INSTALL.md" "$pkgdir/usr/share/doc/$pkgname/INSTALL.md"

    # Install license
    install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"

    # Install shell completions
    install -Dm644 "packaging/completions/voxtype.bash" \
        "$pkgdir/usr/share/bash-completion/completions/voxtype"
    install -Dm644 "packaging/completions/voxtype.zsh" \
        "$pkgdir/usr/share/zsh/site-functions/_voxtype"
    install -Dm644 "packaging/completions/voxtype.fish" \
        "$pkgdir/usr/share/fish/vendor_completions.d/voxtype.fish"
}
