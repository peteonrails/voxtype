# Voxtype macOS Installation Guide

Voxtype is a push-to-talk voice-to-text tool with fast, local speech recognition using Parakeet or Whisper.

> **Note:** macOS support is in beta. The binaries are currently unsigned, which requires a few extra steps during installation. Once we have signed and notarized binaries, this process will be simpler.

## Requirements

- macOS 13 (Ventura) or later
- Apple Silicon (M1/M2/M3/M4)
- Microphone access
- Input Monitoring permission (for global hotkey)

## Installation via Homebrew (Recommended)

```bash
# Add the tap
brew tap peteonrails/voxtype

# Install
brew install --cask peteonrails/voxtype/voxtype
```

The Cask automatically:
- Installs Voxtype.app to /Applications
- Creates CLI symlink (`voxtype` command)
- Sets up auto-start at login
- Starts the daemon

### First-Time Security Setup

Because the app is unsigned, macOS will block it on first run. This is a one-time setup:

1. **Allow the app to run:**
   - Open **System Settings** > **Privacy & Security**
   - Scroll down to find "Voxtype.app was blocked"
   - Click **Open Anyway**

2. **Grant Input Monitoring permission (required for hotkey):**
   - Open **System Settings** > **Privacy & Security** > **Input Monitoring**
   - Enable **Voxtype**

3. **Restart the daemon** to pick up permissions:
   ```bash
   launchctl stop io.voxtype.daemon
   launchctl start io.voxtype.daemon
   ```

### Download a Speech Model

```bash
# Recommended: Parakeet (fast, accurate)
voxtype setup --download --model parakeet-tdt-0.6b-v3-int8

# Or use Whisper
voxtype setup --download --model base.en
```

## Usage

Hold **Right Option** (⌥) to record, release to transcribe. Text is typed into the active application.

### Quick Commands

```bash
voxtype status              # Check daemon status
voxtype record start        # Start recording manually
voxtype record stop         # Stop and transcribe
voxtype setup check         # Verify setup
voxtype menubar             # Show menu bar status icon
```

### Menu Bar Icon (Optional)

For a status icon showing recording state:

```bash
voxtype menubar
```

This shows:
- 🎙️ Ready (idle)
- 🔴 Recording
- ⏳ Transcribing

## Configuration

Config file: `~/Library/Application Support/voxtype/config.toml`

```toml
# Transcription engine
engine = "parakeet"  # or "whisper"

[hotkey]
key = "RIGHTALT"     # Right Option key
mode = "push_to_talk"  # or "toggle"

[parakeet]
model = "parakeet-tdt-0.6b-v3-int8"

[whisper]
model = "base.en"

[output]
mode = "type"  # or "clipboard", "paste"
```

See [CONFIGURATION.md](CONFIGURATION.md) for full options.

## Troubleshooting

### Hotkey not working

1. Verify Input Monitoring permission is granted:
   - System Settings > Privacy & Security > Input Monitoring
   - Voxtype must be enabled

2. Restart the daemon:
   ```bash
   launchctl stop io.voxtype.daemon
   launchctl start io.voxtype.daemon
   ```

3. Check daemon logs:
   ```bash
   tail -f ~/Library/Logs/voxtype/stdout.log
   ```

### "Voxtype was blocked" / "damaged app"

This happens because the app is unsigned. Go to System Settings > Privacy & Security and click "Open Anyway".

### Model not found

```bash
voxtype setup --download --model parakeet-tdt-0.6b-v3-int8
```

### Daemon not starting

```bash
# Check status
launchctl list | grep voxtype

# View logs
tail -f ~/Library/Logs/voxtype/stderr.log

# Manual start for debugging
voxtype daemon
```

### "Another instance is already running"

```bash
# Clean up stale state
pkill -9 voxtype
rm -rf /tmp/voxtype
launchctl start io.voxtype.daemon
```

## Uninstalling

```bash
brew uninstall --cask voxtype
```

This removes:
- Voxtype.app from /Applications
- LaunchAgent (auto-start)
- CLI symlink

To also remove data:
```bash
rm -rf ~/Library/Application\ Support/voxtype
rm -rf ~/Library/Logs/voxtype
```

## Building from Source

```bash
git clone https://github.com/peteonrails/voxtype.git
cd voxtype
cargo build --release --features parakeet
```

## Getting Help

- GitHub Issues: https://github.com/peteonrails/voxtype/issues
- Documentation: https://voxtype.io
