Name:           voxtype
Version:        0.4.0
Release:        1%{?dist}
Summary:        Push-to-talk voice-to-text for Linux

License:        MIT
URL:            https://github.com/peteonrails/voxtype
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  clang-devel
BuildRequires:  alsa-lib-devel
BuildRequires:  systemd-rpm-macros
BuildRequires:  cmake

Recommends:     wtype
Recommends:     wl-clipboard
Suggests:       ydotool
Suggests:       libnotify
Suggests:       pipewire

%description
Voxtype is a push-to-talk voice-to-text daemon for Linux.
Optimized for Wayland, works on X11 too.
Hold a hotkey while speaking, release to transcribe and output text at
your cursor position.

Features:
- Works on any Linux desktop using kernel-level input (evdev)
- Fully offline transcription using whisper.cpp
- Fallback chain: wtype (Wayland, CJK support), ydotool (X11), clipboard
- Configurable hotkeys, models, and output modes

Note: User must be in the 'input' group for hotkey detection.

This package includes tiered binaries:
- voxtype-avx2: CPU - Compatible with most CPUs from 2013+ (Intel Haswell, AMD Zen)
- voxtype-avx512: CPU - Optimized for newer CPUs (AMD Zen 4+, some Intel)
- voxtype-vulkan: GPU - Vulkan acceleration (NVIDIA, AMD, Intel)

The appropriate CPU binary is selected automatically at install time.
GPU acceleration can be enabled with: voxtype setup gpu --enable

%prep
%autosetup -n %{name}-%{version}

%build
export CARGO_HOME=%{_builddir}/cargo

# Build AVX2 baseline binary (compatible with most CPUs from 2013+)
# Disable AVX-512 in both Rust code and whisper.cpp to prevent SIGILL on older CPUs
# -C target-feature disables AVX-512 in rustc/LLVM (affects Rust std lib and deps)
# CMAKE_*_FLAGS disable AVX-512 in whisper.cpp via -mno-avx512f
RUSTFLAGS="-C target-cpu=haswell -C target-feature=-avx512f,-avx512bw,-avx512cd,-avx512dq,-avx512vl" \
CMAKE_C_FLAGS="-mno-avx512f" CMAKE_CXX_FLAGS="-mno-avx512f" \
cargo build --release --locked
cp target/release/voxtype target/release/voxtype-avx2

# Build AVX-512 optimized binary (for Zen 4+, some Intel)
cargo clean
cargo build --release --locked
cp target/release/voxtype target/release/voxtype-avx512

# Build Vulkan GPU binary (for GPU acceleration)
cargo clean
RUSTFLAGS="-C target-cpu=haswell -C target-feature=-avx512f,-avx512bw,-avx512cd,-avx512dq,-avx512vl" \
CMAKE_C_FLAGS="-mno-avx512f" CMAKE_CXX_FLAGS="-mno-avx512f" \
cargo build --release --locked --features gpu-vulkan
cp target/release/voxtype target/release/voxtype-vulkan

%install
# Install tiered binaries to /usr/lib/voxtype/
install -D -m 755 target/release/voxtype-avx2 %{buildroot}%{_libdir}/voxtype/voxtype-avx2
install -D -m 755 target/release/voxtype-avx512 %{buildroot}%{_libdir}/voxtype/voxtype-avx512
install -D -m 755 target/release/voxtype-vulkan %{buildroot}%{_libdir}/voxtype/voxtype-vulkan

# Install default configuration
install -D -m 644 config/default.toml %{buildroot}%{_sysconfdir}/voxtype/config.toml

# Install systemd user service
install -D -m 644 packaging/systemd/voxtype.service \
    %{buildroot}%{_userunitdir}/voxtype.service

# Install documentation
install -D -m 644 README.md %{buildroot}%{_docdir}/%{name}/README.md
install -D -m 644 docs/INSTALL.md %{buildroot}%{_docdir}/%{name}/INSTALL.md

# Install license
install -D -m 644 LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE

# Install shell completions
install -D -m 644 packaging/completions/voxtype.bash \
    %{buildroot}%{_datadir}/bash-completion/completions/voxtype
install -D -m 644 packaging/completions/voxtype.zsh \
    %{buildroot}%{_datadir}/zsh/site-functions/_voxtype
install -D -m 644 packaging/completions/voxtype.fish \
    %{buildroot}%{_datadir}/fish/vendor_completions.d/voxtype.fish

%check
export CARGO_HOME=%{_builddir}/cargo
# Only test with AVX2 build to avoid SIGILL in build environments
RUSTFLAGS="-C target-cpu=haswell -C target-feature=-avx512f,-avx512bw,-avx512cd,-avx512dq,-avx512vl" \
CMAKE_C_FLAGS="-mno-avx512f" CMAKE_CXX_FLAGS="-mno-avx512f" \
cargo test --release --locked

%post
%systemd_user_post voxtype.service

# Detect CPU capabilities and symlink the appropriate binary
rm -f %{_bindir}/voxtype

# Check for AVX-512 support (Linux-specific, falls back to AVX2)
if [ -f /proc/cpuinfo ] && grep -q avx512f /proc/cpuinfo 2>/dev/null; then
    VARIANT="avx512"
    ln -sf %{_libdir}/voxtype/voxtype-avx512 %{_bindir}/voxtype
else
    VARIANT="avx2"
    ln -sf %{_libdir}/voxtype/voxtype-avx2 %{_bindir}/voxtype
