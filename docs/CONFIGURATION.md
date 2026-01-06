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

### mode

**Type:** String
**Default:** `"push_to_talk"`
**Required:** No

Activation mode for the hotkey.

**Values:**
- `push_to_talk` - Hold hotkey to record, release to transcribe (default)
- `toggle` - Press hotkey once to start recording, press again to stop

**Example:**
```toml
[hotkey]
key = "SCROLLLOCK"
mode = "toggle"  # Press to start, press again to stop
```

### enabled

**Type:** Boolean
**Default:** `true`
**Required:** No

Enable or disable the built-in hotkey detection.

When set to `false`, voxtype will not listen for keyboard events via evdev. Instead, use the `voxtype record` command to control recording from external sources like compositor keybindings.

**When to disable:**
- You prefer using your compositor's native keybindings (Hyprland, Sway)
- You don't want to add your user to the `input` group
- You want to use key combinations not supported by evdev (e.g., Super+V)

**Example:**
```toml
[hotkey]
enabled = false  # Use compositor keybindings instead
```

**Usage with compositor keybindings:**

When `enabled = false`, control recording via CLI:
```bash
voxtype record start   # Start recording
voxtype record stop    # Stop and transcribe
voxtype record toggle  # Toggle recording state
```

Bind these commands in your compositor config:

**Hyprland:**
```hyprlang
bind = SUPER, V, exec, voxtype record start
bindr = SUPER, V, exec, voxtype record stop
```

**Sway:**
```
bindsym $mod+v exec voxtype record start
bindsym --release $mod+v exec voxtype record stop
```

**Note:** For `toggle` mode to work correctly, you must also set `state_file = "auto"` so voxtype can track its current state.

