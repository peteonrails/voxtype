# Voxtype macOS

This directory contains macOS-specific code, separate from the cross-platform Rust core.

## VoxtypeSetup

A native SwiftUI app that provides:

1. **Setup Wizard** - First-run experience that guides users through:
   - Granting permissions (Microphone, Accessibility, Input Monitoring)
   - Downloading a speech model
   - Installing the LaunchAgent for auto-start

2. **Preferences** - Settings panel for changing:
   - Speech engine (Parakeet vs Whisper)
   - Model selection
   - Auto-start toggle
   - Daemon control

## Building

```bash
cd macos/VoxtypeSetup
swift build -c release

# Or open in Xcode
open Package.swift
```

## Architecture

The SwiftUI app is a thin GUI layer. All actual functionality is delegated to the
`voxtype` Rust binary via CLI calls:

- `VoxtypeCLI.swift` - Wrapper that calls voxtype commands
- `PermissionChecker.swift` - Native macOS permission checking

This keeps business logic in Rust while providing a native Mac experience.

## Distribution

The setup app can be:
1. Bundled inside Voxtype.app as a helper
2. Distributed separately as VoxtypeSetup.app
3. Invoked via `voxtype setup macos --gui` (if integrated)
