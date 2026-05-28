// Voxtype meeting controls popup for Quickshell.
//
// A floating panel that surfaces the active meeting (title, elapsed
// time, chunk count) and exposes Start / Stop / Pause / Resume buttons
// that shell out to the existing `voxtype meeting <action>` CLI
// commands. No new daemon IPC is introduced; every read and every
// action uses surface that already exists.
//
// ## Read path
//
// Live state is sourced from three places that the daemon already
// maintains, in descending order of freshness:
//
//   1. `$XDG_RUNTIME_DIR/voxtype/meeting_state` (FileView, watched) -
//      two-line file: `status\nmeeting_id`. The daemon rewrites this
//      on every state transition (recording -> paused -> recording ->
//      idle), so it's the source of truth for which buttons should be
//      enabled.
//
//   2. `voxtype meeting show <id>` (Process, polled at 2 Hz while a
//      meeting is active) - parses the human-readable output to
//      extract the meeting title and chunk count. Cheaper alternatives
//      (a JSON status command, or a runtime metadata.json with a
//      stable path) don't exist yet; if they land later, swap this
//      poll for a FileView.
//
//   3. Wall-clock elapsed time (client-side Timer, 1 Hz) - the
//      meeting started_at is read out of `meeting show` once and the
//      ticker updates the displayed mm:ss locally so the duration
//      doesn't visibly stutter at the 2 Hz poll boundary.
//
// ## Open/close trigger
//
// Visibility is toggled by the presence of a flag file at
// `$XDG_RUNTIME_DIR/voxtype/meeting-controls.flag`. Bind it from your
// compositor like so:
//
//   # Hyprland
//   bind = SUPER, M, exec, touch $XDG_RUNTIME_DIR/voxtype/meeting-controls.flag
//
//   # Sway
//   bindsym $mod+m exec touch $XDG_RUNTIME_DIR/voxtype/meeting-controls.flag
//
// The widget removes the flag once it picks up the toggle, so a
// subsequent `touch` reliably retoggles. Esc inside the panel also
// closes it (and removes the flag).
//
// A Quickshell IPC handler would be the more idiomatic mechanism, but
// the 0.2.1 IPC API isn't stable across Quickshell builds users have
// installed; a flag file works on every install and matches the
// pattern used elsewhere in voxtype's runtime dir.
//
// ## Why this lives outside shell.qml
//
// shell.qml is the OSD entry point and is composed by the maintainer
// in a separate pass. This file is self-contained: it exposes a
// PanelWindow at the top of its tree so it can be instantiated either
// as a sibling in ShellRoot or hoisted into another shell config.

import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import "voxtype-shared" as VT