See [User Manual - Compositor Keybindings](USER_MANUAL.md#compositor-keybindings) for complete setup instructions.

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

## [audio.feedback]

Controls audio feedback sounds (beeps when recording starts/stops).

### enabled

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, plays audio cues when recording starts and stops.

### theme

**Type:** String
**Default:** `"default"`
**Required:** No

Sound theme to use for audio feedback.

**Built-in themes:**
- `default` - Clear, pleasant two-tone beeps
- `subtle` - Quiet, unobtrusive clicks
- `mechanical` - Typewriter/keyboard-like sounds

**Custom themes:** Specify a path to a directory containing `start.wav`, `stop.wav`, and `error.wav` files.

### volume

**Type:** Float
**Default:** `0.7`
**Required:** No

Volume level for audio feedback, from `0.0` (silent) to `1.0` (full volume).

**Example:**
```toml
[audio.feedback]
enabled = true
theme = "subtle"
volume = 0.5
```

**Custom theme example:**
```toml
[audio.feedback]
enabled = true
theme = "/home/user/.config/voxtype/sounds"
volume = 0.8
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
| Value | Size | Speed | Accuracy | Notes |
|-------|------|-------|----------|-------|
| `tiny` | 39 MB | Fastest | Good | Multilingual |
| `tiny.en` | 39 MB | Fastest | Better | English only |
| `base` | 142 MB | Fast | Better | Multilingual |
| `base.en` | 142 MB | Fast | Good | English only (default) |
| `small` | 466 MB | Medium | Great | Multilingual |
| `small.en` | 466 MB | Medium | Great | English only |
| `medium` | 1.5 GB | Slow | Excellent | Multilingual |
| `medium.en` | 1.5 GB | Slow | Excellent | English only |
| `large-v3` | 3.1 GB | Slowest | Best | Multilingual |
| `large-v3-turbo` | 1.6 GB | Fast | Excellent | Multilingual, GPU recommended |

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

### on_demand_loading

**Type:** Boolean
**Default:** `false`
**Required:** No

Controls when the Whisper model is loaded into memory.

**Values:**
- `false` (default) - Model is loaded at daemon startup and kept in memory. Provides fastest response times but uses memory/VRAM continuously.
- `true` - Model is loaded when recording starts and unloaded after transcription completes. Saves memory/VRAM but adds a brief delay when starting each recording.

**When to use `on_demand_loading = true`:**
- Running on a memory-constrained system
- Using GPU acceleration and want to free VRAM for other applications
- Running multiple GPU-accelerated applications simultaneously
- Using large models (medium, large-v3) that consume significant memory

**When to keep default (`false`):**
- Want the fastest possible response time
- Have plenty of available memory/VRAM
- Using voxtype frequently throughout the day

**Example:**
```toml
[whisper]
model = "large-v3"
on_demand_loading = true  # Free VRAM when not transcribing
```

**Performance note:** On modern systems with SSDs, model loading typically takes under 1 second for base/small models. Larger models (medium, large-v3) may take 2-3 seconds to load.

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
- `paste` - Copy to clipboard then paste with Ctrl+V (requires wl-copy and ydotool)

**Example:**
```toml
[output]
mode = "paste"
```

**Note about paste mode:**
The `paste` mode is designed to work around non-US keyboard layout issues. Instead of typing characters directly (which assumes US keyboard layout), it copies text to the clipboard and then simulates Ctrl+V to paste it. This works regardless of keyboard layout but requires both wl-copy (for clipboard access) and ydotool (for Ctrl+V simulation).

### fallback_to_clipboard

**Type:** Boolean
**Default:** `true`
**Required:** No

When `true` and `mode = "type"`, falls back to clipboard if typing fails.

**Note:** This setting has no effect when `mode = "paste"` since paste mode doesn't use fallback behavior.

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

### auto_submit

**Type:** Boolean
**Default:** `false`
**Required:** No

Automatically send an Enter keypress after outputting the transcribed text. Useful for chat applications, command lines, or forms where you want to auto-submit after dictation.

**Example:**
```toml
[output]
auto_submit = true  # Press Enter after transcription
```

**Note:** This works with all output modes (`type`, `paste`) but has no effect in `clipboard` mode since clipboard-only output doesn't simulate keypresses.

---

## [output.post_process]

Optional post-processing command that runs after transcription. The command receives
the transcribed text on stdin and should output the processed text on stdout.

**Best use cases:**
- **Translation**: Speak in one language, output in another
- **Domain vocabulary**: Medical, legal, or technical term correction
- **Reformatting**: Convert casual dictation to formal prose
- **Filler word removal**: Remove "um", "uh", "like" that Whisper sometimes keeps
- **Custom workflows**: Multi-output scenarios (e.g., translate to 5 languages, save JSON to file, inject only English at cursor)

**Important notes:**
- Adds 2-5 seconds latency depending on model size
- For most users, Whisper large-v3-turbo with Voxtype's built-in `spoken_punctuation` is sufficient
- LLMs interpret text literally—saying "slash" won't produce "/" (use `spoken_punctuation` for that)
- Use **instruct/chat models**, not reasoning models (they output `<think>` blocks)
- Avoid emojis in LLM output—ydotool cannot type them

### command

**Type:** String
**Default:** None (disabled)
**Required:** Yes (if section is present)

The shell command to execute. Text is piped to stdin, processed text read from stdout.

**Examples:**
```toml
# Use Ollama with a small model for quick cleanup
[output.post_process]
command = "ollama run llama3.2:1b 'Clean up this transcription. Fix grammar and remove filler words. Output only the cleaned text:'"

# Simple filler word removal with sed
[output.post_process]
command = "sed 's/\\bum\\b//g; s/\\buh\\b//g; s/\\blike\\b//g'"

# Custom Python script
[output.post_process]
command = "python3 ~/.config/voxtype/cleanup.py"

# LM Studio API (OpenAI-compatible)
[output.post_process]
command = "~/.config/voxtype/lm-studio-cleanup.sh"
```

### timeout_ms

**Type:** Integer
**Default:** `30000` (30 seconds)
**Required:** No

Maximum time in milliseconds to wait for the command to complete. If exceeded,
the original text is used and a warning is logged.

**Recommendations:**
- Simple shell commands: `5000` (5 seconds)
- Local LLMs: `30000-60000` (30-60 seconds)
- Remote APIs: `30000` or higher

**Example:**
```toml
[output.post_process]
command = "ollama run llama3.2:1b 'Clean up:'"
timeout_ms = 45000  # 45 second timeout for LLM
```

### Error Handling

If the post-processing command fails for any reason (command not found, non-zero
exit, timeout, empty output), Voxtype gracefully falls back to the original
transcribed text and logs a warning. This ensures voice-to-text output is never
blocked by post-processing issues.

**Debugging:**
Run voxtype with `-v` or `-vv` to see detailed logs about post-processing:
```bash
voxtype -vv
```

### Example LM Studio Script

For users running LM Studio locally, here's an example script:

```bash
#!/bin/bash
# ~/.config/voxtype/lm-studio-cleanup.sh

INPUT=$(cat)

curl -s http://localhost:1234/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d "{
    \"messages\": [{
      \"role\": \"system\",
      \"content\": \"Clean up this dictated text. Fix spelling, remove filler words (um, uh), add proper punctuation. Output ONLY the cleaned text - no quotes, no emojis, no explanations.\"
    },{
      \"role\": \"user\",
      \"content\": \"$INPUT\"
    }],
    \"temperature\": 0.1
  }" | jq -r '.choices[0].message.content'
