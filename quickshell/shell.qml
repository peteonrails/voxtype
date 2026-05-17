// Voxtype Quickshell shell entry point.
//
// Run standalone for testing:
//   qs -p quickshell
//
// Quickshell treats this file as the entry of a config directory. The
// shared theme/state/audio modules live at `voxtype-shared/` (sibling
// dir) so the relative import below resolves inside Quickshell's qrc
// virtual fs; an earlier layout placed shell.qml in a subdirectory and
// the `../voxtype-shared` traversal landed in `qrc:/qs-blackhole`.
//
// Wave 2 components (engine picker, meeting controls) will be added as
// additional .qml files in this same directory and composed into
// `ShellRoot` below.
//
// Reads the daemon's state file at $XDG_RUNTIME_DIR/voxtype/state, which
// contains exactly one of: idle, recording, streaming, transcribing.

import QtQuick
import Quickshell
import Quickshell.Wayland
import "voxtype-shared" as VT

ShellRoot {
    id: shell

    // State is sourced from the shared StateReader so the engine picker
    // and meeting-controls frontends can subscribe to the same source.
    VT.StateReader {
        id: stateReader
    }

    // Wire the audio bridge even though the POC doesn't render a
    // waveform yet — letting the indicator pulse on VAD makes
    // "recording but silent" visually distinct from "recording with
    // voice", which is a nice quality-of-life cue before the full
    // waveform lands in Wave 2.
    VT.AudioBridge {
        id: audio
    }

    // The OSD surface itself. Hidden when state is idle; visible otherwise.
    PanelWindow {
        id: panel
        visible: stateReader.state !== "idle" && stateReader.state !== ""
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
            radius: VT.Theme.cornerRadius
            color: VT.Theme.bgColor
            border.width: 2

            // Per-state border tint so a Hyprland user can glance at the
            // edge of a screen and know whether voxtype is recording vs
            // streaming vs transcribing without reading the label.
            border.color: stateReader.state === "recording"    ? VT.Theme.recordingColor
                       : stateReader.state === "streaming"     ? VT.Theme.streamingColor
                       : stateReader.state === "transcribing"  ? VT.Theme.transcribingColor
                       :                                         VT.Theme.idleColor

            // Subtle opacity pulse on VAD=1 while recording. When the
            // bridge isn't running yet (or the daemon hasn't opened
            // the audio socket) we fall back to full opacity so the
            // indicator never disappears unexpectedly.
            opacity: (stateReader.state === "recording" && audio.running && !audio.vad) ? 0.78 : 1.0
            Behavior on opacity { NumberAnimation { duration: 120 } }

            Row {
                anchors.fill: parent
                anchors.leftMargin: VT.Theme.padding
                anchors.rightMargin: VT.Theme.padding
                spacing: 12

                Text {
                    width: 28
                    anchors.verticalCenter: parent.verticalCenter
                    horizontalAlignment: Text.AlignHCenter
                    text: stateReader.state === "recording"    ? "󰍬"
                       : stateReader.state === "streaming"     ? "󰜟"
                       : stateReader.state === "transcribing"  ? "󰔟"
                       :                                          "󰍬"
                    font.family: "JetBrainsMono Nerd Font"
                    font.pixelSize: 26
                    color: card.border.color
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: stateReader.state === "recording"    ? "Recording"
                       : stateReader.state === "streaming"     ? "Streaming live"
                       : stateReader.state === "transcribing"  ? "Transcribing"
                       :                                         stateReader.state
                    font.family: "JetBrainsMono Nerd Font"
                    font.bold: true
                    font.pixelSize: 14
                    color: VT.Theme.textColor
                }
            }
        }
    }
}
