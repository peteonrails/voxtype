# Maintainer: Peter Jackson <pete@peteonrails.com>
pkgname=voxtype
pkgver=0.4.1
pkgrel=2
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
sha256sums=('717fedc50ab1f7ab249044c29113b483aa3818a440275b8274af62d6d85e5049')

prepare() {
    cd "$pkgname-$pkgver-$pkgrel"
    export RUSTUP_TOOLCHAIN=stable
    cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    cd "$pkgname-$pkgver-$pkgrel"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target

    # Build native CPU binary (optimized for the user's machine)
    cargo build --frozen --release
    cp target/release/voxtype target/release/voxtype-avx2

    # Build Vulkan GPU binary (for GPU acceleration)
    cargo clean
    cargo build --frozen --release --features gpu-vulkan
    cp target/release/voxtype target/release/voxtype-vulkan
}

check() {
    cd "$pkgname-$pkgver-$pkgrel"
    export RUSTUP_TOOLCHAIN=stable
    cargo test --frozen
}

package() {
    cd "$pkgname-$pkgver-$pkgrel"

    # Install binaries to /usr/lib/voxtype/
    # The install script creates a symlink at /usr/bin/voxtype
    install -Dm755 "target/release/voxtype-avx2" "$pkgdir/usr/lib/voxtype/voxtype-avx2"
    install -Dm755 "target/release/voxtype-vulkan" "$pkgdir/usr/lib/voxtype/voxtype-vulkan"

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