```

Make it executable: `chmod +x ~/.config/voxtype/lm-studio-cleanup.sh`

---

## [text]

Controls text post-processing after transcription.

### spoken_punctuation

**Type:** Boolean
**Default:** `false`
**Required:** No

When `true`, converts spoken punctuation words into their symbol equivalents. Useful for developers and technical writing.

**Supported conversions:**

| Spoken | Symbol |
|--------|--------|
| `period` | `.` |
| `comma` | `,` |
| `question mark` | `?` |
| `exclamation mark` / `exclamation point` | `!` |
| `colon` | `:` |
| `semicolon` | `;` |
| `open paren` / `open parenthesis` | `(` |
| `close paren` / `close parenthesis` | `)` |
| `open bracket` | `[` |
| `close bracket` | `]` |
| `open brace` | `{` |
| `close brace` | `}` |
| `dash` / `hyphen` | `-` |
| `underscore` | `_` |
| `at sign` / `at symbol` | `@` |
| `hash` / `hashtag` | `#` |
| `dollar sign` | `$` |
| `percent` / `percent sign` | `%` |
| `ampersand` | `&` |
| `asterisk` | `*` |
| `plus` / `plus sign` | `+` |
| `equals` / `equals sign` | `=` |
| `slash` / `forward slash` | `/` |
| `backslash` | `\` |
| `pipe` | `\|` |
| `tilde` | `~` |
| `backtick` | `` ` `` |
| `single quote` | `'` |
| `double quote` | `"` |
| `new line` | newline character |
| `new paragraph` | double newline |
| `tab` | tab character |

**Example:**
```toml
[text]
spoken_punctuation = true
```

With this enabled, saying "function open paren close paren" produces `function()`.

### replacements

**Type:** Table (key-value pairs)
**Default:** `{}`
**Required:** No

Custom word replacements applied after transcription. Matching is case-insensitive but preserves word boundaries. Useful for:
- Correcting frequently misheard words
- Expanding abbreviations
- Fixing brand names or technical terms

**Example:**
```toml
[text]
replacements = { "hyperwhisper" = "hyprwhspr", "javascript" = "JavaScript" }
```

If Whisper transcribes "hyperwhisper" (or "HyperWhisper"), it will be replaced with "hyprwhspr".

**Multiple replacements:**
```toml
[text.replacements]
hyperwhisper = "hyprwhspr"
omarchy = "Omarchy"
claude = "Claude"
```

---

## [status]

Controls status display icons for Waybar and other tray integrations.

### icon_theme

**Type:** String
**Default:** `"emoji"`
**Required:** No

The icon theme to use for status display. Determines which icons appear in Waybar and other integrations.

**Built-in themes:**

***Font-based themes*** (require specific fonts installed):

