# Waybar Integration Guide

This guide shows how to add a Voxtype status indicator to Waybar, so you can see at a glance when push-to-talk is active.

## What It Does

The Waybar module displays an icon that changes based on Voxtype's current state:

| State | Icon | Meaning |
|-------|------|---------|
| Idle | 🎙️ | Ready to record |
| Recording | 🎤 | Hotkey held, capturing audio |
| Transcribing | ⏳ | Processing speech to text |

This is useful if you:
- Prefer visual feedback over desktop notifications
- Want to confirm Voxtype is running
- Need to see when transcription is in progress

## Prerequisites

- Voxtype installed and working
- Waybar configured and running

## Setup Steps

### Step 1: Enable the State File

Add this line to your Voxtype config (`~/.config/voxtype/config.toml`):

```toml
state_file = "auto"
```

This tells Voxtype to write its current state to `$XDG_RUNTIME_DIR/voxtype/state` whenever the state changes. The `voxtype status` command reads this file.

Restart Voxtype after making this change:

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
   Should show `idle`, `recording`, or `transcribing`.

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
