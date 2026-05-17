// Voxtype Quickshell OSD — proof of concept
//
// Run standalone for testing:
//   qs -p quickshell/voxtype-osd/shell.qml
//
// Or drop into Omarchy's quickshell plugin tree as a peer of the existing
// `plugins/osd/Osd.qml`. The Omarchy OSD uses Quickshell IPC for one-shot
// volume/brightness toasts; this one watches voxtype's state file so the
// indicator follows the daemon's actual state without poking from outside.
//
// Reads the daemon's state file at $XDG_RUNTIME_DIR/voxtype/state, which
// contains exactly one of: idle, recording, streaming, transcribing.

import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland

ShellRoot {
    id: shell

    property string runtimeDir: Quickshell.env("XDG_RUNTIME_DIR")
                                  || "/run/user/" + Quickshell.env("UID")
    property string statePath: runtimeDir + "/voxtype/state"
    property string state: "idle"

    // Watch the daemon's state file. Quickshell's FileView reads the
    // contents into `text()` on demand; we trigger re-reads via reload()
    // whenever the underlying file changes so the OSD follows the daemon's
    // state machine transitions (idle → recording → transcribing → idle).
    FileView {
        id: stateFile
        path: shell.statePath
        watchChanges: true
        printErrors: false
        onLoaded: shell.state = (text() || "idle").trim()
        onLoadFailed: shell.state = "idle"
        onFileChanged: reload()
    }

    // The OSD surface itself. Hidden when state is idle; visible otherwise.
    PanelWindow {
        id: panel
        visible: shell.state !== "idle" && shell.state !== ""
        anchors { top: true; bottom: true; left: true; right: true }
        color: "transparent"

        WlrLayershell.namespace: "voxtype-osd"
        WlrLayershell.layer: WlrLayer.Overlay
        WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
        exclusionMode: ExclusionMode.Ignore

        Rectangle {
            id: card
            width: 220
            height: 56
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 72
            radius: 8
            color: Qt.rgba(0.08, 0.08, 0.10, 0.95)
            border.width: 2

            // Per-state border tint so a Hyprland user can glance at the
            // edge of a screen and know whether voxtype is recording vs
            // streaming vs transcribing without reading the label.
            border.color: shell.state === "recording"    ? "#e06c75"
                       : shell.state === "streaming"     ? "#61afef"
                       : shell.state === "transcribing"  ? "#e5c07b"
                       :                                   "#abb2bf"

            Row {
                anchors.fill: parent
                anchors.leftMargin: 14
                anchors.rightMargin: 14
                spacing: 12

                Text {
                    width: 28
                    anchors.verticalCenter: parent.verticalCenter
                    horizontalAlignment: Text.AlignHCenter
                    text: shell.state === "recording"    ? "󰍬"
                       : shell.state === "streaming"     ? "󰜟"
                       : shell.state === "transcribing"  ? "󰔟"
                       :                                   "󰍬"
                    font.family: "JetBrainsMono Nerd Font"
                    font.pixelSize: 26
                    color: card.border.color
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: shell.state === "recording"    ? "Recording"
                       : shell.state === "streaming"     ? "Streaming live"
                       : shell.state === "transcribing"  ? "Transcribing"
                       :                                   shell.state
                    font.family: "JetBrainsMono Nerd Font"
                    font.bold: true
                    font.pixelSize: 14
                    color: "#dcdfe4"
                }
            }
        }
    }
}
