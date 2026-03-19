# Voxtype macOS Architecture

This document describes the macOS-specific architecture for Voxtype.

## Component Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        macOS System                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────────┐  ┌──────────────────┐  ┌───────────────┐ │
│  │ VoxtypeMenubar   │  │  VoxtypeSetup    │  │  Voxtype.app  │ │
│  │     (.app)       │  │     (.app)       │  │   (daemon)    │ │
│  │                  │  │                  │  │               │ │
│  │ - Menu bar icon  │  │ - Settings GUI   │  │ - CLI binary  │ │
│  │ - Status display │  │ - Config editor  │  │ - Transcriber │ │
│  │ - Quick settings │  │ - Model manager  │  │ - Hotkey      │ │
│  │ - Opens Setup    │  │ - Permissions    │  │ - Audio       │ │
│  └────────┬─────────┘  └────────┬─────────┘  └───────┬───────┘ │
│           │                     │                     │         │
│           │    Reads config     │    Writes config    │         │
│           └──────────┬──────────┴──────────┬──────────┘         │
│                      │                     │                     │
│                      ▼                     ▼                     │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │          ~/Library/Application Support/voxtype/             ││
│  │  - config.toml (configuration)                              ││
│  │  - models/ (Whisper/Parakeet models)                        ││
│  └─────────────────────────────────────────────────────────────┘│
│                                                                  │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │                    /tmp/voxtype/                            ││
│  │  - state (idle/recording/transcribing)                      ││
│  │  - pid (daemon process ID)                                  ││
│  │  - voxtype.lock (prevents multiple instances)               ││
│  └─────────────────────────────────────────────────────────────┘│
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Applications

### 1. Voxtype.app (Main Binary)

**Location:** `/Applications/Voxtype.app/Contents/MacOS/voxtype`

The core Rust binary that provides:
- `voxtype daemon` - Background service for voice transcription
- `voxtype status` - Check daemon state
- `voxtype record start/stop/toggle` - Manual recording control
- `voxtype setup` - Installation and model management

**Key Files:**
- `src/daemon.rs` - Main event loop
- `src/hotkey_macos.rs` - macOS hotkey detection via rdev
- `src/notification.rs` - macOS notifications via mac-notification-sys
- `src/output/cgevent.rs` - Text output via CGEvent (macOS native)

### 2. VoxtypeMenubar.app (Menu Bar Widget)

**Location:** `/Applications/VoxtypeMenubar.app`

Swift/SwiftUI app that provides:
- Menu bar icon showing daemon status
- Quick access to start/stop recording
- Quick settings (Engine, Output Mode, Hotkey Mode)
- Link to open VoxtypeSetup

**Key Files:**
- `macos/VoxtypeMenubar/Sources/VoxtypeMenubarApp.swift` - App entry point
- `macos/VoxtypeMenubar/Sources/MenuBarView.swift` - Menu dropdown UI
- `macos/VoxtypeMenubar/Sources/VoxtypeStatusMonitor.swift` - Polls /tmp/voxtype/state
- `macos/VoxtypeMenubar/Sources/VoxtypeCLI.swift` - Runs voxtype CLI commands

### 3. VoxtypeSetup.app (Settings Application)

**Location:** `/Applications/VoxtypeSetup.app`

Swift/SwiftUI app that provides:
- Full settings GUI with sidebar navigation
- Model download and management
- Permission status checking
- Daemon control (start/stop/restart)

**Settings Sections:**
- General - Engine selection, daemon status
- Hotkey - Key selection, mode, cancel key
- Audio - Device, max duration, feedback
- Models - Installed models, download new
- Whisper - Language, translate, GPU isolation
- Remote Whisper - Server URL, API key
- Output - Mode, type delay, auto-submit
- Text Processing - Spoken punctuation, replacements
- Notifications - Event triggers, engine icon
- Permissions - macOS permissions status
- Advanced - Config file, logs, auto-start

**Key Files:**
- `macos/VoxtypeSetup/Sources/VoxtypeSetupApp.swift` - App entry point
- `macos/VoxtypeSetup/Sources/Settings/*.swift` - Settings views
- `macos/VoxtypeSetup/Sources/Utilities/ConfigManager.swift` - Config read/write
- `macos/VoxtypeSetup/Sources/Utilities/VoxtypeCLI.swift` - CLI integration

## Configuration

**Config File:** `~/Library/Application Support/voxtype/config.toml`

The ConfigManager (in VoxtypeSetup) handles section-aware config updates to prevent corruption.

## macOS Permissions Required

1. **Microphone** - For audio capture
2. **Input Monitoring** - For global hotkey detection (rdev library)
3. **Accessibility** - For typing text into applications (CGEvent)

## LaunchAgent (Auto-Start)

**Plist Location:** `~/Library/LaunchAgents/io.voxtype.daemon.plist`

Managed via:
- `voxtype setup launchd` - Install service
- `voxtype setup launchd --uninstall` - Remove service

## Build Process

### Building Swift Apps

```bash
# Build VoxtypeMenubar
cd macos/VoxtypeMenubar
./build-app.sh

# Build VoxtypeSetup
cd macos/VoxtypeSetup
./build-app.sh
```

Build scripts:
1. Run `swift build -c release`
2. Create .app bundle structure
3. Generate AppIcon.icns from assets/icon.png
4. Create Info.plist
5. Code sign with entitlements

### Building Rust Binary

**All macOS binaries must include Parakeet support:**

```bash
cargo build --release --features parakeet
cp target/release/voxtype /Applications/Voxtype.app/Contents/MacOS/
```

## Known Issues / TODOs

1. **Notification Icon** - Daemon notifications use default icon (not app icon) because daemon runs as CLI process, not from app bundle context

2. **Audio Feedback** - Currently disabled on macOS due to "use_default" file dialog issue with rodio/cpal

3. **Unsigned Binaries** - Apps are ad-hoc signed, require "Open Anyway" in Security settings

4. **LaunchAgent Conflicts** - If launchd keeps restarting daemon, use `launchctl unload` before manual testing

## File Locations Summary

| Item | Path |
|------|------|
| Main binary | `/Applications/Voxtype.app/Contents/MacOS/voxtype` |
| Menubar app | `/Applications/VoxtypeMenubar.app` |
| Settings app | `/Applications/VoxtypeSetup.app` |
| Config file | `~/Library/Application Support/voxtype/config.toml` |
| Models | `~/Library/Application Support/voxtype/models/` |
| State file | `/tmp/voxtype/state` |
| PID file | `/tmp/voxtype/pid` |
| Lock file | `/tmp/voxtype/voxtype.lock` |
| LaunchAgent | `~/Library/LaunchAgents/io.voxtype.daemon.plist` |
| Logs | `~/Library/Logs/voxtype/` (if enabled) |
