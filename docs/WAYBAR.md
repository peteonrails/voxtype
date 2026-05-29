# Waybar Integration Guide

This guide shows how to add a Voxtype status indicator to Waybar, so you can see at a glance when push-to-talk is active.

## What It Does

The Waybar module displays an icon that changes based on Voxtype's current state:

| State | Default Icon | Meaning |
|-------|--------------|---------|
| Idle | 🎙️ | Ready to record |
| Recording | 🎤 | Hotkey held, capturing audio |
| Transcribing | ⏳ | Processing speech to text |
| Stopped | (empty) | Voxtype not running |

Icons are fully customizable—choose from 10 built-in themes (emoji, Nerd Font, Material Design, etc.) or define your own. See [Customizing Icons](#customizing-icons) below.

This is useful if you:
- Prefer visual feedback over desktop notifications
- Want to confirm Voxtype is running
- Need to see when transcription is in progress

## Prerequisites

- Voxtype installed and working
- Waybar configured and running

## Quick Setup

Run the setup command to get ready-to-use config snippets:

```bash
voxtype setup waybar
```

This outputs the Waybar module JSON and CSS styling, ready to copy into your config files.

## Manual Setup Steps

### Step 1: Verify State File is Enabled

The state file is enabled by default in Voxtype. Verify it's set in your config (`~/.config/voxtype/config.toml`):

```toml
state_file = "auto"  # This is the default
```

This tells Voxtype to write its current state to `$XDG_RUNTIME_DIR/voxtype/state` whenever the state changes. The `voxtype status` command reads this file.

If you've previously disabled it, re-enable it and restart Voxtype:

```bash
systemctl --user restart voxtype
# Or if running manually, stop and restart voxtype
```

### Step 2: Add Waybar Module

Edit your Waybar config file. This is typically one of:
- `~/.config/waybar/config`
- `~/.config/waybar/config.jsonc`

Add the custom module:

```json
"custom/voxtype": {
    "exec": "voxtype status --follow --format json",
    "return-type": "json",
    "format": "{}",
    "tooltip": true
}
```

Then add `"custom/voxtype"` to one of your module lists. For example:

```json
"modules-right": ["custom/voxtype", "pulseaudio", "clock"]
```

### Step 3: Restart Waybar

```bash
# If using systemd
systemctl --user restart waybar

# Or kill and restart
killall waybar && waybar &
```

You should now see a microphone icon in your bar.

## Extended Status Info

Add the `--extended` flag to include model, device, and backend information in the JSON output:

```json
"custom/voxtype": {
    "exec": "voxtype status --follow --format json --extended",
    "return-type": "json",
    "format": "{}",
    "tooltip": true
}
```

With `--extended`, the JSON output includes additional fields:

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

The tooltip will show the model name, audio device, and compute backend (CPU with AVX level, or GPU with Vulkan).

You can use these fields in your Waybar format string:

```json
"custom/voxtype": {
    "exec": "voxtype status --follow --format json --extended",
    "return-type": "json",
    "format": "{} [{model}]",
    "tooltip": true
}
```

This displays the icon followed by the model name, e.g., "🎙️ [base.en]".

## Optional: Custom Styling

Add these styles to your Waybar stylesheet (`~/.config/waybar/style.css`) to make the recording state more visible:

```css
#custom-voxtype {
    padding: 0 8px;
    font-size: 14px;
}

#custom-voxtype.recording {
    color: #ff5555;
    animation: pulse 1s infinite;
}

#custom-voxtype.transcribing {
    color: #f1fa8c;
}

@keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
}
```

This makes the icon:
- Turn red and pulse when recording
- Turn yellow when transcribing

## Customizing Icons

Voxtype supports multiple icon themes and custom icons. You can customize icons either through Voxtype's config or directly in your Waybar config.

### Option 1: Use Voxtype Icon Themes (Simplest)

Add to your `~/.config/voxtype/config.toml`:

```toml
[status]
icon_theme = "nerd-font"
```

Or use the `--icon-theme` CLI flag (useful for testing or Waybar config):

```bash
voxtype status --format json --icon-theme nerd-font
```

**Available themes:**

| Theme | idle | recording | transcribing | stopped | Requirements |
|-------|------|-----------|--------------|---------|--------------|
| `emoji` | 🎙️ | 🎤 | ⏳ | (empty) | None (default) |
| `nerd-font` | U+F130 | U+F111 | U+F110 | U+F131 | Nerd Font |
| `material` | U+F036C | U+F040A | U+F04CE | U+F036D | Material Design Icons |
| `phosphor` | U+E43A | U+E438 | U+E225 | U+E43B | Phosphor Icons |
| `codicons` | U+EB51 | U+EBFC | U+EB4C | U+EB52 | VS Code Codicons |
| `omarchy` | U+EC12 | U+EC1C | U+EC1C | U+EC12 | Omarchy font |
| `minimal` | ○ | ● | ◐ | × | None |
| `dots` | ◯ | ⬤ | ◔ | ◌ | None |
| `arrows` | ▶ | ● | ↻ | ■ | None |
| `text` | [MIC] | [REC] | [...] | [OFF] | None |

After changing the theme, restart the Voxtype daemon:

```bash
systemctl --user restart voxtype
```

### Option 2: Use Waybar's format-icons (More Control)

Voxtype outputs an `alt` field in JSON that enables Waybar's native `format-icons` feature. This lets you define icons directly in your Waybar config:

```json
"custom/voxtype": {
    "exec": "voxtype status --follow --format json",
    "return-type": "json",
    "format": "{icon}",
    "format-icons": {
        "idle": "",
        "recording": "",
        "transcribing": "",
        "outputting": "",
        "stopped": ""
    },
    "tooltip": true
}
```

The `alt` field values are: `idle`, `recording`, `transcribing`, `outputting`, `stopped`.

**Nerd Font example:**
```json
"format-icons": {
    "idle": "\uf130",
    "recording": "\uf111",
    "transcribing": "\uf110",
    "stopped": "\uf131"
}
```

**Material Design Icons example:**
```json
"format-icons": {
    "idle": "\U000f036c",
    "recording": "\U000f040a",
    "transcribing": "\U000f04ce",
    "stopped": "\U000f036d"
}
```

### Option 3: Override Specific Icons

You can override individual icons without changing the whole theme:

```toml
[status]
icon_theme = "emoji"

[status.icons]
recording = "🔴"  # Just change the recording icon
```

### Option 4: Custom Theme File

Create a custom theme file (e.g., `~/.config/voxtype/icons.toml`):

```toml
idle = "🟢"
recording = "🔴"
transcribing = "🟡"
stopped = "⚪"
```

Then reference it in your config:

```toml
[status]
icon_theme = "~/.config/voxtype/icons.toml"
```

### Icon Reference Table

| Theme | Icon Name | Codepoint | Description |
|-------|-----------|-----------|-------------|
| `nerd-font` | nf-fa-microphone | U+F130 | Microphone (idle) |
| `nerd-font` | nf-fa-circle | U+F111 | Filled circle (recording) |
| `nerd-font` | nf-fa-spinner | U+F110 | Spinner (transcribing) |
| `nerd-font` | nf-fa-microphone-slash | U+F131 | Muted mic (stopped) |
| `material` | mdi-microphone | U+F036C | Microphone (idle) |
| `material` | mdi-record | U+F040A | Record dot (recording) |
| `material` | mdi-sync | U+F04CE | Sync spinner (transcribing) |
| `material` | mdi-microphone-off | U+F036D | Muted mic (stopped) |

## Alternative: Polling Mode

The `--follow` flag keeps a persistent connection and updates instantly. If you prefer polling instead (slightly less CPU when idle, but 1-second delay on updates):

```json
"custom/voxtype": {
    "exec": "voxtype status --format json",
    "return-type": "json",
    "format": "{}",
    "interval": 1,
    "tooltip": true
}
```

## Troubleshooting

### Module shows nothing

1. Verify Voxtype is running:
   ```bash
   systemctl --user status voxtype
   ```

2. Verify state file exists:
   ```bash
   cat $XDG_RUNTIME_DIR/voxtype/state
   ```
   Should show `idle`, `recording`, `transcribing`, or `outputting`.

3. Test the status command manually:
   ```bash
   voxtype status --format json
   ```
   Should output JSON like `{"text":"🎙️","tooltip":"Voxtype: idle","class":"idle"}`.

### Module shows error or wrong icon

Make sure `state_file = "auto"` is set in your config and Voxtype was restarted after adding it.

### Recording state not updating

The `--follow` mode uses inotify to watch for file changes. If this isn't working, try the polling approach with `"interval": 1`.

## Polybar Alternative

If you use Polybar instead of Waybar:

```ini
[module/voxtype]
type = custom/script
exec = voxtype status --format text
interval = 1
format = <label>
label = %output%
```

## Complete Example

Here's a complete Waybar config snippet:

```json
{
    "layer": "top",
    "position": "top",
    "modules-left": ["sway/workspaces"],
    "modules-center": ["sway/window"],
    "modules-right": ["custom/voxtype", "pulseaudio", "clock"],

    "custom/voxtype": {
        "exec": "voxtype status --follow --format json",
        "return-type": "json",
        "format": "{}",
        "tooltip": true
    },

    "clock": {
        "format": "{:%H:%M}"
    },

    "pulseaudio": {
        "format": "{volume}%"
    }
}
```

## See Also

- [Configuration Reference](CONFIGURATION.md#state_file) - Full `state_file` documentation
- [User Manual](USER_MANUAL.md#with-waybar-status-indicator) - Integration examples
