# Omarchy Integration

This directory contains files for integrating Voxtype into [Omarchy](https://github.com/basecamp/omarchy).

## Files

- **migration.sh** - Migration script for existing Omarchy users
- **waybar-module.jsonc** - Waybar module configuration
- **waybar-style.css** - Waybar CSS styling for voxtype states

## Submitting to Omarchy

To add Voxtype to Omarchy, submit a PR with the following changes:

### 1. Add to package list

Edit `install/omarchy-base.packages` and add `voxtype` and `wtype` in alphabetical order:

```
typora
tzupdate
ufw
voxtype    # <-- add here
waybar
...
wtype      # <-- add here (for keyboard simulation)
```

### 2. Add migration script

Copy `migration.sh` to `migrations/` with a timestamp filename:

```bash
# Generate timestamp: date +%s
cp migration.sh migrations/$(date +%s).sh
```

### 3. Optional: Add Waybar integration

The migration script automatically:
- Enables `state_file = "auto"` in voxtype config
- Adds the voxtype module to Waybar
- Starts the systemd user service

## Usage

After installation, users can:
- Hold **SCROLLLOCK** to record voice
- Release to transcribe and type the text
- See status in Waybar (microphone icon)

Run `voxtype setup --download` to download a whisper model if needed.

## Dependencies

Voxtype requires packages already in Omarchy:
- `wl-clipboard` - for clipboard support
- `libnotify` - for notifications

Optional (for keyboard simulation):
- `wtype` - Wayland typing (recommended)
- `ydotool` - X11/TTY fallback
