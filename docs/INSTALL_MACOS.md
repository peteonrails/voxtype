# Voxtype macOS Installation Guide

Voxtype is a push-to-talk voice-to-text tool with fast, local speech recognition using Whisper.

> **Note:** macOS support is in beta. The binaries are currently unsigned, which requires a few extra steps during installation. Once we have signed and notarized binaries, this process will be simpler.

## Requirements

- macOS 13 (Ventura) or later
- Apple Silicon or Intel (the release binary is universal)
- Microphone access
- Input Monitoring permission (for global hotkey)

## Installation via Homebrew (Recommended)

```bash
# Add the tap
brew tap peteonrails/voxtype

# Install
brew install --cask peteonrails/voxtype/voxtype
```

The Cask installs a single `voxtype` command-line tool into your Homebrew
prefix and clears the quarantine flag on it. There is no app bundle, no
`/Applications` entry, and no login item: start the daemon yourself, or install
the formula instead of the cask if you want a `brew services` job.

The fastest way through the rest of setup is the TUI:

```bash
voxtype configure
```

### First-Time Security Setup

The binary is not signed or notarized yet, so macOS asks about it on first run.
This is a one-time setup:

1. **Grant Input Monitoring permission (required for the hotkey):**
   - Open **System Settings** > **Privacy & Security** > **Input Monitoring**
   - Add and enable the terminal you run `voxtype` from

2. **Grant Microphone access** when macOS prompts on the first recording.

3. **Restart the daemon** to pick up permissions:
   ```bash
   pkill voxtype
   voxtype daemon
   ```

### Download a Speech Model

```bash
# Small and fast
voxtype setup --download --model base.en

# Or larger and more accurate (Metal keeps it quick on Apple Silicon)
voxtype setup --download --model large-v3-turbo
```

### Available Engines

The macOS build includes two engines:

- **Whisper**, which runs locally with Metal acceleration, or against a remote Whisper server (see `[whisper] mode` in [CONFIGURATION.md](CONFIGURATION.md)).
- **Soniox**, a cloud engine (see [SONIOX.md](SONIOX.md)).

Parakeet, Moonshine, SenseVoice, Paraformer, Dolphin, Omnilingual, Cohere, the GGUF engine and OpenVINO are Linux-only for now. `voxtype setup onnx --enable` switches between Linux binary variants, so it does not apply on macOS.

## Usage

Hold **Right Option** (⌥) to record, release to transcribe. Text is typed into the active application.

### Quick Commands

```bash
voxtype status              # Check daemon status
voxtype record start        # Start recording manually
voxtype record stop         # Stop and transcribe
voxtype doctor              # Verify setup, with fixes
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

Config file: `~/.config/voxtype/config.toml`, honoring `$XDG_CONFIG_HOME` if set.

Older installs that wrote to `~/Library/Application Support/voxtype/config.toml`
keep using it until you move the file to `~/.config/voxtype/`.

```toml
# Transcription engine
engine = "whisper"  # or "soniox"

[hotkey]
key = "RIGHTALT"     # Right Option key
mode = "push_to_talk"  # or "toggle"

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
voxtype setup --download --model base.en
```

### "engine unavailable" or "not compiled into this binary"

The config names an engine the macOS build does not include, such as
`parakeet`. Switch back to Whisper:

```bash
voxtype config set engine whisper
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

This removes the `voxtype` command-line tool and the staged DMG payload.

To also remove data:
```bash
rm -rf ~/Library/Application\ Support/voxtype
rm -rf ~/Library/Logs/voxtype
```

## Building from Source

```bash
git clone https://github.com/peteonrails/voxtype.git
cd voxtype
cargo build --release --features gpu-metal
```

This matches the release build: Whisper with Metal, plus Soniox. The ONNX and
GGUF engine features are not part of the macOS release or its CI, and are
untested on macOS.

## Getting Help

- GitHub Issues: https://github.com/peteonrails/voxtype/issues
- Documentation: https://voxtype.io
