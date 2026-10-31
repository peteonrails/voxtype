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
// Visibility follows `daemonState` unless `osdSuppressed` is set or
// `activeOutput` is false: hidden when suppressed, off the active
// output, or idle/empty, shown otherwise. The component is layered as a WlrLayer.Overlay surface so
// it floats above all other windows without taking input focus.

import QtQuick
import QtQuick.Effects
import QtQuick.Shapes
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

    /// False on outputs that shouldn't show the HUD right now. Wired by
    /// the parent from the compositor's focused output; defaults to true
    /// so a standalone surface behaves as before.
    property bool activeOutput: true

    /// The audio bridge instance whose frameReceived signal drives the
    /// waveform. Passed in by the parent so it's shared with sibling
    /// widgets that also want VAD / peak data.
    property var audio: null

    /// Resolved style loader from VT.StyleLoader. Provides semantic colors,
    /// visual recipe layers, and optional custom QML package entry.
    property var style: null

    /// The daemon's recording.json and outcome.json, from VT.StateReader:
    /// the session in flight (start time, limit, engine, model) and how
    /// the last one ended. Null when unavailable (older daemons).
    property var session: null
    property var outcome: null

    // ----- session clock -----

    property real _now: Date.now()
    Timer {
        interval: 250
        repeat: true
        running: panel.visible && panel.session !== null
        triggeredOnStart: true
        onTriggered: panel._now = Date.now()
    }
    readonly property bool _sessionLive: session !== null && !!session.started_at_ms
        && (daemonState === "recording" || daemonState === "streaming")
    readonly property real elapsedSecs: _sessionLive
        ? Math.max(0, (_now - session.started_at_ms) / 1000) : 0
    readonly property real limitSecs: session && session.max_duration_secs > 0 ? session.max_duration_secs : 0
    readonly property real remainingSecs: limitSecs > 0 ? Math.max(0, limitSecs - elapsedSecs) : 0
    /// The last stretch before max_duration_secs stops the recording.
    readonly property bool countingDown: _sessionLive && limitSecs > 0
        && remainingSecs <= Math.min(15, limitSecs / 2)

    function _clock(secs) {
        const s = Math.max(0, Math.floor(secs));
        const sec = s % 60;
        return Math.floor(s / 60) + ":" + (sec < 10 ? "0" : "") + sec;
    }

    readonly property string clockText: countingDown ? _clock(Math.ceil(remainingSecs)) : _clock(elapsedSecs)
    readonly property color clockColor: !countingDown ? panel._styleColor("muted", VT.Theme.textColor)
        : remainingSecs <= 5 ? panel._styleColor("error", VT.Theme.meterHighColor)
        : panel._styleColor("warning", VT.Theme.meterMidColor)

    /// Shown briefly when a session starts, so a --model override or a
    /// modifier-selected model is confirmed before speaking.
    readonly property string modelText: session && session.model
        ? (session.engine ? session.engine + " · " : "") + session.model : ""
    property bool _modelChip: false
    property real _chipShownFor: 0
    Timer {
        id: modelChipTimer
        interval: 1800
        onTriggered: panel._modelChip = false
    }
    onSessionChanged: {
        _syncCustomItem();
        // Once per session, keyed on its start time.
        if (session && session.started_at_ms && session.started_at_ms !== _chipShownFor) {
            _chipShownFor = session.started_at_ms;
            _modelChip = true;
            modelChipTimer.restart();
        }
    }

    // ----- result card -----

    /// How the session that just ended went: { kind, chars, limit }, or
    /// null. Held for a moment after idle so the HUD can confirm the
    /// result instead of vanishing.
    property var flash: null
    readonly property bool flashing: flash !== null
    // A --no-osd session stays hidden through its result too. The marker
    // is cleared on idle, before the outcome is read, so latch it.
    property bool _sessionSuppressed: false
    onOsdSuppressedChanged: if (osdSuppressed) _sessionSuppressed = true

    Timer {
        id: flashTimer
        onTriggered: panel.flash = null
    }

    onOutcomeChanged: {
        _syncCustomItem();
        const o = outcome;
        if (!o || !o.at_ms || Date.now() - o.at_ms > 3000) return;
        // A cancel or an accidental tap needs no confirmation.
        const shown = { ok: 1, no_speech: 1, empty: 1, error: 1, output_failed: 1 };
        if (!shown[o.outcome] || _sessionSuppressed || !activeOutput) return;
        flash = { kind: o.outcome, chars: o.chars || 0, limit: !!o.limit_reached };
        flashTimer.interval = o.outcome === "ok" && !o.limit_reached ? 1000 : 2600;
        flashTimer.restart();
    }

    readonly property string flashIcon: !flash ? ""
        : flash.kind === "ok" ? "done"
        : flash.kind === "no_speech" || flash.kind === "empty" ? "nospeech"
        : "error"
    readonly property color flashColor: !flash ? stateColor
        : flash.kind === "ok" ? panel._styleColor("success", VT.Theme.meterLowColor)
        : flash.kind === "no_speech" || flash.kind === "empty" ? panel._styleColor("warning", VT.Theme.meterMidColor)
        : panel._styleColor("error", VT.Theme.meterHighColor)
    readonly property string flashText: {
        if (!flash) return "";
        let text = flash.kind === "ok" ? (flash.chars === 1 ? "1 character" : flash.chars + " characters")
            : flash.kind === "no_speech" ? "No speech detected"
            : flash.kind === "empty" ? "Nothing transcribed"
            : flash.kind === "output_failed" ? "Couldn't type the text"
            : "Transcription failed";
        if (flash.limit) text = "Time limit reached · " + text;
        return text;
    }

    /// True while the HUD should be on screen. The window itself stays
    /// mapped a little longer (`_leaving`) so the exit animation can play.
    readonly property bool shown: !osdSuppressed && activeOutput
        && ((daemonState !== "idle" && daemonState !== "") || flashing)
    property bool _leaving: false
    visible: shown || _leaving

    /// The state the built-in visuals show: the daemon state while shown,
    /// and the last non-idle state while the exit animation plays, so the
    /// card fades out as it was rather than flashing to idle colors.
    property string shownState: "recording"

    /// 0 = gone, 1 = fully present. Drives the enter/exit motion.
    property real presence: 0.0
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
        shownState === "recording"    ? panel._styleColor("recording", VT.Theme.recordingColor)
      : shownState === "streaming"    ? panel._styleColor("streaming", VT.Theme.streamingColor)
      : shownState === "transcribing" ? panel._styleColor("transcribing", VT.Theme.transcribingColor)
      :                                 panel._styleColor("idle", VT.Theme.idleColor)

    // Capacity of the waveform history: 3 s @ 100 Hz.
    readonly property int waveformColumns: Math.round(VT.Theme.waveformWindowSecs * 100)

    // Live meter state, written at audio rate (100 Hz) and read by the
    // renderer once per animation tick. A plain object mutated in place:
    // frames change no QML properties, so they trigger no bindings.
    // `ring` holds recent per-frame peaks (0.0..1.0), newest last. dBFS
    // values match src/osd/visual.rs's PeakHold.
    property var _meter: ({
        peak: 0, rms: 0, vad: false,
        peakDbfs: -120, heldDbfs: -120, lastTsMs: 0,
        ring: []
    })
    readonly property string customQmlUrl: _customQmlUrl()
    readonly property bool hasCustomQml: customQmlUrl.length > 0
    /// Set when the custom QML package fails to load, so the built-in
    /// surface can take over instead of leaving the OSD blank.
    property bool customQmlFailed: false
    /// True while a custom QML package is selected and loading/loaded.
    /// Every built-in element keys its visibility off this so a broken
    /// package falls back to the default card.
    readonly property bool customActive: hasCustomQml && !customQmlFailed
    // Smoothed on the renderer's tick, so decorations need no Behaviors.
    readonly property real voiceEnergy: recipeRenderer.voiceEnergy
    readonly property real orbHaloEnergy: recipeRenderer.haloEnergy
    readonly property bool recordingVisual: shownState === "recording"

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

    readonly property bool shellLayout: _styleLayout() === "shell"
    readonly property var shellChrome: style && style.config ? style.config.shell : null
    readonly property real shellScale: Math.max(0.5, Math.min(3, _shellNumber("spacing_scale", 1)))
    readonly property real shellFontSize: _shellNumber("font_size", 12)
    readonly property string textFamily: shellLayout ? "monospace" : Qt.application.font.family
    readonly property real stripGap: shellLayout ? 16 * shellScale : 10
    readonly property real stripIconWidth: shellLayout ? 28 * shellScale : 28

    function _shellNumber(key, fallback) {
        const value = shellChrome && shellChrome[key] !== undefined ? Number(shellChrome[key]) : fallback;
        return isFinite(value) && value >= 0 ? value : fallback;
    }

    function _foregroundColor() {
        return panel._styleColor(shellLayout && shellChrome ? shellChrome.text : "foreground", VT.Theme.textColor);
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
        const tint = panel._styleColor(value, VT.Theme.bgColor);
        return shellLayout ? Qt.alpha(tint, 0.97) : tint;
    }

    function _frameBorderColor() {
        const value = _frameValue("border", "state");
        if (value === "none" || value === "transparent") return "transparent";
        if (value === "state") return panel.stateColor;
        return panel._styleColor(value, panel.stateColor);
    }

    function _frameBorderWidth() {
        const value = _frameValue("border", "state");
        return shellLayout || value === "none" || value === "transparent" || value === "aura" ? 0 : 2;
    }

    readonly property bool _auraFrame: _frameValue("border", "state") === "aura"

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
        if (layout === "pill") return Math.max(240, Math.min(VT.Theme.defaultWidthPx, 320));
        if (layout === "shell") return 320 * shellScale;
        if (layout === "tile") return 176;
        if (layout === "orb") return 168;
        return VT.Theme.defaultWidthPx;
    }

    function _cardHeight() {
        const layout = _styleLayout();
        if (layout === "minimal") return 48;
        if (layout === "pill") return 52;
        if (layout === "shell") return 60 * shellScale + 2 * _shellNumber("border_width", 2);
        if (layout === "tile") return 176;
        if (layout === "orb") return 168;
        return 72;
    }

    function _cardRadius() {
        const layout = _styleLayout();
        if (layout === "orb") return Math.round(_cardWidth() / 2);
        if (layout === "pill") return _cardHeight() / 2;
        if (layout === "shell") return style && style.shellCornerRadius !== undefined
            ? Math.min(style.shellCornerRadius, _cardWidth() / 2, _cardHeight() / 2) : 0;
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

    /// Whether text outside the card goes below it: below unless the card
    /// sits near the bottom edge, where it would fall off the screen.
    readonly property bool _outsideBelow: _cardY() + _cardHeight() + 60 < height

    /// Horizontal inset of the strip content (icon, renderer, clock). The
    /// pill's round ends need more room than a rounded rectangle's corners.
    function _contentInset() {
        if (_isBlockLayout()) return 0;
        if (shellLayout) return 16 * shellScale + _shellNumber("border_width", 2);
        return _styleLayout() === "pill" ? 20 : VT.Theme.padding;
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
        if ("session" in customLoader.item) customLoader.item.session = session;
        if ("outcome" in customLoader.item) customLoader.item.outcome = outcome;
        if ("assetRoot" in customLoader.item) {
            customLoader.item.assetRoot = style && style.config ? style.config.asset_root || "" : "";
        }
    }

    function _resetMeters() {
        const m = _meter;
        m.peak = 0;
        m.rms = 0;
        m.vad = false;
        m.peakDbfs = -120;
        m.heldDbfs = -120;
        m.lastTsMs = 0;
        m.ring.length = 0;
        recipeRenderer.reset();
    }

    Connections {
        target: panel.audio
        // Hidden surfaces (idle, --no-osd, every output but the ones
        // showing) skip the 100 Hz frame work entirely.
        enabled: panel.audio !== null && panel.visible
        function onFrameReceived(peak, rms, vad, tsMs) {
            const m = panel._meter;
            m.peak = peak;
            m.rms = rms;
            m.vad = vad;

            // Newest peak on the right; drop the oldest when full.
            const r = m.ring;
            r.push(peak);
            if (r.length > panel.waveformColumns) {
                r.splice(0, r.length - panel.waveformColumns);
            }

            // dBFS from linear peak. Floor at -120 to match visual.rs's
            // PeakHold::held_dbfs sentinel for "effectively silent".
            const dbfs = peak > 0.0 ? 20 * Math.log10(peak) : -120;
            m.peakDbfs = dbfs;

            // Held-peak: snap up on a louder peak, otherwise decay at
            // peakDecayDbPerSec. dt comes from the frame timestamps so
            // a paused daemon (no frames) doesn't unrealistically decay
            // before we receive the next frame.
            const dt = (m.lastTsMs > 0 ? Math.max(0, tsMs - m.lastTsMs) : 10) / 1000;
            m.lastTsMs = tsMs;
            if (dbfs > m.heldDbfs) {
                m.heldDbfs = dbfs;
            } else {
                const decayed = m.heldDbfs - VT.Theme.peakDecayDbPerSec * dt;
                m.heldDbfs = decayed < -120 ? -120 : decayed;
            }
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
        } else {
            if (daemonState === "recording" || daemonState === "streaming") {
                if (shownState !== "recording" && shownState !== "streaming" || flashing) {
                    // A new session: drop the previous one's result.
                    flashTimer.stop();
                    flash = null;
                }
                if (!osdSuppressed) _sessionSuppressed = false;
            }
            shownState = daemonState;
        }
        _syncCustomItem();
    }

    onShownChanged: {
        if (shown) {
            exitAnimation.stop();
            _leaving = false;
            enterAnimation.restart();
        } else if (presence > 0) {
            _leaving = true;
            enterAnimation.stop();
            exitAnimation.restart();
        }
    }

    // A surface skips frames while hidden, so whatever it held is stale
    // when it reappears (focus moved here mid-recording, or a --no-osd
    // recording ended). Start clean rather than replaying old waveform
    // and decaying the held peak across the whole hidden interval.
    onVisibleChanged: {
        if (visible) {
            _resetMeters();
        }
    }

    NumberAnimation {
        id: enterAnimation
        target: panel
        property: "presence"
        to: 1.0
        duration: 220
        easing.type: Easing.OutCubic
    }

    SequentialAnimation {
        id: exitAnimation
        NumberAnimation {
            target: panel
            property: "presence"
            to: 0.0
            duration: 160
            easing.type: Easing.InCubic
        }
        ScriptAction { script: panel._leaving = false }
    }

    onAudioChanged: _syncCustomItem()
    onStyleChanged: _syncCustomItem()

    // Re-sync on live style reloads (theme switches) too, so a custom
    // package's `recipe` and `assetRoot` don't go stale.
    Connections {
        target: panel.style
        ignoreUnknownSignals: true
        function onConfigChanged() { panel._syncCustomItem(); }
    }

    // Everything visible lives on this stage so enter/exit motion moves
    // it as one piece: fade, a slight scale, and a short rise from (or
    // sink toward) the screen edge the HUD is anchored to.
    Item {
        id: stage
        anchors.fill: parent
        opacity: panel.presence
        readonly property bool fromTop: panel._stylePosition().indexOf("top") === 0
        transform: [
            Scale {
                origin.x: card.x + card.width / 2
                origin.y: card.y + card.height / 2
                xScale: panel.customActive ? 1.0 : 0.94 + 0.06 * panel.presence
                yScale: xScale
            },
            Translate {
                y: panel.customActive ? 0 : (1 - panel.presence) * (stage.fromTop ? -12 : 12)
            }
        ]

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

        // Energy-driven values come pre-smoothed from the renderer's tick,
        // so they bind directly. Behaviors here would restart an animation
        // on every tick; only the recording fade-in/out animates.
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
            shadowOpacity: 0.18 + panel.voiceEnergy * 0.34
            shadowBlur: 0.68 + panel.voiceEnergy * 0.22
            shadowScale: 1.02 + panel.voiceEnergy * 0.055
            blurMax: 64
            opacity: panel.recordingVisual ? 1.0 : 0.0
            Behavior on opacity { NumberAnimation { duration: 130; easing.type: Easing.OutCubic } }
        }

        Item {
            anchors.fill: parent
            visible: !panel.customActive && panel._frameHaloEnabled() && !panel._isOrbLayout()
            opacity: panel.recordingVisual ? 1.0 : 0.0
            Behavior on opacity { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

            Rectangle {
                id: frameHalo
                width: card.width + 3
                height: card.height + 3
                x: card.x - 1.5
                y: card.y - 1.5
                radius: card.radius + 1.5
                color: "transparent"
                border.color: panel._styleColor("accent", panel.stateColor)
                border.width: 1 + panel.voiceEnergy * 1.4
                opacity: 0.12 + panel.voiceEnergy * 0.28
            }
        }

        // Soft dark backdrop behind the orb: a static radial gradient that
        // only scales and fades with energy, so it never re-rasterizes.
        Item {
            id: orbBackdrop
            visible: !panel.customActive && panel._isOrbLayout()
            readonly property real maxOuter: card.width * 0.96
            x: card.x + card.width / 2 - maxOuter
            y: card.y + card.height / 2 + 8 - maxOuter
            width: maxOuter * 2
            height: maxOuter * 2
            opacity: panel.recordingVisual ? 1.0 : 0.0
            Behavior on opacity { NumberAnimation { duration: 140; easing.type: Easing.OutCubic } }

            Shape {
                anchors.fill: parent
                preferredRendererType: Shape.CurveRenderer
                opacity: 0.62 + panel.orbHaloEnergy * 0.16
                scale: (0.84 + panel.orbHaloEnergy * 0.12) / 0.96
                ShapePath {
                    strokeColor: "transparent"
                    fillGradient: RadialGradient {
                        centerX: orbBackdrop.maxOuter; centerY: orbBackdrop.maxOuter
                        centerRadius: orbBackdrop.maxOuter
                        focalX: orbBackdrop.maxOuter; focalY: orbBackdrop.maxOuter
                        focalRadius: orbBackdrop.maxOuter * 0.05
                        GradientStop { position: 0.0; color: Qt.rgba(0, 0, 0, 0.54) }
                        GradientStop { position: 0.42; color: Qt.rgba(0, 0, 0, 0.58) }
                        GradientStop { position: 0.74; color: Qt.rgba(0, 0, 0, 0.18) }
                        GradientStop { position: 1.0; color: Qt.rgba(0, 0, 0, 0.0) }
                    }
                    PathRectangle {
                        width: orbBackdrop.width
                        height: orbBackdrop.height
                        radius: orbBackdrop.maxOuter
                    }
                }
            }
        }

        // Orb halo: one soft accent glow hugging the orb's edge that swells
        // and brightens with the voice. A static gradient, so energy changes
        // only move a transform and an opacity.
        Item {
            id: orbGlow
            visible: !panel.customActive && panel._isOrbLayout() && panel._frameHaloEnabled()
            readonly property real outer: card.width * 0.92
            readonly property real edge: (card.width / 2) / outer
            x: card.x + card.width / 2 - outer
            y: card.y + card.height / 2 - outer
            width: outer * 2
            height: outer * 2
            opacity: panel.recordingVisual ? 1.0 : 0.0
            Behavior on opacity { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

            Shape {
                id: glowShape
                anchors.fill: parent
                preferredRendererType: Shape.CurveRenderer
                readonly property color tint: panel._styleColor("accent", panel.stateColor)
                opacity: 0.30 + panel.orbHaloEnergy * 0.70
                scale: 0.90 + panel.orbHaloEnergy * 0.10
                ShapePath {
                    strokeColor: "transparent"
                    fillGradient: RadialGradient {
                        centerX: orbGlow.outer; centerY: orbGlow.outer; centerRadius: orbGlow.outer
                        focalX: orbGlow.outer; focalY: orbGlow.outer
                        GradientStop { position: 0.0; color: "transparent" }
                        GradientStop { position: orbGlow.edge * 0.96; color: "transparent" }
                        GradientStop { position: orbGlow.edge * 1.04; color: Qt.rgba(glowShape.tint.r, glowShape.tint.g, glowShape.tint.b, 0.42) }
                        GradientStop { position: orbGlow.edge + (1 - orbGlow.edge) * 0.45; color: Qt.rgba(glowShape.tint.r, glowShape.tint.g, glowShape.tint.b, 0.12) }
                        GradientStop { position: 1.0; color: "transparent" }
                    }
                    PathRectangle {
                        width: orbGlow.width
                        height: orbGlow.height
                        radius: orbGlow.outer
                    }
                }
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
            border.color: panel.flashing && panel._frameValue("border", "state") === "state"
            ? panel.flashColor : panel._frameBorderColor()
            // Dim slightly between words. Smoothed on the renderer's tick: a
            // Behavior here would animate on every VAD flip, several times a
            // second, and keep the window redrawing at the display's full
            // refresh rate.
            opacity: !panel.shellLayout && (panel.recordingVisual && panel.audio && panel.audio.running)
                     ? 0.78 + 0.22 * recipeRenderer.voiceActivity : 1.0

            Loader {
                anchors.fill: parent
                active: panel.shellLayout
                sourceComponent: Component {
                    VT.ShellBorder {
                        visible: thickness > 0
                            && panel._frameValue("border", "accent") !== "none"
                            && panel._frameValue("border", "accent") !== "transparent"
                            && panel._frameValue("border", "accent") !== "aura"
                        cornerRadius: card.radius
                        thickness: panel._shellNumber("border_width", 2)
                        angle: panel._shellNumber("border_angle", 0)
                        opacity: panel._shellNumber("border_alpha", 1)
                        colors: {
                            const configured = panel.shellChrome ? panel.shellChrome.border_colors : null;
                            const stops = configured && configured.length > 0 ? configured : ["accent"];
                            return stops.map(function(c) { return panel._styleColor(c, panel.stateColor); });
                        }
                    }
                }
            }

            Row {
                anchors.fill: parent
                anchors.leftMargin: panel._contentInset()
                anchors.rightMargin: panel._contentInset()
                spacing: panel.stripGap

                // Lifted toward the foreground: some theme palettes map the
                // state roles to dark, low-contrast colors.
                VT.StateIcon {
                    visible: !panel._isBlockLayout()
                    width: panel.stripIconWidth
                    height: panel.shellLayout ? 22 * panel.shellScale : 22
                    anchors.verticalCenter: parent.verticalCenter
                    daemonState: panel.shownState
                    result: panel.flashIcon
                    phase: recipeRenderer.phase
                    color: Qt.tint(panel.flashing ? panel.flashColor : panel.stateColor,
                                   Qt.alpha(panel._foregroundColor(), panel.flashing ? 0.2 : 0.5))
                }

                Column {
                    width: panel._isBlockLayout()
                        ? card.width
                        : card.width - panel.stripIconWidth - 2 * panel._contentInset() - panel.stripGap
                            - (stripClock.visible ? stripClock.width + panel.stripGap : 0)
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 4

                    Item {
                        width: parent.width
                        height: panel._isBlockLayout()
                            ? card.height
                            : panel.shellLayout
                                ? card.height - 2 * panel._contentInset()
                            : panel._styleLayout() === "pill"
                                ? card.height - 16
                                : Math.max(24, card.height - 2 * VT.Theme.padding)

                        VT.RecipeRenderer {
                            id: recipeRenderer
                            anchors.fill: parent
                            opacity: panel.flashing ? 0.0 : 1.0
                            Behavior on opacity { NumberAnimation { duration: 140 } }
                            // Items inside a hidden PanelWindow still report
                            // visible, so the renderer can't see a --no-osd
                            // hide on its own.
                            active: panel.visible
                            style: panel.style
                            meter: panel._meter
                            daemonState: panel.shownState
                            waveformColumns: panel.waveformColumns
                            waveformGain: VT.Theme.waveformGain
                            meterFloorDbfs: VT.Theme.meterFloorDbfs
                        }

                        Text {
                            anchors.fill: parent
                            visible: !panel._isBlockLayout()
                            opacity: panel.flashing ? 1.0 : 0.0
                            Behavior on opacity { NumberAnimation { duration: 140 } }
                            verticalAlignment: Text.AlignVCenter
                            elide: Text.ElideRight
                            text: panel.flashText
                            font.family: panel.textFamily
                            font.pixelSize: panel.shellLayout ? panel.shellFontSize * 1.167 : 15
                            font.weight: Font.Medium
                            color: panel._foregroundColor()
                        }
                    }
                }

                Text {
                    id: stripClock
                    visible: !panel._isBlockLayout() && panel.session !== null
                    width: panel.shellLayout ? 38 * Math.max(panel.shellScale, panel.shellFontSize / 12) : 38
                    anchors.verticalCenter: parent.verticalCenter
                    horizontalAlignment: Text.AlignRight
                    opacity: panel._sessionLive ? 1.0 : 0.0
                    Behavior on opacity { NumberAnimation { duration: 140 } }
                    text: panel.clockText
                    font.family: panel.textFamily
                    font.pixelSize: panel.shellLayout ? panel.shellFontSize * 1.083 : 13
                    font.weight: panel.countingDown ? Font.DemiBold : Font.Normal
                    font.features: { "tnum": 1 }
                    color: panel.clockColor
                }
            }

            // Final seconds before max_duration_secs: a bar along the
            // card's bottom edge drains with the time left.
            Rectangle {
                visible: panel.countingDown && !panel._isBlockLayout()
                anchors.bottom: parent.bottom
                anchors.bottomMargin: Math.max(2, card.radius / 3)
                x: card.radius
                height: 2
                radius: 1
                width: (card.width - 2 * card.radius)
                    * Math.min(1, panel.remainingSecs / Math.max(1, Math.min(15, panel.limitSecs / 2)))
                color: panel.clockColor
            }
        }

        // Aura frame (`border = "aura"`): a gradient edge in the theme's
        // state colors that turns slowly and quickens and brightens with
        // the voice, over a blurred copy of itself for the glow. The angle
        // comes from the renderer's tick, so it never runs a free animation.
        Item {
            id: aura
            visible: !panel.customActive && panel._auraFrame
            x: card.x
            y: card.y
            width: card.width
            height: card.height
            opacity: panel.shownState === "idle" ? 0.0 : 1.0
            Behavior on opacity { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
            readonly property real pad: 18
            readonly property real energy: panel.flashing ? 0.6 : recipeRenderer.voiceEnergy

            ConicalGradient {
                id: auraGradient
                centerX: aura.pad + aura.width / 2
                centerY: aura.pad + aura.height / 2
                angle: recipeRenderer.spin
                GradientStop { position: 0.0; color: panel.flashing ? panel.flashColor : panel._styleColor("accent", panel.stateColor) }
                GradientStop { position: 0.3; color: panel.flashing ? panel.flashColor : panel._styleColor("recording", panel.stateColor) }
                GradientStop { position: 0.62; color: panel.flashing ? panel.flashColor : panel._styleColor("transcribing", panel.stateColor) }
                GradientStop { position: 1.0; color: panel.flashing ? panel.flashColor : panel._styleColor("accent", panel.stateColor) }
            }

            component AuraRing: Shape {
                id: ringShape
                property real thickness: 2
                x: -aura.pad
                y: -aura.pad
                width: aura.width + 2 * aura.pad
                height: aura.height + 2 * aura.pad
                preferredRendererType: Shape.CurveRenderer
                ShapePath {
                    strokeColor: "transparent"
                    fillRule: ShapePath.OddEvenFill
                    fillGradient: auraGradient
                    PathRectangle {
                        x: aura.pad
                        y: aura.pad
                        width: aura.width
                        height: aura.height
                        radius: card.radius
                    }
                    PathRectangle {
                        x: aura.pad + ringShape.thickness
                        y: aura.pad + ringShape.thickness
                        width: aura.width - 2 * ringShape.thickness
                        height: aura.height - 2 * ringShape.thickness
                        radius: Math.max(0, card.radius - ringShape.thickness)
                    }
                }
            }

            AuraRing {
                id: auraGlowSource
                visible: false
                thickness: 5
            }

            MultiEffect {
                source: auraGlowSource
                x: auraGlowSource.x
                y: auraGlowSource.y
                width: auraGlowSource.width
                height: auraGlowSource.height
                blurEnabled: true
                blur: 1.0
                blurMax: 32
                blurMultiplier: 0.5
                brightness: 0.15
                opacity: 0.5 + 0.5 * aura.energy
            }

            AuraRing {
                thickness: 1.5 + aura.energy
                opacity: 0.7 + 0.3 * aura.energy
            }
        }

        // Block layouts (tile, orb) have no room inside the card: their
        // clock sits just outside it, and their result is a large icon
        // over the card with the message beneath.
        Text {
            visible: panel._isBlockLayout() && !panel.customActive && panel.session !== null
            opacity: panel._sessionLive ? 1.0 : 0.0
            Behavior on opacity { NumberAnimation { duration: 140 } }
            anchors.horizontalCenter: card.horizontalCenter
            y: panel._outsideBelow ? card.y + card.height + 10 : card.y - height - 10
            text: panel.clockText
            font.pixelSize: 13
            font.weight: panel.countingDown ? Font.DemiBold : Font.Normal
            font.features: { "tnum": 1 }
            color: panel.clockColor
        }

        // Backed by the card background so the result stays readable
        // over a busy wallpaper when the layout draws no card of its own.
        Rectangle {
            visible: panel._isBlockLayout() && !panel.customActive
            opacity: panel.flashing ? 1.0 : 0.0
            Behavior on opacity { NumberAnimation { duration: 140 } }
            anchors.centerIn: card
            width: Math.min(card.width, card.height) * 0.62
            height: width
            radius: width / 2
            color: panel._styleColor("background", VT.Theme.bgColor)
            border.width: 1.5
            border.color: Qt.alpha(panel.flashColor, 0.6)

            VT.StateIcon {
                anchors.centerIn: parent
                width: parent.width * 0.58
                height: width
                strokeWidth: 1.8
                daemonState: panel.shownState
                result: panel.flashIcon
                color: panel.flashColor
            }
        }

        Rectangle {
            visible: panel._isBlockLayout() && !panel.customActive
            opacity: panel.flashing ? 1.0 : 0.0
            Behavior on opacity { NumberAnimation { duration: 140 } }
            anchors.horizontalCenter: card.horizontalCenter
            y: panel._outsideBelow ? card.y + card.height + 10 : card.y - height - 10
            width: flashLabel.implicitWidth + 24
            height: flashLabel.implicitHeight + 10
            radius: height / 2
            color: panel._styleColor("background", VT.Theme.bgColor)

            Text {
                id: flashLabel
                anchors.centerIn: parent
                text: panel.flashText
                font.pixelSize: 14
                font.weight: Font.Medium
                color: panel._styleColor("foreground", VT.Theme.textColor)
            }
        }

        // Engine and model, briefly, as a recording starts (#854).
        Rectangle {
            id: modelChip
            visible: !panel.customActive && panel.modelText.length > 0
            opacity: panel._modelChip && panel._sessionLive ? 1.0 : 0.0
            Behavior on opacity { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
            anchors.horizontalCenter: card.horizontalCenter
            // Opposite the block-layout clock, so the two never overlap.
            y: panel._outsideBelow ? card.y - height - 10 : card.y + card.height + 10
            width: modelLabel.implicitWidth + 20
            height: modelLabel.implicitHeight + 8
            radius: panel.shellLayout ? Math.min(height / 2, panel._cardRadius()) : height / 2
            color: panel.shellLayout ? panel._frameBackgroundColor() : panel._styleColor("background", VT.Theme.bgColor)
            border.width: 1
            border.color: Qt.alpha(panel._styleColor("foreground", VT.Theme.textColor), 0.18)
            Text {
                id: modelLabel
                anchors.centerIn: parent
                textFormat: Text.PlainText
                text: panel.modelText
                font.family: panel.textFamily
                font.pixelSize: panel.shellLayout ? panel.shellFontSize : 12
                color: panel._foregroundColor()
            }
        }
    }
}
