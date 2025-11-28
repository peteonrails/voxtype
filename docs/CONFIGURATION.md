# Voxtype Configuration Reference

Complete reference for all configuration options in Voxtype.

## Configuration File Location

Voxtype looks for configuration in the following locations (in order):

1. Path specified via `-c` / `--config` flag
2. `~/.config/voxtype/config.toml` (XDG config directory)
3. `/etc/voxtype/config.toml` (system-wide default)
4. Built-in defaults

## Configuration Sections

---

## [hotkey]

Controls which key triggers push-to-talk recording.

### key

**Type:** String
**Default:** `"SCROLLLOCK"`
**Required:** No

The main key to hold for recording. Must be a valid Linux evdev key name.

**Common values:**
- `SCROLLLOCK` - Scroll Lock key (recommended)
- `PAUSE` - Pause/Break key
- `RIGHTALT` - Right Alt key
- `F13` through `F24` - Extended function keys
- `INSERT` - Insert key
- `HOME` - Home key
- `END` - End key
- `PAGEUP` - Page Up key
- `PAGEDOWN` - Page Down key
- `DELETE` - Delete key

**Example:**
```toml
[hotkey]
key = "PAUSE"
```

**Finding key names:**
```bash
sudo evtest
# Select keyboard, press desired key, note KEY_XXXX name
```

### modifiers

**Type:** Array of strings
**Default:** `[]`
**Required:** No

Additional modifier keys that must be held along with the main key.

**Valid modifiers:**
- `LEFTCTRL`, `RIGHTCTRL`
- `LEFTALT`, `RIGHTALT`
- `LEFTSHIFT`, `RIGHTSHIFT`
- `LEFTMETA`, `RIGHTMETA`

**Example:**
```toml
[hotkey]
key = "SCROLLLOCK"
modifiers = ["LEFTCTRL"]  # Requires Ctrl+ScrollLock
```

---

## [audio]

Controls audio capture settings.

### device

**Type:** String
**Default:** `"default"`
**Required:** No

The audio input device to use. Use `"default"` for the system default microphone.

**Finding device names:**
```bash
pactl list sources short
```

**Example:**
```toml
[audio]
device = "alsa_input.usb-Blue_Microphones_Yeti-00.analog-stereo"
```

### sample_rate

**Type:** Integer
**Default:** `16000`
**Required:** No

Audio sample rate in Hz. Whisper expects 16000 Hz; other rates will be resampled.

**Recommended:** Keep at `16000` unless your hardware requires otherwise.

```toml
[audio]
sample_rate = 16000
```

### max_duration_secs

**Type:** Integer
**Default:** `60`
**Required:** No

Maximum recording duration in seconds. Recording automatically stops after this limit as a safety measure.

**Example:**
```toml
[audio]
max_duration_secs = 120  # Allow 2-minute recordings
```

---

## [whisper]

Controls the Whisper speech-to-text engine.

### model

**Type:** String
**Default:** `"base.en"`
**Required:** No

Which Whisper model to use for transcription.

**Model names:**
| Value | Size | Speed | Accuracy |
|-------|------|-------|----------|
| `tiny` | 39 MB | Fastest | Good |
| `tiny.en` | 39 MB | Fastest | Better (English) |
| `base` | 142 MB | Fast | Better |
| `base.en` | 142 MB | Fast | Good (English) |
| `small` | 466 MB | Medium | Great |
| `small.en` | 466 MB | Medium | Great (English) |
| `medium` | 1.5 GB | Slow | Excellent |
| `medium.en` | 1.5 GB | Slow | Excellent (English) |
| `large-v3` | 3.1 GB | Slowest | Best |

**Custom model path:**
```toml
[whisper]
model = "/home/user/models/custom-whisper.bin"
```

### language

**Type:** String
**Default:** `"en"`
**Required:** No

Language code for transcription.

**Common values:**
- `en` - English
- `auto` - Auto-detect language
- `es` - Spanish
- `fr` - French
- `de` - German
- `ja` - Japanese
- `zh` - Chinese

