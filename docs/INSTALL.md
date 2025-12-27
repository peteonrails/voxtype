# Voxtype Installation Guide

This guide covers all methods for installing Voxtype on Linux systems.

## Table of Contents

- [System Requirements](#system-requirements)
- [Quick Install](#quick-install)
- [Installation Methods](#installation-methods)
  - [Arch Linux (AUR)](#arch-linux-aur)
  - [Debian/Ubuntu](#debianubuntu)
  - [Fedora/RHEL](#fedorarhel)
  - [Building from Source](#building-from-source)
  - [Cargo Install](#cargo-install)
- [Post-Installation Setup](#post-installation-setup)
- [Whisper Model Download](#whisper-model-download)
- [Starting Voxtype](#starting-voxtype)
- [Verifying Installation](#verifying-installation)
- [Uninstallation](#uninstallation)

---

## System Requirements

### Supported Platforms

- **Linux** with any desktop environment (GNOME, KDE Plasma, Sway, Hyprland, i3, etc.)
- Optimized for Wayland, works on X11 too
- Architectures: x86_64, aarch64

### Runtime Dependencies

| Component | Required | Purpose |
|-----------|----------|---------|
| Linux desktop | Yes | Wayland or X11 |
| PipeWire or PulseAudio | Yes | Audio capture |
| `input` group membership | Yes | Hotkey detection via evdev |
| wtype | Recommended | Keyboard simulation on Wayland (best CJK support) |
| ydotool | Recommended | Keyboard simulation on X11 (or Wayland fallback) |
| wl-clipboard | Recommended | Clipboard fallback on Wayland |
| libnotify | Optional | Desktop notifications |

### Build Dependencies (source builds only)

| Package | Arch | Debian/Ubuntu | Fedora |
|---------|------|---------------|--------|
| Rust toolchain | `rustup` | `rustc cargo` | `rust cargo` |
| ALSA dev libs | `alsa-lib` | `libasound2-dev` | `alsa-lib-devel` |
| Clang | `clang` | `libclang-dev` | `clang-devel` |
| CMake | `cmake` | `cmake` | `cmake` |
| pkg-config | `pkgconf` | `pkg-config` | `pkgconf` |

### GPU Acceleration

**Vulkan (included in packages):** Packages include a pre-built Vulkan binary. Just install the runtime and enable:

| Distro | Install Runtime | Enable GPU |
|--------|-----------------|------------|
| Arch | `sudo pacman -S vulkan-icd-loader` | `sudo voxtype setup gpu --enable` |
| Debian/Ubuntu | `sudo apt install libvulkan1` | `sudo voxtype setup gpu --enable` |
| Fedora | `sudo dnf install vulkan-loader` | `sudo voxtype setup gpu --enable` |

**Other backends (build from source):** For CUDA, Metal, or ROCm, build from source:

| GPU Backend | Build Dependencies | Build Command |
|-------------|-------------------|---------------|
| CUDA | `cuda` / `nvidia-cuda-toolkit` | `cargo build --release --features gpu-cuda` |
| Metal | (macOS only) | `cargo build --release --features gpu-metal` |
| HIP/ROCm | ROCm SDK | `cargo build --release --features gpu-hipblas` |

---

## System Requirements

Pre-built packages require **glibc 2.38 or newer**:

| Distro | Minimum Version | glibc |
|--------|-----------------|-------|
| Ubuntu | 24.04 (Noble) | 2.39 |
| Fedora | 39+ | 2.38 |
| Arch Linux | Rolling | 2.40+ |
| Debian | Trixie (13) | 2.38 |

Older distributions (Ubuntu 22.04, Debian Bookworm) can build from source.

---

## Quick Install

### One-liner (from source)

```bash
# Install dependencies, build, and setup (Arch)
sudo pacman -S --needed base-devel rust clang alsa-lib wtype wl-clipboard && \
git clone https://github.com/peteonrails/voxtype && cd voxtype && \
cargo build --release && \
sudo cp target/release/voxtype /usr/local/bin/ && \
sudo usermod -aG input $USER && \
echo "Log out and back in, then run: voxtype setup --download"
```

---

## Installation Methods

### Arch Linux (AUR)

#### Using an AUR helper (recommended)

```bash
# Using paru
paru -S voxtype

# Using yay
yay -S voxtype
```

#### Manual AUR build

```bash
git clone https://aur.archlinux.org/voxtype.git
cd voxtype
makepkg -si
```

#### Dependencies installed automatically

- `alsa-lib` (runtime)
- `cargo`, `clang` (build-time)

#### Optional dependencies

```bash
# Install recommended optional packages
sudo pacman -S wtype wl-clipboard libnotify
# For X11 or as Wayland fallback:
sudo pacman -S ydotool
```

---

### Debian/Ubuntu

#### From .deb package

```bash
# Download the latest release
wget https://github.com/peteonrails/voxtype/releases/download/v0.1.0/voxtype_0.1.0-1_amd64.deb

# Install
sudo dpkg -i voxtype_0.1.0-1_amd64.deb

# Install any missing dependencies
sudo apt-get install -f
```

#### Building the .deb package

```bash
# Install build dependencies
sudo apt install build-essential cargo rustc libclang-dev libasound2-dev \
    pkg-config debhelper devscripts

# Clone and build
git clone https://github.com/peteonrails/voxtype
cd voxtype

# Build the package
dpkg-buildpackage -us -uc -b

# Install
sudo dpkg -i ../voxtype_0.1.0-1_*.deb
```

#### Install recommended packages

```bash
# For Wayland:
sudo apt install wtype wl-clipboard libnotify-bin
# For X11 or as fallback:
sudo apt install ydotool
```

---

### Fedora/RHEL

#### From COPR (when available)

```bash
sudo dnf copr enable pete/voxtype
sudo dnf install voxtype
```

#### From .rpm package

```bash
# Download the latest release
wget https://github.com/peteonrails/voxtype/releases/download/v0.1.0/voxtype-0.1.0-1.fc39.x86_64.rpm

# Install
sudo dnf install ./voxtype-0.1.0-1.fc39.x86_64.rpm
```

#### Building the .rpm package

```bash
# Install build dependencies
sudo dnf install cargo rust clang-devel alsa-lib-devel rpm-build rpmdevtools

# Setup rpmbuild directories
rpmdev-setuptree

# Download source tarball to SOURCES
wget -O ~/rpmbuild/SOURCES/voxtype-0.1.0.tar.gz \
    https://github.com/peteonrails/voxtype/archive/v0.1.0.tar.gz

# Copy spec file
cp packaging/rpm/voxtype.spec ~/rpmbuild/SPECS/

# Build
rpmbuild -ba ~/rpmbuild/SPECS/voxtype.spec

# Install
sudo dnf install ~/rpmbuild/RPMS/x86_64/voxtype-0.1.0-1.*.rpm
```

#### Install recommended packages

```bash
# For Wayland:
sudo dnf install wtype wl-clipboard libnotify
# For X11 or as fallback:
sudo dnf install ydotool
```

---

### Building from Source

#### 1. Install Rust (if not already installed)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

#### 2. Install build dependencies

**Arch Linux:**
```bash
sudo pacman -S base-devel clang alsa-lib
```

**Debian/Ubuntu:**
```bash
sudo apt install build-essential libclang-dev libasound2-dev pkg-config
```

**Fedora:**
```bash
sudo dnf install @development-tools clang-devel alsa-lib-devel
```

#### 3. Clone and build

```bash
git clone https://github.com/peteonrails/voxtype
cd voxtype
cargo build --release
```

#### 4. Install

```bash
# Install binary
sudo install -Dm755 target/release/voxtype /usr/local/bin/voxtype

# Install config (optional - will be created on first run)
sudo install -Dm644 config/default.toml /etc/voxtype/config.toml

# Install systemd service (optional)
install -Dm644 packaging/systemd/voxtype.service \
    ~/.config/systemd/user/voxtype.service
```

---

### Cargo Install

The simplest method if you have Rust installed:

```bash
# Install build dependencies first (see above)

# Install from crates.io (when published)
cargo install voxtype

# Or install from git
cargo install --git https://github.com/peteonrails/voxtype
```

---

## Post-Installation Setup

### 1. Add user to input group

Voxtype uses the Linux evdev subsystem to detect hotkeys, which requires `input` group membership:

```bash
sudo usermod -aG input $USER
```

**Important:** You must log out and back in for the group change to take effect. Verify with:

```bash
groups | grep input
```

### 2. Install typing backend

**On Wayland (recommended):** Install wtype for best CJK/Unicode support
```bash
# Fedora:
sudo dnf install wtype
# Arch:
sudo pacman -S wtype
# Ubuntu:
sudo apt install wtype
```

**On X11:** Install and enable ydotool
```bash
# Fedora:
sudo dnf install ydotool
# Arch:
sudo pacman -S ydotool
# Ubuntu:
sudo apt install ydotool

# Enable and start the daemon
systemctl --user enable --now ydotool
```

Voxtype uses wtype on Wayland (no daemon needed), ydotool on X11, and falls back to clipboard if neither is available.

### 3. Verify audio setup

Ensure your audio system is working:

```bash
# List audio sources
pactl list sources short

# Test recording (speak and listen)
arecord -d 3 -f S16_LE -r 16000 test.wav && aplay test.wav && rm test.wav
```

---

## Whisper Model Download

Voxtype needs a Whisper model for speech recognition. Use the built-in setup command:

```bash
# Interactive setup (checks dependencies and offers to download)
voxtype setup

# Download the default model (base.en)
voxtype setup --download

# Interactive model selection (choose from available models)
voxtype setup model
```

### Available Models

| Model | Size | Speed | Accuracy | Best For |
|-------|------|-------|----------|----------|
| tiny.en | 39 MB | Fastest | Good | Quick notes, low-end hardware |
| **base.en** | 142 MB | Fast | Better | **Recommended for most users** |
| small.en | 466 MB | Medium | Great | Higher accuracy needs |
| medium.en | 1.5 GB | Slow | Excellent | Professional transcription |
| large-v3 | 3.1 GB | Slowest | Best | Maximum accuracy, multilingual |

`.en` models are English-only but faster and more accurate for English content.

### Manual Model Download

If you prefer to download manually:

```bash
mkdir -p ~/.local/share/voxtype/models

# Download base.en (recommended)
curl -L -o ~/.local/share/voxtype/models/ggml-base.en.bin \
    https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
```

---

## Starting Voxtype

### Manual start

```bash
# Run in foreground (for testing)
voxtype

# With verbose output
voxtype -v

# With debug logging
voxtype -vv
```

### Systemd user service

```bash
# Enable and start
systemctl --user enable --now voxtype

# Check status
systemctl --user status voxtype

# View logs
journalctl --user -u voxtype -f
```

### Usage

1. Run `voxtype` (daemon starts listening)
2. Hold **ScrollLock** (or your configured hotkey)
3. Speak your text
4. Release the key
5. Text appears at cursor (or in clipboard)

Press **Ctrl+C** to stop the daemon.

---

## Verifying Installation

Run the setup command to verify everything is working:

```bash
voxtype setup
```

This checks:
- [x] User in `input` group
- [x] Audio system accessible
- [x] wtype or ydotool available (optional)
- [x] Whisper model downloaded
- [x] Configuration valid

### Test transcription

```bash
# Record a test file
arecord -d 5 -f S16_LE -r 16000 test.wav

# Transcribe it
voxtype transcribe test.wav

# Clean up
rm test.wav
```

---

## Uninstallation

### Arch Linux

```bash
sudo pacman -R voxtype
```

### Debian/Ubuntu

```bash
sudo apt remove voxtype
```

### Fedora

```bash
sudo dnf remove voxtype
```

### Manual/Cargo install

```bash
# Remove binary
sudo rm /usr/local/bin/voxtype
# or
cargo uninstall voxtype

# Remove config and data (optional)
rm -rf ~/.config/voxtype
rm -rf ~/.local/share/voxtype

# Remove systemd service
rm ~/.config/systemd/user/voxtype.service
systemctl --user daemon-reload
```

---

## Troubleshooting

See the [Troubleshooting Guide](TROUBLESHOOTING.md) for common issues and solutions.

### Quick fixes

**"Cannot open input device"**
```bash
sudo usermod -aG input $USER
# Log out and back in
```

**Text not typing (Wayland)**
```bash
# Install wtype
sudo pacman -S wtype  # or apt/dnf
```

**Text not typing (X11)**
```bash
systemctl --user enable --now ydotool
```

**No audio captured**
```bash
# Check PipeWire/PulseAudio is running
pactl info
# Check default source
pactl get-default-source
```

---

## Getting Help

- **Documentation:** https://voxtype.dev/docs
- **Issues:** https://github.com/peteonrails/voxtype/issues
- **Discussions:** https://github.com/peteonrails/voxtype/discussions
