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
// Visibility follows `daemonState` unless `osdSuppressed` is set:
// hidden when suppressed or idle/empty, shown
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

    /// Set while the in-flight recording was started with `--no-osd`. Wired by
    /// the parent from VT.StateReader.osdSuppressed. Keeps the surface hidden
    /// for tools that draw their own dictation UI, without touching the
    /// daemon's reported state.
    property bool osdSuppressed: false

    /// The audio bridge instance whose frameReceived signal drives the
    /// waveform. Passed in by the parent so it's shared with sibling
    /// widgets that also want VAD / peak data.
    property var audio: null

    /// Resolved style loader from VT.StyleLoader. Provides semantic colors,
    /// visual recipe layers, and optional custom QML package entry.
    property var style: null

    // The classic card shows while audio frames arrive and hides 150 ms
    // after they stop, like the GTK4 and native frontends. Recipe styles
    // follow the daemon state.
    visible: !osdSuppressed && (classicCard
        ? framesFresh
        : daemonState !== "idle" && daemonState !== "")
    anchors {
        top: true
        bottom: true
        left: true
        right: true
    }
    color: "transparent"

    // Put the surface on the focused output. Without an explicit screen
    // Quickshell leaves the window on Qt's default screen, so with two
    // monitors the OSD could show up on the one you are not looking at.
    // The target only changes while the OSD is hidden, so it never jumps
    // between outputs mid-recording. The compositor probe is created from
    // a string so a Quickshell build without that module still loads this
    // surface; with no probe the screen is left as before.
    property var _focusProbe: null
    property var _targetScreen: null
    readonly property string _focusedOutputName: _focusProbe && _focusProbe.name ? _focusProbe.name : ""

    function _createFocusProbe() {
        const hyprland = (Quickshell.env("HYPRLAND_INSTANCE_SIGNATURE") || "") !== "";
        const i3 = (Quickshell.env("SWAYSOCK") || "") !== "" || (Quickshell.env("I3SOCK") || "") !== "";
        const source = hyprland
            ? 'import QtQuick; import Quickshell.Hyprland; QtObject { readonly property string name: Hyprland.focusedMonitor ? Hyprland.focusedMonitor.name : "" }'
            : i3
                ? 'import QtQuick; import Quickshell.I3; QtObject { readonly property string name: I3.focusedMonitor ? I3.focusedMonitor.name : "" }'
                : "";
        if (!source) return null;
        try {
            return Qt.createQmlObject(source, panel, "voxtypeFocusedOutput");
        } catch (e) {
            console.warn("voxtype OSD: cannot track the focused output:", e);
            return null;
        }
    }

    function _screenNamed(name) {
        if (!name) return null;
        const screens = Quickshell.screens;
        for (let i = 0; i < screens.length; i++) {
            if (screens[i].name === name) return screens[i];
        }
        return null;
    }

    function _retargetScreen() {
        if (panel.visible) return;
        const target = _screenNamed(_focusedOutputName);
        if (target) panel._targetScreen = target;
    }

    Component.onCompleted: {
        _focusProbe = _createFocusProbe();
        _retargetScreen();
    }
    on_FocusedOutputNameChanged: _retargetScreen()
    onVisibleChanged: _retargetScreen()

    Connections {
        target: Quickshell
        function onScreensChanged() {
            panel._targetScreen = null;
            panel._retargetScreen();
        }
    }

    Binding {
        target: panel
        property: "screen"
        value: panel._targetScreen
        when: panel._targetScreen !== null
    }

    WlrLayershell.namespace: "voxtype-osd"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
    // The classic card spans the output's usable height (between bars),
    // like the GTK4 and native surfaces with exclusive zone 0, so all
    // three place it at the same spot.
    exclusionMode: classicCard ? ExclusionMode.Normal : ExclusionMode.Ignore
    exclusiveZone: 0

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

    // Frames across the visible waveform window (100 per second).
    readonly property int waveformColumns: Math.round(panel._styleNumber("waveform_window_secs", VT.Theme.waveformWindowSecs) * 100)

    // Recent per-frame peaks (0.0..1.0), oldest first. Mutated in place
    // (no change signal per frame); the renderer reads it on every paint.
    // Holds the visible window plus the playout delay on both sides.
    property var ring: []
    // Signed sample range per frame, parallel to `ring`.
    property var ringMin: []
    property var ringMax: []
    // Classic card: true while frames keep arriving (see frameIdleTimer).
    property bool framesFresh: false
    readonly property bool classicCard: !!(style && style.config && style.config.classic_card)
    // Frames received this recording, and the wall-clock origin of the
    // scroll clock. Together they place every frame at a fixed position in
    // time, so the waveform scrolls at a constant rate and past audio never
    // changes shape. Mirrors osd::ipc::FrameRing.
    property int framesReceived: 0
    property real clockOriginMs: -1
    readonly property real playoutDelayFrames: 5

    property real currentPeakDbfs: -120
    property real currentPeak: 0
    property real currentRms: 0
    readonly property string customQmlUrl: _customQmlUrl()
    readonly property bool hasCustomQml: customQmlUrl.length > 0
    /// Set when the custom QML package fails to load, so the built-in
    /// surface can take over instead of leaving the OSD blank.
    property bool customQmlFailed: false
    /// True while a custom QML package is selected and loading/loaded.
    /// Every built-in element keys its visibility off this so a broken
    /// package falls back to the default card.
    readonly property bool customActive: hasCustomQml && !customQmlFailed
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
        return isFinite(value) ? Math.max(0.0, Math.min(1.0, value)) : 0.85;
    }

    function _styleNumber(key, fallback) {
        const value = style && style.config && style.config[key] !== undefined
            ? Number(style.config[key])
            : fallback;
        return isFinite(value) ? value : fallback;
    }

    // `[osd] opacity`: the card's opacity, fixed while it is shown.
    function _styleOpacity() {
        return Math.max(0.0, Math.min(1.0, _styleNumber("opacity", VT.Theme.defaultOpacity)));
    }

    function _styleWaveformGain() {
        const value = style && style.config && style.config.waveform_gain !== undefined
            ? Number(style.config.waveform_gain)
            : VT.Theme.waveformGain;
        return isFinite(value) && value > 0 ? value : VT.Theme.waveformGain;
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
        if (classicCard) return Math.max(1, _styleNumber("width_px", VT.Theme.defaultWidthPx));
        const layout = _styleLayout();
        if (layout === "wide") return Math.max(VT.Theme.defaultWidthPx, 560);
        if (layout === "minimal") return Math.min(VT.Theme.defaultWidthPx, 260);
        if (layout === "tile") return 176;
        if (layout === "orb") return 168;
        return VT.Theme.defaultWidthPx;
    }

    function _cardHeight() {
        if (classicCard) return Math.max(1, _styleNumber("height_px", VT.Theme.defaultHeightPx));
        const layout = _styleLayout();
        if (layout === "minimal") return 48;
        if (layout === "tile") return 176;
        if (layout === "orb") return 168;
        return 72;
    }

    function _cardRadius() {
        if (classicCard) return 0;
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

    // Mirrors osd::placement::card_top. panel.height is the full height
    // of the output this surface is on, in its logical pixels.
    function _cardY() {
        const position = _stylePosition();
        const margin = _styleMarginPx();
        const area = panel.height;
        const h = _cardHeight();
        const fraction = _styleTopMargin();
        let y;
        if ((position === "top-left") || (position === "top-right")) y = margin;
        else if ((position === "bottom-left") || (position === "bottom-right")) y = area - h - margin;
        else if (position === "top-center") y = fraction <= 0.5 ? area * fraction : area * (1 - fraction) - h;
        else y = area * fraction;
        const lowest = area - h - margin;
        if (lowest < margin) return Math.max(0, (area - h) / 2);
        return Math.max(margin, Math.min(lowest, y));
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
        ringMin = [];
        ringMax = [];
        framesReceived = 0;
        clockOriginMs = -1;
        currentPeakDbfs = -120;
        currentPeak = 0;
        currentRms = 0;
        recipeRenderer.resetMeter();
        classicRenderer.resetMeter();
        recipeRenderer.repaint();
    }

    Connections {
        target: panel.audio
        enabled: panel.audio !== null
        function onFrameReceived(peak, rms, vad, tsMs) {
            panel.currentPeak = peak;
            panel.currentRms = rms;
            panel.currentPeakDbfs = peak > 0.0 ? 20 * Math.log10(peak) : -120;

            // Scroll clock: frames advance at 100 per second of wall time,
            // held back by the playout delay so capture batches arriving in
            // bursts still scroll smoothly. Batch arrivals never move the
            // clock; only an underrun re-anchors it, resuming where the
            // display stopped instead of jumping.
            // A new burst after the classic card went idle is a new
            // recording: start from an empty window, like FrameRing does
            // on a sequence break.
            if (panel.classicCard && !panel.framesFresh) panel._resetMeters();
            const now = Date.now();
            if (panel.clockOriginMs < 0) panel.clockOriginMs = now;
            const elapsed = (now - panel.clockOriginMs) / 10;
            if (elapsed > panel.framesReceived + panel.playoutDelayFrames) {
                const stoppedAt = Math.max(0, panel.framesReceived - 1);
                panel.clockOriginMs = now - (stoppedAt + panel.playoutDelayFrames) * 10;
            } else if (elapsed < panel.framesReceived - 4 * panel.playoutDelayFrames) {
                // A backlog (frames queued in the bridge's pipe while QML was
                // busy, e.g. at startup) arrived faster than real time. Catch
                // the clock up rather than trailing the meter for good.
                panel.clockOriginMs = now - Math.max(0, panel.framesReceived - 1) * 10;
            }

            const r = panel.ring;
            const rMin = panel.ringMin;
            const rMax = panel.ringMax;
            r.push(peak);
            rMin.push(panel.audio.sampleMin);
            rMax.push(panel.audio.sampleMax);
            const capacity = panel.waveformColumns + 2 * panel.playoutDelayFrames + 2;
            if (r.length > capacity) {
                const extra = r.length - capacity;
                r.splice(0, extra);
                rMin.splice(0, extra);
                rMax.splice(0, extra);
            }
            panel.framesReceived += 1;
            panel.framesFresh = true;
            frameIdleTimer.restart();
            // No repaint here: the renderer repaints on every display frame
            // while the OSD is visible.
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
        visible: !panel.customActive && !panel.classicCard && panel._frameGlowEnabled() && !panel._isOrbLayout()
        anchors.centerIn: card
        width: frameGlowSource.width
        height: frameGlowSource.height
        source: frameGlowSource
        autoPaddingEnabled: true
        shadowEnabled: true
        shadowHorizontalOffset: 0
        shadowVerticalOffset: 0
        shadowColor: panel._styleColor("accent", panel.stateColor)
        // A steady glow: it used to swell and fade with the voice, which
        // read as the whole OSD changing transparency while you spoke.
        shadowOpacity: 0.3
        shadowBlur: 0.75
        shadowScale: 1.04
        blurMax: 64
        opacity: panel.daemonState === "recording" ? panel._styleOpacity() : 0.0
        Behavior on opacity { NumberAnimation { duration: 110; easing.type: Easing.OutCubic } }
    }

    Rectangle {
        id: frameHalo
        visible: !panel.customActive && !panel.classicCard && panel._frameHaloEnabled() && !panel._isOrbLayout()
        width: card.width + 3
        height: card.height + 3
        anchors.centerIn: card
        radius: card.radius + 1.5
        color: "transparent"
        border.color: panel._styleColor("accent", panel.stateColor)
        border.width: 1.5
        opacity: panel.daemonState === "recording" ? 0.24 * panel._styleOpacity() : 0.0
        Behavior on opacity { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }
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
        // The classic card's background is painted by the renderer, so
        // it matches the GTK4 card pixel for pixel.
        color: panel.classicCard ? "transparent" : panel._frameBackgroundColor()
        border.width: panel.classicCard ? 0 : panel._frameBorderWidth()
        border.color: panel._frameBorderColor()
        // `[osd] opacity`, constant while shown. It used to drop to 0.78
        // whenever the bridge's voice-activity flag (peak above -40 dBFS)
        // was off, which flickered between words.
        opacity: panel._styleOpacity()

        VT.RecipeRenderer {
            id: classicRenderer
            visible: panel.classicCard
            anchors.fill: parent
            classic: true
            style: panel.style
            ring: panel.ring
            ringMin: panel.ringMin
            ringMax: panel.ringMax
            framesReceived: panel.framesReceived
            clockOriginMs: panel.clockOriginMs
            playoutDelayFrames: panel.playoutDelayFrames
            peakDecayDbPerSec: panel._styleNumber("peak_decay_db_per_sec", VT.Theme.peakDecayDbPerSec)
            daemonState: panel.daemonState
            waveformColumns: panel.waveformColumns
            waveformGain: panel._styleWaveformGain()
        }

        Row {
            visible: !panel.classicCard
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
                        framesReceived: panel.framesReceived
                        clockOriginMs: panel.clockOriginMs
                        playoutDelayFrames: panel.playoutDelayFrames
                        peakDecayDbPerSec: panel._styleNumber("peak_decay_db_per_sec", VT.Theme.peakDecayDbPerSec)
                        daemonState: panel.daemonState
                        vad: panel.audio && panel.audio.vad
                        currentPeakDbfs: panel.currentPeakDbfs
                        peak: panel.currentPeak
                        rms: panel.currentRms
                        waveformColumns: panel.waveformColumns
                        waveformGain: panel._styleWaveformGain()
                    }
                }
            }
        }
    }

    // Hide the classic card 150 ms after the last frame, as GTK4 does.
    Timer {
        id: frameIdleTimer
        interval: 150
        repeat: false
        onTriggered: panel.framesFresh = false
    }
}
