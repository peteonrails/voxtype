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
// bridges into each widget that needs them.
//
// All three Wave 2 widgets now live in the ShellRoot:
//   - OsdSurface: state-driven recording HUD, one per output. Reads
//     daemonState + audio bridge to render the waveform.
//   - EnginePicker: floating engine switcher, toggled via the
//     `$XDG_RUNTIME_DIR/voxtype/engine-picker.flag` file. Self-sources
//     its state from config.toml; doesn't need the daemon state or
//     audio bridge.
//   - MeetingControls: meeting status + start/stop/pause/resume,
//     toggled via the `$XDG_RUNTIME_DIR/voxtype/meeting-controls.flag`
//     file. Self-sources state from
//     `$XDG_RUNTIME_DIR/voxtype/meeting_state` and `voxtype meeting
//     show`; doesn't need the daemon state or audio bridge either.
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

    /// Name of the output the compositor reports as focused, or "" when
    /// it reports none (unsupported compositor, or Quickshell built
    /// without the module).
    readonly property string focusedOutputName: _focusTracker ? _focusTracker.name : ""

    /// The focused ShellScreen, or null when focus is unknown.
    readonly property var focusedScreen: {
        const screens = Quickshell.screens;
        for (let i = 0; i < screens.length; i++) {
            if (screens[i].name === focusedOutputName) return screens[i];
        }
        return null;
    }

    property var _focusTracker: null

    // Quickshell has no compositor-neutral focused-output API, so use the
    // compositor's own module when we're running under it. Built from a
    // string because an `import` of a module this Quickshell build lacks
    // fails the whole file; createQmlObject turns that into a catchable
    // error and we fall back to showing the HUD on every output.
    function _createFocusTracker() {
        const sources = [];
        if (Quickshell.env("HYPRLAND_INSTANCE_SIGNATURE")) {
            sources.push("import QtQuick; import Quickshell.Hyprland; QtObject { "
                + "readonly property string name: Hyprland.focusedMonitor ? Hyprland.focusedMonitor.name : \"\" }");
        }
        if (Quickshell.env("SWAYSOCK") || Quickshell.env("I3SOCK")) {
            sources.push("import QtQuick; import Quickshell.I3; QtObject { "
                + "readonly property string name: I3.focusedMonitor ? I3.focusedMonitor.name : \"\" }");
        }
        for (let i = 0; i < sources.length; i++) {
            try {
                return Qt.createQmlObject(sources[i], shell, "voxtype-focus-tracker");
            } catch (e) {
                console.warn("voxtype: focused-output tracking unavailable:", e);
            }
        }
        return null;
    }

    Component.onCompleted: _focusTracker = _createFocusTracker()

    VT.StateReader {
        id: stateReader
    }

    // Not `id: audio`: OsdSurface has an `audio` property, and whether
    // `audio: audio` resolves to this id or to the surface's own property
    // depends on the Qt version. When it picks the property, the waveform
    // never moves (#774).
    VT.AudioBridge {
        id: audioBridge
    }

    VT.StyleLoader {
        id: osdStyle
    }

    // One surface per output, shown on the focused one so the HUD follows
    // the user between monitors (#775). When focus is unknown every output
    // shows it rather than none. Hidden surfaces skip all per-frame work,
    // so the extra outputs cost nothing while they aren't drawn. Variants
    // also rebuilds surfaces as outputs are unplugged or powered off.
    Variants {
        model: Quickshell.screens

        OsdSurface {
            required property var modelData
            screen: modelData
            // By name, not object identity, so this doesn't depend on
            // Quickshell handing out the same wrapper for each screen.
            activeOutput: shell.focusedScreen === null || shell.focusedOutputName === modelData.name
            daemonState: stateReader.state
            osdSuppressed: stateReader.osdSuppressed
            session: stateReader.session
            outcome: stateReader.outcome
            audio: audioBridge
            style: osdStyle
        }
    }

    EnginePicker {
        id: enginePicker
    }

    MeetingControls {
        id: meetingControls
    }

    // Open the keyboard-driven popups on the focused output. Updated only
    // while closed so a popup never jumps monitors under the user, and
    // RestoreNone keeps the last value when the binding pauses.
    Binding {
        target: enginePicker
        property: "screen"
        value: shell.focusedScreen
        when: shell.focusedScreen !== null && !enginePicker.open
        restoreMode: Binding.RestoreNone
    }

    Binding {
        target: meetingControls
        property: "screen"
        value: shell.focusedScreen
        when: shell.focusedScreen !== null && !meetingControls.open
        restoreMode: Binding.RestoreNone
    }
}
