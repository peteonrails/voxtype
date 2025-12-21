# Maintainer: Peter Jackson <pete@peteonrails.com>
pkgname=voxtype-bin
pkgver=0.4.1
pkgrel=8
pkgdesc="Push-to-talk voice-to-text for Linux (pre-built binaries)"
arch=('x86_64')
url="https://voxtype.io"
license=('MIT')
depends=(
    'alsa-lib'
    'curl'
    'gcc-libs'
    'glibc'
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
provides=('voxtype')
conflicts=('voxtype')
backup=('etc/voxtype/config.toml')
install=voxtype-bin.install
_github="https://github.com/peteonrails/voxtype/releases/download/v$pkgver-$pkgrel"
source=(
    "voxtype-avx2::$_github/voxtype-$pkgver-linux-x86_64-avx2"
    "voxtype-avx512::$_github/voxtype-$pkgver-linux-x86_64-avx512"
    "voxtype-vulkan::$_github/voxtype-$pkgver-linux-x86_64-vulkan"
    "config.toml::https://raw.githubusercontent.com/peteonrails/voxtype/v$pkgver-$pkgrel/config/default.toml"
    "voxtype.service::https://raw.githubusercontent.com/peteonrails/voxtype/v$pkgver-$pkgrel/packaging/systemd/voxtype.service"
    "voxtype.bash::https://raw.githubusercontent.com/peteonrails/voxtype/v$pkgver-$pkgrel/packaging/completions/voxtype.bash"
    "voxtype.zsh::https://raw.githubusercontent.com/peteonrails/voxtype/v$pkgver-$pkgrel/packaging/completions/voxtype.zsh"
    "voxtype.fish::https://raw.githubusercontent.com/peteonrails/voxtype/v$pkgver-$pkgrel/packaging/completions/voxtype.fish"
    "LICENSE::https://raw.githubusercontent.com/peteonrails/voxtype/v$pkgver-$pkgrel/LICENSE"
    "README.md::https://raw.githubusercontent.com/peteonrails/voxtype/v$pkgver-$pkgrel/README.md"
)
sha256sums=(
    '6f3b0bce4777221139006fab3e96e091226c6a625be8499e39c3ffb3040fc679'  # voxtype-avx2
    '10a54747ba3cbc453b5bb456d5b676619393fb05db5124a83224c7e684b6a6e5'  # voxtype-avx512
    'd5171f84f43728f492e93604950d108bf9c76c8a6c1eeda5861605339efddcc9'  # voxtype-vulkan
    'SKIP'  # config.toml
    'SKIP'  # voxtype.service
    'SKIP'  # voxtype.bash
    'SKIP'  # voxtype.zsh
    'SKIP'  # voxtype.fish
    'SKIP'  # LICENSE
    'SKIP'  # README.md
)

package() {
    # Install CPU binaries to /usr/lib/voxtype/
    install -Dm755 "$srcdir/voxtype-avx2" "$pkgdir/usr/lib/voxtype/voxtype-avx2"
    install -Dm755 "$srcdir/voxtype-avx512" "$pkgdir/usr/lib/voxtype/voxtype-avx512"

    # Install Vulkan GPU binary
    install -Dm755 "$srcdir/voxtype-vulkan" "$pkgdir/usr/lib/voxtype/voxtype-vulkan"

    # Create symlink based on CPU capabilities (updated in .install file)
    # Default to avx2 for safety
    mkdir -p "$pkgdir/usr/bin"
    ln -sf /usr/lib/voxtype/voxtype-avx2 "$pkgdir/usr/bin/voxtype"

    # Install default configuration
    install -Dm644 "$srcdir/config.toml" "$pkgdir/etc/voxtype/config.toml"

    # Install systemd user service
    install -Dm644 "$srcdir/voxtype.service" "$pkgdir/usr/lib/systemd/user/voxtype.service"

    # Install documentation
    install -Dm644 "$srcdir/README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"

    # Install license
    install -Dm644 "$srcdir/LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"

    # Install shell completions
    install -Dm644 "$srcdir/voxtype.bash" "$pkgdir/usr/share/bash-completion/completions/voxtype"
    install -Dm644 "$srcdir/voxtype.zsh" "$pkgdir/usr/share/zsh/site-functions/_voxtype"
    install -Dm644 "$srcdir/voxtype.fish" "$pkgdir/usr/share/fish/vendor_completions.d/voxtype.fish"
}
