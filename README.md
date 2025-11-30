# Voxtype

Push-to-talk voice-to-text for Wayland Linux systems.

Hold a hotkey (default: ScrollLock) while speaking, release to transcribe and output the text at your cursor position.

## Features

- **Works on all Wayland compositors** - Uses kernel-level input (evdev) instead of compositor-specific protocols
- **Fully offline** - Uses whisper.cpp for local transcription, no internet required
- **Fallback chain** - Types via ydotool, falls back to clipboard if unavailable
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
device = "default"
sample_rate = 16000
max_duration_secs = 60

# Audio feedback (sound cues when recording starts/stops)
# [audio.feedback]
# enabled = true
# theme = "default"   # "default", "subtle", "mechanical", or path to custom dir
# volume = 0.7        # 0.0 to 1.0

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

## CLI Options

```
voxtype [OPTIONS] [COMMAND]

Commands:
  daemon      Run as background daemon (default)
  transcribe  Transcribe an audio file
  setup       Check dependencies and download models
  config      Show current configuration
  status      Show daemon state (for Waybar integration)

Options:
  -c, --config <FILE>  Path to config file
  -v, --verbose        Increase verbosity (-v, -vv)
  -q, --quiet          Quiet mode (errors only)
  --clipboard          Force clipboard mode
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

## Feedback

We want to hear from you! Voxtype is a young project and your feedback helps make it better.

- **Something not working?** If Voxtype doesn't install cleanly, doesn't work on your system, or is buggy in any way, please [open an issue](https://github.com/peteonrails/voxtype/issues). I actively monitor and respond to issues.
- **Like Voxtype?** I don't accept donations, but if you find it useful, a star on the [GitHub repository](https://github.com/peteonrails/voxtype) would mean a lot!

## License

MIT
