Name:           voxtype
Version:        1.2.2
Release:        1%{?dist}
Summary:        Push-to-talk voice-to-text for Linux

License:        MIT
URL:            https://github.com/peteonrails/voxtype
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

# Binaries live in /usr/lib/voxtype on every architecture, not %%{_libdir}
# (/usr/lib64 on Fedora x86_64): `voxtype setup gpu`, the wrapper script and
# the OSD launcher all look for them there.
%global vt_libdir %{_prefix}/lib/voxtype

# Build with the same flags as the release Docker images, not the LTO and
# hardening flags Fedora's %%build exports by default.
%undefine _auto_set_build_flags
# Release profile strips the binaries, so there is no debuginfo to extract.
%global debug_package %{nil}

# GGUF engines (Parakeet and Cohere .gguf models) run in voxtype-gguf-worker,
# which links transcribe.cpp; the daemon only carries the client.
%global vt_gguf gguf,cohere-gguf,parakeet-gguf

# Same AVX-512/GFNI floor for every build except the AVX-512 variant.
# CMAKE_*_FLAGS replace the flags the cmake crate would pass to whisper.cpp,
# -fPIC included. Ubuntu's and Arch's GCC build PIE by default so the loss
# goes unnoticed there; Fedora's does not, and the link fails with
# "relocation R_X86_64_32S against .bss" unless -fPIC is put back.
%global vt_haswell_env RUSTFLAGS="-C target-cpu=haswell -C target-feature=-avx512f,-avx512bw,-avx512cd,-avx512dq,-avx512vl,-gfni" GGML_NATIVE=OFF GGML_AVX512=OFF CMAKE_C_FLAGS="-fPIC -mno-avx512f -mno-gfni" CMAKE_CXX_FLAGS="-fPIC -mno-avx512f -mno-gfni"

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  clang-devel
BuildRequires:  alsa-lib-devel
BuildRequires:  systemd-rpm-macros
BuildRequires:  cmake
BuildRequires:  gcc-c++
BuildRequires:  pkgconfig
# voxtype-vulkan
BuildRequires:  vulkan-headers
BuildRequires:  vulkan-loader-devel
BuildRequires:  glslc
# voxtype-gguf-worker-vulkan (ggml-vulkan's find_package(SPIRV-Headers))
BuildRequires:  spirv-headers-devel
# voxtype-osd-gtk4
BuildRequires:  gtk4-devel
BuildRequires:  gtk4-layer-shell-devel

Requires:       curl
Requires:       pipewire-alsa
Recommends:     wtype
Recommends:     wl-clipboard
Recommends:     %{name}-osd-gtk4 = %{version}-%{release}
Suggests:       ydotool
Suggests:       libnotify
Suggests:       quickshell

%description
Voxtype is a push-to-talk voice-to-text daemon for Linux.
Optimized for Wayland, works on X11 too.
Hold a hotkey while speaking, release to transcribe and output text at
your cursor position.

Features:
- Fully offline transcription using whisper.cpp, plus Parakeet and Cohere
  GGUF models through transcribe.cpp
- Native compositor keybinding support (Hyprland, Sway, River)
- Fallback chain: wtype (Wayland, CJK support), ydotool (X11), clipboard
- Configurable hotkeys, models, and output modes
- On-screen waveform display while recording (voxtype-osd)

Recommended: Use compositor keybindings (no special permissions needed).
See post-install instructions for setup.

This package includes tiered binaries:
- voxtype-avx2: CPU - Compatible with most CPUs from 2013+ (Intel Haswell, AMD Zen)
- voxtype-avx512: CPU - Optimized for newer CPUs (AMD Zen 4+, some Intel)
- voxtype-vulkan: GPU - Vulkan acceleration (NVIDIA, AMD, Intel)

The appropriate CPU binary is selected automatically at install time.
GPU acceleration can be enabled with: voxtype setup gpu --enable

It also includes the OSD launcher (voxtype-osd), the Quickshell OSD
frontend and its audio bridge, OSD styles, and the Omarchy Settings
plugin. The GTK4 OSD frontend is in the voxtype-osd-gtk4 package.

%package osd-gtk4
Summary:        GTK4 waveform OSD for Voxtype
Requires:       %{name}%{?_isa} = %{version}-%{release}

%description osd-gtk4
The GTK4 frontend for voxtype-osd: a layer-shell overlay that shows the
recording waveform. voxtype-osd uses it automatically when it is
installed and falls back to the other frontends when it is not.

%prep
%autosetup -n %{name}-%{version}

%build
export CARGO_HOME=%{_builddir}/cargo
# Each variant is copied to variants/ because the `cargo clean` before the
# next variant wipes everything under target/.