fi

# Restore SELinux context if available
if command -v restorecon >/dev/null 2>&1; then
    restorecon %{_bindir}/voxtype 2>/dev/null || true
fi

# Detect GPU for Vulkan acceleration recommendation
GPU_DETECTED=""
if [ -d /dev/dri ]; then
    if ls /dev/dri/renderD* >/dev/null 2>&1; then
        if command -v lspci >/dev/null 2>&1; then
            GPU_INFO=$(lspci 2>/dev/null | grep -i 'vga\|3d\|display' | head -1 | sed 's/.*: //')
            if [ -n "$GPU_INFO" ]; then
                GPU_DETECTED="$GPU_INFO"
            fi
        fi
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
    echo "    sudo voxtype setup gpu --enable"
    echo ""
    echo "  Requires: vulkan-loader package"
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

%preun
%systemd_user_preun voxtype.service

%postun
%systemd_user_postun_with_restart voxtype.service
# Remove symlink on package removal
rm -f %{_bindir}/voxtype

%files
%{_licensedir}/%{name}/LICENSE
%{_docdir}/%{name}/README.md
%{_docdir}/%{name}/INSTALL.md
%{_libdir}/voxtype/voxtype-avx2
%{_libdir}/voxtype/voxtype-avx512
%{_libdir}/voxtype/voxtype-vulkan
# The symlink is created by %post, mark as ghost so rpm -V doesn't complain
%ghost %{_bindir}/voxtype
%config(noreplace) %{_sysconfdir}/voxtype/config.toml
%{_userunitdir}/voxtype.service
%{_datadir}/bash-completion/completions/voxtype
%{_datadir}/zsh/site-functions/_voxtype
%{_datadir}/fish/vendor_completions.d/voxtype.fish

%changelog
* Wed Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.4.0-1
- Add compositor keybinding support via 'voxtype record' command
- Add hotkey.enabled config option to disable built-in evdev hotkey
- Add 'voxtype setup waybar --install/--uninstall' for automated waybar integration
- Signal-based IPC (SIGUSR1/SIGUSR2) for external recording control
- Users can now use Hyprland/Sway keybindings without input group membership

* Wed Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.3.3-2
- Fix SIGILL crash on CPUs without AVX-512 support (Issue #4)
- Root cause: Rust std library was generating AVX-512 instructions
- Add explicit -C target-feature flags to disable all AVX-512 variants
- Add post-build verification to prevent future regressions

* Wed Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.3.3-1
- Add --extended flag to voxtype status command
- Extended status includes model, device, and backend in JSON output
- Enhanced Waybar tooltip with model/device/backend info

* Wed Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.3.2-1
- Ship Vulkan GPU binary alongside CPU binaries
- Add voxtype setup gpu command to switch between CPU and GPU backends
- Post-install now detects GPU and recommends enabling Vulkan acceleration

* Wed Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.3.1-1
- Add on-demand model loading option (saves VRAM when not transcribing)
- Add paste output mode for non-US keyboard layouts
- Improve portability: better CPU detection, POSIX-compatible scripts

* Tue Dec 17 2025 Peter Jackson <pete@peteonrails.com> - 0.3.0-2
- Add tiered CPU binaries (AVX2 baseline + AVX-512 optimized)
- Fix SIGILL crash on CPUs without AVX-512 support (Issue #4)
- Post-install script now auto-detects CPU and selects appropriate binary

* Mon Dec 15 2025 Peter Jackson <pete@peteonrails.com> - 0.3.0-1
- Add wtype support for better CJK/Unicode text output (Korean, Chinese, Japanese)
- wtype is now primary output method on Wayland (no daemon required)
- Fallback chain: wtype -> ydotool -> clipboard
- Fix systemd service not starting on login after logout
- Add output chain detection to setup and config commands
- Update positioning: optimized for Wayland, works on X11 too

* Sat Dec 14 2025 Peter Jackson <pete@peteonrails.com> - 0.2.2-1
- Code cleanup: fix clippy warnings, derive Default, refactor build_stream
- Wire up CLI commands: setup model --list, setup systemd --status, setup waybar --json/--css

* Sat Dec 14 2025 Peter Jackson <pete@peteonrails.com> - 0.2.1-1
- Add text processing: word replacements and spoken punctuation
- Add setup subcommands: systemd, waybar, model (interactive selection)
- Add large-v3-turbo model support

* Sat Dec 13 2025 Peter Jackson <pete@peteonrails.com> - 0.2.0-1
- Add optional GPU acceleration (Vulkan, CUDA, Metal, HIP/ROCm)
- Upgrade whisper-rs to 0.15.1
- Add multilingual support with language auto-detection
- Add translation to English from any language
- Fix ydotool double-typing with --key-hold parameter
- Fix Arch PKGBUILD build dependencies (cmake, pkgconf)
- Contributors: jvantillo (GPU acceleration patch)

* Sat Nov 29 2025 Peter Jackson <pete@peteonrails.com> - 0.1.2-1
- Add toggle mode: press hotkey once to start/stop recording
- Add audio feedback with configurable sound themes

* Fri Nov 28 2025 Peter Jackson <pete@peteonrails.com> - 0.1.1-1
- Add Waybar integration and status command

* Thu Nov 28 2025 Peter Jackson <pete@peteonrails.com> - 0.1.0-1
- Initial release
