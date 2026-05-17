# Voxtype Installation Guide

This guide covers installing Voxtype on Linux and macOS.

## Table of Contents

- [System Requirements](#system-requirements)
- [Quick Install](#quick-install)
- [Installation Methods](#installation-methods)
  - [Arch Linux](#arch-linux)
  - [Debian / Ubuntu](#debian--ubuntu)
  - [Fedora / RHEL](#fedora--rhel)
  - [macOS](#macos)
  - [NixOS](#nixos)
  - [AppImage](#appimage)
  - [Build from Source](#build-from-source)
- [Post-Install Setup](#post-install-setup)
- [GPU Acceleration](#gpu-acceleration)
- [Verifying the Install](#verifying-the-install)
- [Uninstall](#uninstall)
- [Troubleshooting](#troubleshooting)

---

## System Requirements

### Platforms

- **Linux:** any desktop. Optimized for Wayland, works on X11.
- **macOS:** Apple Silicon (arm64), macOS 13 Ventura or later.
- Architectures: x86_64 (with AVX2) and aarch64 on Linux; arm64 on macOS.

### CPU (prebuilt Linux binaries)

Prebuilt binaries target the **x86-64-v3 baseline**: AVX2, FMA, BMI1, BMI2, F16C, MOVBE. Supported:

- Intel Haswell (2013) and newer
- AMD Excavator (2015), Ryzen (any generation), EPYC

Older CPUs need to [build from source](#build-from-source) with `-C target-cpu=native`. Voxtype installs a SIGILL handler at startup that prints a helpful message instead of a raw "Illegal instruction" crash when the CPU is too old.

### glibc (Linux .deb and .rpm)

| Distro | Minimum version | glibc |
|--------|----------------|-------|
| Ubuntu | 24.04 (Noble) | 2.39 |
| Debian | Trixie (13) | 2.41 |
| Fedora | 40 | 2.39 |
| Arch Linux | Rolling | 2.40+ |

Older distros (Ubuntu 22.04, Debian Bookworm, Fedora 39) can [build from source](#build-from-source) instead.

### Runtime dependencies

| Component | Required? | Purpose |
|-----------|-----------|---------|
| PipeWire (with `pipewire-alsa`) or PulseAudio | Yes | Audio capture |
| `input` group membership | Yes (for evdev hotkeys) | Detecting hotkeys outside the compositor |
| `wtype` | Recommended (Wayland) | Best Unicode/CJK typing support |
| `dotool` | Recommended (KDE/GNOME Wayland) | Typing on compositors that don't speak the virtual-keyboard protocol |
| `ydotool` | Fallback (X11/TTY) | Requires a daemon |
| `wl-clipboard` | Recommended | Clipboard fallback |
| `libnotify` | Optional | Desktop notifications |
| `playerctl` | Optional | Pause MPRIS players while recording |
| `gtk4-layer-shell` | Optional | Runtime for the GTK4 OSD visualizer |

> **PipeWire users:** install `pipewire-alsa` so ALSA-based apps like Voxtype can capture audio. Without it you get "device not available" errors.

### Build dependencies (source builds only)

| Package | Arch | Debian/Ubuntu | Fedora |
|---------|------|---------------|--------|
| Rust toolchain | `rustup` | `cargo rustc` | `rust cargo` |
| ALSA dev | `alsa-lib` | `libasound2-dev` | `alsa-lib-devel` |
| Clang | `clang` | `libclang-dev` | `clang-devel` |
| CMake | `cmake` | `cmake` | `cmake` |
| pkg-config | `pkgconf` | `pkg-config` | `pkgconf` |

For the GTK4 OSD frontend, additionally install `gtk4` + `gtk4-layer-shell`.

---

## Quick Install

Pick your distro from the list below. The fastest path on each:

- **Arch:** `paru -S voxtype-bin`
- **Debian/Ubuntu:** `sudo apt install ./voxtype_0.7.1-1_amd64.deb`
- **Fedora:** `sudo dnf install ./voxtype-0.7.1-1.x86_64.rpm`
- **macOS:** `brew install --cask voxtype`
- **NixOS:** `nix profile install github:peteonrails/voxtype#vulkan`
- **AppImage:** download, `chmod +x`, run.

After install, see [Post-Install Setup](#post-install-setup) for the model download, hotkey, and systemd-service steps.

---

## Installation Methods

### Arch Linux

Three AUR packages, two prebuilt channels plus a from-source option:

- **`voxtype-bin`** — prebuilt binaries from the latest stable release. **Recommended for most users.**
- **`voxtype-bin-rc`** — prebuilt binaries from the latest GitHub *pre-release* (e.g., `v0.7.3-rc1`). Use this if you want to help test upcoming features (streaming, new engines, etc.) before they ship to stable. Otherwise stay on `voxtype-bin`.
- **`voxtype`** — builds from source via cargo, 20+ minutes.

```bash
# Recommended: stable prebuilt binaries
paru -S voxtype-bin       # or: yay -S voxtype-bin

# Release-candidate channel (for testers)
paru -S voxtype-bin-rc    # or: yay -S voxtype-bin-rc

# Or build from source
paru -S voxtype           # or: yay -S voxtype

# Or manual AUR clone
git clone https://aur.archlinux.org/voxtype-bin.git
cd voxtype-bin && makepkg -si
```

`voxtype-bin-rc` conflicts with both `voxtype-bin` and `voxtype`. Switching channels is a remove-then-install:

```bash
# stable -> RC
yay -R voxtype-bin && yay -S voxtype-bin-rc

# RC -> stable (do this once the next stable ships)
yay -R voxtype-bin-rc && yay -S voxtype-bin
```

Recommended optional packages:

```bash
sudo pacman -S wtype wl-clipboard libnotify gtk4-layer-shell pipewire-alsa
```

The post-install hook auto-picks the right CUDA variant (`cuda-12` vs `cuda-13`) based on your installed libcudart, and sets up `/usr/bin/voxtype` as a wrapper that dispatches to the matching binary.

### Debian / Ubuntu

Requires Ubuntu 24.04+ or Debian Trixie+ (glibc 2.39+). Older versions: [build from source](#build-from-source).

```bash
wget https://github.com/peteonrails/voxtype/releases/download/v0.7.1/voxtype_0.7.1-1_amd64.deb
sudo apt install ./voxtype_0.7.1-1_amd64.deb
```

Recommended optional packages:

```bash
sudo apt install wtype wl-clipboard libnotify-bin playerctl pipewire-alsa
```

The .deb ships every Linux binary variant (avx2, avx512, vulkan, plus ONNX CPU/CUDA/MIGraphX) under `/usr/lib/voxtype/`. Run `sudo voxtype setup gpu --enable` after install to pick a GPU binary.

### Fedora / RHEL

Requires Fedora 40+ (glibc 2.39+).

```bash
wget https://github.com/peteonrails/voxtype/releases/download/v0.7.1/voxtype-0.7.1-1.x86_64.rpm
sudo dnf install ./voxtype-0.7.1-1.x86_64.rpm
```

Recommended optional packages:

```bash
sudo dnf install wtype wl-clipboard libnotify playerctl pipewire-alsa
```

Fedora's ydotool ships as a system service that needs extra setup. See [TROUBLESHOOTING.md](TROUBLESHOOTING.md#ydotool-daemon-not-running) if you need ydotool specifically; otherwise `wtype` (Wayland) or `dotool` (KDE/GNOME Wayland) is the better default.

### macOS

Apple Silicon only. Uses Microsoft ONNX Runtime so every engine is available, including Parakeet on the Neural Engine path.

```bash
brew install --cask voxtype
```

Or download `voxtype-0.7.1-macOS-arm64.dmg` from the [latest release](https://github.com/peteonrails/voxtype/releases/latest). First launch opens a setup wizard that walks you through accessibility permissions, model download, and the FN-key hotkey.

### NixOS

A flake ships every variant:

```bash
# Imperative install
nix profile install github:peteonrails/voxtype/v0.7.1#vulkan

# Available outputs: default, vulkan, cuda, rocm, osdGtk4, osdNative
nix build github:peteonrails/voxtype/v0.7.1#osdGtk4

# Or pin in your flake inputs
inputs.voxtype.url = "github:peteonrails/voxtype/v0.7.1";
```

### Linux arm64 (aarch64) — manual install

The 0.7.2 release ships two experimental arm64 Linux binaries for
Raspberry Pi 4/5, Ampere servers, Snapdragon X laptops, and AWS
Graviton instances. They are not yet wired into the .deb / .rpm / AUR
packages, so install is manual:

```bash
# Whisper engine, CPU-only
curl -L https://github.com/peteonrails/voxtype/releases/download/v0.7.2/voxtype-0.7.2-linux-aarch64-cpu \
  -o /tmp/voxtype
chmod 755 /tmp/voxtype
sudo mv /tmp/voxtype /usr/local/bin/voxtype
voxtype --version

# Or, for the ONNX engines (Parakeet, Moonshine, SenseVoice, Paraformer, etc.)
curl -L https://github.com/peteonrails/voxtype/releases/download/v0.7.2/voxtype-0.7.2-linux-aarch64-onnx \
  -o /tmp/voxtype
chmod 755 /tmp/voxtype
sudo mv /tmp/voxtype /usr/local/bin/voxtype
```

These binaries are CPU-only. No CUDA, MIGraphX, or Vulkan support on
arm64 in 0.7.2 because the Jetson CUDA toolchain is awkward and there
is no mainstream consumer arm64 hardware with Vulkan GPUs yet.

Once installed, the rest of the setup (input group, models, hotkey)
is the same as on x86_64. Skip to [Post-Install Setup](#post-install-setup).

If you run into issues, please file a GitHub issue with your hardware
details (CPU, distro, kernel). Real-world reports from arm64 users
inform the v0.7.3 work that will integrate arm64 into the package
tooling.

### AppImage

Self-contained binary for distros that don't have a packaged voxtype.

```bash
wget https://github.com/peteonrails/voxtype/releases/download/v0.7.1/voxtype-0.7.1-x86_64.AppImage
chmod +x voxtype-0.7.1-x86_64.AppImage
mv voxtype-0.7.1-x86_64.AppImage ~/.local/bin/voxtype
```

Three AppImage variants ship: CPU, Vulkan, ONNX engines. The CPU one is the smallest and the right default for most users.

### Build from Source

For older distros, custom CPU targets (pre-Haswell hardware), or non-default feature combinations.

```bash
# 1. Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Install build dependencies for your distro
# Arch:
sudo pacman -S rustup alsa-lib clang cmake pkgconf
# Debian/Ubuntu:
sudo apt install cargo libasound2-dev libclang-dev cmake pkg-config
# Fedora:
sudo dnf install rust cargo alsa-lib-devel clang-devel cmake pkgconf

# 3. Clone and build
git clone https://github.com/peteonrails/voxtype
cd voxtype
cargo build --release

# 4. Install
sudo install -Dm755 target/release/voxtype /usr/local/bin/voxtype
```

#### Feature flags

| Feature | What it adds |
|---------|--------------|
| `gpu-vulkan` | Vulkan GPU for Whisper (NVIDIA/AMD/Intel) |
| `gpu-cuda` | CUDA GPU for Whisper (NVIDIA) |
| `gpu-hipblas` | ROCm/HIP for Whisper (AMD) |
| `parakeet` | Parakeet ASR engine (ONNX-based) |
| `parakeet-migraphx` | Parakeet on AMD MIGraphX |
| `parakeet-cuda` | Parakeet on NVIDIA CUDA |
| `moonshine`, `sensevoice`, `paraformer`, `dolphin`, `omnilingual`, `cohere` | Additional ONNX engines |
| `osd-gtk4` | GTK4 on-screen visualizer |
| `osd-native` | wgpu + egui on-screen visualizer |

Example: full ONNX engine set with MIGraphX:

```bash
cargo build --release --features parakeet-migraphx,moonshine,sensevoice,paraformer,dolphin,omnilingual,cohere,ml-diarization
```

#### Pre-Haswell CPUs

If you're on a pre-2013 Intel or pre-2015 AMD CPU, set `target-cpu=native` to use whatever instructions your CPU actually supports:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

---

## Post-Install Setup

> Most settings can be configured interactively with `voxtype configure` (or by searching for "Voxtype Configuration" in Walker, fuzzel, rofi, KRunner, or GNOME Activities). The steps below set up the system-level pieces the TUI can't change for you.

### 1. Add yourself to the input group

For kernel-level hotkey detection via evdev:

```bash
sudo usermod -aG input $USER
```

Log out and back in for the group change to take effect. Verify:

```bash
groups | grep input
```

If you only use compositor keybindings (Hyprland, Sway, River, KDE custom shortcuts) you can skip this step.

### 2. Download a Whisper model

```bash
# Interactive: checks the install and offers to download
voxtype setup

# Or directly download the default (base.en, 142 MB)
voxtype setup --download

# Or pick another model interactively
voxtype setup model
```

| Model | Size | Speed (CPU) | Best for |
|-------|------|-------------|----------|
| `tiny.en` | 39 MB | Fastest | Quick notes, low-end hardware |
| **`base.en`** | 142 MB | Fast | **Recommended default** |
| `small.en` | 466 MB | Medium | Higher accuracy |
| `medium.en` | 1.5 GB | Slow | Professional transcription |
| `large-v3-turbo` | 1.6 GB | Fast on GPU | Multilingual, best accuracy |

`.en` models are English-only but faster than their multilingual siblings.

For the ONNX engines (Parakeet, Cohere, etc.) the equivalent is `voxtype setup model` — pick an engine and it downloads the right model from HuggingFace.

### 3. Start the daemon

```bash
# Via systemd user service (recommended; auto-starts on login)
systemctl --user enable --now voxtype

# Or run in foreground for testing
voxtype daemon -v
```

### 4. Wire up a hotkey

The default is **Pause** key via evdev (kernel-level). To change, run `voxtype configure` and edit the `[hotkey]` section.

**Compositor keybindings (preferred):** disable evdev (`[hotkey] enabled = false`) and bind in your compositor instead. Voxtype provides `voxtype record start/stop/toggle` commands.

```toml
# ~/.config/voxtype/config.toml
[hotkey]
enabled = false
```

Hyprland (`~/.config/hypr/hyprland.conf`):

```
bind  = SUPER, V, exec, voxtype record start
bindr = SUPER, V, exec, voxtype record stop
```

Sway (`~/.config/sway/config`):

```
bindsym $mod+v exec voxtype record start
bindsym --release $mod+v exec voxtype record stop
```

River (`~/.config/river/init`):

```
riverctl map normal Super V spawn 'voxtype record start'
riverctl map -release normal Super V spawn 'voxtype record stop'
```

KDE Plasma: System Settings → Shortcuts → Custom Shortcuts. See [USER_MANUAL.md](USER_MANUAL.md#kde-plasma) for the full walkthrough.

GNOME / X11 / other: leave evdev enabled (`[hotkey] enabled = true`) and use the configured key.

---

## GPU Acceleration

Linux packages ship a Vulkan Whisper binary and per-vendor ONNX engine binaries. After install, point the wrapper at the right one:

```bash
sudo voxtype setup gpu --enable
```

The command auto-detects your GPU and installs the matching runtime symlinks. To override:

```bash
sudo voxtype setup gpu --enable --backend vulkan      # Whisper on Vulkan
sudo voxtype setup gpu --enable --backend onnx-cuda   # ONNX engines on NVIDIA
sudo voxtype setup gpu --enable --backend onnx-migraphx  # ONNX engines on AMD
```

### Runtime packages

| Vendor | Whisper (Vulkan) | ONNX (engine-specific) |
|--------|------------------|------------------------|
| NVIDIA | `vulkan-icd-loader` (Arch) / `libvulkan1` (Debian) / `vulkan-loader` (Fedora) | `cuda` (CUDA 13, driver 580+) or `cuda12.6` (CUDA 12, driver 525+) |
| AMD | same Vulkan loader | `rocm-hip-runtime` 7.x for MIGraphX |
| Intel | same Vulkan loader | n/a (CPU only) |

NVIDIA users: the AUR `voxtype-bin` post-install hook auto-picks `voxtype-onnx-cuda-12` or `-13` based on your installed libcudart. The .deb/.rpm don't have that hook; run `voxtype setup gpu --enable` after install.

AMD users on ROCm < 7.x will silently fall back to CPU when the MIGraphX EP fails to register. First MIGraphX inference compiles the model graph (~30-60s on Radeon RX 7000-class); subsequent runs are fast.

For details on what each ONNX engine supports per GPU, see the matrix in [CONFIGURATION.md](CONFIGURATION.md#engine-and-backend-matrix).

---

## Verifying the Install

```bash
voxtype setup
```

Checks:

- [x] User in `input` group (if evdev hotkey is enabled)
- [x] Audio source accessible
- [x] Typing backend present (`wtype` / `dotool` / `ydotool` / clipboard)
- [x] Whisper model downloaded
- [x] Configuration parses cleanly
- [x] GPU backend is registering (for variants with GPU support)

Test transcription against a WAV file:

```bash
arecord -d 5 -f S16_LE -r 16000 test.wav
voxtype transcribe test.wav
rm test.wav
```

Or use the smoke-test command which exercises the full pipeline:

```bash
voxtype info             # daemon state, active binary, engine, model
systemctl --user status voxtype
journalctl --user -u voxtype -f
```

---

## Uninstall

```bash
# Arch
sudo pacman -R voxtype          # or voxtype-bin

# Debian / Ubuntu
sudo apt remove voxtype

# Fedora
sudo dnf remove voxtype

# macOS
brew uninstall --cask voxtype

# Manual / cargo install
sudo rm /usr/local/bin/voxtype
cargo uninstall voxtype

# Remove user data (optional)
rm -rf ~/.config/voxtype ~/.local/share/voxtype
systemctl --user disable --now voxtype
rm -f ~/.config/systemd/user/voxtype.service
systemctl --user daemon-reload
```

---

## Troubleshooting

For the full troubleshooting guide see [TROUBLESHOOTING.md](TROUBLESHOOTING.md). Common quick fixes:

**"Cannot open input device"** — User isn't in `input` group, or hasn't logged out/in since adding:

```bash
sudo usermod -aG input $USER  # then log out and back in
```

**Text isn't typing (Wayland)** — Install `wtype` for wlroots compositors, or `dotool` for KDE/GNOME:

```bash
sudo apt install wtype     # or pacman -S / dnf install
```

**No audio captured / "device not available"** — PipeWire users need the ALSA bridge:

```bash
sudo apt install pipewire-alsa
```

**Daemon shows "CPU (native)" but I installed the GPU binary** — Run the wrapper setup so `/usr/bin/voxtype` points at the right variant:

```bash
sudo voxtype setup gpu --enable
```

**SIGILL ("Illegal instruction") on startup** — Your CPU is older than our Haswell baseline (AVX2 + FMA + BMI1/2). Build from source with `RUSTFLAGS="-C target-cpu=native"` or open an issue with your `lscpu` output.

---

## Getting Help

- **Website:** [voxtype.io](https://voxtype.io)
- **User manual:** [USER_MANUAL.md](USER_MANUAL.md)
- **Configuration reference:** [CONFIGURATION.md](CONFIGURATION.md)
- **Issues:** [github.com/peteonrails/voxtype/issues](https://github.com/peteonrails/voxtype/issues)
- **Discussions:** [github.com/peteonrails/voxtype/discussions](https://github.com/peteonrails/voxtype/discussions)
