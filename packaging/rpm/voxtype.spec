Name:           voxtype
Version:        1.3.2
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

# Same AVX-512/GFNI floor for every build.
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
# voxtype-osd-gtk4 and voxtype-meetings-gtk4
BuildRequires:  gtk4-devel
BuildRequires:  gtk4-layer-shell-devel

Requires:       curl
Requires:       pipewire-alsa
Recommends:     wtype
Recommends:     wl-clipboard
Recommends:     %{name}-osd-gtk4 = %{version}-%{release}
Recommends:     %{name}-meetings-gtk4 = %{version}-%{release}
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

This package includes two builds:
- voxtype-avx2: CPU - Compatible with most CPUs from 2013+ (Intel Haswell, AMD Zen)
- voxtype-vulkan: GPU - Vulkan acceleration (NVIDIA, AMD, Intel)

The CPU build is used by default.
GPU acceleration can be enabled with: voxtype setup gpu --enable

It also includes the OSD launcher (voxtype-osd), the Quickshell OSD
frontend and its audio bridge, OSD styles, and the Omarchy Settings
plugin. The GTK4 OSD frontend is in the voxtype-osd-gtk4 package, and the
GTK4 meetings window in voxtype-meetings-gtk4.

%package osd-gtk4
Summary:        GTK4 waveform OSD for Voxtype
Requires:       %{name}%{?_isa} = %{version}-%{release}

%description osd-gtk4
The GTK4 frontend for voxtype-osd: a layer-shell overlay that shows the
recording waveform. voxtype-osd uses it automatically when it is
installed and falls back to the other frontends when it is not.

%package meetings-gtk4
Summary:        GTK4 meetings window for Voxtype
Requires:       %{name}%{?_isa} = %{version}-%{release}

%description meetings-gtk4
The GTK4 meetings window: the live transcript, past meetings, notes and
the assistant in a normal window. `voxtype meeting view` opens it on
desktops without Quickshell, and uses the Quickshell view when that is
installed.

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

# CPU GGUF worker for the AVX2 binary. .cargo/config.toml
# holds its transcribe.cpp ISA floor (TRANSCRIBE_CMAKE_ARGS).
%{vt_haswell_env} cargo build --release --locked -p voxtype-gguf-worker
install -D -m 755 target/release/voxtype-gguf-worker variants/voxtype-gguf-worker

# Build Vulkan GPU binary (for GPU acceleration)
cargo clean
%{vt_haswell_env} cargo build --release --locked --features gpu-vulkan
install -D -m 755 target/release/voxtype variants/voxtype-vulkan

# Vulkan GGUF worker. voxtype-vulkan looks beside itself for
# voxtype-gguf-worker-vulkan before the generic CPU worker.
%{vt_haswell_env} cargo build --release --locked -p voxtype-gguf-worker --features vulkan
install -D -m 755 target/release/voxtype-gguf-worker variants/voxtype-gguf-worker-vulkan

# OSD launcher, its GTK4 and Quickshell frontends, the audio bridge the
# Quickshell OSD reads levels from, and the GTK4 meetings window. Same
# invocation as Dockerfile.onnx, so the daemon binaries above stay free of
# GTK4.
%{vt_haswell_env} cargo build --release --locked --features osd-gtk4,meetings-gtk4 \
    --bin voxtype-osd --bin voxtype-osd-gtk4 \
    --bin voxtype-osd-quickshell --bin voxtype-audio-bridge \
    --bin voxtype-meetings-gtk4

# Man pages (compiles only clap and clap_mangen)
cargo xtask man target/man

%install
# Install tiered binaries to /usr/lib/voxtype/
install -D -m 755 variants/voxtype-avx2 %{buildroot}%{vt_libdir}/voxtype-avx2
install -D -m 755 variants/voxtype-vulkan %{buildroot}%{vt_libdir}/voxtype-vulkan
install -D -m 755 variants/voxtype-gguf-worker %{buildroot}%{vt_libdir}/voxtype-gguf-worker
install -D -m 755 variants/voxtype-gguf-worker-vulkan %{buildroot}%{vt_libdir}/voxtype-gguf-worker-vulkan

# OSD. The launcher finds its frontends in its own directory, so only it
# gets a /usr/bin link. The Quickshell launcher execs the audio bridge by
# basename, so the bridge goes on PATH.
install -D -m 755 target/release/voxtype-osd %{buildroot}%{vt_libdir}/voxtype-osd
install -D -m 755 target/release/voxtype-osd-gtk4 %{buildroot}%{vt_libdir}/voxtype-osd-gtk4
# `voxtype meeting view` finds the meetings window beside the daemon.
install -D -m 755 target/release/voxtype-meetings-gtk4 %{buildroot}%{vt_libdir}/voxtype-meetings-gtk4
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

