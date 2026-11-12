// The meeting view in voxtype's standalone Quickshell shell.
//
// The view itself lives in voxtype-meetings/ and is the same one the Omarchy
// plugin shows; this file is only its host here: the shell's theme, and the
// ways the view opens and closes.
//
// ## Opening
//
// - On its own when a meeting starts, if `[meeting] show_view_on_start` is on
//   (the default). The view watches meeting.sock whether or not it is open.
// - `voxtype meeting view [ID] [--tab TAB]`, which is also what an Ask
//   notification's Open action runs. Every host reads the request it writes.
// - The old keybinding still toggles it: touching
//   `$XDG_RUNTIME_DIR/voxtype/meeting-controls.flag`. For example (Hyprland):
//
//     bind = SUPER, M, exec, mkdir -p $XDG_RUNTIME_DIR/voxtype && touch $XDG_RUNTIME_DIR/voxtype/meeting-controls.flag
//
//   The flag is removed once read, so the next touch toggles again.
//
// It replaces the earlier MeetingControls popup (status plus start, stop,
// pause and resume), whose controls are all in the view's header; the file
// keeps its name and the `open` and `screen` properties shell.qml binds.

import QtQuick
import Quickshell
import Quickshell.Io
import "voxtype-shared" as VT
import "voxtype-meetings"

Scope {
    id: root

    /// Path or PATH-resolvable name of the voxtype CLI.
    property string voxtypeBinary: "voxtype"

    /// Whether the view is up.
    property bool open: false

    /// The output to open on (shell.qml follows the focused one).
    property var screen: null

    readonly property string runtimeDir: {
        const xdg = Quickshell.env("XDG_RUNTIME_DIR");
        return (xdg && xdg.length > 0 ? xdg : "/tmp") + "/voxtype";
    }

    readonly property alias meetings: meetings

    MeetingTheme {
        id: shellTheme
        fg: VT.Theme.textColor
        bg: Qt.rgba(VT.Theme.bgColor.r, VT.Theme.bgColor.g, VT.Theme.bgColor.b, 0.97)
        border: VT.Theme.accentColor
        accent: VT.Theme.accentColor
        urgent: VT.Theme.recordingColor
        radius: VT.Theme.cornerRadius
        padding: VT.Theme.padding
    }

    Meetings {
        id: meetings
        bin: root.voxtypeBinary
        theme: shellTheme
        shown: root.open
        screen: root.screen
        host: "standalone"
        onOpenRequested: root.open = true
        onCloseRequested: root.open = false
    }

    // ----- meeting-controls.flag (toggle) -----

    FileView {
        id: flagFile
        path: root.runtimeDir + "/meeting-controls.flag"
        watchChanges: true
        printErrors: false

        onLoaded: {
            root.open = !root.open;
            removeFlag.running = true;
        }

        onFileChanged: reload()
    }

    Process {
        id: removeFlag
        command: ["rm", "-f", root.runtimeDir + "/meeting-controls.flag"]
    }
}
