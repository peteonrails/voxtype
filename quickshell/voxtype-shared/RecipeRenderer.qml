// Declarative voice-meter renderer for VoxType Quickshell OSD recipes.
//
// Each recipe layer becomes its own scene-graph item (Rectangles, Text,
// and QtQuick.Shapes paths) instead of strokes on a software Canvas, so
// the GPU does the drawing and a frame only updates a few numbers.
// Static geometry (gradients, the shadow ellipse, ring circles) is never
// re-rasterized; only the waveform and meter paths change per frame.
//
// One animation tick drives everything. It reads the newest audio values,
// smooths them, and bumps `_frame`, which the layer items bind to.

import QtQuick
import QtQuick.Shapes

Item {
    id: root

    property var style: null
    /// False while the host window is hidden. Item.visible can't be used
    /// for this: it stays true inside an unmapped window.
    property bool active: true
    property var ring: []
    property string daemonState: "idle"
    property bool vad: false
    property real currentPeakDbfs: -120
    property real heldDbfs: -120
    property real rms: 0.0
    property real peak: 0.0
    property int waveformColumns: 300
    /// Unused since levels moved to a decibel scale; kept so hosts that
    /// still set it don't fail to load.
    property real waveformGain: 10.0
    property real meterFloorDbfs: -60.0

    /// Optional live meter, read once per tick instead of through the
    /// properties above: { peak, rms, vad, peakDbfs, heldDbfs, ring }.
    /// A host that receives audio at 100 Hz can mutate this object in
    /// place, so frames cost no binding updates between ticks.
    property var meter: null

    /// Smoothed voice energy (0..1) for host decorations such as glows
    /// and halos, updated on the same tick as the layers.
    property real voiceEnergy: 0.0
    /// A hotter mix of the same signal, used by the orb halo rings.
    property real haloEnergy: 0.0
    /// Voice activity eased toward 0 or 1 over about 120 ms.
    property real voiceActivity: 0.0
    /// Animation time in seconds, for host decorations that should move
    /// in step with the layers (e.g. StateIcon's spinner).
    readonly property alias phase: root._phase
    /// Rotation in degrees for decorations that turn with the voice (the
    /// aura frame). Integrated on the tick, so a speed change never jumps
    /// the angle the way `phase * speed` would.
    readonly property alias spin: root._spin

    property real _peak: 0.0
    property real _rms: 0.0
    property real _vad: 0.0
    property real _energy: 0.0
    property real _phase: 0.0
    property real _spin: 0.0
    property real _spinSpeed: 0.0
    property real _liveFill: 0.0
    property real _heldFill: 0.0
    property real _lastTick: Date.now()
    /// Incremented once per tick. Bindings that read the level arrays
    /// (mutated in place) depend on this to refresh.
    property int _frame: 0
    property var _barLevels: []
    property var _waveLevels: []
    // Line strands: amplitude per strand, each eased at its own rate so
    // the strands drift apart instead of moving in lockstep.
    property var _lineLevels: [0, 0, 0]
    // Dot slots, center-out: slot 0 is now, outer slots are slightly older.
    property var _dotLevels: []
    // Layers sorted by order, cached so the tick doesn't re-sort.
    property var _sortedLayers: []
    property bool _wantsBars: false
    property bool _wantsWave: false
    property bool _wantsLine: false
    property bool _wantsDots: false

    onStyleChanged: _rebuildLayers()
    Component.onCompleted: _rebuildLayers()

    Connections {
        target: root.style
        ignoreUnknownSignals: true
        function onConfigChanged() { root._rebuildLayers(); }
    }

    function _rebuildLayers() {
        const cfg = style && style.config ? style.config : null;
        const visual = cfg && cfg.visual ? cfg.visual : null;
        const layers = visual && visual.layers ? visual.layers : [];
        const sorted = layers.slice().sort(function(a, b) {
            return (a.order || 0) - (b.order || 0);
        });
        let bars = false;
        let wave = false;
        let line = false;
        let dots = false;
        for (let i = 0; i < sorted.length; i++) {
            const kind = sorted[i].type || "waveform";
            if (kind === "bars") bars = true;
            if (kind === "waveform") wave = true;
            if (kind === "line") line = true;
            if (kind === "dots") dots = true;
        }
        _wantsBars = bars;
        _wantsWave = wave;
        _wantsLine = line;
        _wantsDots = dots;
        _sortedLayers = sorted;
    }

    // Numeric layer field with an explicit-unset check. Unset tunables are
    // omitted from the runtime JSON, so `undefined` means "use the layer
    // type's default" while explicit zeros are honored.
    function _num(value, fallback) {
        return value === undefined || value === null ? fallback : Number(value);
    }

    // StyleLoader resolves roles to QML colors (#AARRGGBB for rgba()), so
    // the result binds straight into color properties.
    function _color(role, fallback) {
        if (style && style.color) return style.color(role, fallback);
        return fallback;
    }

    function _alpha(c, a) {
        return Qt.rgba(c.r, c.g, c.b, c.a * a);
    }

    function _clamp(v, lo, hi) {
        return Math.max(lo, Math.min(hi, v));
    }

    function _easeOutCubic(t) {
        const u = 1 - _clamp(t, 0, 1);
        return 1 - u * u * u;
    }

    function _approach(current, target, stiffness, dt) {
        const amount = 1 - Math.exp(-Math.max(0.01, stiffness) * dt);
        return current + (target - current) * amount;
    }

    function _signal(spec) {
        const src = spec.source || "peak";
        if (src === "rms") return _rms;
        if (src === "vad") return _vad;
        if (src === "state") return daemonState === "recording" ? 1.0 : 0.0;
        if (src === "none") return 0.0;
        return _peak;
    }

    function _meterFill(dbfs) {
        const floor = meterFloorDbfs;
        if (!isFinite(dbfs) || dbfs <= floor) return 0.0;
        const clipped = Math.min(dbfs, 0);
        return _clamp((clipped - floor) / -floor, 0, 1);
    }

    function _ring() {
        return meter ? meter.ring : ring;
    }

    function _sampleRing(r, index, count, fallbackPhase) {
        if (r && r.length > 0) {
            // Map columns onto the ring's full capacity, right-aligned, so
            // a partially filled ring reads as silence on the left instead
            // of stretching early samples across the full width (which made
            // the trace shrink as history accumulated).
            const capacity = Math.max(r.length, waveformColumns);
            const pad = capacity - r.length;
            const start = Math.floor(index * capacity / Math.max(1, count)) - pad;
            const end = Math.max(start + 1, Math.floor((index + 1) * capacity / Math.max(1, count)) - pad);
            // Loudest sample in the bucket: averaging 10 ms peaks smooths
            // away the syllable attacks that make speech read as speech.
            let loudest = 0.0;
            for (let i = Math.max(0, start); i < Math.min(end, r.length); i++) {
                if (r[i] > loudest) loudest = r[i];
            }
            return _clamp(loudest, 0, 1);
        }
        const breath = 0.5 + 0.5 * Math.sin(fallbackPhase * 0.35);
        return 0.010 + breath * 0.006;
    }

    // Linear peak (0..1) to a 0..1 display level on a decibel scale.
    // Speech peaks span roughly -40..-6 dBFS, so a linear scale either
    // flattens quiet talkers or (with gain) pins everyone else at the
    // ceiling. -52 dBFS (room noise) maps to 0, -6 dBFS to full height.
    function _perceptual(linear) {
        if (!(linear > 0)) return 0.0;
        return _clamp((20 * Math.log10(linear) + 52) / 46, 0, 1);
    }

    // True while audio frames are flowing. In every other state the last
    // frame's values are stale and must not drive the visuals.
    function _live() {
        return daemonState === "recording" || daemonState === "streaming";
    }

    // Per-column temporal smoothing: rise quickly, fall more slowly, which
    // reads as intentional meter inertia instead of sample-by-sample jitter.
    // While transcribing there is no audio, so the columns ease into a
    // slow travelling ripple that says "working" instead of freezing on
    // the last words. Updates `levels` in place; no per-tick allocation.
    function _updateLevels(levels, count, rise, fall, dt) {
        while (levels.length < count) levels.push(0.0);
        levels.length = count;
        const r = _ring();
        const live = _live();
        const working = daemonState === "transcribing";
        for (let i = 0; i < count; i++) {
            let target = 0.0;
            if (live) {
                target = r && r.length > 0
                    ? _perceptual(_sampleRing(r, i, count, 0))
                    : _sampleRing(r, i, count, _phase + i * 0.17);
            } else if (working) {
                const t = i / Math.max(1, count - 1);
                target = 0.10 + 0.22 * Math.pow(0.5 + 0.5 * Math.sin(_phase * 3.2 - t * Math.PI * 4.0), 2);
            }
            const stiffness = target > levels[i] ? rise : fall;
            levels[i] = _approach(levels[i], target, working ? 6.0 : stiffness, dt);
        }
    }

    function _levelAt(levels, index, count) {
        if (!levels || levels.length === 0) return 0.0;
        const t = count <= 1 ? 0 : index / (count - 1);
        const f = t * (levels.length - 1);
        const lo = Math.floor(f);
        const hi = Math.min(levels.length - 1, lo + 1);
        const mix = f - lo;
        return levels[lo] * (1 - mix) + levels[hi] * mix;
    }

    // Loudest of the newest `span` ring samples, `offset` samples back
    // from the newest one, on the display (decibel) scale.
    function _recent(offset, span) {
        const r = _ring();
        if (!r || r.length === 0) return 0.0;
        const end = r.length - offset;
        let loudest = 0.0;
        for (let i = Math.max(0, end - span); i < Math.max(0, end); i++) {
            if (r[i] > loudest) loudest = r[i];
        }
        return _perceptual(loudest);
    }

    readonly property int _dotSlots: 3

    function _updateLine(dt) {
        const live = _live();
        const working = daemonState === "transcribing";
        // Recording reads the voice; transcribing holds a calm shimmer.
        const voice = live ? Math.max(_recent(0, 4), 0.05) : working ? 0.24 : 0.0;
        const rates = [22.0, 11.0, 6.0];
        for (let k = 0; k < 3; k++) {
            const stiffness = voice > _lineLevels[k] ? rates[k] : rates[k] * 0.45;
            _lineLevels[k] = _approach(_lineLevels[k], voice, stiffness, dt);
        }
    }

    function _updateDots(dt) {
        while (_dotLevels.length < _dotSlots) _dotLevels.push(0.0);
        const live = _live();
        const working = daemonState === "transcribing";
        for (let d = 0; d < _dotSlots; d++) {
            let target = 0.0;
            if (live) {
                // Outer dots show the voice ~70 ms later than inner ones, so
                // each syllable ripples outward from the center.
                target = _recent(d * 7, 5);
            }
            const stiffness = target > _dotLevels[d] ? 24.0 : 9.0;
            _dotLevels[d] = _approach(_dotLevels[d], target, stiffness, dt);
        }
    }

    function _stateLabel() {
        if (daemonState === "streaming") return "Streaming";
        if (daemonState === "transcribing") return "Transcribing";
        if (daemonState === "recording") return "Recording";
        return "Idle";
    }

    /// Advance one frame now. The tick already runs whenever the renderer
    /// is active; this exists for hosts that reset state and want the
    /// change visible before the next tick.
    function repaint() {
        _tick();
    }

    /// Drop all smoothed state, e.g. when a new recording starts.
    function reset() {
        _peak = 0;
        _rms = 0;
        _vad = 0;
        _energy = 0;
        _liveFill = 0;
        _heldFill = 0;
        voiceEnergy = 0;
        haloEnergy = 0;
        voiceActivity = 0;
        _barLevels.length = 0;
        _waveLevels.length = 0;
        _dotLevels.length = 0;
        _lineLevels = [0, 0, 0];
        _frame++;
    }

    function _tick() {
        const now = Date.now();
        const dt = Math.min(0.05, Math.max(0.001, (now - _lastTick) / 1000));
        _lastTick = now;

        const idle = !_live();
        const m = meter;
        const peakIn = idle ? 0 : (m ? m.peak : peak);
        const rmsIn = idle ? 0 : (m ? m.rms : rms);
        const vadIn = idle ? false : (m ? m.vad : vad);
        const peakDbfs = idle ? -120 : (m ? m.peakDbfs : currentPeakDbfs);
        const heldIn = idle ? -120 : (m ? m.heldDbfs : heldDbfs);

        _peak = _approach(_peak, _clamp(peakIn, 0, 1), 14.0, dt);
        _rms = _approach(_rms, _clamp(rmsIn, 0, 1), 10.0, dt);
        _vad = _approach(_vad, vadIn ? 1 : 0, 8.0, dt);
        _energy = _approach(_energy, Math.max(_peak, _rms * 1.6, _vad * 0.08), 7.0, dt);
        // Eased so the meter drains smoothly when audio stops instead of
        // snapping to empty (or staying lit on the last frame).
        _liveFill = idle
            ? _approach(_liveFill, 0, 10.0, dt)
            : _clamp(Math.max(_meterFill(peakDbfs), _energy * 0.08), 0, 1);
        _heldFill = idle ? _approach(_heldFill, 0, 10.0, dt) : _meterFill(heldIn);

        const rawVoice = _clamp(rmsIn * 6.0 + peakIn * 0.85, 0, 1);
        const rawHalo = _clamp(rmsIn * 10.0 + peakIn * 1.8, 0, 1);
        voiceEnergy = _approach(voiceEnergy, rawVoice, 18.0, dt);
        haloEnergy = _approach(haloEnergy, rawHalo, 18.0, dt);
        voiceActivity = _approach(voiceActivity, vadIn ? 1 : 0, 25.0, dt);

        if (_wantsBars) _updateLevels(_barLevels, 40, 18.0, 6.5, dt);
        // Stiffer than bars so scrolling history trails only ~2 columns.
        if (_wantsWave) _updateLevels(_waveLevels, 96, 30.0, 14.0, dt);
        if (_wantsLine) _updateLine(dt);
        if (_wantsDots) _updateDots(dt);
        const spinTarget = daemonState === "transcribing" ? 150 : 28 + 140 * voiceEnergy;
        _spinSpeed = _approach(_spinSpeed, spinTarget, 3.0, dt);
        _spin = (_spin + _spinSpeed * dt) % 360;
        _phase += dt;
        _frame++;
    }

    Timer {
        interval: 16
        repeat: true
        running: root.active && root.visible && root.daemonState !== "idle"
        triggeredOnStart: true
        onTriggered: root._tick()
    }

    Repeater {
        model: root._sortedLayers
        // Not anchored: a Loader with a size forces it onto its item,
        // which would override each layer's own rect.
        delegate: Loader {
            required property var modelData
            readonly property var spec: modelData
            sourceComponent: {
                const kind = modelData.type || "waveform";
                if (kind === "shadow") return shadowLayer;
                if (kind === "waveform") return waveformLayer;
                if (kind === "bars") return barsLayer;
                if (kind === "meter") return meterLayer;
                if (kind === "pulse") return pulseLayer;
                if (kind === "background") return backgroundLayer;
                if (kind === "ring") return ringLayer;
                if (kind === "icon") return iconLayer;
                if (kind === "label") return labelLayer;
                if (kind === "line") return lineLayer;
                if (kind === "dots") return dotsLayer;
                return null;
            }
        }
    }

    // Places itself on the layer rect: values <= 1 are fractions of the
    // renderer, larger values are pixels.
    component LayerItem: Item {
        readonly property var spec: parent && parent.spec ? parent.spec : ({})
        readonly property real _lx: root._num(spec.x, 0)
        readonly property real _ly: root._num(spec.y, 0)
        readonly property real _lw: root._num(spec.width, 1)
        readonly property real _lh: root._num(spec.height, 1)
        x: _lx <= 1 ? _lx * root.width : _lx
        y: _ly <= 1 ? _ly * root.height : _ly
        width: _lw <= 1 ? _lw * root.width : _lw
        height: _lh <= 1 ? _lh * root.height : _lh
    }

    // Horizontal fill that fades a ribbon out at both ends.
    component EdgeFade: LinearGradient {
        id: fade
        property color tint
        property real span: 1
        x1: 0; y1: 0
        x2: span; y2: 0
        GradientStop { position: 0.0; color: Qt.alpha(fade.tint, 0) }
        GradientStop { position: 0.22; color: fade.tint }
        GradientStop { position: 0.78; color: fade.tint }
        GradientStop { position: 1.0; color: Qt.alpha(fade.tint, 0) }
    }

    // Rounded rect radius clamped the way the old canvas path did.
    function _radius(r, w, h) {
        return _clamp(r || 0, 0, Math.max(0, Math.min(w, h) / 2));
    }

    Component {
        id: backgroundLayer
        LayerItem {
            id: bg
            readonly property color tint: root._color(spec.color || "accent", "#66c7ff")
            readonly property color surface: root._color("surface", "#8c1a1a1f")
            opacity: root._num(spec.opacity, 0.18)
                * root._clamp(0.18 + root._signal(spec) * root._num(spec.gain, 1.0) * 0.8, 0.08, 1.0)
            Shape {
                anchors.fill: parent
                preferredRendererType: Shape.CurveRenderer
                ShapePath {
                    strokeColor: "transparent"
                    fillGradient: LinearGradient {
                        x1: 0; y1: 0
                        x2: bg.width; y2: bg.height
                        GradientStop { position: 0.0; color: bg.tint }
                        GradientStop { position: 0.55; color: bg.surface }
                        GradientStop { position: 1.0; color: bg.tint }
                    }
                    PathRectangle {
                        width: bg.width
                        height: bg.height
                        radius: root._radius(root._num(bg.spec.radius, 12), bg.width, bg.height)
                    }
                }
            }
        }
    }

    Component {
        id: shadowLayer
        LayerItem {
            id: sh
            readonly property real outer: Math.min(width, height)
                * root._clamp(root._num(spec.radius, 0.58), 0.25, 0.85)
            // Shadow defaults to black; a recipe can recolor it (e.g. the
            // theme background role) for tinted backdrops.
            readonly property color tint: root._color(spec.color, "#000000")
            opacity: root._num(spec.opacity, 0.9)
                * root._clamp(0.34 + root._signal(spec) * root._num(spec.gain, 1.0) * 0.34, 0.18, 0.92)
            Shape {
                x: sh.width / 2 - sh.outer
                y: sh.height * 0.55 - sh.outer
                width: sh.outer * 2
                height: sh.outer * 2
                preferredRendererType: Shape.CurveRenderer
                transform: Scale {
                    origin.x: sh.outer
                    origin.y: sh.outer
                    xScale: 1.08
                    yScale: 0.92
                }
                ShapePath {
                    strokeColor: "transparent"
                    fillGradient: RadialGradient {
                        centerX: sh.outer; centerY: sh.outer; centerRadius: sh.outer
                        focalX: sh.outer; focalY: sh.outer; focalRadius: sh.outer * 0.08
                        GradientStop { position: 0.0; color: sh.tint }
                        GradientStop { position: 0.36; color: root._alpha(sh.tint, 0.58) }
                        GradientStop { position: 0.72; color: root._alpha(sh.tint, 0.24) }
                        GradientStop { position: 1.0; color: root._alpha(sh.tint, 0.0) }
                    }
                    PathRectangle {
                        width: sh.outer * 2
                        height: sh.outer * 2
                        radius: sh.outer
                    }
                }
            }
        }
    }

    Component {
        id: pulseLayer
        LayerItem {
            id: pl
            readonly property real breath: 0.5 + 0.5 * Math.sin(root._phase * 1.6 * root._num(spec.speed, 1.0))
            readonly property real inset: 2 + breath * 5
            Rectangle {
                x: pl.inset
                y: pl.inset
                width: Math.max(0, pl.width - pl.inset * 2)
                height: Math.max(0, pl.height - pl.inset * 2)
                radius: root._radius(root._num(pl.spec.radius, 12), width, height)
                color: root._color(pl.spec.color || "accent", "#66c7ff")
                opacity: root._num(pl.spec.opacity, 0.24)
                    * root._clamp(0.22 + root._signal(pl.spec) * root._num(pl.spec.gain, 1.0) * 1.2 + pl.breath * 0.12, 0.0, 1.0)
            }
        }
    }

    Component {
        id: ringLayer
        LayerItem {
            id: rg
            readonly property real breath: 0.5 + 0.5 * Math.sin(root._phase * 2.4 * root._num(spec.speed, 1.0))
            readonly property real base: Math.min(width, height)
            readonly property real stroke: Math.max(2, root._num(spec.radius, 3))
            readonly property real safeRadius: Math.max(1, base / 2 - Math.max(10, stroke * 2.7))
            readonly property real response: Math.pow(
                root._clamp(root._signal(spec) * root._num(spec.gain, 1.0), 0, 1),
                Math.max(0.01, root._num(spec.response_curve, 1.0)))
            readonly property real ringRadius: Math.min(
                safeRadius,
                base * root._num(spec.max_scale, 0.46),
                base * (root._num(spec.base_scale, 0.32)
                        + response * root._num(spec.response_scale, 0.18)
                        + breath * root._num(spec.breath_scale, 0.045)))
            readonly property color tint: root._color(spec.color || "accent", "#66c7ff")
            readonly property real layerOpacity: root._num(spec.opacity, 1.0)

            // Soft glow, mid, and core strokes, each a circle whose border
            // is centered on the ring's radius.
            Repeater {
                model: 3
                delegate: Rectangle {
                    required property int index
                    readonly property real lineWidth: [10, 5, rg.stroke][index]
                    readonly property real r: rg.ringRadius + index * 2
                    x: rg.width / 2 - r - lineWidth / 2
                    y: rg.height / 2 - r - lineWidth / 2
                    width: 2 * r + lineWidth
                    height: width
                    radius: width / 2
                    color: "transparent"
                    border.width: lineWidth
                    border.color: rg.tint
                    opacity: rg.layerOpacity * [0.16, 0.32, 0.95][index]
                }
            }

            Rectangle {
                readonly property real angle: root._phase * 2.2
                x: rg.width / 2 + Math.cos(angle) * rg.ringRadius - 2.5
                y: rg.height / 2 + Math.sin(angle) * rg.ringRadius - 2.5
                width: 5
                height: 5
                radius: 2.5
                color: rg.tint
                opacity: rg.layerOpacity * 0.55
            }
        }
    }

    Component {
        id: barsLayer
        LayerItem {
            id: bl
            readonly property int count: Math.min(28, Math.max(12, Math.floor(width / 17)))
            readonly property real gap: Math.max(2.0, width / count * 0.24)
            readonly property real barW: Math.max(2.0, width / count - gap)
            readonly property real gain: root._num(spec.gain, 1.0)
            readonly property real layerOpacity: root._num(spec.opacity, 1.0)
            readonly property real barRadius: root._num(spec.radius, 5)
            readonly property bool mirror: spec.mirror !== false
            readonly property real breath: 0.97 + 0.03 * Math.sin(root._phase * 0.85)
            readonly property color tint: root._color(spec.color || "accent", "#66c7ff")
            readonly property color haloTint: root._color("foreground", tint)
            readonly property color baseTint: root._color("accent", tint)

            Repeater {
                model: bl.count
                delegate: Item {
                    id: bar
                    required property int index
                    readonly property real position: bl.count <= 1 ? 0.5 : index / (bl.count - 1)
                    readonly property real center: (bl.count - 1) / 2
                    readonly property real weight: 0.72 + 0.28 * Math.cos((index - center) / Math.max(1, center) * Math.PI)
                    readonly property real level: {
                        root._frame;
                        const sample = root._levelAt(root._barLevels, index, bl.count);
                        return root._clamp(sample * bl.gain * weight * bl.breath + root._energy * 0.045, 0.028, 1.0);
                    }
                    readonly property real barH: Math.max(3, bl.height * level)
                    x: index * (bl.barW + bl.gap) + bl.gap * 0.5
                    y: bl.mirror ? (bl.height - barH) / 2 : bl.height - barH
                    width: bl.barW
                    height: barH

                    Rectangle {
                        x: -1.5
                        y: -1.5
                        width: parent.width + 3
                        height: parent.height + 3
                        radius: root._radius(bl.barRadius + 2, width, height)
                        color: bl.tint
                        opacity: bl.layerOpacity * 0.18
                    }
                    Rectangle {
                        anchors.fill: parent
                        radius: root._radius(bl.barRadius, width, height)
                        opacity: bl.layerOpacity * (0.48 + 0.52 * bar.position)
                        gradient: Gradient {
                            GradientStop { position: 0.0; color: bl.haloTint }
                            GradientStop { position: 0.45; color: bl.tint }
                            GradientStop { position: 1.0; color: bl.baseTint }
                        }
                    }
                }
            }
        }
    }

    Component {
        id: waveformLayer
        LayerItem {
            id: wf
            // One point every ~6 px reads as a smooth curve and keeps the
            // per-frame path small.
            readonly property int cols: Math.max(24, Math.min(80, Math.floor(width / 6)))
            readonly property bool mirror: spec.mirror !== false
            readonly property real gain: root._num(spec.gain, 1.0)
            readonly property real layerOpacity: root._num(spec.opacity, 1.0)
            readonly property color tint: root._color(spec.color || "accent", "#66c7ff")

            // Closed envelope outline, rebuilt once per tick. Temporally
            // smoothed levels get 3-tap neighbor smoothing: ring samples
            // hop between column buckets as history scrolls, which reads
            // as single-column flicker.
            readonly property var outline: {
                root._frame;
                const n = cols;
                // mirror=false anchors the envelope to the bottom edge
                // instead of reflecting it around the vertical center.
                const cy = mirror ? height / 2 : height - 1;
                const amp = Math.max(1, mirror ? height / 2 - 1.5 : height - 2.5);
                const raw = new Array(n);
                for (let i = 0; i < n; i++) raw[i] = root._levelAt(root._waveLevels, i, n);
                const pts = new Array(n);
                const lower = mirror ? new Array(n) : null;
                for (let i = 0; i < n; i++) {
                    const prev = raw[Math.max(0, i - 1)];
                    const next = raw[Math.min(n - 1, i + 1)];
                    const s = prev * 0.25 + raw[i] * 0.5 + next * 0.25;
                    const t = n <= 1 ? 0 : i / (n - 1);
                    // Taper the ends so the trace starts and finishes at
                    // the baseline instead of being cut off by the card.
                    const taper = Math.min(1, t * 12, (1 - t) * 12);
                    const e = Math.max(0.02, root._clamp(s * gain, 0, 1) * taper) * amp;
                    const x = t * width;
                    pts[i] = Qt.point(x, cy - e);
                    if (mirror) lower[n - 1 - i] = Qt.point(x, cy + e);
                }
                if (mirror) return pts.concat(lower, [pts[0]]);
                return pts.concat([Qt.point(width, cy), Qt.point(0, cy), pts[0]]);
            }

            Shape {
                anchors.fill: parent
                preferredRendererType: Shape.CurveRenderer
                ShapePath {
                    // Densest along the baseline and fading toward the peaks,
                    // so the body reads as glow and the outline as the signal.
                    fillGradient: LinearGradient {
                        x1: 0; y1: 0
                        x2: 0; y2: wf.height
                        GradientStop { position: 0.0; color: root._alpha(wf.tint, wf.layerOpacity * 0.04) }
                        GradientStop { position: wf.mirror ? 0.5 : 1.0; color: root._alpha(wf.tint, wf.layerOpacity * 0.42) }
                        GradientStop { position: 1.0; color: root._alpha(wf.tint, wf.layerOpacity * (wf.mirror ? 0.04 : 0.42)) }
                    }
                    strokeColor: root._alpha(wf.tint, wf.layerOpacity * 0.95)
                    strokeWidth: 1.5
                    joinStyle: ShapePath.RoundJoin
                    PathPolyline { path: wf.outline }
                }
            }
        }
    }

    Component {
        id: meterLayer
        LayerItem {
            id: mt
            readonly property real layerOpacity: root._num(spec.opacity, 1.0)
            readonly property real meterRadius: root._num(spec.radius, height / 2)
            readonly property real fillW: width * root._liveFill

            Rectangle {
                anchors.fill: parent
                radius: root._radius(mt.meterRadius, width, height)
                color: root._color("muted", "#40ffffff")
                opacity: mt.layerOpacity * 0.28
            }

            // The gradient spans the whole track so the fill's color says
            // how hot the level is; only the filled part is drawn.
            Shape {
                anchors.fill: parent
                visible: mt.fillW > 0
                opacity: mt.layerOpacity
                preferredRendererType: Shape.CurveRenderer
                ShapePath {
                    strokeColor: "transparent"
                    fillGradient: LinearGradient {
                        x1: 0; y1: 0
                        x2: mt.width; y2: 0
                        // spec.color sets the low-zone color; the mid/high
                        // stops keep the warning/error roles so hot levels
                        // still read as hot.
                        GradientStop { position: 0.0; color: root._color(mt.spec.color || "success", "#4dd973") }
                        GradientStop { position: 0.72; color: root._color("warning", "#f2cc4d") }
                        GradientStop { position: 1.0; color: root._color("error", "#f2594d") }
                    }
                    PathRectangle {
                        width: mt.fillW
                        height: mt.height
                        radius: root._radius(mt.meterRadius, mt.fillW, mt.height)
                    }
                }
            }

            Rectangle {
                visible: root._heldFill > 0.01
                x: Math.max(0, mt.width * root._heldFill - 1.5)
                y: -2
                width: 3
                height: mt.height + 4
                radius: 1.5
                color: root._color(mt.spec.secondary_color || "foreground", "#fcfbf8")
                opacity: mt.layerOpacity
            }
        }
    }

    Component {
        id: iconLayer
        LayerItem {
            id: ic
            readonly property real size: Math.max(8, Math.min(width, height)
                * root._clamp(root._num(spec.gain, 1.0), 0.25, 2.5))
            StateIcon {
                anchors.centerIn: parent
                width: ic.size
                height: ic.size
                daemonState: root.daemonState
                phase: root._phase
                color: root._color(ic.spec.color, root._color(root.daemonState, "#66c7ff"))
                opacity: root._num(ic.spec.opacity, 1.0)
                // Scale rather than resize so the pulse costs a transform.
                scale: 1.0 + root._signal(ic.spec) * 0.08
                    + Math.sin(root._phase * 1.8 * root._num(ic.spec.speed, 1.0)) * 0.025
            }
        }
    }

    Component {
        id: labelLayer
        LayerItem {
            id: lb
            Text {
                anchors.fill: parent
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                fontSizeMode: Text.HorizontalFit
                minimumPixelSize: 6
                text: root._stateLabel()
                font.weight: Font.DemiBold
                font.pixelSize: Math.round(Math.max(9, Math.min(lb.height * 0.62, lb.width / 5.5)
                    * root._clamp(root._num(lb.spec.gain, 1.0), 0.25, 2.5)))
                color: root._color(lb.spec.color || "foreground", "#fcfbf8")
                opacity: root._num(lb.spec.opacity, 1.0)
                    * root._clamp(0.72 + root._signal(lb.spec) * 0.28, 0.0, 1.0)
            }
        }
    }

    // A flowing voice line: three thin strands that swell with the voice,
    // pinned to the baseline at both ends. It shows the live level, not
    // history, so it answers "is it hearing me" at a glance. Each strand is
    // a filled ribbon so a horizontal gradient can fade its ends.
    Component {
        id: lineLayer
        LayerItem {
            id: ln
            readonly property int pts: Math.max(32, Math.min(64, Math.floor(width / 4)))
            readonly property real gain: root._num(spec.gain, 1.0)
            readonly property real layerOpacity: root._num(spec.opacity, 1.0)
            readonly property real speed: root._num(spec.speed, 1.0)
            readonly property real core: Math.max(1.2, root._num(spec.radius, 2.6))
            readonly property color tint: root._color(spec.color || "accent", "#66c7ff")
            readonly property color tint2: root._color(spec.secondary_color || "foreground", "#fcfbf8")
            readonly property color tint3: Qt.tint(tint, Qt.alpha(tint2, 0.5))
            // Per strand: cycles across the width, drift speed, amplitude.
            readonly property var strands: [[1.1, 2.6, 1.0], [1.55, -1.9, 0.8], [2.0, 3.1, 0.6]]

            function ribbon(k, thickness) {
                root._frame;
                const n = pts;
                const st = strands[k];
                const cy = height / 2;
                // Square-root response: quiet speech still visibly moves the
                // line, loud speech doesn't pin it to the edges.
                const amp = Math.max(1, height / 2 - 2)
                    * Math.sqrt(root._clamp(root._lineLevels[k] * gain, 0, 1)) * st[2];
                const working = root.daemonState === "transcribing";
                const ph = root._phase * st[1] * speed * (working ? 1.6 : 1.0);
                const xs = new Array(n);
                const ys = new Array(n);
                for (let i = 0; i < n; i++) {
                    const t = i / (n - 1);
                    const w = Math.pow(Math.sin(Math.PI * t), 2.2);
                    xs[i] = t * width;
                    ys[i] = cy + amp * w * (Math.sin(2 * Math.PI * st[0] * t + ph)
                        + 0.25 * Math.sin(2 * Math.PI * st[0] * 1.6 * t - ph * 1.3)) / 1.25;
                }
                const half = thickness / 2;
                const upper = new Array(n);
                const lower = new Array(n);
                for (let i = 0; i < n; i++) {
                    const a = Math.max(0, i - 1);
                    const b = Math.min(n - 1, i + 1);
                    const dx = xs[b] - xs[a];
                    const dy = ys[b] - ys[a];
                    const len = Math.max(0.001, Math.sqrt(dx * dx + dy * dy));
                    const nx = -dy / len * half;
                    const ny = dx / len * half;
                    upper[i] = Qt.point(xs[i] + nx, ys[i] - ny);
                    lower[n - 1 - i] = Qt.point(xs[i] - nx, ys[i] + ny);
                }
                return upper.concat(lower, [upper[0]]);
            }

            readonly property var glowPath: ribbon(0, core * 3.5)
            readonly property var path0: ribbon(0, core)
            readonly property var path1: ribbon(1, core * 0.7)
            readonly property var path2: ribbon(2, core * 0.6)

            // The glow is soft and faint, so it skips the curve renderer's
            // antialiasing, which is the costly part of redrawing a path.
            Shape {
                anchors.fill: parent
                opacity: ln.layerOpacity
                preferredRendererType: Shape.GeometryRenderer
                ShapePath {
                    strokeColor: "transparent"
                    fillGradient: EdgeFade { span: ln.width; tint: root._alpha(ln.tint, 0.2) }
                    PathPolyline { path: ln.glowPath }
                }
            }

            Shape {
                anchors.fill: parent
                opacity: ln.layerOpacity
                preferredRendererType: Shape.CurveRenderer
                ShapePath {
                    strokeColor: "transparent"
                    fillGradient: EdgeFade { span: ln.width; tint: root._alpha(ln.tint3, 0.55) }
                    PathPolyline { path: ln.path2 }
                }
                ShapePath {
                    strokeColor: "transparent"
                    fillGradient: EdgeFade { span: ln.width; tint: root._alpha(ln.tint2, 0.7) }
                    PathPolyline { path: ln.path1 }
                }
                ShapePath {
                    strokeColor: "transparent"
                    fillGradient: EdgeFade { span: ln.width; tint: ln.tint }
                    PathPolyline { path: ln.path0 }
                }
            }
        }
    }

    // Voice-assistant dots: a short row of round dots that stretch into
    // capsules with the voice. The center dot is now; each step out is a
    // few tens of ms older, so syllables ripple outward. Transcribing turns
    // them into a travelling bounce.
    Component {
        id: dotsLayer
        LayerItem {
            id: dt
            readonly property real gain: root._num(spec.gain, 1.0)
            readonly property real layerOpacity: root._num(spec.opacity, 1.0)
            readonly property real dotW: root._clamp(root._num(spec.radius, height * 0.3), 3, Math.max(3, height))
            readonly property int count: {
                let n = Math.floor(width / (dotW * 2.0));
                n = Math.max(3, Math.min(root._dotSlots * 2 - 1, n));
                return n % 2 === 0 ? n - 1 : n;
            }
            readonly property real spacing: dotW * 0.85
            readonly property real x0: (width - (count * dotW + (count - 1) * spacing)) / 2
            readonly property color tint: root._color(spec.color || "accent", "#66c7ff")
            readonly property color tint2: root._color(spec.secondary_color || spec.color || "accent", "#66c7ff")

            Repeater {
                model: dt.count
                delegate: Rectangle {
                    id: dot
                    required property int index
                    readonly property int dist: Math.abs(index - (dt.count - 1) / 2)
                    readonly property real level: {
                        root._frame;
                        if (root.daemonState === "transcribing") {
                            const wave = Math.max(0, Math.sin(root._phase * 5.0 - index * 0.75));
                            return 0.42 * wave * wave * wave;
                        }
                        const v = root._dotLevels.length > dist ? root._dotLevels[dist] : 0;
                        // A faint ripple while listening keeps silence from
                        // looking frozen.
                        const idle = root._live()
                            ? 0.05 * (0.5 + 0.5 * Math.sin(root._phase * 2.2 - dist * 0.9)) : 0;
                        return root._clamp(Math.max(v * dt.gain * (1 - dist * 0.1), idle), 0, 1);
                    }
                    x: dt.x0 + index * (dt.dotW + dt.spacing)
                    width: dt.dotW
                    height: dt.dotW + (dt.height - dt.dotW) * level
                    y: (dt.height - height) / 2
                    radius: dt.dotW / 2
                    color: dist === 0 ? dt.tint : Qt.tint(dt.tint, Qt.alpha(dt.tint2, Math.min(1, dist * 0.35)))
                    opacity: dt.layerOpacity * (0.62 + 0.38 * Math.min(1, level * 2.5))
                }
            }
        }
    }
}
