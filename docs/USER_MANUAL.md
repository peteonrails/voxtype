# Voxtype User Manual

Voxtype is a push-to-talk voice-to-text tool for Linux. Optimized for Wayland, works on X11 too. This manual covers everything you need to know to use Voxtype effectively.

## Table of Contents

- [Getting Started](#getting-started)
- [Basic Usage](#basic-usage)
- [Commands](#commands)
- [Configuration](#configuration)
- [Hotkeys](#hotkeys)
- [Compositor Keybindings](#compositor-keybindings)
- [Canceling Transcription](#canceling-transcription)
- [Whisper Models](#whisper-models)
- [Remote Whisper Servers](#remote-whisper-servers)
- [Output Modes](#output-modes)
- [Post-Processing with LLMs](#post-processing-with-llms)
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
voxtype status --format json --icon-theme nerd-font  # Use specific icon theme
```

**Options:**

| Option | Description |
|--------|-------------|
| `--format text` | Human-readable output (default) |
| `--format json` | JSON output for status bars |
| `--follow` | Watch for state changes and output continuously |
| `--extended` | Include model, device, and backend in JSON output |
| `--icon-theme THEME` | Override icon theme (emoji, nerd-font, material, etc.) |

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
voxtype record cancel  # Cancel recording or transcription in progress
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

2. Ensure state file is enabled (required for toggle mode, enabled by default):
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

### Modifier Key Issues

If you use a multi-key combination (e.g., `SUPER+CTRL+X`) and release keys slowly, the typed output may trigger compositor shortcuts instead of inserting text. See [Output Hooks (Compositor Integration)](#output-hooks-compositor-integration) for an automatic fix.

---

## Canceling Transcription

You can cancel recording or transcription at any time. When canceled, no text is output.

### With Compositor Keybindings

Use `voxtype record cancel` bound to a key (typically Escape):

**Hyprland** (in your submap):
```hyprlang
bind = , Escape, exec, voxtype record cancel
bind = , Escape, submap, reset
```

**Sway** (in your mode):
```
bindsym Escape exec voxtype record cancel; mode "default"
```

**River**:
```bash
riverctl map voxtype_suppress None Escape spawn "voxtype record cancel"
riverctl map voxtype_suppress None Escape enter-mode normal
```

If you use `voxtype setup compositor`, these bindings are generated automatically.

### With Evdev Hotkeys

Configure a cancel key in your `~/.config/voxtype/config.toml`:

```toml
[hotkey]
key = "SCROLLLOCK"
cancel_key = "ESC"  # Press Escape to cancel
```

Any valid evdev key name works. Common choices:
- `ESC` - Escape key
- `BACKSPACE` - Backspace key
- `F12` - Function key

### What Gets Canceled

- **During recording**: Audio capture stops, recorded audio is discarded
- **During transcription**: Transcription is aborted, no text is output
- **While idle**: No effect

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

## Remote Whisper Servers

Voxtype can offload transcription to a remote server running whisper.cpp or any OpenAI-compatible Whisper API. This feature was designed for users who self-host Whisper servers on their own hardware (e.g., a home GPU server), but it can also connect to cloud-based services.

> **Privacy Notice**
>
> When using remote transcription, your audio is transmitted over the network to the configured server. **If privacy is a concern, you should carefully consider who operates the remote server and how your data is handled.**
>
> - **Self-hosted servers**: You maintain full control over your data
> - **Cloud services (OpenAI, etc.)**: Your audio is processed by third parties and may be subject to their privacy policies, data retention, and usage terms
>
> For maximum privacy, use Voxtype's default local transcription mode, which processes all audio entirely on your machine with no network connectivity.

### Why Use Remote Transcription?

Remote transcription is valuable when:

1. **Your local hardware is too slow**: Larger Whisper models (large-v3, large-v3-turbo) require significant compute. A laptop CPU might take 10-30x the audio duration, while a remote GPU can transcribe in real-time or faster.

2. **You have a home GPU server**: Many users have a separate machine with a powerful GPU. Remote transcription lets you leverage that hardware while using Voxtype's superior output method (virtual keyboard instead of clipboard paste).

3. **Teams sharing infrastructure**: Organizations with centralized ML inference servers can share a single Whisper deployment.

4. **Thin clients**: Use Voxtype on a lightweight laptop while offloading compute to more powerful hardware on your network.

### Setting Up a Self-Hosted Whisper Server

The recommended server is **whisper.cpp server**, which implements the OpenAI Whisper API:

```bash
# Clone whisper.cpp
git clone https://github.com/ggerganov/whisper.cpp
cd whisper.cpp

# Build the server
make server

# Download a model
./models/download-ggml-model.sh large-v3-turbo

# Run the server (adjust for your GPU)
./server -m models/ggml-large-v3-turbo.bin --host 0.0.0.0 --port 8080
```

For GPU acceleration, build with CUDA, Vulkan, or Metal support:

```bash
# CUDA (NVIDIA)
make server GGML_CUDA=1

# Vulkan (AMD/NVIDIA/Intel)
make server GGML_VULKAN=1

# Metal (macOS)
make server GGML_METAL=1
```

### Configuring Voxtype for Remote Transcription

Edit your `~/.config/voxtype/config.toml`:

```toml
[whisper]
# Switch to remote backend
backend = "remote"

# Language setting still applies
language = "en"

# Your whisper.cpp server address
remote_endpoint = "http://192.168.1.100:8080"

# Model name sent to server (whisper.cpp ignores this, but OpenAI requires it)
remote_model = "whisper-1"

# Request timeout in seconds (increase for large files or slow networks)
remote_timeout_secs = 30

# Optional: API key (not needed for whisper.cpp, required for OpenAI)
# remote_api_key = "your-api-key"
```

### Using with Cloud Services (OpenAI, etc.)

While this feature was built for self-hosted servers, it also works with OpenAI's hosted Whisper API and other compatible services:

```toml
[whisper]
backend = "remote"
language = "en"
remote_endpoint = "https://api.openai.com"
remote_model = "whisper-1"
remote_timeout_secs = 30
```

Set your API key via environment variable (more secure than config file):

```bash
export VOXTYPE_WHISPER_API_KEY="sk-..."
```

> **Cloud Service Considerations**
>
> - **Cost**: OpenAI charges per minute of audio (~$0.006/min)
> - **Privacy**: Your audio is transmitted to and processed by OpenAI's servers
> - **Latency**: Network round-trip adds delay compared to local processing
> - **Reliability**: Depends on internet connectivity and service availability
>
> For most users, local transcription with GPU acceleration provides better privacy, lower latency, and no ongoing costs.

### Security Recommendations

1. **Use HTTPS for non-local servers**: Voxtype warns if you configure an HTTP endpoint for non-localhost addresses, as audio would be transmitted unencrypted.

2. **Prefer environment variables for API keys**: Use `VOXTYPE_WHISPER_API_KEY` instead of putting keys in config files.

3. **Firewall your self-hosted server**: If running whisper.cpp server, ensure it's only accessible from trusted networks.

### Troubleshooting Remote Transcription

**Connection refused**:
- Verify the server is running and listening on the expected port
- Check firewall rules on both client and server
- Ensure the endpoint URL includes the protocol (`http://` or `https://`)

**Timeout errors**:
- Increase `remote_timeout_secs` in your config
- Check network latency between client and server
- For large audio files, the server may need more time to process

**Authentication errors**:
- Verify your API key is correct
- Check if the key is set via environment variable or config
- Ensure the `Authorization: Bearer` header is being sent

**Run with verbose logging** to diagnose issues:

```bash
voxtype -vv
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

**Compositor Compatibility:**

wtype does not work on all Wayland compositors. KDE Plasma and GNOME do not support the virtual keyboard protocol that wtype requires.

| Desktop | wtype | ydotool | Notes |
|---------|-------|---------|-------|
| Hyprland, Sway, River | ✓ | ✓ | wtype recommended (best CJK support) |
| KDE Plasma (Wayland) | ✗ | ✓ | Use ydotool (daemon required) |
| GNOME (Wayland) | ✗ | ✓ | Use ydotool (daemon required) |
| X11 (any) | ✗ | ✓ | Use ydotool (daemon required) |

**KDE Plasma and GNOME users:** You must set up ydotool for type mode to work. Install ydotool and start the daemon:

```bash
systemctl --user enable --now ydotool
```

See [Troubleshooting: wtype not working on KDE/GNOME](TROUBLESHOOTING.md#wtype-not-working-on-kde-plasma-or-gnome-wayland) for details.

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

## Output Hooks (Compositor Integration)

Voxtype can run commands before and after typing output. This is primarily useful for compositor integration—for example, switching to a Hyprland submap that blocks modifier keys during typing.

### Why Output Hooks?

When using compositor keybindings with modifiers (e.g., `SUPER+CTRL+X`), if you release the keys slowly, held modifiers can interfere with typed output. For example, if SUPER is still held when voxtype types "hello", you might trigger SUPER+h, SUPER+e, etc. instead of inserting text.

Output hooks solve this by letting you block modifier keys at the compositor level during typing.

### Automatic Setup (Recommended)

If you're experiencing modifier key interference (typed text triggering shortcuts instead of inserting characters), use the compositor setup command to automatically install a fix:

```bash
# For Hyprland
voxtype setup compositor hyprland
hyprctl reload
systemctl --user restart voxtype

# For Sway
voxtype setup compositor sway
swaymsg reload
systemctl --user restart voxtype

# For River
voxtype setup compositor river
# Restart River or source the new config
systemctl --user restart voxtype
```

**Note:** This command does NOT set up keybindings for voxtype. It only installs a workaround that blocks modifier keys during text output. See [Compositor Keybindings](#compositor-keybindings) to set up your push-to-talk hotkey.

This command:
1. Writes a modifier-blocking submap/mode to `~/.config/hypr/conf.d/voxtype-submap.conf` (or `sway/conf.d/voxtype-mode.conf`, or `river/conf.d/voxtype-mode.sh`)
2. Adds pre/post output hooks to your voxtype config
3. Checks that your compositor sources the conf.d directory

If voxtype crashes while typing, press **F12** to escape the submap.

Use `--status` to check installation status, `--uninstall` to remove, or `--show` to view the configuration without installing.

### Manual Configuration

If you prefer manual setup, add these to your voxtype config:

```toml
[output]
# Command to run BEFORE typing (e.g., switch to modifier-blocking submap)
pre_output_command = "hyprctl dispatch submap voxtype_suppress"

# Command to run AFTER typing (e.g., reset to default submap)
post_output_command = "hyprctl dispatch submap reset"
```

See `voxtype setup compositor hyprland --show` for the full submap configuration.

### Other Uses

Output hooks are generic shell commands—you can use them for any compositor or custom workflow:

- **Sway**: Use `swaymsg` commands
- **River**: Use `riverctl` commands
- **Notifications**: `pre_output_command = "notify-send 'Typing...'"`
- **Logging**: `post_output_command = "echo $(date) >> ~/voxtype.log"`

---

## Post-Processing with LLMs

Voxtype can pipe transcriptions through an external command before output, enabling integration with local LLMs for text cleanup, grammar correction, and filler word removal.

### When to Use Post-Processing

Post-processing is most valuable for specific workflows:

- **Translation**: Speak in one language, output in another
- **Domain vocabulary**: Medical, legal, or technical term correction
- **Reformatting**: Convert casual dictation to formal prose
- **Stubborn filler words**: Remove "um", "uh", "like" that Whisper occasionally keeps
- **Custom workflows**: Multi-output scenarios (e.g., translate to 5 languages, save to file, inject only one at cursor)

**Important**: For most users, Whisper large-v3-turbo with Voxtype's built-in `spoken_punctuation` feature is sufficient. Post-processing adds 2-5 seconds of latency and provides marginal improvement for general transcription.

**Limitations**:
- LLMs interpret text literally—saying "slash" won't produce "/" (use `spoken_punctuation` instead)
- Use instruct/chat models, not reasoning models (they output `<think>` blocks)
- Avoid emojis in LLM output—ydotool cannot type them

### Basic Configuration

Add an `[output.post_process]` section to your config:

```toml
[output.post_process]
# Command receives text on stdin, outputs cleaned text on stdout
command = "ollama run llama3.2:1b 'Clean up this dictation. Fix grammar, remove filler words. Output only the cleaned text:'"
timeout_ms = 30000  # 30 second timeout (generous for LLM)
```

### Example Commands

**Ollama (recommended for simplicity):**
```toml
[output.post_process]
command = "ollama run llama3.2:1b 'Clean up this transcription. Fix grammar and remove filler words. Output only the cleaned text:'"
timeout_ms = 30000
```

**Simple sed-based cleanup (fast, no LLM):**
```toml
[output.post_process]
command = "sed 's/\\bum\\b//g; s/\\buh\\b//g; s/\\blike\\b//g'"
timeout_ms = 5000
```

**Custom script:**
```toml
[output.post_process]
command = "~/.config/voxtype/cleanup.sh"
timeout_ms = 45000
```

**Swedish Chef mode (for fun):**
```toml
[output.post_process]
command = "/path/to/swedish-chef.sh"
timeout_ms = 1000
```
See `examples/swedish-chef.sh` for a script that transforms your dictation into Swedish Chef speak. Bork bork bork!

### LM Studio Script Example

For users running LM Studio locally:

```bash
#!/bin/bash
# ~/.config/voxtype/lm-studio-cleanup.sh

INPUT=$(cat)

curl -s http://localhost:1234/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d "{
    \"messages\": [{
      \"role\": \"system\",
      \"content\": \"Clean up this dictated text. Fix spelling, remove filler words (um, uh), add proper punctuation. Output ONLY the cleaned text, nothing else.\"
    },{
      \"role\": \"user\",
      \"content\": \"$INPUT\"
    }],
    \"temperature\": 0.1
  }" | jq -r '.choices[0].message.content'
```

Make it executable: `chmod +x ~/.config/voxtype/lm-studio-cleanup.sh`

### Timeout Recommendations

| Use Case | Timeout |
|----------|---------|
| Simple shell commands (sed, tr) | 5000ms (5 seconds) |
| Local LLMs (Ollama, llama.cpp) | 30000-60000ms (30-60 seconds) |
| Remote APIs | 30000ms or higher |

### Error Handling

Post-processing is designed to be fault-tolerant:

- **Command not found**: Falls back to original text
- **Timeout**: Falls back to original text
- **Non-zero exit**: Falls back to original text
- **Empty output**: Falls back to original text

This ensures voice-to-text always produces output, even when the LLM is slow or unavailable.

### Debugging

Run Voxtype with verbose logging to see post-processing in action:

```bash
voxtype -vv
```

You'll see log messages like:
```
[DEBUG] After text processing: "um so I think we should um fix the bug"
[DEBUG] Post-processed (47 -> 32 chars)
[DEBUG] After post-processing: "I think we should fix the bug"
```

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

1. Ensure state file is enabled in `~/.config/voxtype/config.toml` (enabled by default):
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

The module displays (default emoji theme):
- 🎙️ when idle (ready to record)
- 🎤 when recording (hotkey held)
- ⏳ when transcribing

**Customizing icons:** Choose from 10 built-in themes or define your own:

```toml
[status]
icon_theme = "nerd-font"  # or: material, phosphor, codicons, minimal, dots, arrows, text
```

Available themes include Nerd Font, Material Design Icons, Phosphor, VS Code Codicons, and several universal themes that don't require special fonts (minimal, dots, arrows, text).

**Extended status info:** Use `--extended` to include model, device, and backend in the JSON output and tooltip:

```json
"custom/voxtype": {
    "exec": "voxtype status --follow --format json --extended",
    "return-type": "json",
    "format": "{}",
    "tooltip": true
}
```

See [WAYBAR.md](WAYBAR.md) for complete icon customization options, styling, and troubleshooting.

### With Polybar

Similar to Waybar, ensure `state_file = "auto"` is set (the default) and create a custom script:

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
