// Voxtype on-screen display surface (Quickshell frontend).
//
// Renders a glassy card with:
//   - State icon + tint (recording / streaming / transcribing)
//   - Scrolling waveform of recent mic peaks (3-second window, 100 Hz)
//   - Peak meter bar with held-peak tick (-60 dB floor, color zones at
//     -12 and -3 dBFS to match the GTK4 and native frontends)
//
// Driven entirely from a parent-provided AudioBridge: the parent is
// expected to subscribe to AudioBridge once and pass it in via the
// `audio` property so the OsdSurface, EnginePicker, and MeetingControls
// share one sidecar process rather than each spawning their own.
//
// Visibility follows `daemonState`: hidden when idle/empty, shown
// otherwise. The component is layered as a WlrLayer.Overlay surface so
// it floats above all other windows without taking input focus.

import QtQuick
import Quickshell
import Quickshell.Wayland
import "voxtype-shared" as VT

PanelWindow {
    id: panel

    /// Current daemon state: idle / recording / streaming / transcribing.
    /// Wired by the parent (typically from VT.StateReader.state).
    property string daemonState: "idle"

    /// The audio bridge instance whose frameReceived signal drives the
    /// waveform. Passed in by the parent so it's shared with sibling
    /// widgets that also want VAD / peak data.
    property var audio: null

    visible: daemonState !== "idle" && daemonState !== ""
    anchors {
        top: true
        bottom: true
        left: true
        right: true
    }
    color: "transparent"

    WlrLayershell.namespace: "voxtype-osd"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
    exclusionMode: ExclusionMode.Ignore

    // Per-state tint, shared by icon + card border so a Hyprland user
    // can read the daemon's state from screen-edge color alone.
    readonly property color stateColor:
        daemonState === "recording"    ? VT.Theme.recordingColor
      : daemonState === "streaming"    ? VT.Theme.streamingColor
      : daemonState === "transcribing" ? VT.Theme.transcribingColor
      :                                  VT.Theme.idleColor

    // Ring of recent per-frame peaks (0.0..1.0). Capacity = 3 s @ 100 Hz.
    // Stored as a plain array; we shift() when full to keep newest-on-right.
    readonly property int waveformColumns: Math.round(VT.Theme.waveformWindowSecs * 100)
    property var ring: []

    // Peak meter state (kept in dBFS so the held-peak decay math matches
    // src/osd/visual.rs's PeakHold verbatim).
    property real currentPeakDbfs: -120
    property real heldDbfs: -120
    property real lastFrameTsMs: 0

    function _resetMeters() {
        ring = [];
        currentPeakDbfs = -120;
        heldDbfs = -120;
        lastFrameTsMs = 0;
        waveCanvas.requestPaint();
        meterCanvas.requestPaint();
    }

    Connections {
        target: panel.audio
        enabled: panel.audio !== null
        function onFrameReceived(peak, rms, vad, tsMs) {
            // Push the new peak onto the ring; drop the oldest when full.
            const r = panel.ring.slice();
            r.push(peak);
            while (r.length > panel.waveformColumns) {
                r.shift();
            }
            panel.ring = r;

            // dBFS from linear peak. Floor at -120 to match visual.rs's
            // PeakHold::held_dbfs sentinel for "effectively silent".
            const dbfs = peak > 0.0 ? 20 * Math.log10(peak) : -120;
            panel.currentPeakDbfs = dbfs;

            // Held-peak: snap up on a louder peak, otherwise decay at
            // peakDecayDbPerSec. dt comes from the frame timestamps so
            // a paused daemon (no frames) doesn't unrealistically decay
            // before we receive the next frame.
            const dtMs = panel.lastFrameTsMs > 0
                ? Math.max(0, tsMs - panel.lastFrameTsMs)
                : 10;
            panel.lastFrameTsMs = tsMs;
            const dt = dtMs / 1000;
            if (dbfs > panel.heldDbfs) {
                panel.heldDbfs = dbfs;
            } else {
                const decayed = panel.heldDbfs - VT.Theme.peakDecayDbPerSec * dt;
                panel.heldDbfs = decayed < -120 ? -120 : decayed;
            }

            waveCanvas.requestPaint();
            meterCanvas.requestPaint();
        }
        function onDisconnected() {
            panel._resetMeters();
        }
    }

    // Clear when the daemon's state moves out of recording so the
    // waveform doesn't show stale audio from the previous recording on
    // the next one.
    onDaemonStateChanged: {
        if (daemonState === "idle" || daemonState === "") {
            _resetMeters();
        }
    }

    Rectangle {
        id: card
        width: VT.Theme.defaultWidthPx
        height: 72
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 72
        radius: VT.Theme.cornerRadius
        color: VT.Theme.bgColor
        border.width: 2
        border.color: panel.stateColor
        opacity: (panel.daemonState === "recording" && panel.audio && panel.audio.running && !panel.audio.vad)
                 ? 0.78 : 1.0
        Behavior on opacity { NumberAnimation { duration: 120 } }

        Row {
            anchors.fill: parent
            anchors.leftMargin: VT.Theme.padding
            anchors.rightMargin: VT.Theme.padding
            spacing: 10

            Text {
                width: 28
                anchors.verticalCenter: parent.verticalCenter
                horizontalAlignment: Text.AlignHCenter
                text: panel.daemonState === "recording"    ? "󰍬"
                   : panel.daemonState === "streaming"     ? "󰜟"
                   : panel.daemonState === "transcribing"  ? "󰔟"
                   :                                          "󰍬"
                font.family: "JetBrainsMono Nerd Font"
                font.pixelSize: 26
                color: panel.stateColor
            }

            Column {
                width: card.width - 28 - 2 * VT.Theme.padding - 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 4

                Canvas {
                    id: waveCanvas
                    width: parent.width
                    height: 36

                    onPaint: {
                        const ctx = getContext("2d");
                        ctx.clearRect(0, 0, width, height);
                        const r = panel.ring;
                        if (r.length === 0) {
                            return;
                        }

                        const cy = height / 2;
                        const maxHalf = height / 2 - 1;
                        const cols = panel.waveformColumns;
                        const colW = width / cols;
                        // Empty columns on the left when the ring isn't
                        // full yet, so newest data lands flush against
                        // the right edge.
                        const startIdx = cols - r.length;

                        ctx.strokeStyle = VT.Theme.waveformColor;
                        ctx.lineWidth = Math.max(1, colW);
                        ctx.lineCap = "butt";
                        ctx.beginPath();
                        for (let i = 0; i < r.length; i++) {
                            const x = (startIdx + i) * colW + colW / 2;
                            const halfH = Math.min(
                                maxHalf,
                                r[i] * maxHalf * VT.Theme.waveformGain
                            );
                            ctx.moveTo(x, cy - halfH);
                            ctx.lineTo(x, cy + halfH);
                        }
                        ctx.stroke();
                    }
                }

                Canvas {
                    id: meterCanvas
                    width: parent.width
                    height: 6

                    onPaint: {
                        const ctx = getContext("2d");
                        ctx.clearRect(0, 0, width, height);

                        const floor = VT.Theme.meterFloorDbfs;
                        const span = -floor;

                        // Current peak → fill width
                        let fill = 0;
                        if (panel.currentPeakDbfs > floor) {
                            const clipped = Math.min(panel.currentPeakDbfs, 0);
                            fill = Math.max(0, Math.min(1, (clipped - floor) / span));
                        }

                        // Zone color matches src/osd/visual.rs::MeterZone.
                        let zone = VT.Theme.meterLowColor;
                        if (panel.currentPeakDbfs >= -3) {
                            zone = VT.Theme.meterHighColor;
                        } else if (panel.currentPeakDbfs >= -12) {
                            zone = VT.Theme.meterMidColor;
                        }

                        ctx.fillStyle = zone;
                        ctx.fillRect(0, 0, width * fill, height);

                        // Held-peak tick. Skip if floor or below so the
                        // tick doesn't pin to the left edge during silence.
                        if (panel.heldDbfs > floor) {
                            const clippedHeld = Math.min(panel.heldDbfs, 0);
                            const heldFill = Math.max(0, Math.min(1,
                                (clippedHeld - floor) / span));
                            ctx.fillStyle = VT.Theme.waveformPeakColor;
                            ctx.fillRect(
                                Math.max(0, width * heldFill - 1),
                                0, 2, height
                            );
                        }
                    }
                }
            }
        }
    }
}
