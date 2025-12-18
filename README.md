# Voxtype

[![Voxtype - Voice to Text for Linux](website/images/og-preview.png)](https://voxtype.io)

**[voxtype.io](https://voxtype.io)**

Push-to-talk voice-to-text for Linux. Optimized for Wayland, works on X11 too.

Hold a hotkey (default: ScrollLock) while speaking, release to transcribe and output the text at your cursor position.

## Features

- **Works on any Linux desktop** - Uses kernel-level input (evdev). Works on Wayland and X11
- **Fully offline** - Uses whisper.cpp for local transcription, no internet required
- **Fallback chain** - Types via wtype (best CJK support), falls back to ydotool, then clipboard
- **Push-to-talk or Toggle mode** - Hold to record, or press once to start/stop
- **Audio feedback** - Optional sound cues when recording starts/stops
- **Configurable** - Choose your hotkey, model size, output mode, and more
- **Waybar integration** - Optional status indicator shows recording state in your bar

## Quick Start

```bash
# 1. Build
cargo build --release

# 2. One-time setup
sudo usermod -aG input $USER
# Log out and back in

# 3. Install typing backend
# For Wayland (recommended):
# Fedora:
sudo dnf install wtype
# Arch:
sudo pacman -S wtype
# Ubuntu:
sudo apt install wtype

# For X11 (or as fallback):
# Fedora:
sudo dnf install ydotool
# Arch:
sudo pacman -S ydotool
# Ubuntu:
sudo apt install ydotool
# Then start the daemon:
systemctl --user enable --now ydotool

# 4. Download whisper model
./target/release/voxtype setup --download

# 5. Run
./target/release/voxtype
```

## Usage

1. Run `voxtype` (it runs as a foreground daemon)
2. Hold **ScrollLock** (or your configured hotkey)
3. Speak
4. Release the key
5. Text appears at your cursor (or in clipboard if typing isn't available)

Press Ctrl+C to stop the daemon.

### Toggle Mode

If you prefer to press once to start recording and again to stop (instead of holding):

```bash
# Via command line
voxtype --toggle

# Or in config.toml
[hotkey]
key = "SCROLLLOCK"
mode = "toggle"
```

## Configuration

Config file location: `~/.config/voxtype/config.toml`

```toml
[hotkey]
key = "SCROLLLOCK"  # Or: PAUSE, F13-F24, RIGHTALT, etc.
modifiers = []      # Optional: ["LEFTCTRL", "LEFTALT"]
# mode = "toggle"   # Uncomment for toggle mode (press to start/stop)

[audio]
device = "default"  # Or specific device from `pactl list sources short`
sample_rate = 16000
max_duration_secs = 60

# Audio feedback (sound cues when recording starts/stops)
# [audio.feedback]
# enabled = true
# theme = "default"   # "default", "subtle", "mechanical", or path to custom dir
# volume = 0.7        # 0.0 to 1.0

[whisper]
model = "base.en"   # tiny, base, small, medium, large-v3, large-v3-turbo
language = "en"     # Or "auto" for detection, or language code (es, fr, de, etc.)
translate = false   # Translate non-English speech to English
# threads = 4       # CPU threads for inference (omit for auto-detect)
# on_demand_loading = true  # Load model only when recording (saves memory)

[output]
mode = "type"       # "type", "clipboard", or "paste"
fallback_to_clipboard = true
type_delay_ms = 0   # Increase if characters are dropped
# Note: "paste" mode copies to clipboard then simulates Ctrl+V
#       Useful for non-US keyboard layouts where ydotool typing fails

[output.notification]
on_recording_start = false  # Notify when PTT activates
on_recording_stop = false   # Notify when transcribing
on_transcription = true     # Show transcribed text

# Text processing (word replacements, spoken punctuation)
# [text]
# spoken_punctuation = true  # Say "period" → ".", "open paren" → "("
# replacements = { "hyperwhisper" = "hyprwhspr", "javascript" = "JavaScript" }

# State file for Waybar/polybar integration
# state_file = "auto"  # Or custom path like "/tmp/voxtype-state"
```

### Audio Feedback

Enable audio feedback to hear a sound when recording starts and stops:

```toml
[audio.feedback]
enabled = true
theme = "default"  # Built-in themes: default, subtle, mechanical
volume = 0.7       # 0.0 to 1.0
```

**Built-in themes:**
- `default` - Clear, pleasant two-tone beeps
- `subtle` - Quiet, unobtrusive clicks
- `mechanical` - Typewriter/keyboard-like sounds

**Custom themes:** Point `theme` to a directory containing `start.wav`, `stop.wav`, and `error.wav` files.

### Text Processing

Voxtype can post-process transcribed text with word replacements and spoken punctuation.

**Word replacements** fix commonly misheard words:

```toml
[text]
replacements = { "hyperwhisper" = "hyprwhspr", "javascript" = "JavaScript" }
```

**Spoken punctuation** (opt-in) converts spoken words to symbols - useful for developers:

```toml
[text]
spoken_punctuation = true
```

With this enabled, saying "function open paren close paren" outputs `function()`. Supports period, comma, brackets, braces, newlines, and many more. See [CONFIGURATION.md](docs/CONFIGURATION.md#text) for the full list.

## CLI Options

```
voxtype [OPTIONS] [COMMAND]

Commands:
  daemon      Run as background daemon (default)
  transcribe  Transcribe an audio file
  setup       Setup and installation utilities
  config      Show current configuration
  status      Show daemon state (for Waybar integration)

Setup subcommands:
  voxtype setup              Run basic dependency checks (default)
  voxtype setup --download   Download the configured Whisper model
  voxtype setup systemd      Install/manage systemd user service
  voxtype setup waybar       Generate Waybar module configuration
  voxtype setup model        Interactive model selection and download

Options:
  -c, --config <FILE>  Path to config file
  -v, --verbose        Increase verbosity (-v, -vv)
  -q, --quiet          Quiet mode (errors only)
  --clipboard          Force clipboard mode
  --paste              Force paste mode (clipboard + Ctrl+V)
  --model <MODEL>      Override whisper model
  --hotkey <KEY>       Override hotkey
  --toggle             Use toggle mode (press to start/stop)
```

## Whisper Models

| Model | Size | English WER | Speed |
|-------|------|-------------|-------|
| tiny.en | 39 MB | ~10% | Fastest |
| base.en | 142 MB | ~8% | Fast |
| small.en | 466 MB | ~6% | Medium |
| medium.en | 1.5 GB | ~5% | Slow |
| large-v3 | 3 GB | ~4% | Slowest |
| large-v3-turbo | 1.6 GB | ~4% | Fast |

For most uses, `base.en` provides a good balance of speed and accuracy. If you have a GPU, `large-v3-turbo` offers excellent accuracy with fast inference.

### Multilingual Support

The `.en` models are English-only but faster and more accurate for English. For other languages, use `large-v3` which supports 99 languages.

**Use Case 1: Transcribe in the spoken language** (speak French, output French)
```toml
[whisper]
model = "large-v3"
language = "auto"     # Auto-detect and transcribe in that language
translate = false
```

**Use Case 2: Translate to English** (speak French, output English)
```toml
[whisper]
model = "large-v3"
language = "auto"     # Auto-detect the spoken language
translate = true      # Translate output to English
```

**Use Case 3: Force a specific language** (always transcribe as Spanish)
```toml
[whisper]
model = "large-v3"
language = "es"       # Force Spanish transcription
translate = false
```

With GPU acceleration, `large-v3` achieves sub-second inference while supporting all languages.

## GPU Acceleration

Voxtype supports optional GPU acceleration for significantly faster inference. With GPU acceleration, even the `large-v3` model can achieve sub-second inference times.

### Building with GPU Support

GPU acceleration requires building from source with the appropriate feature flag:

**Vulkan (AMD, NVIDIA, Intel - recommended for AMD)**
```bash
# Install Vulkan development libraries
# Arch:
sudo pacman -S vulkan-devel

# Ubuntu/Debian:
sudo apt install libvulkan-dev

# Fedora:
sudo dnf install vulkan-devel

# Build with Vulkan support
cargo build --release --features gpu-vulkan
```

**CUDA (NVIDIA)**
```bash
# Install CUDA toolkit first, then:
cargo build --release --features gpu-cuda
```

**Metal (macOS/Apple Silicon)**
```bash
cargo build --release --features gpu-metal
```

**HIP/ROCm (AMD alternative)**
```bash
cargo build --release --features gpu-hipblas
```

### Performance Comparison

Results vary by hardware. Example on AMD RX 6800:

| Model | CPU | Vulkan GPU |
|-------|-----|------------|
| base.en | ~7x realtime | ~35x realtime |
| large-v3 | ~1x realtime | ~5x realtime |

## Requirements

### Runtime Dependencies

- **Linux desktop** (Wayland or X11 - GNOME, KDE, Sway, Hyprland, i3, etc.)
- **PipeWire** or **PulseAudio** (for audio capture)
- **wtype** (for typing output on Wayland) - *recommended, best CJK/Unicode support*
- **ydotool** + daemon - *for X11 or as Wayland fallback*
- **wl-clipboard** (for clipboard fallback on Wayland)

### Permissions

- User must be in the `input` group (for evdev access)

### Installing Dependencies

**Fedora:**
```bash
sudo dnf install wtype wl-clipboard
```

**Ubuntu/Debian:**
```bash
sudo apt install wtype wl-clipboard
```

**Arch:**
```bash
sudo pacman -S wtype wl-clipboard
```

## Building from Source

```bash
# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install build dependencies
# Fedora:
sudo dnf install alsa-lib-devel

# Ubuntu:
sudo apt install libasound2-dev

# Build
cargo build --release

# Binary is at: target/release/voxtype
```

## Waybar Integration

Add to your Waybar config:

```json
"custom/voxtype": {
    "exec": "voxtype status --follow --format json",
    "return-type": "json",
    "format": "{}",
    "tooltip": true
}
```

First, enable the state file in your voxtype config:

```toml
state_file = "auto"
```

## Troubleshooting

### "Cannot open input device" error

Add your user to the input group:

```bash
sudo usermod -aG input $USER
# Log out and back in
```

### Text not appearing / typing not working

Voxtype uses wtype (preferred) or ydotool as fallback for typing output:

```bash
# Check if wtype is installed
which wtype

# If using ydotool fallback (X11/TTY), start the daemon:
systemctl --user start ydotool
systemctl --user enable ydotool  # Start on login
```

### No audio captured

Check your default audio input:

```bash
# List audio sources
pactl list sources short

# Test recording
arecord -d 3 -f S16_LE -r 16000 test.wav
aplay test.wav
```

### Text appears slowly

If characters are being dropped, increase the delay:

```toml
[output]
type_delay_ms = 10
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Daemon                              │
├─────────────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │   Hotkey     │  │    Audio     │  │   Text Output    │  │
│  │  (evdev)     │──│   (cpal)     │──│ (wtype/ydotool)  │  │
│  └──────────────┘  └──────────────┘  └──────────────────┘  │
│         │               │                    │              │
│         │               ▼                    │              │
│         │        ┌──────────────┐            │              │
│         │        │   Whisper    │            │              │
│         └───────▶│  (whisper-rs)│────────────┘              │
│                  └──────────────┘                           │
└─────────────────────────────────────────────────────────────┘
```

**Why evdev?** Neither Wayland nor X11 provide a standard way to capture global hotkeys that works everywhere. Using evdev (the Linux input subsystem) works on all desktops but requires the user to be in the `input` group.

**Why wtype + ydotool?** On Wayland, wtype uses the virtual-keyboard protocol for text input, with excellent Unicode/CJK support and no daemon required. On X11 (or as a fallback), ydotool uses uinput for text injection. This combination ensures Voxtype works on any Linux desktop.

## Feedback

We want to hear from you! Voxtype is a young project and your feedback helps make it better.

- **Something not working?** If Voxtype doesn't install cleanly, doesn't work on your system, or is buggy in any way, please [open an issue](https://github.com/peteonrails/voxtype/issues). I actively monitor and respond to issues.
- **Like Voxtype?** I don't accept donations, but if you find it useful:
  - A [GitHub star](https://github.com/peteonrails/voxtype) helps others discover the project
  - Arch users: a vote on the [AUR package](https://aur.archlinux.org/packages/voxtype) helps keep it maintained

## Contributors

- [Peter Jackson](https://github.com/peteonrails) - Creator and maintainer
- [jvantillo](https://github.com/jvantillo) - GPU acceleration patch, whisper-rs 0.15.1 compatibility
- [materemias](https://github.com/materemias) - Paste output mode, on-demand model loading, PKGBUILD fix

## License

MIT
