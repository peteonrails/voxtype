# Voxtype

Push-to-talk voice-to-text for Wayland Linux systems.

Hold a hotkey (default: ScrollLock) while speaking, release to transcribe and output the text at your cursor position.

## Features

- **Works on all Wayland compositors** - Uses kernel-level input (evdev) instead of compositor-specific protocols
- **Fully offline** - Uses whisper.cpp for local transcription, no internet required
- **Fallback chain** - Types via ydotool, falls back to clipboard if unavailable
- **Push-to-talk** - Natural workflow: hold to record, release to transcribe
- **Configurable** - Choose your hotkey, model size, output mode, and more

## Quick Start

```bash
# 1. Build
cargo build --release

# 2. One-time setup
sudo usermod -aG input $USER
# Log out and back in

# 3. Start ydotool daemon (for typing output)
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
5. Text appears at your cursor (or in clipboard if ydotool isn't available)

Press Ctrl+C to stop the daemon.

## Configuration

Config file location: `~/.config/voxtype/config.toml`

```toml
[hotkey]
key = "SCROLLLOCK"  # Or: PAUSE, F13-F24, RIGHTALT, etc.
modifiers = []      # Optional: ["LEFTCTRL", "LEFTALT"]

[audio]
device = "default"
sample_rate = 16000
max_duration_secs = 60

[whisper]
model = "base.en"   # tiny, base, small, medium, large-v3
language = "en"     # Or "auto" for detection
translate = false

[output]
mode = "type"       # "type" or "clipboard"
fallback_to_clipboard = true
type_delay_ms = 0

[output.notification]
on_recording_start = false  # Notify when PTT activates
on_recording_stop = false   # Notify when transcribing
on_transcription = true     # Show transcribed text
```

## CLI Options

```
voxtype [OPTIONS] [COMMAND]

Commands:
  daemon      Run as background daemon (default)
  transcribe  Transcribe an audio file
  setup       Check dependencies and download models
  config      Show current configuration

Options:
  -c, --config <FILE>  Path to config file
  -v, --verbose        Increase verbosity (-v, -vv)
  -q, --quiet          Quiet mode (errors only)
  --clipboard          Force clipboard mode
  --model <MODEL>      Override whisper model
  --hotkey <KEY>       Override hotkey
```

## Whisper Models

| Model | Size | English WER | Speed |
|-------|------|-------------|-------|
| tiny.en | 39 MB | ~10% | Fastest |
| base.en | 142 MB | ~8% | Fast |
| small.en | 466 MB | ~6% | Medium |
| medium.en | 1.5 GB | ~5% | Slow |
| large-v3 | 3 GB | ~4% | Slowest |

For most uses, `base.en` provides a good balance of speed and accuracy.

## Requirements

### Runtime Dependencies

- **Wayland compositor** (any - GNOME, KDE, Sway, Hyprland, etc.)
- **PipeWire** or **PulseAudio** (for audio capture)
- **ydotool** + daemon (for typing output) - *optional, falls back to clipboard*
- **wl-clipboard** (for clipboard fallback)

### Permissions

- User must be in the `input` group (for evdev access)

### Installing Dependencies

**Fedora:**
```bash
sudo dnf install ydotool wl-clipboard
```

**Ubuntu/Debian:**
```bash
sudo apt install ydotool wl-clipboard
```

**Arch:**
```bash
sudo pacman -S ydotool wl-clipboard
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

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Daemon                              │
├─────────────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │   Hotkey     │  │    Audio     │  │   Text Output    │  │
│  │  (evdev)     │──│   (cpal)     │──│  (ydotool/clip)  │  │
│  └──────────────┘  └──────────────┘  └──────────────────┘  │
│         │               │                    │              │
│         │               ▼                    │              │
│         │        ┌──────────────┐            │              │
│         │        │   Whisper    │            │              │
│         └───────▶│  (whisper-rs)│────────────┘              │
│                  └──────────────┘                           │
└─────────────────────────────────────────────────────────────┘
```

**Why evdev?** Wayland doesn't provide a standard way to capture global hotkeys. Using evdev (the Linux input subsystem) works on all compositors but requires the user to be in the `input` group.

**Why ydotool?** Similarly, Wayland doesn't provide a standard way to simulate keyboard input. ydotool uses the uinput kernel interface, which works on all compositors.

## License

MIT

---

# Voxtype (Old README content below - can be deleted)

Push-to-talk voice-to-text for Wayland Linux systems.

Hold a hotkey, speak, release the hotkey, and your words appear at the cursor position (or in the clipboard).

## Features

- **Works on all Wayland compositors** - Uses kernel-level input (evdev) and output (ydotool)
- **Fully offline** - Speech recognition via whisper.cpp, no network required
- **Low latency** - Optimized for short push-to-talk recordings
- **Configurable** - Choose your hotkey, model size, output mode
- **Fallback chain** - Falls back to clipboard if typing fails

## Requirements

### System

- Linux with Wayland
- PipeWire or PulseAudio (for audio capture)
- User must be in the `input` group (for hotkey detection)

### Tools

- **ydotool** - For typing text (recommended)
- **wl-copy** - For clipboard fallback (wl-clipboard package)

### Whisper Model

Download a model from [Hugging Face](https://huggingface.co/ggerganov/whisper.cpp/tree/main):

```bash
mkdir -p ~/.local/share/voxtype/models
curl -L -o ~/.local/share/voxtype/models/ggml-base.en.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
```

Or run `voxtype setup` for interactive setup.

## Installation

### From Source

```bash
# Install build dependencies (Fedora)
sudo dnf install alsa-lib-devel