# Voxtype Meetings launcher entry, and the icon both entries use
install -D -m 644 packaging/voxtype-meetings.desktop \
    %{buildroot}%{_datadir}/applications/voxtype-meetings.desktop
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

# Both builds in this package are compiled for Haswell (AVX2). The x86-64-v2
# baseline build that runs without AVX2 ships only in the prebuilt release
# packages.
if [ "$(uname -m)" = "x86_64" ] && [ -r /proc/cpuinfo ] \
    && ! grep -qw avx2 /proc/cpuinfo; then
    echo ""
    echo "WARNING: This CPU has no AVX2, and the voxtype builds in this"
    echo "package need it: they stop with an illegal instruction error."
    echo "Install the prebuilt .rpm from"
    echo "https://github.com/peteonrails/voxtype/releases instead; it"
    echo "includes voxtype-baseline, which runs on this CPU."
fi

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
    echo "CPU build: voxtype-avx2"

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
    echo "  1. Bind push-to-talk in your compositor (no special permissions):"
    echo "       voxtype setup compositor hyprland   (or sway, river)"
    echo "     or bind 'voxtype record toggle' in your desktop's keyboard shortcuts,"
    echo "     then set enabled = false under [hotkey] in ~/.config/voxtype/config.toml"
    echo ""
    echo "     If you can't bind keys in your desktop, the built-in hotkey can"
    echo "     read the keyboard instead if you join the 'input' group, but that"
    echo "     lets every program read every keystroke:"
    echo "       sudo usermod -aG input \$USER   (then log out and back in)"
    echo ""
    echo "  2. Download a whisper model:"
    echo "     voxtype setup --download"
    echo ""
    echo "  3. Start voxtype:"
    echo "     systemctl --user enable --now voxtype"
    echo ""
fi
echo "Using a coding agent (Claude Code, Codex, Gemini CLI, ...)? Run"
echo "  voxtype setup skill"
echo "to teach it to configure voxtype for this machine."
echo ""

%posttrans
# /usr/bin/voxtype is a symlink to the variant in use. Runs in %%posttrans
# because the %%postun of packages built from earlier versions of this spec
# deletes the link unconditionally, after the new %%post has run.
#
# An existing link that still resolves is kept: `voxtype setup gpu --enable`
# points it at voxtype-vulkan, and an upgrade must not undo that. A missing
# or dangling one is recreated: older packages put the binaries in
# /usr/lib64/voxtype, and 1.2.2-rc3 shipped a voxtype-avx512 variant that
# this package no longer has.
if [ ! -e %{_bindir}/voxtype ]; then
    rm -f %{_bindir}/voxtype
    ln -s %{vt_libdir}/voxtype-avx2 %{_bindir}/voxtype
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
%{_datadir}/applications/voxtype-meetings.desktop
%{_datadir}/icons/hicolor/scalable/apps/voxtype.svg
%{_datadir}/icons/hicolor/128x128/apps/voxtype.png

%files osd-gtk4
%{vt_libdir}/voxtype-osd-gtk4

%files meetings-gtk4
%{vt_libdir}/voxtype-meetings-gtk4

