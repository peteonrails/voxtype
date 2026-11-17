// Omarchy Shell uses Hyprland's effective rounding, not a fixed pill radius.
// Query only on activation/config reload; other layouts start no processes.
import QtQuick
import Quickshell
import Quickshell.Io

QtObject {
    id: root
    property bool enabled: false
    property real cornerRadius: 0
    property var _events: null

    function applyRounding(raw) {
        try {
            const n = JSON.parse(raw || "{}").int;
            if (typeof n === "number" && isFinite(n) && n >= 0) cornerRadius = n;
        } catch (e) { /* Keep the previous radius on unavailable/malformed IPC. */ }
    }

    function refresh() {
        if (enabled && Quickshell.env("HYPRLAND_INSTANCE_SIGNATURE") && !_query.running) {
            _query.running = true;
        }
    }

    function _activate() {
        if (!enabled) return;
        if (!_events && Quickshell.env("HYPRLAND_INSTANCE_SIGNATURE")) {
            // Optional import: non-Hyprland Quickshell builds still load the
            // design, with square corners, instead of failing the whole OSD.
            try {
                _events = Qt.createQmlObject('import QtQuick; import Quickshell.Hyprland; '
                    + 'QtObject { id: tracker; signal configReloaded(); '
                    + 'property Connections events: Connections { target: Hyprland; '
                    + 'function onRawEvent(event) { if (event && event.name === "configreloaded") '
                    + 'tracker.configReloaded(); } } }', root, "voxtype-shell-rounding");
            } catch (e) {
                console.warn("voxtype: live shell rounding unavailable:", e);
            }
        }
        refresh();
    }

    onEnabledChanged: _activate()
    Component.onCompleted: _activate()

    property Connections _reloads: Connections {
        target: root._events
        enabled: root.enabled
        function onConfigReloaded() { root.refresh(); }
    }

    property Process _query: Process {
        command: ["hyprctl", "-j", "getoption", "decoration:rounding"]
        stdout: StdioCollector {
            waitForEnd: true
            onStreamFinished: root.applyRounding(text)
        }
    }
}
