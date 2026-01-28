# Voxtype macOS Installation Guide

Voxtype is a push-to-talk voice-to-text tool that uses Whisper for fast, local speech recognition.

## Requirements

- macOS 11 (Big Sur) or later
- Apple Silicon (M1/M2/M3) or Intel Mac
- Accessibility permissions for global hotkey detection

## Installation

### Option 1: Homebrew (Recommended)

```bash
# Add the tap
brew tap peteonrails/voxtype

# Install
brew install --cask voxtype
```

### Option 2: Direct Download

1. Download the latest DMG from [GitHub Releases](https://github.com/peteonrails/voxtype/releases)
2. Open the DMG and drag `voxtype` to `/usr/local/bin`

```bash
# Or install via command line
curl -L https://github.com/peteonrails/voxtype/releases/latest/download/voxtype-macos-universal.dmg -o voxtype.dmg
hdiutil attach voxtype.dmg
cp /Volumes/Voxtype/voxtype /usr/local/bin/
hdiutil detach /Volumes/Voxtype
rm voxtype.dmg
```

### Option 3: Build from Source

```bash
git clone https://github.com/peteonrails/voxtype.git
cd voxtype
cargo build --release --features gpu-metal
cp target/release/voxtype /usr/local/bin/
```

## Setup

### 1. Grant Accessibility Permissions

Voxtype needs Accessibility permissions to detect global hotkeys.

1. Open **System Preferences** (or System Settings on macOS 13+)
2. Go to **Privacy & Security** > **Accessibility**
3. Click the lock icon to make changes
4. Add and enable `voxtype` (or Terminal if running from terminal)

### 2. Download a Whisper Model

```bash
# Interactive model selection
voxtype setup model

# Or download a specific model
voxtype setup --download --model base.en
```

Available models:
- `tiny.en` / `tiny` - Fastest, lowest accuracy (39 MB)
- `base.en` / `base` - Good balance, recommended (142 MB)
- `small.en` / `small` - Better accuracy (466 MB)
- `medium.en` / `medium` - High accuracy (1.5 GB)
- `large-v3` - Best accuracy (3.1 GB) **Pro only**
- `large-v3-turbo` - Fast + accurate (1.6 GB) **Pro only**

### 3. Configure Hotkey

Edit `~/.config/voxtype/config.toml`:

```toml
[hotkey]
key = "F13"  # Or any key: SCROLLLOCK, PAUSE, etc.
modifiers = []  # Optional: ["CTRL"], ["CMD"], etc.
mode = "push_to_talk"  # Or "toggle"
```

### 4. Start Voxtype

**Manual start:**
```bash
voxtype daemon
```

**Auto-start on login (recommended):**
```bash
voxtype setup launchd
```

## Usage

1. Hold the hotkey (default: F13) to record
2. Speak your text
3. Release the hotkey to transcribe
4. Text is typed into the active window

### Quick Reference

```bash
voxtype daemon              # Start the daemon
voxtype status              # Check if daemon is running
voxtype setup model         # Download/switch models
voxtype setup launchd       # Install as LaunchAgent
voxtype check-update        # Check for updates
voxtype --help              # Show all options
```

## Troubleshooting

### "Accessibility permissions required"

1. Check System Preferences > Privacy & Security > Accessibility
2. Ensure voxtype (or Terminal) is added and enabled
3. Try removing and re-adding the app

### Hotkey not working

1. Check that the hotkey isn't used by another app
2. Try a different key (F13, SCROLLLOCK, PAUSE are good choices)
3. Ensure Accessibility permissions are granted

### "Model not found"

```bash
voxtype setup model  # Download a model
```

### Daemon not starting

```bash
# Check logs
tail -f ~/Library/Logs/voxtype/stderr.log

# Verify permissions
ls -la /usr/local/bin/voxtype
```

### LaunchAgent issues

```bash
# Check status
launchctl list | grep voxtype

# View logs
tail -f ~/Library/Logs/voxtype/stdout.log

# Reload service
launchctl unload ~/Library/LaunchAgents/io.voxtype.daemon.plist
launchctl load ~/Library/LaunchAgents/io.voxtype.daemon.plist
```

## Uninstalling

### Homebrew

```bash
brew uninstall --cask voxtype
```

### Manual

```bash
# Stop and remove LaunchAgent
launchctl unload ~/Library/LaunchAgents/io.voxtype.daemon.plist
rm ~/Library/LaunchAgents/io.voxtype.daemon.plist

# Remove binary
rm /usr/local/bin/voxtype

# Remove config and data (optional)
rm -rf ~/.config/voxtype
rm -rf ~/.local/share/voxtype
rm -rf ~/Library/Logs/voxtype
```

## Configuration

Config file: `~/.config/voxtype/config.toml`

```toml
[hotkey]
key = "F13"
modifiers = []
mode = "push_to_talk"  # or "toggle"

[audio]
device = "default"
sample_rate = 16000
max_duration_secs = 60

[whisper]
model = "base.en"
language = "en"

[output]
mode = "type"  # or "clipboard", "paste"
```

See [CONFIGURATION.md](CONFIGURATION.md) for full options.

## Getting Help

- GitHub Issues: https://github.com/peteonrails/voxtype/issues
- Documentation: https://voxtype.io/docs
- Email: support@voxtype.io