| Theme | idle | recording | transcribing | stopped | Font Required |
|-------|------|-----------|--------------|---------|---------------|
| `emoji` | 🎙️ | 🎤 | ⏳ | (empty) | None (default) |
| `nerd-font` | U+F130 | U+F111 | U+F110 | U+F131 | [Nerd Font](https://www.nerdfonts.com/) |
| `material` | U+F036C | U+F040A | U+F04CE | U+F036D | [Material Design Icons](https://materialdesignicons.com/) |
| `phosphor` | U+E43A | U+E438 | U+E225 | U+E43B | [Phosphor Icons](https://phosphoricons.com/) |
| `codicons` | U+EB51 | U+EBFC | U+EB4C | U+EB52 | [Codicons](https://github.com/microsoft/vscode-codicons) |
| `omarchy` | U+EC12 | U+EC1C | U+EC1C | U+EC12 | Omarchy font |

***Universal themes*** (no special fonts required):

| Theme | idle | recording | transcribing | stopped | Description |
|-------|------|-----------|--------------|---------|-------------|
| `minimal` | ○ | ● | ◐ | × | Simple Unicode circles |
| `dots` | ◯ | ⬤ | ◔ | ◌ | Geometric shapes |
| `arrows` | ▶ | ● | ↻ | ■ | Media player style |
| `text` | [MIC] | [REC] | [...] | [OFF] | Plain text labels |

**Icon codepoint reference:**

| Theme | idle | recording | transcribing | stopped |
|-------|------|-----------|--------------|---------|
| `nerd-font` | microphone | circle | spinner | microphone-slash |
| `material` | mdi-microphone | mdi-record | mdi-sync | mdi-microphone-off |
| `phosphor` | ph-microphone | ph-record | ph-circle-notch | ph-microphone-slash |
| `codicons` | codicon-mic | codicon-record | codicon-sync | codicon-mute |

**Custom theme:** Specify a path to a TOML file containing custom icons.

**Example:**
```toml
[status]
icon_theme = "nerd-font"
```

**Custom theme file format** (`~/.config/voxtype/icons.toml`):
```toml
idle = "🎙️"
recording = "🔴"
transcribing = "⏳"
stopped = ""
```

### [status.icons]

Per-state icon overrides. These take precedence over the theme.

**Type:** Table
**Default:** Empty (use theme icons)
**Required:** No

Override specific icons without creating a full custom theme.

**Example:**
```toml
[status]
icon_theme = "emoji"

[status.icons]
recording = "🔴"  # Override just the recording icon
```

### Waybar Integration

Voxtype outputs an `alt` field in JSON that enables Waybar's `format-icons` feature. You can either:

1. **Use voxtype's icon themes** (simpler):
   ```toml
   [status]
   icon_theme = "nerd-font"
   ```

2. **Override in Waybar config** (more control):
   ```jsonc
   "custom/voxtype": {
       "exec": "voxtype status --follow --format json",
       "return-type": "json",
       "format": "{icon}",
       "format-icons": {
           "idle": "",
           "recording": "",
           "transcribing": "",
           "stopped": ""
       },
       "tooltip": true
   }
   ```

The `alt` field values match state names: `idle`, `recording`, `transcribing`, `stopped`.

See [User Manual - Waybar Integration](USER_MANUAL.md#with-waybar-status-indicator) for complete setup instructions.

---

## state_file

**Type:** String
**Default:** `"auto"`
**Required:** No

Path to a state file for external integrations like Waybar or Polybar. When configured, the daemon writes its current state to this file whenever state changes.

**Values written:**
- `idle` - Ready for input
- `recording` - Push-to-talk active, capturing audio
- `transcribing` - Processing audio through Whisper

**Special values:**
- `"auto"` - Uses `$XDG_RUNTIME_DIR/voxtype/state` (default, recommended)
- `"disabled"` - Turns off state file (also accepts `"none"`, `"off"`, `"false"`)

**Example:**
```toml
# Use automatic location (default)
state_file = "auto"

# Or specify explicit path
state_file = "/tmp/voxtype-state"

# Disable state file
state_file = "disabled"
```

**Usage with `voxtype status`:**

Once enabled, you can monitor the state:

```bash
# One-shot check
voxtype status

# JSON output for scripts
voxtype status --format json

# Continuous monitoring (for Waybar)
voxtype status --follow --format json
```

**Waybar module example:**

```json
"custom/voxtype": {
    "exec": "voxtype status --follow --format json",
    "return-type": "json",
    "format": "{}",
    "tooltip": true
}
```

See [User Manual - Waybar Integration](USER_MANUAL.md#with-waybar-status-indicator) for complete setup instructions.

---

## CLI Overrides

Most configuration options can be overridden via command line:

| Config Option | CLI Flag |
|--------------|----------|
| Config file | `-c`, `--config` |
| hotkey.key | `--hotkey` |
| whisper.model | `--model` |
| output.mode = "clipboard" | `--clipboard` |
| output.mode = "paste" | `--paste` |
| status.icon_theme | `--icon-theme` (status subcommand) |
| Verbosity | `-v`, `-vv`, `-q` |

**Example:**
```bash
voxtype --hotkey PAUSE --model small.en --clipboard
voxtype status --format json --icon-theme nerd-font
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

### GPU with VRAM Optimization

```toml
[whisper]
model = "large-v3-turbo"
on_demand_loading = true  # Free VRAM when not transcribing

[audio.feedback]
enabled = true  # Helpful feedback since model loading adds brief delay
theme = "default"
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

### Developer / Programmer

```toml
[whisper]
model = "base.en"

[text]
# Say "period" to get ".", "open paren" to get "(", etc.
spoken_punctuation = true

# Fix common misheard technical terms
[text.replacements]
javascript = "JavaScript"
typescript = "TypeScript"
python = "Python"
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

### Compositor Keybindings (Hyprland/Sway)

```toml
[hotkey]
enabled = false  # Disable built-in hotkey, use compositor keybindings

# Required for toggle mode
state_file = "auto"

[whisper]
model = "base.en"

[audio.feedback]
enabled = true  # Audio cues helpful when using external triggers
```

Then configure your compositor:

**Hyprland** (`~/.config/hypr/hyprland.conf`):
```hyprlang
bind = SUPER, V, exec, voxtype record start
bindr = SUPER, V, exec, voxtype record stop
```

**Sway** (`~/.config/sway/config`):
```
bindsym $mod+v exec voxtype record start
bindsym --release $mod+v exec voxtype record stop
```