%changelog
* Fri Nov 27 2026 Peter Jackson <pete@peteonrails.com> - 1.3.2-1
- Soniox sends the API key in the WebSocket handshake, ahead of Soniox's
  2027-01-15 cutoff, and [soniox] region selects US, EU, Japan or India
  hosts (#864, #871)
- A recording from an input that delivers only digital zero for 5 s stops
  with a notice that names the source (dead_input guard)
- A failed or stalled streaming session is recovered through batch
  transcription; the stop timeout is 30 s ([streaming_recovery])
- Paste mode bounds clipboard reads at 2 s and restores the clipboard
  in the background, so the result shows as soon as the paste lands
  (#1003, #1079)
- A record start sent while output was still running is ignored (#1004)
- Opt-in hyprland paste driver for XWayland and Electron apps (#938)
- Opt-in xdotool output driver for X11 sessions
- Whisper keeps one state between transcriptions and creates it when
  recording starts (#1022)
- Feedback cues open an output stream per sound, fixing silence after
  resume and a spinning core (#848, #859)
- Quickshell OSD animates on the frame clock (#1078)
- A meeting pill dragged to the left edge stays there
- macOS keeps runtime files in $TMPDIR/voxtype per user, and a background
  user session ignores the hotkey
- setup gpu --enable refuses machines where the Vulkan build would crash
- Below x86-64-v2, every binary says so instead of pointing at baseline
- Input device docs and messages lead with compositor keybindings, not
  the input group
- whisper-cli mode is offered only when whisper-cli is installed, and a
  bare whisper on PATH is never used as whisper-cli
- Meeting summaries, Ask and dictation cleanup can use Omarchy's local
  model or any OpenAI-compatible server
- Dictation history works on atomic Fedora, where /home is a link to
  /var/home and history recorded nothing before
- Whisper retries with beam search when the decoder loops on a phrase,
  and drops the repeated copy when a reduced audio context restarts
  the transcript (#827, #898)
- info variants recommends the baseline build on a CPU without AVX2, and
  no longer the AVX2, Vulkan or ONNX builds that crash there
- The /usr/bin/voxtype wrapper in the release .deb and .rpm runs
  voxtype-baseline on a CPU without AVX2 instead of crashing in the AVX2
  build, and source package installs warn on such a CPU
- Release .deb and .rpm, the source .rpm and install-arch.sh lead their
  setup steps with compositor keybindings; the input group is only the
  fallback for the built-in hotkey
- A model that fails to load at startup no longer stops the daemon and
  restart-loops it: the tray, status, meetings and history keep working,
  the tray shows an error, and the next recording loads the model again
- config validate-selection, doctor and the daemon's startup check catch
  a GGUF gguf_backend the active build's worker lacks, such as vulkan on
  the avx512 build, and name the fix
- Packaged installs that ship only baseline or avx512 (the AUR package on
  a CPU without AVX2) are no longer taken for a source build, so setup gpu
  --enable refuses Vulkan there too

* Sat Nov 21 2026 Peter Jackson <pete@peteonrails.com> - 1.3.1-1
- Dictation history: a private, local record of each dictation, including
  failed and cancelled ones, with the history command, search alongside
  meetings, pause and resume, app exclusions, a history window, a terminal
  reader and tray items
- voxtype paste-last and [hotkey] paste_last_key send the last dictation
  again
- A correction in history can become a text replacement
- text.language sets the language of dictated text; LARD and other
  English-only cleanup run only on English text and stay inside one
  sentence
- Meeting playback: the transcript follows the waveform, and each speaker
  has one colour
- A notice when one side of a meeting records nothing
- Zoom's web client window counts as a call for meeting detection
- The meeting announcement plays at half its old level, set by
  meeting.announcement_volume
- One meeting view when the Omarchy plugin and the standalone shell both
  run, opened on the focused monitor
- config set and unset keep every entry of an inline table
- A profile's post_process_command and output_mode apply again (#1097)
- Configs saved through a symlink keep the link (#1102)
- Keep $ in replacement values (#1059)
- The hidden GTK4 OSD no longer wakes 120 times a second (#1090)
- Streaming keeps audio while inference runs and honours output overrides
- The silence auto-stop clock starts at the first frame
- Parakeet prefers CUDA over MIGraphX on NVIDIA hosts with an AMD iGPU
- Paste keystrokes map super, meta and win to wtype's logo
- Soniox defaults to stt-rt-v6 and stt-async-v6
- voxtype info accel reports the backend for GGUF engines (#1085)
- Media ducking leaves voxtype's own cue sounds alone
- Commands other than the daemon log to stderr, and an empty RUST_LOG is
  treated as unset
- A config save no longer undoes changes made since it loaded

* Wed Nov 18 2026 Peter Jackson <pete@peteonrails.com> - 1.3.0-1
- Meetings: live transcript view, echo cancellation and echo guard, hybrid
  diarization, notes, annotations, line edits, search, sharing, action
  items, recovery of interrupted meetings, import and retranscription
- Ship the GTK4 meetings window in voxtype-meetings-gtk4
- Built-in Kokoro recording announcement; the ElevenLabs voice is a download
- Whisper, VAD, speaker and echo models download from models.voxtype.io
- Notifications for failures that used to reach only the log: output
  fallback and failure, microphone and model load, the evdev hotkey, VAD,
  GPU and NPU fallback, OSD frontend fallback, post-processing and hooks,
  degraded meetings; delivered over D-Bus when notify-send is missing
- voxtype doctor, with the same checks grouped into one startup notification
- Keep failed recordings and undelivered text, with settings to turn each off
- Terminal-aware paste keys (paste_keys = "auto"); OSD scale, placement,
  opacity and native rendering fixes
- voxtype setup lard downloads the LARD model, and a missing model is reported
- Dictated text is logged only at debug level
- The recording announcement waits for the call app's settled mic stream, and
  a detected browser call continues while its app has the mic or a window
  shows it
- Fix a daemon freeze when a speaker with many lines is named during a meeting
- Fix macOS meeting portability

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
- Drop the voxtype-avx512 variant: package builds set SOURCE_DATE_EPOCH,
  which turns off ggml's native tuning, so it carried no AVX-512 code
- Point users with a coding agent at voxtype setup skill after install

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
