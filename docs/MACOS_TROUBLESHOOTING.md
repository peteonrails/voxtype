# Voxtype macOS Troubleshooting Checklist

Use this checklist to debug issues and resume work after context resets.

## Quick Status Check

```bash
# Check if daemon is running
ps aux | grep "[v]oxtype daemon"

# Check daemon status
/Applications/Voxtype.app/Contents/MacOS/voxtype status

# Check state file
cat /tmp/voxtype/state

# Check config
cat "$HOME/Library/Application Support/voxtype/config.toml" | head -50
```

## Common Issues

### 1. "Another voxtype instance is already running"

**Cause:** Stale lock file or launchd keeps restarting daemon.

**Fix:**
```bash
# Stop launchd service
launchctl stop io.voxtype.daemon
launchctl unload ~/Library/LaunchAgents/io.voxtype.daemon.plist

# Kill all instances
pkill -9 voxtype

# Clean up lock files
rm -rf /tmp/voxtype

# Start fresh
/Applications/Voxtype.app/Contents/MacOS/voxtype daemon &
```

### 2. Hotkey Not Working

**Possible causes:**
- Wrong key configured
- Input Monitoring permission not granted
- Daemon not running

**Debug:**
```bash
# Check current hotkey
grep "^key" "$HOME/Library/Application Support/voxtype/config.toml"

# Run daemon with verbose output
pkill voxtype
rm -rf /tmp/voxtype
/Applications/Voxtype.app/Contents/MacOS/voxtype -vv daemon
```

**Fix permissions:**
- System Settings → Privacy & Security → Input Monitoring
- Add `/Applications/Voxtype.app` or the Terminal app

### 3. "use_default" Dialog Appears

**Cause:** `mac-notification-sys` crate looking for bundle identifier.

**Fix:** Use osascript for notifications (already fixed in current code):
```rust
// In src/notification.rs, send_macos_native should use osascript
fn send_macos_native(title: &str, body: &str) {
    send_macos_osascript_sync(title, body);
}
```

### 4. Config Changes Not Taking Effect

**Cause:** Daemon needs restart after config changes.

**Fix:**
```bash
pkill voxtype
rm -rf /tmp/voxtype
/Applications/Voxtype.app/Contents/MacOS/voxtype daemon &
```

### 5. Settings App Config Updates Corrupting File

**Cause:** Old ConfigManager did global regex replace instead of section-aware updates.

**Fix:** ConfigManager now does line-by-line, section-aware updates. If config is corrupted, reset:
```bash
# Backup current config
cp "$HOME/Library/Application Support/voxtype/config.toml" ~/config.toml.bak

# Regenerate default config
/Applications/Voxtype.app/Contents/MacOS/voxtype setup --quiet
```

### 6. Audio Feedback "use_default" Dialog

**Cause:** rodio/cpal audio output stream initialization on macOS.

**Fix:** Disable audio feedback in config:
```toml
[audio.feedback]
enabled = false
```

### 7. Status Shows "stopped" But Daemon Is Running

**Cause:** Multiple daemon processes or state file mismatch.

**Fix:**
```bash
# Clean slate
pkill -9 voxtype
rm -rf /tmp/voxtype
sleep 2
/Applications/Voxtype.app/Contents/MacOS/voxtype daemon &
sleep 3
/Applications/Voxtype.app/Contents/MacOS/voxtype status
```

## Building and Installing

### Rebuild Rust Binary
```bash
cd /Users/pete/workspace/voxtype
cargo build --release
cp target/release/voxtype /Applications/Voxtype.app/Contents/MacOS/
```

### Rebuild Swift Apps
```bash
# Menubar
cd macos/VoxtypeMenubar
./build-app.sh
cp -r .build/VoxtypeMenubar.app /Applications/

# Settings
cd macos/VoxtypeSetup
./build-app.sh
cp -r .build/VoxtypeSetup.app /Applications/
```

### Restart Apps After Rebuild
```bash
pkill -x VoxtypeMenubar
pkill -x VoxtypeSetup
pkill -x voxtype
rm -rf /tmp/voxtype
open /Applications/VoxtypeMenubar.app
/Applications/Voxtype.app/Contents/MacOS/voxtype daemon &
```

## Current Known Issues (as of session)

1. **Notification icon** - Daemon uses osascript so notifications show Script Editor icon, not Voxtype icon. Menubar app notifications show correct icon.

2. **Audio feedback disabled** - Causes "use_default" dialog on macOS.

3. **Hotkey restart required** - Config changes to hotkey require daemon restart. Settings app now has "Restart Now" button.

## File Locations Quick Reference

| Item | Path |
|------|------|
| Config | `~/Library/Application Support/voxtype/config.toml` |
| Models | `~/Library/Application Support/voxtype/models/` |
| State | `/tmp/voxtype/state` |
| Lock | `/tmp/voxtype/voxtype.lock` |
| PID | `/tmp/voxtype/pid` |

## Verification Steps

After fixing an issue, verify:

1. `voxtype status` returns `idle`
2. Pressing hotkey (default: Right Option) starts recording (state becomes `recording`)
3. Releasing hotkey transcribes and types text
4. Notification appears after transcription
