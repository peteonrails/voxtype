# Maintainer: Peter Jackson <pete@peteonrails.com>
pkgname=voxtype
pkgver=0.3.3
pkgrel=1
pkgdesc="Push-to-talk voice-to-text for Linux (optimized for Wayland, works on X11)"
arch=('x86_64' 'aarch64')
url="https://voxtype.io"
license=('MIT')
depends=(
    'alsa-lib'
    'gcc-libs'
    'glibc'
)
makedepends=(
    'cargo'
    'clang'
    'cmake'
    'pkgconf'
    # Uncomment for GPU acceleration:
    # 'vulkan-headers'  # for Vulkan (AMD, NVIDIA, Intel)
    # 'cuda'            # for CUDA (NVIDIA only)
)
optdepends=(
    'wtype: keyboard simulation for Wayland (recommended, best CJK support)'
    'ydotool: keyboard simulation fallback (X11/TTY support)'
    'wl-clipboard: clipboard support'
    'libnotify: desktop notifications'
    'pipewire: audio capture (recommended)'
    'pulseaudio: audio capture (alternative)'
    'vulkan-icd-loader: GPU acceleration via Vulkan (enable with: voxtype setup gpu --enable)'
    'cuda: GPU acceleration for NVIDIA (requires rebuild with gpu-cuda feature)'
)
backup=('etc/voxtype/config.toml')
install=voxtype.install
source=("$pkgname-$pkgver.tar.gz::https://github.com/peteonrails/voxtype/archive/refs/tags/v$pkgver.tar.gz")
sha256sums=('127d026cfd6011dc04db6fa641b497f686e74d6ad17fd37077cb53b275317de9')

prepare() {
    cd "$pkgname-$pkgver"
    export RUSTUP_TOOLCHAIN=stable
    cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
    cd "$pkgname-$pkgver"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target

    # GPU Acceleration (optional) - uncomment ONE of these lines:
    # local _features="--features gpu-vulkan"   # AMD, NVIDIA, Intel (recommended for AMD)
    # local _features="--features gpu-cuda"     # NVIDIA only
    # local _features="--features gpu-hipblas"  # AMD ROCm

    # Build AVX2 baseline binary (compatible with most CPUs from 2013+)
    # Disable AVX-512 via compiler flags (WHISPER_NO_AVX512 doesn't work with whisper-rs-sys 0.14.1)
    CMAKE_C_FLAGS="-mno-avx512f" CMAKE_CXX_FLAGS="-mno-avx512f" cargo build --frozen --release ${_features:-}
    cp target/release/voxtype target/release/voxtype-avx2

    # Build AVX-512 optimized binary (for Zen 4+, some Intel)
    cargo clean
    cargo build --frozen --release ${_features:-}
    cp target/release/voxtype target/release/voxtype-avx512

    # Build Vulkan GPU binary (for GPU acceleration)
    cargo clean
    CMAKE_C_FLAGS="-mno-avx512f" CMAKE_CXX_FLAGS="-mno-avx512f" cargo build --frozen --release --features gpu-vulkan
    cp target/release/voxtype target/release/voxtype-vulkan
}

check() {
    cd "$pkgname-$pkgver"
    export RUSTUP_TOOLCHAIN=stable
    # Only test with AVX2 build to avoid SIGILL in build environments
    CMAKE_C_FLAGS="-mno-avx512f" CMAKE_CXX_FLAGS="-mno-avx512f" cargo test --frozen
}

package() {
    cd "$pkgname-$pkgver"

    # Install tiered binaries to /usr/lib/voxtype/
    # The install script creates a symlink at /usr/bin/voxtype
    install -Dm755 "target/release/voxtype-avx2" "$pkgdir/usr/lib/voxtype/voxtype-avx2"
    install -Dm755 "target/release/voxtype-avx512" "$pkgdir/usr/lib/voxtype/voxtype-avx512"
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
