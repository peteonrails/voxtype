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
//
// Shared theme, state-file reader, and audio bridge wrapper live under
// `quickshell/voxtype-shared/`; this file is intentionally thin so that
// it stays readable as the entry point for the Wave 2 waveform work.

import QtQuick
import Quickshell
import Quickshell.Wayland
import "../voxtype-shared" as VT

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