# Build AVX2 baseline binary (compatible with most CPUs from 2013+)
# Disable AVX-512 and GFNI in both Rust code and whisper.cpp to prevent SIGILL on older CPUs
# -C target-feature disables these in rustc/LLVM (affects Rust std lib and deps)
# CMAKE_*_FLAGS disable them in whisper.cpp
# GGML_AVX512=OFF disables whisper.cpp's auto-detection of AVX-512 on build machine
# GFNI (Galois Field New Instructions) is separate from AVX-512 and unsupported on Zen 3
%{vt_haswell_env} cargo build --release --locked --features %{vt_gguf}
install -D -m 755 target/release/voxtype variants/voxtype-avx2

# CPU GGUF worker, shared by the AVX2 and AVX-512 binaries. .cargo/config.toml
# holds its transcribe.cpp ISA floor (TRANSCRIBE_CMAKE_ARGS).
%{vt_haswell_env} cargo build --release --locked -p voxtype-gguf-worker
install -D -m 755 target/release/voxtype-gguf-worker variants/voxtype-gguf-worker

# Build AVX-512 optimized binary (for Zen 4+, some Intel)
cargo clean
cargo build --release --locked --features %{vt_gguf}
install -D -m 755 target/release/voxtype variants/voxtype-avx512

# Build Vulkan GPU binary (for GPU acceleration)
cargo clean
%{vt_haswell_env} cargo build --release --locked --features gpu-vulkan
install -D -m 755 target/release/voxtype variants/voxtype-vulkan

# Vulkan GGUF worker. voxtype-vulkan looks beside itself for
# voxtype-gguf-worker-vulkan before the generic CPU worker.
%{vt_haswell_env} cargo build --release --locked -p voxtype-gguf-worker --features vulkan
install -D -m 755 target/release/voxtype-gguf-worker variants/voxtype-gguf-worker-vulkan

# OSD launcher, its GTK4 and Quickshell frontends, and the audio bridge the
# Quickshell OSD reads levels from. Same invocation as Dockerfile.onnx, so
# the daemon binaries above stay free of GTK4.
%{vt_haswell_env} cargo build --release --locked --features osd-gtk4 \
    --bin voxtype-osd --bin voxtype-osd-gtk4 \
    --bin voxtype-osd-quickshell --bin voxtype-audio-bridge

# Man pages (compiles only clap and clap_mangen)
cargo xtask man target/man

%install
# Install tiered binaries to /usr/lib/voxtype/
install -D -m 755 variants/voxtype-avx2 %{buildroot}%{vt_libdir}/voxtype-avx2
install -D -m 755 variants/voxtype-avx512 %{buildroot}%{vt_libdir}/voxtype-avx512
install -D -m 755 variants/voxtype-vulkan %{buildroot}%{vt_libdir}/voxtype-vulkan
install -D -m 755 variants/voxtype-gguf-worker %{buildroot}%{vt_libdir}/voxtype-gguf-worker
install -D -m 755 variants/voxtype-gguf-worker-vulkan %{buildroot}%{vt_libdir}/voxtype-gguf-worker-vulkan

# OSD. The launcher finds its frontends in its own directory, so only it
# gets a /usr/bin link. The Quickshell launcher execs the audio bridge by
# basename, so the bridge goes on PATH.
install -D -m 755 target/release/voxtype-osd %{buildroot}%{vt_libdir}/voxtype-osd
install -D -m 755 target/release/voxtype-osd-gtk4 %{buildroot}%{vt_libdir}/voxtype-osd-gtk4
install -D -m 755 target/release/voxtype-osd-quickshell %{buildroot}%{vt_libdir}/voxtype-osd-quickshell
install -d %{buildroot}%{_bindir}
ln -s ../lib/voxtype/voxtype-osd %{buildroot}%{_bindir}/voxtype-osd
install -D -m 755 target/release/voxtype-audio-bridge %{buildroot}%{_bindir}/voxtype-audio-bridge

# Quickshell QML tree, OSD style packages and recipes: the launchers and
# the style resolver fall back to these paths when the user has no copy.
install_tree() {
    install -d "$2"
    cp -a "$1"/. "$2"/
    find "$2" -mindepth 1 \( -name '.*' -o -name '*~' \) -prune -exec rm -rf {} +
    find "$2" -type d -exec chmod 755 {} +
    find "$2" -type f -exec chmod 644 {} +
}
install_tree quickshell %{buildroot}%{_datadir}/voxtype/quickshell
install_tree examples/osd-packages %{buildroot}%{_datadir}/voxtype/osd
install_tree examples/osd-recipes %{buildroot}%{_datadir}/voxtype/osd-recipes

# Omarchy Settings plugin, staged by the same script as the deb and AUR
scripts/stage-omarchy-plugin.sh omarchy-plugin %{buildroot}%{_datadir}/voxtype/omarchy-plugin

