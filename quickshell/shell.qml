// Voxtype Quickshell shell entry point.
//
// Run standalone for testing:
//   qs -p quickshell
//
// Quickshell treats this file as the entry of a config directory. The
// shared theme/state/audio modules live at `voxtype-shared/` (sibling
// dir).
//
// This file is intentionally thin: each Wave 2 feature is its own QML
// component (OsdSurface, EnginePicker, MeetingControls) and `ShellRoot`
// is just the composition root that wires the shared state and audio
// bridges into each one. Adding a new widget = drop a new .qml file in
// this directory and instantiate it below.
//
// State is sourced from the daemon's state file at
// $XDG_RUNTIME_DIR/voxtype/state. Audio frames are sourced from the
// `voxtype-audio-bridge` sidecar which reads
// $XDG_RUNTIME_DIR/voxtype/audio.sock and emits NDJSON on stdout.

import QtQuick
import Quickshell
import "voxtype-shared" as VT

ShellRoot {
    id: shell

    VT.StateReader {
        id: stateReader
    }

    VT.AudioBridge {
        id: audio
    }

    OsdSurface {
        id: osd
        daemonState: stateReader.state
        audio: audio
    }
}
