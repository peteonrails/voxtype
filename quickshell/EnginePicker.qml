// Voxtype engine picker popup for Quickshell.
//
// A floating panel that lists every transcription engine voxtype knows
// about, marks the one currently selected in the on-disk config, and
// switches the active engine by shelling out to
// `voxtype config set engine <name>` (added in PR #382). No new daemon
// IPC is introduced.
//
// ## Read path
//
// 1. Available engines: the canonical list of 8 transcription engines
//    is hardcoded here to match `ENGINE_NAMES` in `src/config_set.rs`.
//    Whether the running binary was compiled with the matching Cargo
//    feature is discovered by spawning `voxtype info variants --json`
//    once when the panel opens and reading the `compiled_features`
//    array (fixed in #384 to enumerate every ONNX engine, not just
//    parakeet + GPU backends). Engines absent from that array are
//    rendered dim with a "(not compiled)" suffix so the user can see
//    why a row won't switch. When the JSON parse fails (older binary
//    that predates #384, or `--json` returning text), the picker
//    falls back to showing every engine as available and relies on
//    `voxtype config set engine`'s feature gate to reject a request
//    that targets an uncompiled engine. Whisper is always available;
//    it is not a Cargo feature.
//
// 2. Currently-active engine: read from
//    `~/.config/voxtype/config.toml` via FileView. The file is parsed
//    line-by-line for `engine = "<name>"`; if the key is absent the
//    default is whisper (matches `TranscriptionEngine::default()`).
//
// ## Open/close trigger
//
// Mirrors `MeetingControls.qml`: the picker watches a flag file at
// `$XDG_RUNTIME_DIR/voxtype/engine-picker.flag`. Touching the flag
// toggles visibility; the picker removes the flag on read so a
// subsequent `touch` reliably retoggles.
//
//   # Hyprland
//   bind = SUPER, E, exec, touch $XDG_RUNTIME_DIR/voxtype/engine-picker.flag
//
//   # Sway
//   bindsym $mod+e exec touch $XDG_RUNTIME_DIR/voxtype/engine-picker.flag
//
// ## Switching
//
// On Enter or click, the picker spawns
// `voxtype config set engine <name>`. Once the process exits, the
// picker re-reads the config file and inspects the CLI's stderr to
// classify the outcome:
//
//   config now contains <name>            → success
//   stderr matches "not compiled"/"unknown engine" → feature gate failure
//   anything else                         → I/O failure
//
// Quickshell 0.2.1's Process binding doesn't reliably surface the
// child exit code as a QML property across all distro builds, so the
// dispatch leans on the post-mutation config file as the source of
// truth and uses stderr only to render an accurate error message when
// the mutation didn't happen.
//
// On success the panel auto-closes after ~1.5 s so the user reads the
// confirmation before it disappears. The picker does NOT restart the
// daemon automatically; restart is the user's choice and matches the
// behavior of the CLI command.
//
// ## Why this lives outside shell.qml
//
// shell.qml is the OSD composition root and is wired up by the
// maintainer. This file is a self-contained sibling: it exposes a
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

    /// Path or PATH-resolvable name of the voxtype CLI. Mirrors
    /// MeetingControls so a single override flips both widgets.
    property string voxtypeBinary: "voxtype"

    /// Directory containing the daemon's runtime files (the
    /// `engine-picker.flag` toggle). Mirrors the resolution in
    /// `Config::runtime_dir()` so the widget never disagrees with the
    /// daemon about where to look.
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

    /// Path to the on-disk config file. Mirrors
    /// `Config::default_path()`: `$XDG_CONFIG_HOME/voxtype/config.toml`
    /// with a `$HOME/.config/voxtype/config.toml` fallback.
    property string configPath: {
        const xdgConfig = Quickshell.env("XDG_CONFIG_HOME");
        if (xdgConfig && xdgConfig.length > 0) {
            return xdgConfig + "/voxtype/config.toml";
        }
        const home = Quickshell.env("HOME");
        if (home && home.length > 0) {
            return home + "/.config/voxtype/config.toml";
        }
        return "/tmp/voxtype-config.toml";
    }

    /// Whether the panel is currently visible. Compositors can flip
    /// this directly as an alternative to the flag-file trigger.
    property bool open: false

    // ----- derived state -----

    /// Currently-selected engine name (the one written to config.toml).
    /// Falls back to "whisper" when the key is absent or unreadable.
    readonly property string activeEngine: _activeEngine

    /// List of engines the running binary advertises via
    /// `voxtype info variants --json` `compiled_features`. Used to
    /// distinguish "definitely available" from "may not be compiled
    /// in" in the row styling.
    readonly property var compiledFeatures: _compiledFeatures

    // ----- panel surface -----

    visible: root.open
    color: "transparent"
    anchors { top: true; bottom: true; left: true; right: true }
    exclusionMode: ExclusionMode.Ignore

    WlrLayershell.namespace: "voxtype-engine-picker"
    WlrLayershell.layer: WlrLayer.Top
    WlrLayershell.keyboardFocus: root.open ? WlrKeyboardFocus.Exclusive
                                           : WlrKeyboardFocus.None

    // ----- engine catalog -----

    // Hardcoded list keeps the UI deterministic and matches
    // `ENGINE_NAMES` in src/config_set.rs. If a new engine lands
    // upstream, add it here.
    readonly property var engines: [
        { name: "whisper",     label: "Whisper",     blurb: "whisper.cpp default; CPU/GPU" },
        { name: "parakeet",    label: "Parakeet",    blurb: "NVIDIA FastConformer; fast English" },
        { name: "moonshine",   label: "Moonshine",   blurb: "Encoder-decoder ASR" },
        { name: "sensevoice",  label: "SenseVoice",  blurb: "Alibaba multilingual CTC" },
        { name: "paraformer",  label: "Paraformer",  blurb: "FunASR CTC encoder" },
        { name: "dolphin",     label: "Dolphin",     blurb: "Dictation-optimized CTC" },
        { name: "omnilingual", label: "Omnilingual", blurb: "FunASR 50+ languages" },
        { name: "cohere",      label: "Cohere",      blurb: "Whisper-style; top of OpenASR" }
    ]

    // ----- internal state -----

    property string _activeEngine: "whisper"
    property var _compiledFeatures: []
    property int _selectedIndex: 0
    // Transient line for switch progress / errors. Cleared after a few
    // seconds or replaced by the next action.
    property string _actionStatus: ""
    // "info" | "ok" | "error" — drives the status line color.
    property string _actionKind: "info"
    // Engine the user just attempted to switch to. Used by exit-code
    // dispatch to compose accurate status messages without races on
    // `_selectedIndex` (the user could have moved the cursor between
    // pressing Enter and the process exit).
    property string _pendingEngine: ""

    // ----- config.toml watcher (read current engine) -----

    FileView {
        id: configFile
        path: root.configPath
        watchChanges: true
        printErrors: false

        onLoaded: {
            const parsed = root._parseEngineFromToml(text() || "");
            if (parsed !== root._activeEngine) {
                root._activeEngine = parsed;
            }
            // Move the selection to the active engine the first time
            // we load (or after a switch). The user can still arrow
            // away before pressing Enter.
            const idx = root._indexOfEngine(parsed);
            if (idx >= 0) {
                root._selectedIndex = idx;
            }
        }

        onLoadFailed: {
            // No config file yet → effective engine is the default
            // (whisper). Keep cursor on Whisper so the empty state is
            // obvious.
            if (root._activeEngine !== "whisper") {
                root._activeEngine = "whisper";
            }
        }

        onFileChanged: reload()
    }

    // ----- engine-picker.flag (toggle visibility) -----

    FileView {
        id: flagFile
        path: root.runtimeDir + "/engine-picker.flag"
        watchChanges: true
        printErrors: false

        onLoaded: {
            root.open = !root.open;
            removeFlagProcess.start();
        }

        onFileChanged: reload()
    }

    Process {
        id: removeFlagProcess
        command: ["rm", "-f", root.runtimeDir + "/engine-picker.flag"]
        running: false

        function start() {
            if (!removeFlagProcess.running) {
                removeFlagProcess.running = true;
            }
        }
    }

    // ----- `voxtype info variants --json` (compiled features) -----

    Process {
        id: featuresProcess
        command: [root.voxtypeBinary, "info", "variants", "--json"]
        running: false

        property string _buffer: ""

        stdout: SplitParser {
            splitMarker: "\n"
            onRead: function(line) { featuresProcess._buffer += line + "\n"; }
        }

        onRunningChanged: {
            if (!featuresProcess.running) {
                root._parseFeaturesJson(featuresProcess._buffer);
                featuresProcess._buffer = "";
            }
        }

        function refresh() {
            if (featuresProcess.running) return;
            featuresProcess._buffer = "";
            featuresProcess.running = true;
        }
    }

    // ----- `voxtype config set engine <name>` (switch) -----

    Process {
        id: switchProcess
        // Command is rebuilt on each invocation since the target
        // engine changes; Process.command is read at start time so
        // updating it before flipping `running` is sufficient.
        command: [root.voxtypeBinary, "config", "set", "engine", "whisper"]
        running: false

        // stderr accumulator. The CLI's error messages identify the
        // failure mode by phrase (e.g. "is not compiled into this
        // binary"), which is the authoritative way to distinguish
        // exit code 2 from exit code 1 without depending on a
        // potentially-absent `exitCode` property on Quickshell's
        // Process binding.
        property string _stderrBuffer: ""

        stderr: SplitParser {
            splitMarker: "\n"
            onRead: function(line) {
                switchProcess._stderrBuffer += line + "\n";
            }
        }

        onRunningChanged: {
            if (!switchProcess.running) {
                root._handleSwitchExit(switchProcess._stderrBuffer);
                switchProcess._stderrBuffer = "";
            }
        }
    }

    Timer {
        id: actionStatusTimer
        interval: 3000
        repeat: false
        onTriggered: {
            root._actionStatus = "";
            root._actionKind = "info";
        }
    }

    // Auto-close after a successful switch so the user sees the
    // confirmation briefly before the panel disappears.
    Timer {
        id: autoCloseTimer
        interval: 1500
        repeat: false
        onTriggered: root._close()
    }

    // ----- lifecycle -----

    // Refresh feature list whenever the panel opens. Cheap (one
    // subprocess) and the user's binary could have been swapped
    // between opens via `voxtype-bin` package upgrades.
    onOpenChanged: {
        if (root.open) {
            configFile.reload();
            featuresProcess.refresh();
            // Reset transient state so a previous "Failed to..." line
            // doesn't carry over into a fresh open.
            root._actionStatus = "";
            root._actionKind = "info";
            autoCloseTimer.stop();
        }
    }

    // ----- helpers -----

    function _indexOfEngine(name) {
        for (let i = 0; i < root.engines.length; ++i) {
            if (root.engines[i].name === name) return i;
        }
        return -1;
    }

    function _parseEngineFromToml(content) {
        // Minimal TOML scan: find `engine = "<value>"` at the top
        // level. The setting lives on the root table in voxtype's
        // schema (see `Config.engine` in src/config.rs), so we stop
        // scanning when a `[section]` header is reached to avoid
        // matching a hypothetical `[xyz] engine = ...` inside a
        // nested table. Comments after the value are tolerated.
        if (!content || content.length === 0) return "whisper";
        const lines = content.split("\n");
        for (let i = 0; i < lines.length; ++i) {
            const raw = lines[i];
            const line = raw.replace(/^\s+/, "");
            if (line.length === 0 || line[0] === "#") continue;
            if (line[0] === "[") break;
            // Match: engine = "<name>"  (single or double quotes,
            // optional trailing comment).
            const m = line.match(/^engine\s*=\s*["']([^"']+)["']/);
            if (m) {
                return m[1];
            }
        }
        return "whisper";
    }

    function _parseFeaturesJson(buf) {
        if (!buf || buf.length === 0) return;
        try {
            const obj = JSON.parse(buf);
            if (obj && Array.isArray(obj.compiled_features)) {
                root._compiledFeatures = obj.compiled_features;
            } else {
                root._compiledFeatures = [];
            }
        } catch (e) {
            // Older voxtype binaries that predate `info variants
            // --json` print human-readable text; treat as "unknown
            // features" and let the switch-time check be authoritative.
            root._compiledFeatures = [];
        }
    }

    function _engineAvailable(name) {
        // Whisper is always available; it's not a Cargo feature.
        if (name === "whisper") return true;
        // If we have no feature info (older binary, parse failure),
        // optimistically show every engine as available and let
        // `config set engine` reject unsupported ones at switch time.
        if (root._compiledFeatures.length === 0) return true;
        return root._compiledFeatures.indexOf(name) >= 0;
    }

    function _selectEngine(idx) {
        if (idx < 0 || idx >= root.engines.length) return;
        root._selectedIndex = idx;
    }

    function _commit() {
        const idx = root._selectedIndex;
        if (idx < 0 || idx >= root.engines.length) return;
        const name = root.engines[idx].name;
        if (switchProcess.running) return;
        root._pendingEngine = name;
        root._actionKind = "info";
        root._actionStatus = "Switching to " + name + "...";
        actionStatusTimer.stop();
        autoCloseTimer.stop();
        switchProcess.command = [
            root.voxtypeBinary, "config", "set", "engine", name
        ];
        switchProcess.running = true;
    }

    function _handleSwitchExit(stderrText) {
        const name = root._pendingEngine;
        // Force a config-file re-read first; FileView will fire
        // onLoaded synchronously below (Quickshell reads synchronously
        // for local files), updating `_activeEngine` in time for the
        // success check.
        configFile.reload();

        const stderr = (stderrText || "");
        const notCompiled = stderr.indexOf("not compiled") >= 0
                         || stderr.indexOf("unknown engine") >= 0;

        if (root._activeEngine === name) {
            root._actionKind = "ok";
            root._actionStatus = "Switched to " + name + ". Restart voxtype to apply.";
            autoCloseTimer.restart();
        } else if (notCompiled) {
            root._actionKind = "error";
            root._actionStatus = "Engine " + name + " isn't compiled into this binary.";
            actionStatusTimer.restart();
        } else {
            root._actionKind = "error";
            root._actionStatus = "Failed to write config (see voxtype logs).";
            actionStatusTimer.restart();
        }
        root._pendingEngine = "";
    }

    function _close() {
        root.open = false;
        autoCloseTimer.stop();
    }

    // ----- card UI -----

    Rectangle {
        id: card
        width: 420
        // Height grows with the engine list + chrome.
        // Header (~28) + 8 rows (~36 each) + status line (~20) + padding.
        height: 28 + root.engines.length * 36 + 20 + 2 * VT.Theme.padding + 16
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.verticalCenter: parent.verticalCenter
        radius: VT.Theme.cornerRadius
        color: VT.Theme.bgColor
        border.width: 2
        border.color: VT.Theme.accentColor

        focus: true

        Keys.onEscapePressed: root._close()
        Keys.onUpPressed: function(event) {
            const next = root._selectedIndex > 0
                       ? root._selectedIndex - 1
                       : root.engines.length - 1;
            root._selectEngine(next);
            event.accepted = true;
        }
        Keys.onDownPressed: function(event) {
            const next = (root._selectedIndex + 1) % root.engines.length;
            root._selectEngine(next);
            event.accepted = true;
        }
        Keys.onReturnPressed: function(event) {
            root._commit();
            event.accepted = true;
        }
        Keys.onEnterPressed: function(event) {
            root._commit();
            event.accepted = true;
        }
        Keys.onPressed: function(event) {
            // Number keys 1..8 jump-select then commit.
            if (event.key >= Qt.Key_1 && event.key <= Qt.Key_9) {
                const idx = event.key - Qt.Key_1;
                if (idx < root.engines.length) {
                    root._selectEngine(idx);
                    root._commit();
                    event.accepted = true;
                }
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: VT.Theme.padding
            spacing: 8

            // --- header ---
            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Text {
                    Layout.fillWidth: true
                    text: "Transcription engine"
                    font.family: "JetBrainsMono Nerd Font"
                    font.bold: true
                    font.pixelSize: 16
                    color: VT.Theme.textColor
                }

                Text {
                    text: "Esc"
                    font.family: "JetBrainsMono Nerd Font"
                    font.pixelSize: 11
                    color: VT.Theme.idleColor
                    opacity: 0.7
                }
            }

            // --- engine rows ---
            Repeater {
                model: root.engines

                EngineRow {
                    Layout.fillWidth: true
                    engineName: modelData.name
                    engineLabel: modelData.label
                    blurb: modelData.blurb
                    shortcut: (index + 1).toString()
                    isActive: modelData.name === root._activeEngine
                    isSelected: index === root._selectedIndex
                    isAvailable: root._engineAvailable(modelData.name)

                    // Mouse click jumps the keyboard cursor to this
                    // row and commits in one go. Hover is intentionally
                    // not bound to selection so mouse motion across
                    // the panel doesn't fight a user who's
                    // arrow-keying through the list.
                    onClicked: {
                        root._selectEngine(index);
                        root._commit();
                    }
                }
            }

            // --- transient status line ---
            Text {
                Layout.fillWidth: true
                text: root._actionStatus
                visible: root._actionStatus.length > 0
                font.family: "JetBrainsMono Nerd Font"
                font.pixelSize: 11
                color: root._actionKind === "ok"    ? VT.Theme.streamingColor
                     : root._actionKind === "error" ? VT.Theme.recordingColor
                     :                                VT.Theme.textColor
                wrapMode: Text.WordWrap
            }
        }
    }

    // ----- inline component definition: engine row -----
    //
    // Mirrors MeetingControls' inline MeetingButton: defined inline so
    // EnginePicker.qml stays a single file. Lift into voxtype-shared
    // if a third widget ever needs the same row styling.

    component EngineRow: Rectangle {
        id: row
        property string engineName: ""
        property string engineLabel: ""
        property string blurb: ""
        property string shortcut: ""
        property bool isActive: false
        property bool isSelected: false
        property bool isAvailable: true

        signal clicked()

        Layout.preferredHeight: 36
        radius: 6
        color: row.isSelected ? Qt.darker(VT.Theme.accentColor, 1.6)
              : mouse.containsMouse ? Qt.rgba(1, 1, 1, 0.08)
              : Qt.rgba(1, 1, 1, 0.03)
        border.width: row.isSelected ? 1 : 0
        border.color: VT.Theme.accentColor
        opacity: row.isAvailable ? 1.0 : 0.55

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 10
            anchors.rightMargin: 10
            spacing: 10

            // Active-engine checkmark; reserves the slot when inactive
            // so the label column stays aligned across all rows.
            Text {
                Layout.preferredWidth: 14
                text: row.isActive ? "✓" : ""
                font.family: "JetBrainsMono Nerd Font"
                font.pixelSize: 14
                font.bold: true
                color: VT.Theme.streamingColor
                horizontalAlignment: Text.AlignHCenter
            }

            Text {
                text: row.engineLabel
                font.family: "JetBrainsMono Nerd Font"
                font.pixelSize: 13
                font.bold: row.isActive
                color: row.isActive ? VT.Theme.accentColor : VT.Theme.textColor
            }

            Text {
                Layout.fillWidth: true
                text: row.isAvailable
                      ? row.blurb
                      : (row.blurb + "  (not compiled)")
                font.family: "JetBrainsMono Nerd Font"
                font.pixelSize: 11
                color: VT.Theme.idleColor
                elide: Text.ElideRight
            }

            Text {
                visible: row.shortcut.length > 0
                text: "[" + row.shortcut + "]"
                font.family: "JetBrainsMono Nerd Font"
                font.pixelSize: 10
                color: VT.Theme.idleColor
                opacity: 0.7
            }
        }

        MouseArea {
            id: mouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: row.clicked()
        }
    }
}
