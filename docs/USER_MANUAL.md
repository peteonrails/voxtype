# Voxtype User Manual

Voxtype is a push-to-talk voice-to-text tool for Linux. Optimized for Wayland, works on X11 too. This manual covers everything you need to know to use Voxtype effectively.

## Table of Contents

- [Getting Started](#getting-started)
- [Basic Usage](#basic-usage)
- [Commands](#commands)
- [Configuration](#configuration)
- [Hotkeys](#hotkeys)
- [Compositor Keybindings](#compositor-keybindings)
- [Whisper Models](#whisper-models)
- [Output Modes](#output-modes)
- [Tips & Best Practices](#tips--best-practices)
- [Keyboard Shortcuts](#keyboard-shortcuts)
- [Integration Examples](#integration-examples)

---

## Getting Started

After installation, you need to complete the initial setup:

```bash
# 1. Ensure you're in the input group
groups | grep input

# 2. Run the setup wizard
voxtype setup --download

# 3. Start voxtype
voxtype
```

The setup command will:
- Check all dependencies
- Verify permissions
- Download the default Whisper model (base.en)
- Create your configuration file

---

## Basic Usage

### The Push-to-Talk Workflow

1. **Start the daemon**: Run `voxtype` in a terminal (or enable the systemd service)
2. **Hold your hotkey**: Default is ScrollLock
3. **Speak clearly**: Talk at a normal pace
4. **Release the hotkey**: Your speech is transcribed
5. **Text appears**: Either typed at cursor or copied to clipboard

### Example Session

```
$ voxtype
[INFO] Voxtype v0.1.0 starting...
[INFO] Using model: base.en
[INFO] Hotkey: SCROLLLOCK
[INFO] Output mode: type (fallback: clipboard)
[INFO] Ready! Hold SCROLLLOCK to record.

# User holds ScrollLock and says "Hello world"
[INFO] Recording started...
[INFO] Recording stopped (1.2s)
[INFO] Transcribing...
[INFO] Transcribed: "Hello world"
[INFO] Typed 11 characters
```

---

## Commands

### `voxtype` or `voxtype daemon`

Run the voice-to-text daemon. This is the main mode of operation.

```bash
voxtype                     # Run with defaults
voxtype -v                  # Verbose output
voxtype -vv                 # Debug output
voxtype --clipboard         # Force clipboard mode
voxtype --model small.en    # Use a different model
voxtype --hotkey PAUSE      # Use different hotkey
```

### `voxtype transcribe <file>`

Transcribe an audio file without running the daemon.

```bash
voxtype transcribe recording.wav
voxtype --model large-v3 transcribe interview.wav  # Use specific model
```

Supported formats: WAV (16-bit PCM, 16kHz mono recommended)

### `voxtype setup`

Check dependencies and optionally download models.

```bash
voxtype setup              # Check dependencies only
voxtype setup --download   # Download default model (base.en)
voxtype setup model        # Interactive model selection
```

### `voxtype config`

Display the current configuration.

```bash
voxtype config
```

### `voxtype status`

Query the daemon's current state (for Waybar/Polybar integration).

```bash
voxtype status                      # Basic status (text format)
voxtype status --format json        # JSON output for Waybar
voxtype status --follow             # Continuously output on state changes
voxtype status --format json --extended  # Include model, device, backend
```

**Options:**

| Option | Description |
|--------|-------------|
| `--format text` | Human-readable output (default) |
| `--format json` | JSON output for status bars |
| `--follow` | Watch for state changes and output continuously |
| `--extended` | Include model, device, and backend in JSON output |

**Example JSON output with `--extended`:**
```json
{
  "text": "🎙️",
  "class": "idle",
  "tooltip": "Voxtype ready\nModel: base.en\nDevice: default\nBackend: CPU (AVX-512)",
  "model": "base.en",
  "device": "default",
  "backend": "CPU (AVX-512)"
}
```

### `voxtype setup gpu`

Manage GPU acceleration backends.

```bash
voxtype setup gpu            # Show current backend status
voxtype setup gpu --enable   # Switch to Vulkan GPU backend (requires sudo)
voxtype setup gpu --disable  # Switch back to CPU backend (requires sudo)
```

### `voxtype record`

Control recording from external sources (compositor keybindings, scripts).

```bash
voxtype record start   # Start recording (sends SIGUSR1 to daemon)
voxtype record stop    # Stop recording and transcribe (sends SIGUSR2 to daemon)
voxtype record toggle  # Toggle recording state
```

This command is designed for use with compositor keybindings (Hyprland, Sway) instead of the built-in hotkey detection. See [Compositor Keybindings](#compositor-keybindings) for setup instructions.

---

## Configuration

Configuration file location: `~/.config/voxtype/config.toml`

### Full Configuration Reference

```toml
[hotkey]
# The key to hold for push-to-talk recording
# Common choices: SCROLLLOCK, PAUSE, RIGHTALT, F13-F24
key = "SCROLLLOCK"

# Optional modifier keys that must be held with the main key
# Examples: ["LEFTCTRL"], ["LEFTCTRL", "LEFTALT"]
modifiers = []

# Enable built-in hotkey detection (default: true)
# Set to false when using compositor keybindings (Hyprland, Sway) instead
# enabled = true

[audio]
# Audio input device
# "default" uses the system default microphone
# List devices with: pactl list sources short
device = "default"

# Sample rate in Hz (whisper expects 16000)
sample_rate = 16000

# Maximum recording duration in seconds (safety limit)
# Recording automatically stops after this time
max_duration_secs = 60

[whisper]
# Model to use for transcription
# Options: tiny, tiny.en, base, base.en, small, small.en, medium, medium.en, large-v3
# .en models are English-only but faster/smaller
# Can also be an absolute path to a .bin model file
model = "base.en"

# Language for transcription
# "en" for English, "auto" for auto-detection
# See: https://github.com/openai/whisper#available-models-and-languages
language = "en"

# Translate non-English speech to English
translate = false

# Number of CPU threads for inference
# Omit to auto-detect optimal thread count
# threads = 4

# Load model on-demand (saves memory/VRAM, slight delay per recording)
# on_demand_loading = true

[output]
# Primary output mode
# "type" - Simulates keyboard input at cursor (wtype on Wayland, ydotool on X11)
# "clipboard" - Copies text to clipboard (requires wl-copy)
# "paste" - Copies to clipboard then simulates Ctrl+V (for non-US keyboard layouts)
mode = "type"

# Fall back to clipboard if typing fails
fallback_to_clipboard = true

# Delay between typed characters in milliseconds
# 0 = fastest, increase if characters are dropped
type_delay_ms = 0

[output.notification]
# Show notification when recording starts (hotkey pressed)
on_recording_start = false

# Show notification when recording stops (transcription begins)
on_recording_stop = false

# Show notification with transcribed text
on_transcription = true
```

### Creating a Custom Configuration

```bash
# Copy the default config
cp /etc/voxtype/config.toml ~/.config/voxtype/config.toml

# Or generate one
mkdir -p ~/.config/voxtype
voxtype config > ~/.config/voxtype/config.toml
```

### Using a Custom Config File

```bash
voxtype -c /path/to/my/config.toml
```

---

## Hotkeys

### Supported Keys

Any key supported by the Linux evdev system can be used as a hotkey:

| Key | Event Name |
|-----|------------|
| Scroll Lock | `SCROLLLOCK` |
| Pause/Break | `PAUSE` |
| Right Alt | `RIGHTALT` |
| Right Ctrl | `RIGHTCTRL` |
| F13-F24 | `F13`, `F14`, ... `F24` |
| Insert | `INSERT` |
| Home | `HOME` |
| End | `END` |
| Page Up | `PAGEUP` |
| Page Down | `PAGEDOWN` |
| Delete | `DELETE` |
| Caps Lock* | `CAPSLOCK` |

*Note: Using Caps Lock may interfere with normal typing.

### Finding Key Names

Use `evtest` to find the event name of any key:

```bash
sudo evtest
# Select your keyboard device
# Press the key you want to use
# Look for "KEY_XXXXX" - use the part after KEY_
```

### Using Modifier Keys

Require modifier keys to be held along with your hotkey:

```toml
[hotkey]
key = "SCROLLLOCK"
modifiers = ["LEFTCTRL"]  # Ctrl+ScrollLock
```

Available modifiers:
- `LEFTCTRL`, `RIGHTCTRL`
- `LEFTALT`, `RIGHTALT`
- `LEFTSHIFT`, `RIGHTSHIFT`
- `LEFTMETA`, `RIGHTMETA` (Super/Windows key)

---

## Compositor Keybindings

If you prefer to use your compositor's native keybindings instead of voxtype's built-in hotkey detection, you can disable the internal hotkey and trigger recording via CLI commands.

### Why Use Compositor Keybindings?

- **No `input` group membership required** - Voxtype's evdev-based hotkey detection requires being in the `input` group
- **Native feel** - Use familiar keybinding configuration patterns
- **Modifier support** - Use any key combination your compositor supports (e.g., Super+V)

### Setup

1. Disable the internal hotkey in `~/.config/voxtype/config.toml`:
   ```toml
   [hotkey]
   enabled = false
   ```

2. Enable the state file (required for toggle mode):
   ```toml
   state_file = "auto"
   ```

3. Configure your compositor keybindings (see examples below).

### Hyprland

Hyprland supports key release events via `bindr`, enabling push-to-talk:

```hyprlang
# Push-to-talk (hold to record, release to transcribe)
bind = , SCROLL_LOCK, exec, voxtype record start
bindr = , SCROLL_LOCK, exec, voxtype record stop

# Or with a modifier
bind = SUPER, V, exec, voxtype record start
bindr = SUPER, V, exec, voxtype record stop

# Toggle mode (press to start/stop)
bind = SUPER, V, exec, voxtype record toggle
```

### Sway

Sway supports key release events via `--release`:

```
# Push-to-talk
bindsym Scroll_Lock exec voxtype record start
bindsym --release Scroll_Lock exec voxtype record stop

# With a modifier
bindsym $mod+v exec voxtype record start
bindsym --release $mod+v exec voxtype record stop

# Toggle mode
bindsym $mod+v exec voxtype record toggle
```

### River

River supports key release events via `-release` in `riverctl map`. Add these to your `~/.config/river/init`:

```bash
# Push-to-talk (hold to record, release to transcribe)
riverctl map normal None Scroll_Lock spawn 'voxtype record start'
riverctl map -release normal None Scroll_Lock spawn 'voxtype record stop'

# With a modifier
riverctl map normal Super V spawn 'voxtype record start'
riverctl map -release normal Super V spawn 'voxtype record stop'

# Toggle mode (press to start/stop)
riverctl map normal Super V spawn 'voxtype record toggle'
```

### Other Compositors/Desktops

For compositors without key release support (GNOME, KDE), use toggle mode:

```bash
# Generic: bind this to your preferred key
voxtype record toggle
```

### Trade-offs

| Approach | Pros | Cons |
|----------|------|------|
| Built-in hotkey (evdev) | Universal, no config needed | Requires `input` group |
| Compositor keybindings | Native feel, no `input` group | Compositor-specific config |

---

## Whisper Models

### Model Comparison

| Model | Size | Download | RAM Usage | Speed | WER* |
|-------|------|----------|-----------|-------|------|
| tiny.en | 39 MB | Fast | ~400 MB | ~10x realtime | ~10% |
| **base.en** | 142 MB | Fast | ~500 MB | ~7x realtime | ~8% |
| small.en | 466 MB | Medium | ~1 GB | ~4x realtime | ~6% |
| medium.en | 1.5 GB | Slow | ~2.5 GB | ~2x realtime | ~5% |
| large-v3 | 3.1 GB | Slow | ~4 GB | ~1x realtime | ~4% |

*WER = Word Error Rate (lower is better)

### Choosing a Model

- **tiny.en**: Best for low-end hardware, quick notes, or when speed is critical
- **base.en**: Best balance for most users (recommended default)
- **small.en**: When you need better accuracy but have decent hardware
- **medium.en**: For professional transcription on modern hardware
- **large-v3**: Maximum accuracy, multilingual support, requires significant RAM

### English vs Multilingual

- `.en` models: English-only, faster, more accurate for English
- Non-.en models: Multilingual support, slightly slower

### Using Custom Models

Point to any whisper.cpp compatible model:

```toml
[whisper]
model = "/path/to/my/custom-model.bin"
```

---

## Output Modes

### Type Mode (Default)

Simulates keyboard input, typing text directly at your cursor position.

**On Wayland**: Uses wtype (recommended, best CJK/Unicode support)
```bash
# Install wtype
# Fedora: sudo dnf install wtype
# Arch: sudo pacman -S wtype
# Ubuntu: sudo apt install wtype
```

**On X11**: Uses ydotool (requires daemon)
```bash
# Install and start ydotool
# Fedora: sudo dnf install ydotool
# Arch: sudo pacman -S ydotool
# Ubuntu: sudo apt install ydotool
systemctl --user enable --now ydotool
```

**Pros**:
- Text appears exactly where your cursor is
- Works in any application
- Most natural workflow
- wtype supports CJK characters (Korean, Chinese, Japanese)

**Cons**:
- ydotool requires daemon and cannot output CJK characters
- May be slow in some applications (increase `type_delay_ms`)

### Clipboard Mode

Copies transcribed text to the clipboard.

**Requires**: wl-copy (wl-clipboard package)

```toml
[output]
mode = "clipboard"
```

**Pros**:
- Simpler setup
- Faster for long text
- No typing delay issues

**Cons**:
- Requires manual paste (Ctrl+V)
- Overwrites clipboard contents

### Paste Mode

Copies text to clipboard, then automatically simulates Ctrl+V to paste it. This mode is designed for **non-US keyboard layouts** where ydotool's direct typing produces wrong characters.

**Requires**: wl-copy (wl-clipboard) and ydotool

```toml
[output]
mode = "paste"
```

**When to use paste mode**:
- You have a non-US keyboard layout (German, French, Dvorak, etc.)
- ydotool typing produces wrong characters (e.g., `z` and `y` swapped on German layout)
- You're on X11 where wtype isn't available

**How it works**:
1. Copies transcribed text to clipboard via `wl-copy`
2. Waits briefly for clipboard to settle
3. Simulates Ctrl+V keypress via `ydotool`

**Pros**:
- Works with any keyboard layout
- Text appears at cursor position (like type mode)
- No character translation issues

**Cons**:
- Requires both wl-copy and ydotool
- Won't work in applications where Ctrl+V has a different meaning (e.g., Vim command mode)
- Overwrites clipboard contents
- No fallback behavior

### Fallback Behavior

Voxtype uses a fallback chain: wtype → ydotool → clipboard

```toml
[output]
mode = "type"
fallback_to_clipboard = true  # Falls back to clipboard if typing fails
```

On Wayland, wtype is tried first (best CJK support), then ydotool, then clipboard. On X11, ydotool is used, falling back to clipboard if unavailable.

---

## Tips & Best Practices

### For Best Transcription Quality

1. **Speak clearly**: Enunciate words, don't mumble
2. **Maintain consistent volume**: Don't trail off at the end
3. **Minimize background noise**: Close windows, turn off fans
4. **Keep microphone distance consistent**: 6-12 inches is ideal
5. **Use a quality microphone**: USB headsets work well

### For Best Performance

1. **Use an English model** (`.en`) if you only need English
2. **Start with base.en**: Only upgrade if you need better accuracy
3. **Set appropriate thread count**: Let it auto-detect or match your CPU cores
4. **Use an SSD**: Model loading is faster from SSD

### For Workflow Efficiency

1. **Use a dedicated hotkey**: ScrollLock or F13+ keys avoid conflicts
2. **Keep recordings short**: Phrase or sentence at a time
3. **Enable notifications**: Visual feedback when transcription completes
4. **Run as systemd service**: Starts automatically on login

### Handling Punctuation

Whisper attempts to add punctuation automatically. For explicit punctuation, say:
- "period" or "full stop"
- "comma"
- "question mark"
- "exclamation point"
- "new line" or "new paragraph"

---

## Keyboard Shortcuts

While Voxtype is running:

| Action | Default Key |
|--------|-------------|
| Start recording | Hold ScrollLock |
| Stop recording | Release ScrollLock |
| Stop daemon | Ctrl+C (in terminal) |

---

## Logging and Debugging

### Verbosity Levels

```bash
voxtype           # Normal output
voxtype -v        # Verbose (shows transcription progress)
voxtype -vv       # Debug (shows all internal operations)
voxtype -q        # Quiet (errors only)
```

### Viewing Service Logs

```bash
journalctl --user -u voxtype -f
```

### Environment Variables

```bash
# Set log level via environment
RUST_LOG=debug voxtype

# Available levels: error, warn, info, debug, trace
RUST_LOG=voxtype=debug voxtype
```

---

## Integration Examples

### With Systemd (Auto-start)

```bash
systemctl --user enable --now voxtype
```

### With Sway/i3

Add to your config:
```
exec --no-startup-id voxtype
```

### With Hyprland

Add to `hyprland.conf`:
```
exec-once = voxtype
```

### With River

Add to `~/.config/river/init`:
```bash
riverctl spawn voxtype
```

### With GNOME

Enable the systemd service, or add Voxtype to Startup Applications.

### With Waybar (Status Indicator)

Voxtype can display a status indicator in Waybar showing when push-to-talk is active.

> **For a complete step-by-step guide, see [WAYBAR.md](WAYBAR.md).**

**Quick setup:**

1. Enable the state file in `~/.config/voxtype/config.toml`:
   ```toml
   state_file = "auto"
   ```

2. Add the Waybar module to your config:
   ```json
   "custom/voxtype": {
       "exec": "voxtype status --follow --format json",
       "return-type": "json",
       "format": "{}",
       "tooltip": true
   }
   ```

3. Add `"custom/voxtype"` to your modules list and restart Waybar.

The module displays:
- 🎙️ when idle (ready to record)
- 🎤 when recording (hotkey held)
- ⏳ when transcribing

**Extended status info:** Use `--extended` to include model, device, and backend in the JSON output and tooltip:

```json
"custom/voxtype": {
    "exec": "voxtype status --follow --format json --extended",
    "return-type": "json",
    "format": "{}",
    "tooltip": true
}
```

See [WAYBAR.md](WAYBAR.md) for styling options, troubleshooting, and Polybar setup.

### With Polybar

Similar to Waybar, enable `state_file = "auto"` and create a custom script:

```ini
[module/voxtype]
type = custom/script
exec = voxtype status --format text
interval = 1
format = <label>
label = %output%
```

---

## Feedback

We want to hear from you! Voxtype is a young project and your feedback helps make it better.

- **Something not working?** If Voxtype doesn't install cleanly, doesn't work on your system, or is buggy in any way, please [open an issue](https://github.com/peteonrails/voxtype/issues). I actively monitor and respond to issues.
- **Like Voxtype?** I don't accept donations, but if you find it useful, a star on the [GitHub repository](https://github.com/peteonrails/voxtype) would mean a lot!

---

## Next Steps

- [Configuration Reference](CONFIGURATION.md) - Detailed config options
- [Waybar Integration](WAYBAR.md) - Status bar indicator setup
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Common issues and solutions
- [FAQ](FAQ.md) - Frequently asked questions
