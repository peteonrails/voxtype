Name:           voxtype
Version:        0.2.1
Release:        1%{?dist}
Summary:        Push-to-talk voice-to-text for Wayland Linux

License:        MIT
URL:            https://github.com/peteonrails/voxtype
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  clang-devel
BuildRequires:  alsa-lib-devel
BuildRequires:  systemd-rpm-macros

Recommends:     ydotool
Recommends:     wl-clipboard
Suggests:       libnotify
Suggests:       pipewire

%description
Voxtype is a push-to-talk voice-to-text daemon for Wayland Linux systems.
Hold a hotkey while speaking, release to transcribe and output text at
your cursor position.

Features:
- Works on all Wayland compositors using kernel-level input (evdev)
- Fully offline transcription using whisper.cpp
- Fallback chain: types via ydotool, falls back to clipboard
- Configurable hotkeys, models, and output modes

Note: User must be in the 'input' group for hotkey detection.

%prep
%autosetup -n %{name}-%{version}

%build
export CARGO_HOME=%{_builddir}/cargo
cargo build --release --locked

%install
# Install binary
install -D -m 755 target/release/voxtype %{buildroot}%{_bindir}/voxtype

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
cargo test --release --locked

%post
%systemd_user_post voxtype.service

%preun
%systemd_user_preun voxtype.service

%postun
%systemd_user_postun_with_restart voxtype.service

%posttrans
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
echo "  3. Enable and start the ydotool daemon:"
echo "     systemctl --user enable --now ydotool"
echo ""
echo "  4. Download a whisper model:"
echo "     voxtype setup --download"
echo ""
echo "  5. Start voxtype:"
echo "     systemctl --user enable --now voxtype"
echo ""

%files
%license LICENSE
%doc README.md
%doc docs/INSTALL.md
%{_bindir}/voxtype
%config(noreplace) %{_sysconfdir}/voxtype/config.toml
%{_userunitdir}/voxtype.service
%{_datadir}/bash-completion/completions/voxtype
%{_datadir}/zsh/site-functions/_voxtype
%{_datadir}/fish/vendor_completions.d/voxtype.fish

%changelog
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
