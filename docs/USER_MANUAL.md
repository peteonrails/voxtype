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
- [Transcription Engines](#transcription-engines)
- [Multi-Model Support](#multi-model-support)
- [Improving Transcription Accuracy](#improving-transcription-accuracy)
- [Whisper Models](#whisper-models)
- [Remote Whisper Servers](#remote-whisper-servers)
- [CLI Backend (whisper-cli)](#cli-backend-whisper-cli)
- [Eager Processing](#eager-processing)
- [Output Modes](#output-modes)
- [Post-Processing with LLMs](#post-processing-with-llms)
- [Profiles](#profiles)
- [Voice Activity Detection](#voice-activity-detection)
- [Meeting Mode](#meeting-mode)
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
voxtype setup vad          # Download the Silero VAD model
voxtype setup onnx         # Switch between Whisper and ONNX engines
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

### `voxtype setup dms`

Install a status widget for DankMaterialShell (KDE Plasma alternative shell).

```bash
voxtype setup dms              # Show setup instructions
voxtype setup dms --install    # Install the QML widget plugin
voxtype setup dms --uninstall  # Remove the widget
voxtype setup dms --qml        # Output raw QML content
```

See [With DankMaterialShell](#with-dankmaterialshell-kde-plasma) for details.

### `voxtype record`

Control recording from external sources (compositor keybindings, scripts).

```bash
voxtype record start                # Start recording (sends SIGUSR1 to daemon)
voxtype record start --file=out.txt # Write transcription to a file
voxtype record start --file         # Write to file_path from config
voxtype record stop                 # Stop recording and transcribe (sends SIGUSR2 to daemon)
voxtype record toggle               # Toggle recording state
voxtype record cancel               # Cancel recording or transcription in progress
```

**Model override:** Use `--model` to specify which model to use for this recording:

```bash
voxtype record start --model large-v3-turbo  # Use a specific model
voxtype record stop                          # Transcribes with the model specified at start
```

The model must be configured as `model`, `secondary_model`, or listed in `available_models` in your config. See [Multi-Model Configuration](CONFIGURATION.md#secondary_model) for setup.

**Output mode override:** Use `--type`, `--clipboard`, or `--paste` to override the output mode:

```bash
voxtype record start --clipboard  # Output to clipboard instead of typing
voxtype record toggle --paste     # Use paste mode for this recording
```

**File output:** The `--file` flag writes transcription to a file instead of typing or using clipboard. Use `--file=path.txt` for a specific file, or `--file` alone to use `file_path` from config. By default, the file is overwritten on each transcription. To append instead, set `file_mode = "append"` in your config file:

```toml
[output]
file_mode = "append"
```

For persistent file output without the CLI flag, use `mode = "file"` with `file_path` in your config. See [Configuration Reference](CONFIGURATION.md) for details.

This command is designed for use with compositor keybindings (Hyprland, Sway) instead of the built-in hotkey detection. See [Compositor Keybindings](#compositor-keybindings) for setup instructions.

### `voxtype meeting`

Continuous meeting transcription with chunked processing and speaker diarization. See [Meeting Mode](#meeting-mode) for full details.

```bash
voxtype meeting start                  # Start a meeting
voxtype meeting start --title "Title"  # Start with a title
voxtype meeting stop                   # Stop the meeting
voxtype meeting pause                  # Pause recording
voxtype meeting resume                 # Resume recording
voxtype meeting status                 # Show current meeting status
voxtype meeting list                   # List past meetings
voxtype meeting export latest          # Export transcript (markdown)
voxtype meeting summarize latest       # Generate AI summary
```

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
# Recording automatically stops and transcribes after this time
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
# "type" - Simulates keyboard input at cursor (wtype/dotool/ydotool)
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

### Configuration Priority

Settings are applied in layers, with later layers overriding earlier ones:

1. **Built-in defaults** (lowest priority)
2. **Config file** (`~/.config/voxtype/config.toml`)
3. **Environment variables** (`VOXTYPE_*`)
4. **CLI flags** (highest priority)

Every config file option has a corresponding `VOXTYPE_*` environment variable and CLI flag. See `voxtype --help` for the full list of CLI flags, and [CONFIGURATION.md](CONFIGURATION.md#voxtype_-configuration-overrides) for the full list of environment variables.

```bash
# Override model and auto-submit via environment
VOXTYPE_MODEL=large-v3-turbo VOXTYPE_AUTO_SUBMIT=true voxtype

# Override via CLI flags (takes priority over env vars and config file)
voxtype --model large-v3-turbo --auto-submit
```

Per-recording overrides are available on `record start` and `record toggle`:

```bash
# Auto-submit just this recording (even if config has auto_submit = false)
voxtype record start --auto-submit

# Disable auto-submit just this recording (even if config has auto_submit = true)
voxtype record toggle --no-auto-submit
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
| Media | `MEDIA` |
| Record | `RECORD` |
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

### Numeric Keycodes

If your key isn't in the built-in list, you can specify it by numeric keycode. Use a prefix to indicate which tool you got the number from, since `wev`/`xev` and `evtest` report different numbers for the same key (XKB keycodes are offset by 8 from kernel keycodes):

```toml
[hotkey]
key = "WEV_234"      # XKB keycode from wev/xev (KEY_MEDIA)
key = "EVTEST_226"   # Kernel keycode from evtest (KEY_MEDIA)
key = "WEV_0xEA"     # Hex also works
```

Prefixes: `WEV_`, `X11_`, `XEV_` (XKB keycode), `EVTEST_` (kernel keycode).

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

## Transcription Engines

Voxtype supports seven speech-to-text engines. Whisper uses whisper.cpp and works with any binary variant. The other six engines run via ONNX Runtime and require an ONNX binary variant (`voxtype-*-onnx-*`).

| Engine | Best For | GPU Required | Languages |
|--------|----------|--------------|-----------|
| **Whisper** (default) | Most users, multilingual | Optional (faster with GPU) | 99+ languages |
| **Parakeet** | Fast CPU inference, English | Optional (CUDA available) | English only |
| **Moonshine** | Very fast CPU inference, small models | No | English + multilingual variants |
| **SenseVoice** | Multilingual CTC, emotion/event detection | No | Chinese, English, Japanese, Korean, Cantonese |
| **Paraformer** | Chinese + English dictation | No | Chinese (with English code-switching) |
| **Dolphin** | Dictation-optimized, fast CTC | No | Chinese + English |
| **Omnilingual** | Broadest language coverage in ONNX engines | No | 50+ languages |

### Selecting an Engine

**Via config file** (`~/.config/voxtype/config.toml`):

```toml
# Default - use Whisper
engine = "whisper"

# ONNX engines (require onnx binary variant)
engine = "parakeet"
engine = "moonshine"
engine = "sensevoice"
engine = "paraformer"
engine = "dolphin"
engine = "omnilingual"
```

**Via CLI flag** (overrides config):

```bash
voxtype --engine whisper daemon
voxtype --engine parakeet daemon
voxtype --engine moonshine daemon
voxtype --engine sensevoice daemon
voxtype --engine paraformer daemon
voxtype --engine dolphin daemon
voxtype --engine omnilingual daemon
```

Valid `--engine` values: `whisper`, `parakeet`, `moonshine`, `sensevoice`, `paraformer`, `dolphin`, `omnilingual`.

### Switching to an ONNX Engine

All engines except Whisper require an ONNX binary variant. Use the setup command to switch:

```bash
voxtype setup onnx --enable    # Switch to ONNX binary
voxtype setup onnx --disable   # Switch back to Whisper binary
voxtype setup onnx --status    # Show current backend
```

### Whisper (Default)

Whisper is OpenAI's speech recognition model, running locally via whisper.cpp. It offers:

- Excellent accuracy across many languages
- Multiple model sizes (tiny to large)
- GPU acceleration via Vulkan (AMD/Intel) or CUDA (NVIDIA)
- Active community and frequent updates

This is the recommended engine for most users.

### Parakeet

Parakeet is NVIDIA's FastConformer-based ASR model. It offers:

- Very fast CPU inference (30x realtime on AVX-512)
- Good accuracy for English dictation
- Proper punctuation and capitalization
- No GPU required (though CUDA acceleration available)

**Requirements:**
- An ONNX-enabled binary (`voxtype-*-onnx-*`)
- The Parakeet model downloaded (~600MB)
- English-only use case

**Configuration:**

```toml
engine = "parakeet"

[parakeet]
model = "parakeet-tdt-0.6b-v3"  # or "parakeet-tdt-0.6b-v3-int8"
# model_type = "tdt"            # "tdt" (recommended) or "ctc", auto-detected if omitted
# on_demand_loading = false
```

See [PARAKEET.md](PARAKEET.md) for detailed setup instructions.

### Moonshine

Moonshine is an encoder-decoder transformer model running via ONNX Runtime. It offers:

- Very fast CPU inference (0.09s for 4s audio on Ryzen 9 9900X3D)
- Small model sizes (tiny: 100MB, base: 237MB)
- English models are MIT-licensed; multilingual models (Japanese, Mandarin, Korean, Arabic) use a community license
- Outputs lowercase without punctuation

**Requirements:**
- An ONNX-enabled binary (`voxtype-*-onnx-*`)
- A Moonshine model downloaded to `~/.local/share/voxtype/models/`

**Configuration:**

```toml
engine = "moonshine"

[moonshine]
model = "base"        # "tiny" (27M params) or "base" (61M params)
quantized = true      # Use quantized models for faster inference (default: true)
# threads = 4         # CPU threads (omit for auto-detect)
# on_demand_loading = false
```

See [MOONSHINE.md](MOONSHINE.md) for detailed setup instructions.

### SenseVoice

SenseVoice is Alibaba's FunAudioLLM CTC encoder-only model. It offers:

- Multilingual support: Chinese, English, Japanese, Korean, Cantonese
- Automatic language detection
- Inverse text normalization (adds punctuation)
- Emotion and audio event detection

**Requirements:**
- An ONNX-enabled binary (`voxtype-*-onnx-*`)
- The SenseVoice model downloaded

**Configuration:**

```toml
engine = "sensevoice"

[sensevoice]
model = "sensevoice-small"  # Default model
language = "auto"           # "auto", "zh", "en", "ja", "ko", "yue"
use_itn = true              # Inverse text normalization (punctuation)
# threads = 4
# on_demand_loading = false
```

### Paraformer

Paraformer is a FunASR CTC encoder model optimized for Chinese with English code-switching. It offers:

- Fast non-autoregressive inference
- Good accuracy for Chinese dictation

**Requirements:**
- An ONNX-enabled binary (`voxtype-*-onnx-*`)
- The Paraformer model downloaded

**Configuration:**

```toml
engine = "paraformer"

[paraformer]
model = "paraformer-zh"  # Default model
# threads = 4
# on_demand_loading = false
```

### Dolphin

Dolphin is a dictation-optimized CTC encoder model. It offers:

- Fast inference tuned for dictation workflows
- CTC architecture for low-latency output

**Requirements:**
- An ONNX-enabled binary (`voxtype-*-onnx-*`)
- The Dolphin model downloaded

**Configuration:**

```toml
engine = "dolphin"

[dolphin]
model = "dolphin-base"  # Default model
# threads = 4
# on_demand_loading = false
```

### Omnilingual

Omnilingual is a FunASR CTC encoder model with the broadest language coverage among the ONNX engines. It offers:

- Support for 50+ languages
- Good accuracy across diverse language families

**Requirements:**
- An ONNX-enabled binary (`voxtype-*-onnx-*`)
- The Omnilingual model downloaded

**Configuration:**

```toml
engine = "omnilingual"

[omnilingual]
model = "omnilingual-large"  # Default model
# threads = 4
# on_demand_loading = false
```

---

## Multi-Model Support

Voxtype can manage multiple Whisper models, letting you switch between them on the fly. Use a fast model for everyday dictation and a more accurate model when precision matters.

### Configuration

```toml
[hotkey]
model_modifier = "LEFTSHIFT"  # Hold Shift + hotkey for secondary model

[whisper]
model = "base.en"                    # Primary model (fast, always loaded)
secondary_model = "large-v3-turbo"   # Secondary model (accurate, on-demand)
available_models = ["medium.en"]     # Additional models for CLI access
max_loaded_models = 2                # Keep up to 2 models in memory
cold_model_timeout_secs = 300        # Evict unused models after 5 minutes
```

### Using Multiple Models

**With hotkey modifier** (evdev mode):
- Normal hotkey press → uses primary model (`base.en`)
- Hold Shift + hotkey → uses secondary model (`large-v3-turbo`)

**With CLI** (compositor keybindings):
```bash
# Use primary model
voxtype record start

# Use secondary model
voxtype record start --model large-v3-turbo

# Use any available model
voxtype record start --model medium.en
```

### Memory Management

Voxtype caches loaded models to avoid reload delays:

- `max_loaded_models`: Maximum models to keep in memory (default: 2)
- `cold_model_timeout_secs`: Auto-evict unused models after this time (default: 300s)
- Primary model is never evicted
- Models load in the background while you speak

### Example: Speed vs Accuracy

```toml
[hotkey]
key = "SCROLLLOCK"
model_modifier = "LEFTSHIFT"

[whisper]
model = "tiny.en"                   # Lightning fast for quick notes
secondary_model = "large-v3-turbo"  # High accuracy when needed

[audio.feedback]
enabled = true  # Helpful audio cues when switching models
```

---

## Improving Transcription Accuracy

Whisper sometimes mistranscribes uncommon words—technical terms, proper nouns, company names, or domain-specific jargon. The `initial_prompt` feature lets you provide hints that improve accuracy for these cases.

### When to Use initial_prompt

Use initial_prompt when you regularly dictate content containing:

- **Technical jargon**: Kubernetes, PostgreSQL, TypeScript, systemd
- **Product/company names**: Voxtype, Hyprland, Waybar, wtype
- **People's names**: Especially non-English names that Whisper might mishear
- **Acronyms**: API, CLI, LLM, CI/CD
- **Domain-specific terms**: Medical, legal, or scientific vocabulary

### Configuration

Add to your `~/.config/voxtype/config.toml`:

```toml
[whisper]
model = "base.en"
initial_prompt = "Technical discussion about Rust, TypeScript, and Kubernetes."
```

The prompt doesn't need to be a complete sentence. A list of expected terms works well:

```toml
# Software development
initial_prompt = "Voxtype, Hyprland, Waybar, Sway, wtype, ydotool, systemd."

# Medical dictation
initial_prompt = "Medical notes: hypertension, myocardial infarction, CT scan, MRI."

# Meeting context
initial_prompt = "Meeting with Zhang Wei, François Dupont, and Priya Sharma."
```

### CLI Override

Override the prompt for a single session:

```bash
voxtype --initial-prompt "Discussion about Terraform and AWS Lambda" daemon
```

### Tips

- Keep prompts short—a sentence or list of terms is sufficient
- Update the prompt when your context changes (different project, client, or domain)
- Combine with a larger model (`small.en` or `medium.en`) for best results on difficult vocabulary
- The prompt guides Whisper's expectations but doesn't guarantee exact transcription

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

## CLI Backend (whisper-cli)

The CLI backend uses `whisper-cli` from whisper.cpp as a subprocess instead of the built-in whisper-rs FFI bindings. This is a fallback for systems where the FFI bindings crash.

### When to Use CLI Backend

Use the CLI backend if:

1. **Voxtype crashes during transcription**: Some systems with glibc 2.42+ (e.g., Ubuntu 25.10) experience crashes due to C++ exceptions crossing the FFI boundary. whisper.cpp works fine when run as a standalone binary.

2. **You want to use a custom whisper.cpp build**: If you've compiled whisper.cpp with specific optimizations or features not available in the bundled whisper-rs bindings.

3. **Debugging transcription issues**: Running whisper-cli as a subprocess makes it easier to isolate and diagnose problems.

### Setting Up CLI Backend

1. Install whisper-cli from [whisper.cpp](https://github.com/ggerganov/whisper.cpp):

```bash
git clone https://github.com/ggerganov/whisper.cpp
cd whisper.cpp
cmake -B build
cmake --build build --config Release
sudo cp build/bin/whisper-cli /usr/local/bin/
```

2. Configure voxtype to use CLI backend:

```toml
[whisper]
backend = "cli"
model = "base.en"
language = "en"

# Optional: specify path if not in PATH
# whisper_cli_path = "/usr/local/bin/whisper-cli"
```

3. Restart the voxtype daemon:

```bash
systemctl --user restart voxtype
```

### How It Works

When using CLI backend, voxtype:

1. Writes recorded audio to a temporary WAV file
2. Runs `whisper-cli` with the configured model and options
3. Parses the JSON output from whisper-cli
4. Cleans up temporary files

This adds minimal overhead compared to the FFI approach since file I/O is fast on modern systems.

### Limitations

- Slightly higher latency than FFI (file I/O overhead)
- Requires separate whisper-cli installation
- No GPU isolation mode (whisper-cli manages its own GPU memory)

---

## Eager Processing

Eager processing transcribes audio in chunks while you're still recording. Instead of waiting until you release the hotkey to start transcription, voxtype begins processing audio in the background as you speak. When you stop recording, the final chunk is transcribed and all results are combined.

### When to Use Eager Processing

Eager processing is most valuable when:

1. **You have slow transcription hardware**: On machines where transcription takes longer than the recording itself, eager processing parallelizes the work to reduce overall wait time.

2. **You make long recordings**: For recordings over 15-30 seconds, starting transcription early means less waiting when you're done speaking.

3. **You use large models**: Larger Whisper models (medium, large-v3) are slower. Eager processing helps hide some of that latency.

**Not recommended when:**
- You use fast models (tiny, base) on modern hardware with GPU acceleration
- Your recordings are typically short (under 5 seconds)
- You're on a laptop and want to minimize battery usage

### How It Works

With eager processing enabled:

1. Audio accumulates as you record
2. Every `eager_chunk_secs` (default: 5 seconds), a chunk is extracted and sent for transcription
3. Chunks overlap by `eager_overlap_secs` (default: 0.5 seconds) to avoid missing words at boundaries
4. When you stop recording, all chunk results are combined and deduplicated
5. The final text is output

The overlap region helps catch words that might be split across chunk boundaries. The deduplication logic matches overlapping text to produce a clean result.

### Configuration

Enable eager processing in `~/.config/voxtype/config.toml`:

```toml
[whisper]
model = "medium.en"

# Enable eager input processing
eager_processing = true

# Chunk duration (default: 5.0 seconds)
eager_chunk_secs = 5.0

# Overlap between chunks (default: 0.5 seconds)
eager_overlap_secs = 0.5
```

Or via CLI flags:

```bash
voxtype --eager-processing --eager-chunk-secs 5.0 daemon
```

### Tuning Chunk Size

The chunk duration affects the trade-off between parallelization and overhead:

| Chunk Size | Pros | Cons |
|------------|------|------|
| 3 seconds | More parallelization, faster for slow models | More boundary handling, slightly higher CPU |
| 5 seconds | Good balance for most cases | - |
| 10 seconds | Fewer chunks to combine | Less parallelization benefit |

For testing, try `eager_chunk_secs = 3.0` to see more chunk messages in the logs.

### Trade-offs

**Benefits:**
- Reduced perceived latency on slow hardware
- Better experience for long recordings
- Parallelizes transcription work across recording time

**Limitations:**
- Boundary handling may occasionally produce artifacts (repeated or dropped words at chunk edges)
- Slightly higher CPU usage during recording
- Adds complexity to the transcription pipeline

For most users with modern hardware and GPU acceleration, the default (disabled) provides the cleanest results. Enable eager processing when latency is a problem that outweighs the small risk of boundary artifacts.

### Verifying It Works

Run voxtype with verbose logging:

```bash
voxtype -vv
```

Then record for 10+ seconds. You should see log messages like:

```
[DEBUG] Spawning eager transcription for chunk 0
[DEBUG] Spawning eager transcription for chunk 1
[DEBUG] Chunk 0 completed: "This is the first part of my recording"
[DEBUG] Chunk 1 completed: "the first part of my recording and here is more"
[DEBUG] Combined eager chunks with deduplication
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

wtype does not work on all Wayland compositors. KDE Plasma and GNOME do not support the virtual keyboard protocol that wtype requires. However, eitype uses the libei/EI protocol which is supported by GNOME and KDE.

| Desktop | wtype | eitype | dotool | ydotool | clipboard | Notes |
|---------|-------|--------|--------|---------|-----------|-------|
| Hyprland, Sway, River | ✓ | * | ✓ | ✓ | wl-copy | wtype recommended (best CJK support) |
| KDE Plasma (Wayland) | ✗ | ✓ | ✓ | ✓ | wl-copy | eitype recommended (native EI protocol) |
| GNOME (Wayland) | ✗ | ✓ | ✓ | ✓ | wl-copy | eitype recommended (native EI protocol) |
| X11 (any) | ✗ | ✗ | ✓ | ✓ | xclip | dotool or ydotool; xclip for clipboard |

\* eitype works on wlroots compositors with libei support.

**KDE Plasma and GNOME users:** Install eitype (recommended) or dotool for type mode to work.

For eitype (recommended for GNOME/KDE):
```bash
cargo install eitype
```

For dotool (recommended for non-US keyboards):
```bash
# Install dotool (check your distribution's package manager)
# User must be in 'input' group for uinput access
sudo usermod -aG input $USER
# Log out and back in for group change to take effect
```

For ydotool:

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
- Overwrites clipboard contents (unless clipboard restoration is enabled)
- No fallback behavior

**Clipboard Restoration**: By default, paste mode overwrites your clipboard with the transcribed text. If you want to preserve your clipboard contents, enable clipboard restoration:

```toml
[output]
mode = "paste"
restore_clipboard = true
```

When enabled, voxtype saves your clipboard content before pasting, then restores it after a brief delay. This works with both text and binary clipboard content (images, files) on Wayland via `wl-paste`, and with text content on X11 via `xclip`. You can also enable it from the command line with `--restore-clipboard` or the `VOXTYPE_RESTORE_CLIPBOARD=true` environment variable.

### Fallback Behavior

Voxtype uses a fallback chain: wtype → eitype → dotool → ydotool → clipboard (wl-copy) → xclip

```toml
[output]
mode = "type"
fallback_to_clipboard = true  # Falls back to clipboard if typing fails
```

On Wayland, wtype is tried first (best CJK support), then eitype (libei protocol, works on GNOME/KDE), then dotool (supports keyboard layouts), then ydotool, then wl-copy (Wayland clipboard). On X11, xclip is available as an additional clipboard fallback.

### Custom Driver Order

You can customize the fallback order or limit which drivers are used:

```toml
[output]
mode = "type"
# Prefer ydotool over wtype, skip dotool entirely
driver_order = ["ydotool", "wtype", "clipboard"]
```

**Available drivers:** `wtype`, `eitype`, `dotool`, `ydotool`, `clipboard` (wl-copy), `xclip` (X11)

**Examples:**

```toml
# X11-only setup (no Wayland)
driver_order = ["ydotool", "xclip"]

# Force ydotool only (no fallback)
driver_order = ["ydotool"]

# GNOME/KDE Wayland (prefer eitype, wtype doesn't work)
driver_order = ["eitype", "dotool", "clipboard"]
```

**CLI override:**

```bash
# Override driver order for this session
voxtype --driver=ydotool,clipboard daemon
```

When `driver_order` is set, `fallback_to_clipboard` is ignored—the driver list explicitly defines what's tried and in what order.

### Typing Options

Additional options for controlling how text is typed:

**Auto-submit (send Enter after typing):**

```toml
[output]
auto_submit = true  # Press Enter after transcription
```

Useful for chat applications or command lines where you want to submit immediately after dictating.

**Shift+Enter for newlines:**

```toml
[output]
shift_enter_newlines = true  # Use Shift+Enter instead of Enter for line breaks
```

Many chat apps (Slack, Discord, Teams) and AI assistants (Cursor) use Enter to send and Shift+Enter for line breaks. Enable this when dictating multi-line messages to prevent premature submission.

**Combining both options:**

```toml
[output]
shift_enter_newlines = true  # Line breaks as Shift+Enter
auto_submit = true           # Final Enter to submit
```

This lets you dictate multi-line messages that are automatically submitted when complete.

**Append text after each transcription:**

```toml
[output]
append_text = " "  # Add a space after each transcription
```

When dictating a paragraph one sentence at a time, each transcription ends without a trailing space. This causes sentences to run together. Setting `append_text = " "` adds a space after each transcription so sentences are properly separated.

This works with all output modes (type, paste, clipboard). The appended text is inserted before `auto_submit` sends Enter, if enabled.

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

## Profiles

Profiles let you define named configurations for different contexts, each with its own post-processing command and output mode. Instead of changing your config file when switching between tasks, use profiles to switch behavior on the fly.

### When to Use Profiles

Profiles are useful when you need different post-processing for different contexts:

- **Slack/Teams**: Casual tone, emoji-friendly formatting
- **Code comments**: Technical terminology, specific formatting
- **Email**: Professional tone, proper salutations
- **Notes**: Bullet points, timestamps

### Defining Profiles

Add profile sections to your `~/.config/voxtype/config.toml`:

```toml
[profiles.slack]
post_process_command = "ollama run llama3.2:1b 'Format this for Slack. Keep it casual and concise:'"

[profiles.code]
post_process_command = "ollama run llama3.2:1b 'Format as a code comment. Be technical and precise:'"
output_mode = "clipboard"  # Override output mode for this profile

[profiles.email]
post_process_command = "ollama run llama3.2:1b 'Format as professional email text:'"
post_process_timeout_ms = 45000  # Allow more time for longer responses
```

### Using Profiles

Specify a profile when starting a recording:

```bash
# Use the slack profile for this recording
voxtype record start --profile slack
voxtype record stop

# Or with toggle mode
voxtype record toggle --profile code
```

**With compositor keybindings (Hyprland example):**

```hyprlang
# Different keybindings for different profiles
bind = SUPER, V, exec, voxtype record start
bindr = SUPER, V, exec, voxtype record stop

bind = SUPER SHIFT, V, exec, voxtype record start --profile slack
bindr = SUPER SHIFT, V, exec, voxtype record stop

bind = SUPER CTRL, V, exec, voxtype record start --profile code
bindr = SUPER CTRL, V, exec, voxtype record stop
```

### Profile Options

Each profile can override these settings:

| Option | Description |
|--------|-------------|
| `post_process_command` | Shell command for text processing (overrides `[output.post_process].command`) |
| `post_process_timeout_ms` | Timeout in milliseconds (overrides `[output.post_process].timeout_ms`) |
| `output_mode` | Output mode: `type`, `clipboard`, or `paste` (overrides `[output].mode`) |

### Profile Behavior

- If a profile doesn't specify an option, the default from your config is used
- Unknown profile names log a warning and fall back to default behavior
- Profiles work with all recording modes (evdev hotkey, compositor keybindings)

### Example: Context-Aware Dictation

```toml
# Default post-processing for general use
[output.post_process]
command = "ollama run llama3.2:1b 'Clean up grammar and punctuation:'"
timeout_ms = 30000

# Slack: casual, concise
[profiles.slack]
post_process_command = "ollama run llama3.2:1b 'Rewrite for Slack. Casual tone, keep it brief:'"

# Code: technical, clipboard output (for pasting into IDE)
[profiles.code]
post_process_command = "ollama run llama3.2:1b 'Format as a code comment in the style of the surrounding code:'"
output_mode = "clipboard"

# Meeting notes: bullet points
[profiles.notes]
post_process_command = "ollama run llama3.2:1b 'Convert to bullet points. Be concise:'"
```

---

## Voice Activity Detection

Voice Activity Detection (VAD) filters silence-only recordings before transcription. This prevents Whisper from hallucinating text when processing silent audio (a known issue where Whisper may output phrases like "Thank you for watching" when given silence).

### Enabling VAD

**Via config file:**
```toml
[vad]
enabled = true
```

**Via command line (single session):**
```bash
voxtype --vad daemon
```

### Configuration Options

| Option | Default | Description |
|--------|---------|-------------|
| `enabled` | `false` | Enable VAD filtering |
| `backend` | `auto` | Detection algorithm: `auto`, `energy`, `whisper` |
| `threshold` | `0.5` | Sensitivity (0.0 = very sensitive, 1.0 = aggressive) |
| `min_speech_duration_ms` | `100` | Minimum speech required (ms) |

### VAD Backends

- **auto** (default): Selects the best backend for your transcription engine
  - Whisper engine → Whisper VAD (requires model download)
  - Parakeet engine → Energy VAD (no model needed)
- **energy**: Fast RMS-based detection. Works with any engine, no model download required.
- **whisper**: Silero VAD via whisper-rs. More accurate but requires downloading the model:
  ```bash
  voxtype setup vad
  ```

### When to Use VAD

VAD is helpful if you:
- Often accidentally trigger recording without speaking
- Experience Whisper hallucinations on silent recordings
- Want faster feedback when recordings contain no speech

### How It Works

Recordings where speech falls below the detection threshold are rejected before transcription, and a "cancelled" feedback sound is played instead of transcribing silence.

---

## Meeting Mode

Meeting mode provides continuous transcription for meetings, with chunked processing, speaker diarization, and export capabilities. Unlike push-to-talk (which transcribes short clips), meeting mode runs continuously and processes audio in chunks for the duration of a meeting.

### Commands

```bash
# Start a new meeting
voxtype meeting start
voxtype meeting start --title "Weekly standup"

# Control a running meeting
voxtype meeting pause
voxtype meeting resume
voxtype meeting stop

# View meeting info
voxtype meeting status          # Current meeting status
voxtype meeting list            # List past meetings
voxtype meeting list --limit 5  # Show last 5 meetings
voxtype meeting show latest     # Show details for most recent meeting
voxtype meeting show <id>       # Show details for a specific meeting

# Export transcripts
voxtype meeting export latest                          # Markdown to stdout
voxtype meeting export latest --format text            # Plain text
voxtype meeting export latest --format json            # JSON
voxtype meeting export <id> --output transcript.md     # Write to file
voxtype meeting export <id> --timestamps --speakers    # Include timestamps and speaker labels
voxtype meeting export <id> --metadata                 # Include metadata header

# Speaker labeling (replace auto-generated IDs with names)
voxtype meeting label latest SPEAKER_00 "Alice"
voxtype meeting label <id> 0 "Bob"

# AI summarization (requires Ollama or remote API)
voxtype meeting summarize latest
voxtype meeting summarize <id> --format json --output summary.json

# Delete a meeting
voxtype meeting delete <id>
voxtype meeting delete <id> --force   # Skip confirmation
```

### Configuration

Meeting mode is disabled by default. Enable it in `~/.config/voxtype/config.toml`:

```toml
[meeting]
enabled = true
chunk_duration_secs = 30         # Audio chunk size for processing
storage_path = "auto"            # Default: ~/.local/share/voxtype/meetings/
retain_audio = false             # Keep raw audio files after transcription
max_duration_mins = 180          # Maximum meeting length (0 = unlimited)

[meeting.audio]
mic_device = "default"           # Microphone (uses audio.device if not set)
loopback_device = "auto"         # Capture remote participants: "auto", "disabled", or device name
echo_cancel = "auto"             # GTCRN neural enhancement + transcript dedup

[meeting.diarization]
enabled = true
backend = "simple"               # "simple", "ml", or "remote"
max_speakers = 10

[meeting.summary]
backend = "disabled"             # "local" (Ollama), "remote", or "disabled"
ollama_url = "http://localhost:11434"
ollama_model = "llama3.2"
timeout_secs = 120
```

### Speaker Labeling

When diarization is enabled, speakers are assigned auto-generated IDs like `SPEAKER_00`, `SPEAKER_01`, etc. Use the `label` command to assign readable names:

```bash
voxtype meeting label latest SPEAKER_00 "Alice"
voxtype meeting label latest 1 "Bob"  # Short form: just the number
```

Labels persist in the meeting data and appear in exports.

### AI Summarization

Meeting summarization uses Ollama (local) or a remote API to generate a summary with key points, action items, and decisions.

```bash
# Requires [meeting.summary] backend set to "local" or "remote"
voxtype meeting summarize latest
voxtype meeting summarize latest --format markdown --output summary.md
```

### Echo Cancellation

When `loopback_device` is enabled, meeting mode captures both your microphone and system audio (remote participants) on separate channels. Without echo cancellation, the remote participants' audio bleeds into your microphone recording and gets transcribed as your speech.

Voxtype uses GTCRN, a lightweight neural speech enhancement model, to clean the mic signal before transcription. The model removes background noise and speaker bleed-through while preserving your voice. A second pass at the transcript level strips any residual echoed phrases.

The GTCRN model (~523 KB) is downloaded automatically the first time you run `voxtype meeting start`. To disable echo cancellation (e.g., if you have PipeWire's `echo-cancel` module configured):

```toml
[meeting.audio]
echo_cancel = "disabled"
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

### With DankMaterialShell (KDE Plasma)

Voxtype includes a QML plugin for [DankMaterialShell](https://github.com/nicman23/dankMaterialShell), an alternative KDE Plasma shell. The widget displays voxtype status with animated icons and supports click-to-toggle recording.

**Automatic installation:**

```bash
voxtype setup dms --install
```

This creates a VoxtypeWidget plugin in `~/.config/DankMaterialShell/plugins/`.

**After installation:**
1. Open DankMaterialShell settings
2. Navigate to the Plugins section
3. Enable the VoxtypeWidget plugin
4. Add the Voxtype widget to your panel

**Widget features:**
- Polls status every 500ms
- Uses Nerd Font icons with color-coded states:
  - Green microphone: idle (ready)
  - Red pulsing dot: recording
  - Yellow spinner: transcribing
  - Gray microphone-slash: stopped/not running
- Click to toggle recording
- Hover for status tooltip

**Requirements:**
- DankMaterialShell installed
- A [Nerd Font](https://www.nerdfonts.com/) for icons
- `state_file = "auto"` in voxtype config (the default)

**Other commands:**

```bash
voxtype setup dms              # Show setup instructions
voxtype setup dms --uninstall  # Remove the widget
voxtype setup dms --qml        # Output raw QML (for scripting)
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
