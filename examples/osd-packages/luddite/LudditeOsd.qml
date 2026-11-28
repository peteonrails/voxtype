import QtQuick

// Luddite OSD: a compact dictation popup in Omarchy Shell's popup chrome.
//
// One row: a microphone icon, a phase label over a level meter, and the
// elapsed time. While recording, the meter shows the input peak on a
// -60..0 dBFS scale, the label warns when the input has been very quiet
// for a second, and the icon and label turn to the theme's error color
// when the system default microphone is muted. While transcribing, an
// hourglass shows and a short bar travels along the meter track.
//
// The package's manifest selects the `shell` layout, so the launcher
// resolves Omarchy's popup tokens into theme.config.shell and the frame
// matches the shell's other popups. Ported from the Luddite Voice OSD
// Omarchy plugin.

Item {
    id: root

    property string daemonState: "idle"
    property var audio: null
    property var theme: null
    property var recipe: null
    property string assetRoot: ""
    property var session: null
    property var outcome: null

    // Bumped when the resolved style changes, so color bindings refresh
    // on an Omarchy theme switch.
    property int _themeRev: 0

    readonly property bool recording: daemonState === "recording" || daemonState === "streaming"
    readonly property bool busy: daemonState === "transcribing"
    readonly property bool shown: recording || busy
    readonly property bool muted: recording && _mute !== null && _mute.muted

    // Level tracking, written per audio frame.
    property real heldDb: -120
    property real lastFrameMs: 0
    property real lastLoudMs: 0
    property real phaseStartMs: Date.now()
    property real nowMs: Date.now()

    readonly property real elapsedSecs: {
        const start = recording && session && session.started_at_ms ? session.started_at_ms : phaseStartMs;
        return Math.max(0, Math.floor((nowMs - start) / 1000));
    }
    readonly property bool quiet: recording && !muted && elapsedSecs >= 2
        && (nowMs - lastFrameMs > 1000 || nowMs - lastLoudMs > 1000)

    // Shell chrome resolved by the launcher, with shell defaults.
    readonly property var shell: { _themeRev; return theme && theme.config ? theme.config.shell : null; }
    readonly property real scaleFactor: Math.max(0.5, Math.min(3, _shellNumber("spacing_scale", 1)))
    readonly property real fontSize: _shellNumber("font_size", 12)
    readonly property real borderWidth: _shellNumber("border_width", 2)

    readonly property color textColor: { _themeRev; return _color(shell && shell.text ? shell.text : "foreground", "#dcdfe4"); }
    readonly property color urgentColor: { _themeRev; return _color("error", "#e06c75"); }
    readonly property color accentColor: { _themeRev; return _color("accent", "#66c7ff"); }
    readonly property color backgroundColor: { _themeRev; return Qt.alpha(_color("background", "#1a1a1f"), 0.97); }
    readonly property var borderColors: {
        _themeRev;
        const stops = shell && shell.border_colors && shell.border_colors.length > 0 ? shell.border_colors : ["accent"];
        return stops.map(function(c) { return root._color(c, root.accentColor); });
    }

    // Sizes follow the shell's spacing scale and font size.
    readonly property real pad: 16 * scaleFactor
    readonly property real gap: 16 * scaleFactor
    readonly property real iconSize: Math.max(28 * scaleFactor, fontSize * 2)
    readonly property real meterHeight: Math.max(2, 6 * scaleFactor)
    readonly property real detailsWidth: Math.max(142 * scaleFactor,
        Math.min(190 * scaleFactor, Math.ceil(Math.max(quietMetrics.advanceWidth, phaseMetrics.advanceWidth))))
    readonly property real elapsedWidth: Math.ceil(elapsedMetrics.advanceWidth)
    readonly property real cardWidth: Math.min(
        2 * borderWidth + 2 * pad + iconSize + detailsWidth + elapsedWidth + 2 * gap,
        Math.max(1, width - 2 * _margin()))
    readonly property real cardHeight: 2 * borderWidth + 2 * pad
        + Math.max(iconSize, phaseMetrics.height + 4 * scaleFactor + meterHeight, elapsedMetrics.height)

    onDaemonStateChanged: {
        phaseStartMs = Date.now();
        nowMs = phaseStartMs;
        if (recording) {
            heldDb = -120;
            lastFrameMs = 0;
            lastLoudMs = phaseStartMs;
        }
    }

    Connections {
        target: root.theme
        ignoreUnknownSignals: true
        function onConfigChanged() { root._themeRev++; }
    }

    Connections {
        target: root.audio
        ignoreUnknownSignals: true
        function onFrameReceived(framePeak, frameRms, vad, tsMs) {
            if (!root.recording) return;
            const now = Date.now();
            const db = framePeak > 0 ? 20 * Math.log10(framePeak) : -120;
            root.lastFrameMs = now;
            if (db >= -65) root.lastLoudMs = now;
            if (db > root.heldDb) root.heldDb = db;
        }
    }

    // Clock and peak decay, only while the popup is shown.
    Timer {
        interval: 50
        repeat: true
        running: root.shown
        onTriggered: {
            root.nowMs = Date.now();
            root.heldDb = Math.max(-120, root.heldDb - 2);
        }
    }

    function _shellNumber(key, fallback) {
        const value = shell && shell[key] !== undefined ? Number(shell[key]) : fallback;
        return isFinite(value) ? value : fallback;
    }

    function _color(role, fallback) {
        if (theme && theme.color) return theme.color(role, fallback);
        return fallback;
    }

    function _config(key, fallback) {
        return theme && theme.config && theme.config[key] !== undefined ? theme.config[key] : fallback;
    }

    function _margin() {
        return Math.max(0, Number(_config("margin_px", 24)));
    }

    function _cardX() {
        const position = String(_config("position", "bottom-center"));
        const margin = _margin();
        if (position.indexOf("left") >= 0) return margin;
        if (position.indexOf("right") >= 0) return Math.max(margin, width - cardWidth - margin);
        return Math.max(margin, (width - cardWidth) / 2);
    }

    // Same placement rules as the built-in card: corners sit margin_px
    // from their edges, centered positions use top_margin as a fraction of
    // the output height (mirrored for top-center above 0.5).
    function _cardY() {
        const position = String(_config("position", "bottom-center"));
        const margin = _margin();
        const maxY = Math.max(margin, height - cardHeight - margin);
        if (position === "top-left" || position === "top-right") return margin;
        if (position === "bottom-left" || position === "bottom-right") return maxY;
        const topMargin = Math.max(0, Math.min(1, Number(_config("top_margin", 0.85))));
        if (position === "top-center") {
            const fromTop = topMargin <= 0.5 ? height * topMargin : height * (1 - topMargin) - cardHeight;
            return Math.max(margin, Math.min(maxY, fromTop));
        }
        return Math.max(margin, Math.min(maxY, height * topMargin));
    }

    // The default input's mute state, from Quickshell's PipeWire service.
    // Built from a string so a Quickshell build without PipeWire support
    // only loses the mute indicator instead of failing the whole package.
    property var _mute: null
    Component.onCompleted: {
        try {
            _mute = Qt.createQmlObject(
                "import QtQuick; import Quickshell.Services.Pipewire; Item { id: watch; "
                + "readonly property var source: Pipewire.defaultAudioSource; "
                + "readonly property bool muted: source && source.audio ? source.audio.muted : false; "
                + "PwObjectTracker { objects: watch.source ? [watch.source] : [] } }",
                root, "luddite-osd-mute");
        } catch (e) {
            console.warn("luddite OSD: microphone mute state unavailable:", e);
        }
    }

    TextMetrics {
        id: phaseMetrics
        font.family: "monospace"
        font.pixelSize: root.fontSize * 1.167
        font.bold: true
        text: "Microphone muted"
    }
    TextMetrics {
        id: quietMetrics
        font: phaseMetrics.font
        text: "Input is very quiet"
    }
    TextMetrics {
        id: elapsedMetrics
        font: phaseMetrics.font
        text: "000s"
    }

    Rectangle {
        id: card
        visible: root.shown
        x: root._cardX()
        y: root._cardY()
        width: root.cardWidth
        height: root.cardHeight
        radius: root.theme ? root.theme.shellCornerRadius || 0 : 0
        color: root.backgroundColor

        LudditeEdge {
            anchors.fill: parent
            cornerRadius: card.radius
            thickness: root.borderWidth
            angle: root._shellNumber("border_angle", 0)
            opacity: root._shellNumber("border_alpha", 1)
            colors: root.borderColors
        }

        Row {
            anchors.fill: parent
            anchors.margins: root.borderWidth + root.pad
            spacing: Math.min(root.gap, Math.max(0, (width - root.iconSize - root.elapsedWidth) / 3))

            LudditeIcon {
                width: root.iconSize
                height: parent.height
                kind: root.busy ? "hourglass" : root.muted ? "muted" : "mic"
                color: root.muted ? root.urgentColor : root.textColor
                strokeWidth: 2.0
            }

            Column {
                anchors.verticalCenter: parent.verticalCenter
                width: Math.max(0, parent.width - root.iconSize - root.elapsedWidth - 2 * parent.spacing)
                spacing: 4 * root.scaleFactor

                Text {
                    width: parent.width
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    maximumLineCount: 1
                    font: phaseMetrics.font
                    text: root.busy ? "Processing…"
                        : root.muted ? "Microphone muted"
                        : root.quiet ? "Input is very quiet"
                        : "Dictating"
                    color: root.muted || root.quiet ? root.urgentColor : root.textColor
                }

                Item {
                    width: parent.width
                    height: root.meterHeight
                    clip: true

                    Rectangle {
                        anchors.fill: parent
                        color: Qt.alpha(root.textColor, 0.45)
                    }
                    Rectangle {
                        id: fill
                        property real travel: 0
                        height: parent.height
                        color: root.accentColor
                        width: root.busy ? parent.width * 0.25
                            : root.muted ? 0
                            : parent.width * Math.max(0, Math.min(1, (root.heldDb + 60) / 60))
                        x: root.busy ? travel * (parent.width - width) : 0

                        SequentialAnimation on travel {
                            running: root.busy
                            loops: Animation.Infinite
                            NumberAnimation { to: 1; duration: 750; easing.type: Easing.InOutSine }
                            NumberAnimation { to: 0; duration: 750; easing.type: Easing.InOutSine }
                        }
                    }
                }
            }

            Text {
                width: root.elapsedWidth
                anchors.verticalCenter: parent.verticalCenter
                textFormat: Text.PlainText
                horizontalAlignment: Text.AlignRight
                font: phaseMetrics.font
                color: root.textColor
                text: root.elapsedSecs + "s"
            }
        }
    }
}
