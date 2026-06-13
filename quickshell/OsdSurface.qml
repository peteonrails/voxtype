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
import QtQuick.Effects
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

    /// Resolved style loader from VT.StyleLoader. Provides semantic colors,
    /// visual recipe layers, and optional custom QML package entry.
    property var style: null

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

    // Subtract the whole panel area from the input region, so pointer
    // events fall through to windows underneath instead of getting
    // eaten by the transparent fullscreen-anchored surface.
    mask: Region {
        intersection: Intersection.Subtract
        x: 0; y: 0
        width: panel.width
        height: panel.height
    }

    // Per-state tint, shared by icon + card border so a Hyprland user
    // can read the daemon's state from screen-edge color alone.
    readonly property color stateColor:
        daemonState === "recording"    ? panel._styleColor("recording", VT.Theme.recordingColor)
      : daemonState === "streaming"    ? panel._styleColor("streaming", VT.Theme.streamingColor)
      : daemonState === "transcribing" ? panel._styleColor("transcribing", VT.Theme.transcribingColor)
      :                                  panel._styleColor("idle", VT.Theme.idleColor)

    // Ring of recent per-frame peaks (0.0..1.0). Capacity = 3 s @ 100 Hz.
    // Stored as a plain array; we shift() when full to keep newest-on-right.
    readonly property int waveformColumns: Math.round(VT.Theme.waveformWindowSecs * 100)
    property var ring: []

    // Peak meter state (kept in dBFS so the held-peak decay math matches
    // src/osd/visual.rs's PeakHold verbatim).
    property real currentPeakDbfs: -120
    property real heldDbfs: -120
    property real currentPeak: 0
    property real currentRms: 0
    property real lastFrameTsMs: 0
    readonly property string customQmlUrl: _customQmlUrl()
    readonly property bool hasCustomQml: customQmlUrl.length > 0
    /// Set when the custom QML package fails to load, so the built-in
    /// surface can take over instead of leaving the OSD blank.
    property bool customQmlFailed: false
    /// True while a custom QML package is selected and loading/loaded.
    /// Every built-in element keys its visibility off this so a broken
    /// package falls back to the default card.
    readonly property bool customActive: hasCustomQml && !customQmlFailed
    readonly property real voiceEnergy: Math.min(1.0, Math.max(0.0, currentRms * 6.0 + currentPeak * 0.85))
    readonly property real orbHaloEnergy: Math.min(1.0, Math.max(0.0, currentRms * 10.0 + currentPeak * 1.8))

    onCustomQmlUrlChanged: customQmlFailed = false

    function _styleColor(role, fallback) {
        if (style && style.color) {
            return style.color(role, fallback);
        }
        return fallback;
    }

    function _styleLayout() {
        return style && style.config && style.config.layout
            ? style.config.layout
            : "compact";
    }

    function _stylePosition() {
        return style && style.config && style.config.position
            ? String(style.config.position)
            : "bottom-center";
    }

    function _styleMarginPx() {
        const value = style && style.config && style.config.margin_px !== undefined
            ? Number(style.config.margin_px)
            : VT.Theme.marginPx;
        return Math.max(0, value);
    }

    function _styleTopMargin() {
        const value = style && style.config && style.config.top_margin !== undefined
            ? Number(style.config.top_margin)
            : 0.85;
        return Math.max(0.0, Math.min(1.0, value));
    }

    function _frameConfig() {
        return style && style.config && style.config.frame
            ? style.config.frame
            : { "background": "background", "border": "state", "glow": true, "halo": true };
    }

    function _frameValue(key, fallback) {
        const frame = _frameConfig();
        const value = frame && frame[key] !== undefined ? String(frame[key]) : fallback;
        return value.length > 0 ? value : fallback;
    }

    function _frameBackgroundColor() {
        const value = _frameValue("background", "background");
        if (value === "none" || value === "transparent") return "transparent";
        return panel._styleColor(value, VT.Theme.bgColor);
    }

    function _frameBorderColor() {
        const value = _frameValue("border", "state");
        if (value === "none" || value === "transparent") return "transparent";
        if (value === "state") return panel.stateColor;
        return panel._styleColor(value, panel.stateColor);
    }

    function _frameBorderWidth() {
        const value = _frameValue("border", "state");
        return value === "none" || value === "transparent" ? 0 : 2;
    }

    function _frameGlowEnabled() {
        const frame = _frameConfig();
        return frame.glow === undefined ? true : !!frame.glow;
    }

    function _frameHaloEnabled() {
        const frame = _frameConfig();
        return frame.halo === undefined ? true : !!frame.halo;
    }

    function _cardWidth() {
        const layout = _styleLayout();
        if (layout === "wide") return Math.max(VT.Theme.defaultWidthPx, 560);
        if (layout === "minimal") return Math.min(VT.Theme.defaultWidthPx, 260);
        if (layout === "tile") return 176;
        if (layout === "orb") return 168;
        return VT.Theme.defaultWidthPx;
    }

    function _cardHeight() {
        const layout = _styleLayout();
        if (layout === "minimal") return 48;
        if (layout === "tile") return 176;
        if (layout === "orb") return 168;
        return 72;
    }

    function _cardRadius() {
        const layout = _styleLayout();
        if (layout === "orb") return Math.round(_cardWidth() / 2);
        if (layout === "tile") return 18;
        return VT.Theme.cornerRadius;
    }

    function _cardX() {
        const position = _stylePosition();
        const margin = _styleMarginPx();
        if (position.indexOf("left") >= 0) return margin;
        if (position.indexOf("right") >= 0) return Math.max(margin, panel.width - _cardWidth() - margin);
        return Math.max(margin, (panel.width - _cardWidth()) / 2);
    }

    function _cardY() {
        const position = _stylePosition();
        const margin = _styleMarginPx();
        if ((position === "top-left") || (position === "top-right")) return margin;
        if ((position === "bottom-left") || (position === "bottom-right")) {
            return Math.max(margin, panel.height - _cardHeight() - margin);
        }
        const y = panel.height * _styleTopMargin();
        return Math.max(margin, Math.min(panel.height - _cardHeight() - margin, y));
    }

    function _isBlockLayout() {
        const layout = _styleLayout();
        return layout === "tile" || layout === "orb" || layout === "custom";
    }

    function _isOrbLayout() {
        return _styleLayout() === "orb";
    }

    function _customQmlUrl() {
        if (style && style.customQmlUrl) {
            return style.customQmlUrl();
        }
        return "";
    }

    function _syncCustomItem() {
        if (!customLoader.item) return;
        if ("daemonState" in customLoader.item) customLoader.item.daemonState = daemonState;
        if ("audio" in customLoader.item) customLoader.item.audio = audio;
        if ("theme" in customLoader.item) customLoader.item.theme = style;
        if ("recipe" in customLoader.item) customLoader.item.recipe = style && style.config ? style.config.visual : null;
        if ("assetRoot" in customLoader.item) {
            customLoader.item.assetRoot = style && style.config ? style.config.asset_root || "" : "";
        }
    }

    function _resetMeters() {
        ring = [];
        currentPeakDbfs = -120;
        heldDbfs = -120;
        currentPeak = 0;
        currentRms = 0;
        lastFrameTsMs = 0;
        recipeRenderer.repaint();
    }

    Connections {
        target: panel.audio
        enabled: panel.audio !== null
        function onFrameReceived(peak, rms, vad, tsMs) {
            panel.currentPeak = peak;
            panel.currentRms = rms;

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
            // No repaint here: the renderer's animation timer already
            // drives painting whenever the OSD is visible.
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
        _syncCustomItem();
    }

    onAudioChanged: _syncCustomItem()
    onStyleChanged: _syncCustomItem()
    onOrbHaloEnergyChanged: {
        if (orbBackdropShadow.visible) {
            orbBackdropShadow.requestPaint();
        }
    }

    Loader {
        id: customLoader
        anchors.fill: parent
        active: panel.customActive
        source: panel.hasCustomQml ? panel.customQmlUrl : ""
        onLoaded: panel._syncCustomItem()
        onStatusChanged: {
            if (status === Loader.Error) {
                console.warn("voxtype OSD: failed to load custom QML, falling back to built-in surface:", source);
                panel.customQmlFailed = true;
            }
        }
    }

    // Glow source lives in the scene (hidden) rather than inline on the
    // MultiEffect property: an unparented item never joins the window's
    // scene graph and may not be rendered into the effect texture.
    Rectangle {
        id: frameGlowSource
        visible: false
        width: card.width
        height: card.height
        radius: card.radius
        color: "transparent"
        border.width: 2
        border.color: panel._styleColor("accent", panel.stateColor)
    }

    MultiEffect {
        id: frameGlow
        visible: !panel.customActive && panel._frameGlowEnabled() && !panel._isOrbLayout()
        anchors.centerIn: card
        width: frameGlowSource.width
        height: frameGlowSource.height
        source: frameGlowSource
        autoPaddingEnabled: true
        shadowEnabled: true
        shadowHorizontalOffset: 0
        shadowVerticalOffset: 0
        shadowColor: panel._styleColor("accent", panel.stateColor)
        shadowOpacity: panel.daemonState === "recording" ? 0.18 + panel.voiceEnergy * 0.34 : 0.0
        shadowBlur: 0.68 + panel.voiceEnergy * 0.22
        shadowScale: 1.02 + panel.voiceEnergy * 0.055
        blurMax: 64
        opacity: panel.daemonState === "recording" ? 1.0 : 0.0
        Behavior on shadowOpacity { NumberAnimation { duration: 130; easing.type: Easing.OutCubic } }
        Behavior on shadowBlur { NumberAnimation { duration: 150; easing.type: Easing.OutCubic } }
        Behavior on shadowScale { NumberAnimation { duration: 150; easing.type: Easing.OutCubic } }
        Behavior on opacity { NumberAnimation { duration: 110; easing.type: Easing.OutCubic } }
    }

    Rectangle {
        id: frameHalo
        visible: !panel.customActive && panel._frameHaloEnabled() && !panel._isOrbLayout()
        width: card.width + 3
        height: card.height + 3
        anchors.centerIn: card
        radius: card.radius + 1.5
        color: "transparent"
        border.color: panel._styleColor("accent", panel.stateColor)
        border.width: 1 + panel.voiceEnergy * 1.4
        opacity: panel.daemonState === "recording" ? 0.12 + panel.voiceEnergy * 0.28 : 0.0
        Behavior on opacity { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }
        Behavior on border.width { NumberAnimation { duration: 130; easing.type: Easing.OutCubic } }
    }

    Canvas {
        id: orbBackdropShadow
        visible: !panel.customActive && panel._isOrbLayout()
        width: card.width + 190
        height: card.height + 190
        anchors.centerIn: card
        anchors.verticalCenterOffset: 8
        opacity: panel.daemonState === "recording" ? 1.0 : 0.0
        Component.onCompleted: requestPaint()
        onVisibleChanged: requestPaint()
        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()
        onPaint: {
            const ctx = getContext("2d");
            ctx.clearRect(0, 0, width, height);
            if (!panel._isOrbLayout()) return;

            const cx = width / 2;
            const cy = height / 2;
            const outer = card.width * (0.84 + panel.orbHaloEnergy * 0.12);
            const alpha = 0.62 + panel.orbHaloEnergy * 0.16;

            ctx.save();
            ctx.translate(cx, cy);
            const gradient = ctx.createRadialGradient(0, 0, outer * 0.05, 0, 0, outer);
            gradient.addColorStop(0.0, "rgba(0, 0, 0, " + (alpha * 0.54) + ")");
            gradient.addColorStop(0.42, "rgba(0, 0, 0, " + (alpha * 0.58) + ")");
            gradient.addColorStop(0.74, "rgba(0, 0, 0, " + (alpha * 0.18) + ")");
            gradient.addColorStop(1.0, "rgba(0, 0, 0, 0.0)");
            ctx.fillStyle = gradient;
            ctx.beginPath();
            ctx.arc(0, 0, outer, 0, Math.PI * 2);
            ctx.fill();
            ctx.restore();
        }
        Behavior on opacity { NumberAnimation { duration: 140; easing.type: Easing.OutCubic } }
    }

    Repeater {
        id: orbHalo
        model: [44, 30, 16]
        delegate: Rectangle {
            required property int modelData
            required property int index
            readonly property int haloIndex: index
            visible: !panel.customActive && panel._isOrbLayout() && panel._frameHaloEnabled()
            width: card.width + modelData + panel.orbHaloEnergy * [34, 24, 14][haloIndex]
            height: card.height + modelData + panel.orbHaloEnergy * [34, 24, 14][haloIndex]
            anchors.centerIn: card
            radius: width / 2
            color: "transparent"
            border.color: panel._styleColor("accent", panel.stateColor)
            border.width: [1.2, 1.8, 2.6][haloIndex] + panel.orbHaloEnergy * [1.6, 2.4, 3.2][haloIndex]
            opacity: panel.daemonState === "recording"
                ? [0.055, 0.11, 0.22][haloIndex] + panel.orbHaloEnergy * [0.18, 0.30, 0.48][haloIndex]
                : 0.0
            scale: 1.0 + panel.orbHaloEnergy * [0.036, 0.022, 0.010][haloIndex]
            Behavior on opacity { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }
            Behavior on scale { NumberAnimation { duration: 145; easing.type: Easing.OutCubic } }
            Behavior on border.width { NumberAnimation { duration: 130; easing.type: Easing.OutCubic } }
            Behavior on width { NumberAnimation { duration: 145; easing.type: Easing.OutCubic } }
            Behavior on height { NumberAnimation { duration: 145; easing.type: Easing.OutCubic } }
        }
    }

    Rectangle {
        id: card
        visible: !panel.customActive
        width: panel._cardWidth()
        height: panel._cardHeight()
        x: panel._cardX()
        y: panel._cardY()
        radius: panel._cardRadius()
        color: panel._frameBackgroundColor()
        border.width: panel._frameBorderWidth()
        border.color: panel._frameBorderColor()
        opacity: (panel.daemonState === "recording" && panel.audio && panel.audio.running && !panel.audio.vad)
                 ? 0.78 : 1.0
        Behavior on opacity { NumberAnimation { duration: 120 } }

        Row {
            anchors.fill: parent
            anchors.leftMargin: panel._isBlockLayout() ? 0 : VT.Theme.padding
            anchors.rightMargin: panel._isBlockLayout() ? 0 : VT.Theme.padding
            spacing: 10

            Text {
                visible: !panel._isBlockLayout()
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
                width: panel._isBlockLayout()
                    ? card.width
                    : card.width - 28 - 2 * VT.Theme.padding - 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 4

                Item {
                    width: parent.width
                    height: panel._isBlockLayout()
                        ? card.height
                        : Math.max(24, card.height - 2 * VT.Theme.padding)

                    VT.RecipeRenderer {
                        id: recipeRenderer
                        anchors.fill: parent
                        style: panel.style
                        ring: panel.ring
                        daemonState: panel.daemonState
                        vad: panel.audio && panel.audio.vad
                        currentPeakDbfs: panel.currentPeakDbfs
                        heldDbfs: panel.heldDbfs
                        peak: panel.currentPeak
                        rms: panel.currentRms
                        waveformColumns: panel.waveformColumns
                        waveformGain: VT.Theme.waveformGain
                        meterFloorDbfs: VT.Theme.meterFloorDbfs
                    }
                }
            }
        }
    }
}