**Example:**
```toml
[whisper]
language = "auto"  # Auto-detect spoken language
```

### translate

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, translates non-English speech to English.

**Example:**
```toml
[whisper]
language = "auto"
translate = true  # Translate everything to English
```

### threads

**Type:** Integer
**Default:** Auto-detected
**Required:** No

Number of CPU threads for Whisper inference. If omitted, automatically detects optimal thread count.

**Example:**
```toml
[whisper]
threads = 4  # Limit to 4 threads
```

**Tip:** For best performance, set to your physical core count (not hyperthreads).

---

## [output]

Controls how transcribed text is delivered.

### mode

**Type:** String
**Default:** `"type"`
**Required:** No

Primary output method.

**Values:**
- `type` - Simulate keyboard input at cursor position (requires ydotool)
- `clipboard` - Copy text to clipboard (requires wl-copy)

**Example:**
```toml
[output]
mode = "clipboard"
```

### fallback_to_clipboard

**Type:** Boolean
**Default:** `true`
**Required:** No

When `true` and `mode = "type"`, falls back to clipboard if typing fails.

**Example:**
```toml
[output]
mode = "type"
fallback_to_clipboard = true  # Use clipboard if ydotool fails
```

---

## [output.notification]

Controls desktop notifications at various stages.

### on_recording_start

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, shows a notification when recording starts (hotkey pressed).

### on_recording_stop

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, shows a notification when recording stops (transcription begins).

### on_transcription

**Type:** Boolean
**Default:** `true`
**Required:** No

When `true`, shows a notification with the transcribed text after transcription completes.

**Requires:** `notify-send` (libnotify)

**Example:**
```toml
[output.notification]
on_recording_start = true   # Notify when PTT activates
on_recording_stop = true    # Notify when transcribing
on_transcription = true     # Show transcribed text
```

### type_delay_ms

**Type:** Integer
**Default:** `0`
**Required:** No

Delay in milliseconds between each typed character. Increase if characters are being dropped.

**Example:**
```toml
[output]
type_delay_ms = 10  # 10ms delay between characters
```

---

## CLI Overrides

Most configuration options can be overridden via command line:

| Config Option | CLI Flag |
|--------------|----------|
| Config file | `-c`, `--config` |
| hotkey.key | `--hotkey` |
| whisper.model | `--model` |
| output.mode = "clipboard" | `--clipboard` |
| Verbosity | `-v`, `-vv`, `-q` |

**Example:**
```bash
voxtype --hotkey PAUSE --model small.en --clipboard
```

---

## Environment Variables

### RUST_LOG

Controls log verbosity via the tracing crate.

```bash
RUST_LOG=debug voxtype
RUST_LOG=voxtype=trace voxtype
```

### XDG_CONFIG_HOME

Overrides the config directory location (default: `~/.config`).

```bash
XDG_CONFIG_HOME=/custom/config voxtype
# Looks for: /custom/config/voxtype/config.toml
```

### XDG_DATA_HOME

Overrides the data directory location (default: `~/.local/share`).

```bash
XDG_DATA_HOME=/custom/data voxtype
# Models stored in: /custom/data/voxtype/models/
```

---

## Example Configurations

### Minimal Configuration

```toml
[whisper]
model = "base.en"
```

### High Accuracy

```toml
[whisper]
model = "medium.en"
threads = 8

[output.notification]
on_transcription = true
```

### Low Latency

```toml
[whisper]
model = "tiny.en"

[audio]
max_duration_secs = 30

[output]
type_delay_ms = 0
```

### Multilingual

```toml
[whisper]
model = "large-v3"
language = "auto"
translate = true  # Translate to English
```

### Custom Hotkey

```toml
[hotkey]
key = "F13"
modifiers = ["LEFTCTRL", "LEFTSHIFT"]

[output]
mode = "clipboard"

[output.notification]
on_transcription = true
```

### Server/Headless

```toml
[hotkey]
key = "F24"

[output]
mode = "clipboard"

[output.notification]
on_recording_start = false
on_recording_stop = false
on_transcription = false  # No desktop notifications
```
