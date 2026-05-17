// Voxtype audio-bridge wrapper.
//
// Spawns the `voxtype-audio-bridge` sidecar binary as a child Process and
// parses one NDJSON object per line of stdout. The sidecar reads from
// the daemon's audio Unix socket (`$XDG_RUNTIME_DIR/voxtype/audio.sock`)
// and emits frames in this locked protocol:
//
//   {"peak":0.421,"rms":0.180,"vad":1,"ts_ms":1234567}
//   {"status":"connected"}
//   {"status":"disconnected"}
//
// Usage:
//
//   import "voxtype-shared" as VT
//   VT.AudioBridge {
//       id: audio
//       onFrameReceived: function(peak, rms, vad, tsMs) {
//           // Push into a ring buffer, drive an animation, etc.
//       }
//       onDisconnected: console.warn("daemon audio socket dropped")
//   }
//
// Quickshell.Io.Process restarts automatically when `running` is true
// and the child exits, so a crash of the bridge binary heals on its
// own after `restartDelayMs` milliseconds.

import QtQuick
import Quickshell
import Quickshell.Io

Item {
    id: root

    /// Path or PATH-resolvable name of the audio-bridge binary. The
    /// AUR/.deb/.rpm packages install it as `voxtype-audio-bridge` in
    /// `/usr/bin`, so the default works for end-user installs. For
    /// development, override with the cargo target path:
    ///
    ///   VT.AudioBridge {
    ///       bridgeBinary: "/path/to/target/release/voxtype-audio-bridge"
    ///   }
    property string bridgeBinary: "voxtype-audio-bridge"

    /// Additional command-line arguments to pass to the bridge.
    /// Defaults to empty; the bridge auto-discovers
    /// `$XDG_RUNTIME_DIR/voxtype/audio.sock`.
    property var bridgeArgs: []

    /// Milliseconds to wait before respawning the bridge after it
    /// exits. Stops a wedged binary from pegging the CPU in a tight
    /// restart loop while still recovering quickly under normal
    /// conditions.
    property int restartDelayMs: 1000

    /// True once the bridge process has emitted at least one frame.
    /// Distinct from `process.running`, which only reflects whether
    /// the OS process exists. A bridge that's connected to the daemon
    /// but waiting for audio will have `process.running == true` and
    /// `running == false`.
    property bool running: false

    /// Latest peak amplitude, 0.0..=1.0.
    property real peak: 0.0

    /// Latest RMS amplitude, 0.0..=1.0.
    property real rms: 0.0

    /// Latest voice-activity-detection result.
    property bool vad: false

    /// Latest frame timestamp in milliseconds (monotonic clock from
    /// the daemon).
    property var tsMs: 0

    /// Emitted on every parsed audio frame. Consumers that need the
    /// full history (waveform renderer) should push to their own ring
    /// buffer in this handler.
    signal frameReceived(real peak, real rms, bool vad, var tsMs)

    /// Emitted on `{"status":"connected"}` lines from the bridge,
    /// indicating the audio socket is open.
    signal connected()

    /// Emitted on `{"status":"disconnected"}` lines or when the
    /// bridge process exits.
    signal disconnected()

    function _handleLine(line) {
        const trimmed = (line || "").trim();
        if (trimmed.length === 0) return;
        let obj;
        try {
            obj = JSON.parse(trimmed);
        } catch (e) {
            // Non-JSON line from the bridge (e.g. an unexpected log
            // print to stdout). Swallow rather than crash the OSD.
            console.warn("voxtype audio-bridge: non-JSON stdout line:", trimmed);
            return;
        }
        if (obj.status === "connected") {
            root.connected();
            return;
        }
        if (obj.status === "disconnected") {
            if (root.running) {
                root.running = false;
            }
            root.disconnected();
            return;
        }
        if (typeof obj.peak === "number" && typeof obj.rms === "number") {
            root.peak = obj.peak;
            root.rms = obj.rms;
            root.vad = !!obj.vad;
            root.tsMs = obj.ts_ms !== undefined ? obj.ts_ms : 0;
            if (!root.running) {
                root.running = true;
            }
            root.frameReceived(root.peak, root.rms, root.vad, root.tsMs);
        }
    }

    Process {
        id: process
        command: [root.bridgeBinary].concat(root.bridgeArgs)
        running: true

        stdout: SplitParser {
            splitMarker: "\n"
            onRead: function(data) { root._handleLine(data); }
        }

        // Restart the bridge when it exits. Quickshell's Process with
        // `running: true` would respawn instantly; the small delay
        // keeps a crash-looping binary from hammering the CPU.
        onRunningChanged: {
            if (!process.running) {
                if (root.running) {
                    root.running = false;
                    root.disconnected();
                }
                restartTimer.restart();
            }
        }
    }

    Timer {
        id: restartTimer
        interval: root.restartDelayMs
        repeat: false
        onTriggered: {
            if (!process.running) {
                process.running = true;
            }
        }
    }
}