# Install build dependencies (Ubuntu/Debian)
sudo apt install libasound2-dev

# Build
cargo build --release

# Install
sudo cp target/release/voxtype /usr/local/bin/
```

### Setup

```bash
# Add user to input group (required for hotkey detection)
sudo usermod -aG input $USER
# Log out and back in for group change to take effect

# Enable ydotool daemon
systemctl --user enable --now ydotool

# Run setup to verify everything works
voxtype setup
```

## Usage

### Basic

```bash
# Run with defaults (ScrollLock as hotkey)
voxtype

# Use a different hotkey
voxtype --hotkey PAUSE
voxtype --hotkey F13

# Force clipboard mode (no typing, just copy)
voxtype --clipboard

# Use a different model
voxtype --model small.en
```

### Configuration

Create `~/.config/voxtype/config.toml`:

```toml
[hotkey]
key = "SCROLLLOCK"
modifiers = []  # e.g., ["LEFTCTRL"] for Ctrl+ScrollLock

[audio]
device = "default"
sample_rate = 16000
max_duration_secs = 60

[whisper]
model = "base.en"  # tiny, base, small, medium, large-v3
language = "en"    # or "auto" for detection

[output]
mode = "type"      # or "clipboard"
fallback_to_clipboard = true

[output.notification]
on_transcription = true
```

### Commands

```bash
# Run as daemon (default)
voxtype daemon

# Transcribe an audio file
voxtype transcribe recording.wav

# Interactive setup and diagnostics
voxtype setup

# Show current configuration
voxtype config
```

### Systemd Service

Create `~/.config/systemd/user/voxtype.service`:

```ini
[Unit]
Description=Voxtype voice-to-text daemon
After=pipewire.service

[Service]
ExecStart=/usr/local/bin/voxtype daemon
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

Then:

```bash
systemctl --user daemon-reload
systemctl --user enable --now voxtype
```

## Models

| Model | Size | Speed | Quality | Best For |
|-------|------|-------|---------|----------|
| tiny.en | 39 MB | Fastest | Good | Quick notes |
| base.en | 142 MB | Fast | Better | **Recommended** |
| small.en | 466 MB | Medium | Great | Accuracy-focused |
| medium.en | 1.5 GB | Slow | Excellent | High accuracy |
| large-v3 | 3.1 GB | Slowest | Best | Maximum accuracy |

`.en` models are English-only but faster and more accurate for English.

## Troubleshooting

### "Cannot open input device" error

Add your user to the input group:

```bash
sudo usermod -aG input $USER
# Log out and back in
```

### "ydotool daemon not running" error

```bash
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
│                      voxtype daemon                         │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │   evdev     │  │    cpal     │  │      ydotool        │  │
│  │  (hotkey)   │──│   (audio)   │──│   (text output)     │  │
│  │             │  │             │  │   or wl-copy        │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
│                          │                                  │
│                          ▼                                  │
│                   ┌─────────────┐                           │
│                   │ whisper.cpp │                           │
│                   │   (STT)     │                           │
│                   └─────────────┘                           │
└─────────────────────────────────────────────────────────────┘
```

## License

MIT