# Man pages
install -d %{buildroot}%{_mandir}/man1
install -m 644 target/man/*.1 %{buildroot}%{_mandir}/man1/

# Install default configuration
install -D -m 644 config/default.toml %{buildroot}%{_sysconfdir}/voxtype/config.toml

# Install systemd user service
install -D -m 644 packaging/systemd/voxtype.service \
    %{buildroot}%{_userunitdir}/voxtype.service

# Install documentation
install -D -m 644 README.md %{buildroot}%{_docdir}/%{name}/README.md
install -D -m 644 docs/INSTALL.md %{buildroot}%{_docdir}/%{name}/INSTALL.md
install -D -m 644 THIRD_PARTY.md %{buildroot}%{_docdir}/%{name}/THIRD_PARTY.md

# Install license
install -D -m 644 LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE

# Install shell completions
install -D -m 644 packaging/completions/voxtype.bash \
    %{buildroot}%{_datadir}/bash-completion/completions/voxtype
install -D -m 644 packaging/completions/voxtype.zsh \
    %{buildroot}%{_datadir}/zsh/site-functions/_voxtype
install -D -m 644 packaging/completions/voxtype.fish \
    %{buildroot}%{_datadir}/fish/vendor_completions.d/voxtype.fish

# Install configuration TUI launcher (.desktop entry + terminal-picker script)
install -D -m 644 packaging/voxtype-configure.desktop \
    %{buildroot}%{_datadir}/applications/voxtype-configure.desktop
install -D -m 755 packaging/scripts/voxtype-configure-launcher \
    %{buildroot}%{_bindir}/voxtype-configure-launcher
install -D -m 644 assets/icon.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/voxtype.svg
install -D -m 644 assets/icon.png \
    %{buildroot}%{_datadir}/icons/hicolor/128x128/apps/voxtype.png

%check
export CARGO_HOME=%{_builddir}/cargo
# Only test with AVX2 build to avoid SIGILL in build environments
%{vt_haswell_env} cargo test --release --locked

%post
%systemd_user_post voxtype.service

if [ "$1" -eq 1 ]; then
    # Detect GPU for Vulkan acceleration recommendation
    GPU_DETECTED=""
    if [ -d /dev/dri ] && ls /dev/dri/renderD* >/dev/null 2>&1; then
        if command -v lspci >/dev/null 2>&1; then
            GPU_DETECTED=$(lspci 2>/dev/null | grep -i 'vga\|3d\|display' | head -1 | sed 's/.*: //')
        fi
        if [ -z "$GPU_DETECTED" ]; then
            GPU_DETECTED="GPU detected (install pciutils for details)"
        fi
    fi

    echo ""
    echo "=== Voxtype Post-Installation ==="
    echo ""
    echo "The AVX2 or AVX-512 build is selected for your CPU automatically."

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
fi

%posttrans
# /usr/bin/voxtype is a symlink to the variant in use. Runs in %%posttrans
# because the %%postun of packages built from earlier versions of this spec
# deletes the link unconditionally, after the new %%post has run.
#
# An existing link that still resolves is kept: `voxtype setup gpu --enable`
# points it at voxtype-vulkan, and an upgrade must not undo that. A missing
# or dangling one (older packages put the binaries in /usr/lib64/voxtype) is
# recreated for this CPU.
if [ ! -e %{_bindir}/voxtype ]; then
    rm -f %{_bindir}/voxtype
    if grep -qw avx512f /proc/cpuinfo 2>/dev/null; then
        ln -s %{vt_libdir}/voxtype-avx512 %{_bindir}/voxtype
    else
        ln -s %{vt_libdir}/voxtype-avx2 %{_bindir}/voxtype
    fi
    if command -v restorecon >/dev/null 2>&1; then
        restorecon %{_bindir}/voxtype 2>/dev/null || true
    fi
fi

%preun
%systemd_user_preun voxtype.service

%postun
%systemd_user_postun_with_restart voxtype.service
# Remove the symlink only on erase; on upgrade %%posttrans keeps it.
if [ "$1" -eq 0 ]; then
    rm -f %{_bindir}/voxtype
fi

%files
%{_licensedir}/%{name}/LICENSE
%{_docdir}/%{name}/README.md
%{_docdir}/%{name}/INSTALL.md
%{_docdir}/%{name}/THIRD_PARTY.md
%dir %{vt_libdir}
%{vt_libdir}/voxtype-avx2
%{vt_libdir}/voxtype-avx512
%{vt_libdir}/voxtype-vulkan
%{vt_libdir}/voxtype-gguf-worker
%{vt_libdir}/voxtype-gguf-worker-vulkan
%{vt_libdir}/voxtype-osd
%{vt_libdir}/voxtype-osd-quickshell
# The symlink is created by %%posttrans, mark as ghost so rpm -V doesn't complain
%ghost %{_bindir}/voxtype
%{_bindir}/voxtype-osd
%{_bindir}/voxtype-audio-bridge
%{_datadir}/voxtype/
%{_mandir}/man1/voxtype*.1*
%config(noreplace) %{_sysconfdir}/voxtype/config.toml
%{_userunitdir}/voxtype.service
%{_datadir}/bash-completion/completions/voxtype
%{_datadir}/zsh/site-functions/_voxtype
%{_datadir}/fish/vendor_completions.d/voxtype.fish
%{_datadir}/applications/voxtype-configure.desktop
%{_bindir}/voxtype-configure-launcher
%{_datadir}/icons/hicolor/scalable/apps/voxtype.svg
%{_datadir}/icons/hicolor/128x128/apps/voxtype.png

%files osd-gtk4
%{vt_libdir}/voxtype-osd-gtk4

%changelog
* Thu Nov 12 2026 Peter Jackson <pete@peteonrails.com> - 1.2.2-1
- Stable 1.2.2: automatic OSD frontend selection and Settings engine/model fixes
- Handle failed output pipes, paste fallback and unresponsive audio-device lookups
- Include the opt-in coding-agent setup skill and text cleanup fixes
- Retain the source packaging, worker, icon and build repairs from the release candidates
- Ship the OSD: voxtype-osd launcher, Quickshell frontend, audio bridge,
  QML tree, OSD styles and recipes; GTK4 frontend in voxtype-osd-gtk4
- Ship the Omarchy Settings plugin and man pages, as the release RPM does
- Install binaries in /usr/lib/voxtype, where voxtype setup gpu looks,
  instead of /usr/lib64/voxtype
- Keep the /usr/bin/voxtype link (and a setup gpu choice) across upgrades
- Add the Vulkan build dependencies voxtype-vulkan needs
- Build the GGUF client into every variant and ship the CPU and Vulkan
  GGUF workers, so Parakeet and Cohere .gguf models work

* Sat Dec 20 2025 Peter Jackson <pete@peteonrails.com> - 0.4.1-1
- Replace reqwest/rustls with curl for model downloads
- Fixes ring crate LTO linking errors on Arch Linux (Issue #10)
- Add curl as runtime dependency

* Thu Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.4.0-2
- Fix SIGILL crash on Zen 3 CPUs caused by GFNI instructions (Issue #4)
- GFNI (Galois Field New Instructions) is separate from AVX-512
- Add -gfni to RUSTFLAGS and -mno-gfni to CMAKE flags
- Extend binary verification to detect both AVX-512 and GFNI instructions

* Thu Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.4.0-1
- Add compositor keybinding support via 'voxtype record' command
- Add hotkey.enabled config option to disable built-in evdev hotkey
- Add 'voxtype setup waybar --install/--uninstall' for automated waybar integration
- Signal-based IPC (SIGUSR1/SIGUSR2) for external recording control
- Users can now use Hyprland/Sway keybindings without input group membership

* Thu Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.3.3-2
- Fix SIGILL crash on CPUs without AVX-512 support (Issue #4)
- Root cause: Rust std library was generating AVX-512 instructions
- Add explicit -C target-feature flags to disable all AVX-512 variants
- Add post-build verification to prevent future regressions

* Thu Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.3.3-1
- Add --extended flag to voxtype status command
- Extended status includes model, device, and backend in JSON output
- Enhanced Waybar tooltip with model/device/backend info

* Thu Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.3.2-1
- Ship Vulkan GPU binary alongside CPU binaries
- Add voxtype setup gpu command to switch between CPU and GPU backends
- Post-install now detects GPU and recommends enabling Vulkan acceleration

* Thu Dec 18 2025 Peter Jackson <pete@peteonrails.com> - 0.3.1-1
- Add on-demand model loading option (saves VRAM when not transcribing)
- Add paste output mode for non-US keyboard layouts
- Improve portability: better CPU detection, POSIX-compatible scripts

* Wed Dec 17 2025 Peter Jackson <pete@peteonrails.com> - 0.3.0-2
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

* Sun Dec 14 2025 Peter Jackson <pete@peteonrails.com> - 0.2.2-1
- Code cleanup: fix clippy warnings, derive Default, refactor build_stream
- Wire up CLI commands: setup model --list, setup systemd --status, setup waybar --json/--css

* Sun Dec 14 2025 Peter Jackson <pete@peteonrails.com> - 0.2.1-1
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

* Fri Nov 28 2025 Peter Jackson <pete@peteonrails.com> - 0.1.0-1
- Initial release