PanelWindow {
    id: root

    // ----- public API -----

    /// Path or PATH-resolvable name of the voxtype CLI. AUR/.deb/.rpm
    /// packages install it as `/usr/bin/voxtype`; the bare name works
    /// for any standard install. Dev override:
    ///
    ///   MeetingControls { voxtypeBinary: "/path/to/target/release/voxtype" }
    property string voxtypeBinary: "voxtype"

    /// Directory containing the daemon's runtime files (the
    /// `meeting_state` file and the open/close flag). Mirrors the
    /// resolution in `Config::runtime_dir()` so the widget never
    /// disagrees with the daemon about where to look.
    property string runtimeDir: {
        const xdg = Quickshell.env("XDG_RUNTIME_DIR");
        if (xdg && xdg.length > 0) {
            return xdg + "/voxtype";
        }
        const uid = Quickshell.env("UID");
        if (uid && uid.length > 0) {
            return "/run/user/" + uid + "/voxtype";
        }
        return "/run/user/1000/voxtype";
    }

    /// Whether the panel is currently visible. Compositors can flip
    /// this directly (e.g. via Quickshell IPC) as an alternative to
    /// the flag-file pattern.
    property bool open: false

    // ----- derived state -----

    /// One of: "idle", "recording", "paused". Mirrors the first line
    /// of the daemon's meeting_state file.
    readonly property string meetingStatus: _meetingStatus
    /// Currently active meeting id, or "" when no meeting is active.
    readonly property string meetingId: _meetingId
    /// Display title parsed from `voxtype meeting show`. Falls back
    /// to the meeting id when the show command hasn't completed yet.
    readonly property string meetingTitle: _meetingTitle
    /// Chunks committed to storage so far (parsed from
    /// `voxtype meeting show`). 0 while the show poll is in flight.
    readonly property int chunkCount: _chunkCount

    // ----- panel surface -----

    visible: root.open
    color: "transparent"
    anchors { top: true; bottom: true; left: true; right: true }
    exclusionMode: ExclusionMode.Ignore

    WlrLayershell.namespace: "voxtype-meeting-controls"
    WlrLayershell.layer: WlrLayer.Top
    WlrLayershell.keyboardFocus: root.open ? WlrKeyboardFocus.Exclusive
                                           : WlrKeyboardFocus.None

    // ----- internal state -----

    property string _meetingStatus: "idle"
    property string _meetingId: ""
    property string _meetingTitle: ""
    property int _chunkCount: 0
    // started_at as a JS timestamp (ms since epoch). 0 means unknown.
    property var _startedAtMs: 0
    // Elapsed seconds, refreshed by the 1 Hz tick when a meeting is
    // active. Kept as a property so the QML binding refreshes the
    // text without re-parsing the show output.
    property int _elapsedSecs: 0
    // Short transient string describing the last action ("Starting
    // meeting...", "Stop requested", error text). Cleared after a
    // few seconds.
    property string _actionStatus: ""

    // ----- meeting_state file (status + id) -----

    FileView {
        id: stateFile
        path: root.runtimeDir + "/meeting_state"
        watchChanges: true
        printErrors: false

        onLoaded: {
            const raw = (text() || "").trim();
            const lines = raw.length > 0 ? raw.split("\n") : ["idle"];
            const status = (lines[0] || "idle").trim();
            const id = (lines[1] || "").trim();
            if (status !== root._meetingStatus) {
                root._meetingStatus = status;
            }
            if (id !== root._meetingId) {
                root._meetingId = id;
                // Force a metadata refresh on the next tick when the
                // meeting id changes (new meeting started, or the
                // previous one ended).
                root._meetingTitle = "";
                root._chunkCount = 0;
                root._startedAtMs = 0;
                if (id.length > 0) {
                    showProcess.refresh();
                }
            }
        }

        onLoadFailed: {
            if (root._meetingStatus !== "idle") {
                root._meetingStatus = "idle";
            }
            if (root._meetingId !== "") {
                root._meetingId = "";
                root._meetingTitle = "";
                root._chunkCount = 0;
                root._startedAtMs = 0;
            }
        }

        onFileChanged: reload()
    }

    // ----- meeting-controls.flag (toggle visibility) -----

    FileView {
        id: flagFile
        path: root.runtimeDir + "/meeting-controls.flag"
        watchChanges: true
        printErrors: false

        onLoaded: {
            // Any time the flag file appears, toggle the panel and
            // consume the flag so a subsequent `touch` retoggles.
            root.open = !root.open;
            removeFlagProcess.start();
        }

        onFileChanged: reload()
    }

    Process {
        id: removeFlagProcess
        command: ["rm", "-f", root.runtimeDir + "/meeting-controls.flag"]
        running: false

        function start() {
            if (!removeFlagProcess.running) {
                removeFlagProcess.running = true;
            }
        }
    }

    // ----- voxtype meeting show <id> poll (title + chunk count) -----

    Process {
        id: showProcess
        // Command is rebuilt every refresh because the meeting id is
        // dynamic and Process.command is read at start time.
        command: [root.voxtypeBinary, "meeting", "show", root._meetingId || "latest"]
        running: false

        property string _buffer: ""

        stdout: SplitParser {
            splitMarker: "\n"
            onRead: function(line) { showProcess._buffer += line + "\n"; }
        }

        onRunningChanged: {
            if (!showProcess.running) {
                root._parseShowOutput(showProcess._buffer);
                showProcess._buffer = "";
            }
        }

        function refresh() {
            // Only poll when a meeting id is known. Without one,
            // `meeting show latest` would surface the most recent
            // completed meeting, which would be misleading.
            if (root._meetingId.length === 0) return;
            if (showProcess.running) return;
            showProcess._buffer = "";
            showProcess.running = true;
        }
    }

    // 2 Hz poll while a meeting is active. Stops itself when the
    // meeting ends so an idle voxtype install doesn't spawn a
    // subprocess every 500 ms forever.
    Timer {
        id: showPollTimer
        interval: 2000
        repeat: true
        running: root._meetingId.length > 0
                 && (root._meetingStatus === "recording"
                     || root._meetingStatus === "paused")
        onTriggered: showProcess.refresh()
    }

    // 1 Hz wall-clock tick for the elapsed counter. Independent of
    // the show-poll so the displayed mm:ss never appears to freeze.
    Timer {
        id: elapsedTimer
        interval: 1000
        repeat: true
        running: root._meetingStatus === "recording" && root._startedAtMs > 0
        onTriggered: root._recomputeElapsed()
    }

    // Transient status text fades after 3 s so a successful "Stop
    // requested" doesn't sit there until the daemon catches up.
    Timer {
        id: actionStatusTimer
        interval: 3000
        repeat: false
        onTriggered: root._actionStatus = ""
    }

    // ----- action processes (start / stop / pause / resume) -----

    Process {
        id: startProcess
        command: [root.voxtypeBinary, "meeting", "start"]
        running: false
        onRunningChanged: if (!startProcess.running) root._actionDone("Start requested")
    }

    Process {
        id: stopProcess
        command: [root.voxtypeBinary, "meeting", "stop"]
        running: false
        onRunningChanged: if (!stopProcess.running) root._actionDone("Stop requested")
    }

    Process {
        id: pauseProcess
        command: [root.voxtypeBinary, "meeting", "pause"]
        running: false
        onRunningChanged: if (!pauseProcess.running) root._actionDone("Pause requested")
    }

    Process {
        id: resumeProcess
        command: [root.voxtypeBinary, "meeting", "resume"]
        running: false
        onRunningChanged: if (!resumeProcess.running) root._actionDone("Resume requested")
    }

    // ----- helpers -----

    function _zeroPad(n) {
        return (n < 10 ? "0" : "") + n;
    }

    function _formatElapsed(secs) {
        if (secs <= 0) return "00:00";
        const h = Math.floor(secs / 3600);
        const m = Math.floor((secs % 3600) / 60);
        const s = secs % 60;
        if (h > 0) return _zeroPad(h) + ":" + _zeroPad(m) + ":" + _zeroPad(s);
        return _zeroPad(m) + ":" + _zeroPad(s);
    }

    function _recomputeElapsed() {
        if (root._startedAtMs <= 0) {
            root._elapsedSecs = 0;
            return;
        }
        root._elapsedSecs = Math.max(0, Math.floor((Date.now() - root._startedAtMs) / 1000));
    }

    function _parseShowOutput(buf) {
        // Parses the human-readable `voxtype meeting show` output.
        // Format (see src/main.rs `MeetingAction::Show`):
        //
        //   <title>
        //   ============
        //
        //   ID:       <uuid>
        //   Started:  2026-05-17 19:42 UTC
        //   Ended:    2026-05-17 19:45 UTC   (only when ended)
        //   Duration: 0m 12s                  (only when ended)
        //   Status:   Active
        //   Chunks:   3
        //   ...
        //
        // Plain-text parsing is fragile, but the format is exercised
        // by the regression suite (`test_meeting_show_*`), so an
        // unannounced change would fail CI before reaching users.
        if (!buf || buf.length === 0) return;
        const lines = buf.split("\n");
        if (lines.length === 0) return;

        // Title is the first non-empty line.
        let title = "";
        for (let i = 0; i < lines.length; ++i) {
            const t = lines[i].trim();
            if (t.length > 0) {
                title = t;
                break;
            }
        }

        let startedAt = 0;
        let chunks = 0;
        for (let i = 0; i < lines.length; ++i) {
            const line = lines[i];
            if (line.indexOf("Started:") === 0) {
                // "Started:  2026-05-17 19:42 UTC"
                const value = line.substring(8).trim();
                const parsed = Date.parse(value.replace(" UTC", "Z").replace(" ", "T"));
                if (!isNaN(parsed)) startedAt = parsed;
            } else if (line.indexOf("Chunks:") === 0) {
                const value = line.substring(7).trim();
                const n = parseInt(value, 10);
                if (!isNaN(n)) chunks = n;
            }
        }

        if (title.length > 0 && title !== root._meetingTitle) {
            root._meetingTitle = title;
        }
        if (startedAt > 0 && startedAt !== root._startedAtMs) {
            root._startedAtMs = startedAt;
            root._recomputeElapsed();
        }
        if (chunks !== root._chunkCount) {
            root._chunkCount = chunks;
        }
    }

    function _actionDone(label) {
        root._actionStatus = label;
        actionStatusTimer.restart();
        // Nudge the state machine so the buttons update without
        // waiting for the next inotify cycle.
        stateFile.reload();
        if (root._meetingId.length > 0) {
            showProcess.refresh();
        }
    }

    function _runStart() {
        if (startProcess.running) return;
        root._actionStatus = "Starting...";
        startProcess.running = true;
    }
    function _runStop() {
        if (stopProcess.running) return;
        root._actionStatus = "Stopping...";
        stopProcess.running = true;
    }
    function _runPause() {
        if (pauseProcess.running) return;
        root._actionStatus = "Pausing...";
        pauseProcess.running = true;
    }
    function _runResume() {
        if (resumeProcess.running) return;
        root._actionStatus = "Resuming...";
        resumeProcess.running = true;
    }

    function _close() {
        root.open = false;
    }

    // ----- card UI -----

    Rectangle {
        id: card
        width: 380
        height: 200
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.verticalCenter: parent.verticalCenter
        radius: VT.Theme.cornerRadius
        color: VT.Theme.bgColor
        border.width: 2
        border.color: root._meetingStatus === "recording" ? VT.Theme.recordingColor
                    : root._meetingStatus === "paused"    ? VT.Theme.transcribingColor
                    :                                       VT.Theme.idleColor

        // Esc closes; 1..4 trigger the four actions when they apply.
        // The Item handles focus inside the layer-shell surface
        // because PanelWindow itself isn't a focus scope.
        focus: true
        Keys.onEscapePressed: root._close()
        Keys.onPressed: function(event) {
            if (event.key === Qt.Key_1 && _startEnabled) {
                root._runStart();
                event.accepted = true;
            } else if (event.key === Qt.Key_2 && _stopEnabled) {
                root._runStop();
                event.accepted = true;
            } else if (event.key === Qt.Key_3 && _pauseEnabled) {
                root._runPause();
                event.accepted = true;
            } else if (event.key === Qt.Key_4 && _resumeEnabled) {
                root._runResume();
                event.accepted = true;
            }
        }

        // Button enable rules track the daemon's state machine:
        //   idle      -> only Start
        //   recording -> Stop + Pause
        //   paused    -> Stop + Resume
        readonly property bool _startEnabled: root._meetingStatus === "idle"
        readonly property bool _stopEnabled: root._meetingStatus === "recording"
                                          || root._meetingStatus === "paused"
        readonly property bool _pauseEnabled: root._meetingStatus === "recording"
        readonly property bool _resumeEnabled: root._meetingStatus === "paused"

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: VT.Theme.padding
            spacing: 10

            // --- header: title + close hint ---
            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Text {
                    Layout.fillWidth: true
                    text: root._meetingStatus === "idle"
                          ? "No active meeting"
                          : (root._meetingTitle.length > 0
                             ? root._meetingTitle
                             : "Meeting " + root._meetingId.substring(0, 8))
                    font.family: "JetBrainsMono Nerd Font"
                    font.bold: true
                    font.pixelSize: 16
                    color: VT.Theme.textColor
                    elide: Text.ElideRight
                }

                Text {
                    text: "Esc"
                    font.family: "JetBrainsMono Nerd Font"
                    font.pixelSize: 11
                    color: VT.Theme.idleColor
                    opacity: 0.7
                }
            }

            // --- stats row: state + elapsed + chunks ---
            RowLayout {
                Layout.fillWidth: true
                spacing: 18

                Text {
                    text: root._meetingStatus === "recording" ? "Recording"
                        : root._meetingStatus === "paused"    ? "Paused"
                        :                                       "Idle"
                    font.family: "JetBrainsMono Nerd Font"
                    font.pixelSize: 12
                    color: card.border.color
                }

                Text {
                    text: "Elapsed " + root._formatElapsed(root._elapsedSecs)
                    font.family: "JetBrainsMono Nerd Font"
                    font.pixelSize: 12
                    color: VT.Theme.textColor
                    opacity: root._meetingStatus === "idle" ? 0.5 : 1.0
                }

                Text {
                    text: "Chunks " + root._chunkCount
                    font.family: "JetBrainsMono Nerd Font"
                    font.pixelSize: 12
                    color: VT.Theme.textColor
                    opacity: root._meetingStatus === "idle" ? 0.5 : 1.0
                }

                Item { Layout.fillWidth: true }
            }

            // --- button row ---
            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                MeetingButton {
                    label: "Start"
                    shortcut: "1"
                    enabled: card._startEnabled
                    onClicked: root._runStart()
                }
                MeetingButton {
                    label: "Stop"
                    shortcut: "2"
                    enabled: card._stopEnabled
                    onClicked: root._runStop()
                }
                MeetingButton {
                    label: "Pause"
                    shortcut: "3"
                    enabled: card._pauseEnabled
                    onClicked: root._runPause()
                }
                MeetingButton {
                    label: "Resume"
                    shortcut: "4"
                    enabled: card._resumeEnabled
                    onClicked: root._runResume()
                }
            }

            // --- transient action status line ---
            Text {
                Layout.fillWidth: true
                text: root._actionStatus
                visible: root._actionStatus.length > 0
                font.family: "JetBrainsMono Nerd Font"
                font.pixelSize: 11
                color: VT.Theme.streamingColor
                elide: Text.ElideRight
            }
        }
    }

    // ----- inline component definition: button -----
    //
    // Defined as an inline component so MeetingControls.qml stays a
    // single file. If a second widget ever needs the same button
    // styling, lift it into voxtype-shared.

    component MeetingButton: Rectangle {
        id: btn
        property string label: ""
        property string shortcut: ""
        signal clicked()

        // `enabled` is inherited from Item. Qt's built-in property already
        // controls visual disabled state and gates input events on children
        // (including the MouseArea below), so a redeclaration would only
        // shadow it and trigger a QML propertyCache warning.

        Layout.fillWidth: true
        Layout.preferredHeight: 36
        radius: 6
        color: mouse.pressed && btn.enabled ? Qt.darker(VT.Theme.accentColor, 1.4)
              : mouse.containsMouse && btn.enabled ? Qt.darker(VT.Theme.accentColor, 1.2)
              : Qt.rgba(1, 1, 1, 0.06)
        border.width: 1
        border.color: btn.enabled ? VT.Theme.accentColor : VT.Theme.idleColor
        opacity: btn.enabled ? 1.0 : 0.4

        Row {
            anchors.centerIn: parent
            spacing: 6

            Text {
                text: btn.label
                font.family: "JetBrainsMono Nerd Font"
                font.pixelSize: 13
                font.bold: true
                color: VT.Theme.textColor
                anchors.verticalCenter: parent.verticalCenter
            }

            Text {
                visible: btn.shortcut.length > 0
                text: "[" + btn.shortcut + "]"
                font.family: "JetBrainsMono Nerd Font"
                font.pixelSize: 10
                color: VT.Theme.idleColor
                anchors.verticalCenter: parent.verticalCenter
            }
        }

        MouseArea {
            id: mouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: btn.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: { if (btn.enabled) btn.clicked(); }
        }
    }
}
