import QtQuick

// Aegis HUD: a circular assistant-style voice HUD.
//
// Most of the HUD never changes shape: the disc, scanlines, tick rings and
// segmented rings are painted once into their own canvases, then turned
// and faded with item transforms, which cost the GPU almost nothing. Only
// the voice-driven parts (the waveform arc, the level strip, the two
// center arcs) repaint each frame, in a canvas no bigger than the HUD.
// Repainting everything into a full-screen canvas every frame is what
// made earlier versions stutter.

Item {
    id: root

    property string daemonState: "idle"
    property var audio: null
    property var theme: null
    property var recipe: null
    property string assetRoot: ""

    property real peak: 0.0
    property real rms: 0.0
    property real vadLevel: 0.0
    // Display levels on a decibel scale (0..1), eased per tick.
    property real smoothPeak: 0.0
    property real smoothRms: 0.0
    property real phase: 0.0
    // Ring angles in radians, integrated per tick. Computing them as
    // `phase * speed` with a voice-driven speed made the rings jump
    // whenever the level changed, further the longer a recording ran.
    property real spinSlow: 0.0
    property real spinFast: 0.0
    property real lastTickMs: Date.now()
    // Newest-last ring of linear peaks, 10 ms apart, mutated in place.
    property var samples: []
    // Per-column display levels, eased toward the history each tick so
    // columns don't flicker as samples scroll between them.
    property var arcLevels: []
    property var stripLevels: []
    // Hysteresis on voice activity, so the labels don't flip on every
    // short pause between words.
    property bool voiceLock: false
    // Telemetry numbers refresh a few times a second, not every frame.
    property real shownPeak: 0.0
    property real shownRms: 0.0
    property real _telemetryAge: 0.0
    // Bumped when the theme's colors change, so color bindings refresh.
    property int _themeRev: 0

    readonly property real hudSize: _hudSize()
    readonly property real cx: _hudX(hudSize) + hudSize / 2
    readonly property real cy: _hudY(hudSize) + hudSize / 2
    readonly property real r: hudSize / 2
    readonly property real energy: _clamp(smoothPeak * 0.8 + smoothRms * 0.4 + vadLevel * 0.1, 0.0, 1.0)

    readonly property color stateColor: { _themeRev; return _stateColor(); }
    readonly property color accentColor: { _themeRev; return _color("accent", "#38D8FF"); }
    readonly property color fgColor: { _themeRev; return _color("foreground", "#D8FAFF"); }
    readonly property color mutedColor: { _themeRev; return _color("muted", "rgba(154, 221, 232, 0.62)"); }
    readonly property color gridColor: { _themeRev; return _color("grid", "rgba(73, 213, 255, 0.20)"); }
    readonly property color glowColor: { _themeRev; return _color("glow", "rgba(56, 216, 255, 0.36)"); }
    readonly property color amberColor: { _themeRev; return _color("amber", "#FFB84A"); }
    readonly property color bgColor: { _themeRev; return _color("background", "rgba(3, 11, 18, 0.72)"); }

    Connections {
        target: root.theme
        ignoreUnknownSignals: true
        function onConfigChanged() { root._themeRev++; }
    }

    function _configValue(key, fallback) {
        return theme && theme.config && theme.config[key] !== undefined
            ? theme.config[key]
            : fallback;
    }

    function _color(role, fallback) {
        if (theme && theme.color) return theme.color(role, fallback);
        return fallback;
    }

    function _stateColor() {
        if (daemonState === "streaming") return _color("streaming", "#4A8DFF");
        if (daemonState === "transcribing") return _color("transcribing", "#FFB84A");
        if (daemonState === "recording") return _color("recording", "#38D8FF");
        return _color("idle", "rgba(154, 221, 232, 0.58)");
    }

    function _stateLabel() {
        if (daemonState === "streaming") return "STREAM";
        if (daemonState === "transcribing") return "DECODE";
        if (daemonState === "recording") return voiceLock ? "VOICE" : "ARMED";
        return "STANDBY";
    }

    function _clamp(v, lo, hi) {
        return Math.max(lo, Math.min(hi, v));
    }

    function _approach(current, target, stiffness, dt) {
        const amount = 1.0 - Math.exp(-Math.max(0.01, stiffness) * dt);
        return current + (target - current) * amount;
    }

    function _hudSize() {
        return Math.min(340, Math.max(248, Math.min(root.width, root.height) * 0.32));
    }

    function _hudX(size) {
        const position = String(_configValue("position", "bottom-center"));
        const margin = Math.max(0, Number(_configValue("margin_px", 24)));
        if (position.indexOf("left") >= 0) return margin;
        if (position.indexOf("right") >= 0) return Math.max(margin, root.width - size - margin);
        return Math.max(margin, (root.width - size) / 2);
    }

    function _hudY(size) {
        const position = String(_configValue("position", "bottom-center"));
        const margin = Math.max(0, Number(_configValue("margin_px", 24)));
        if (position === "top-left" || position === "top-right") return margin;
        if (position === "bottom-left" || position === "bottom-right") {
            return Math.max(margin, root.height - size - margin);
        }
        const topMargin = _clamp(Number(_configValue("top_margin", 0.74)), 0.0, 1.0);
        return Math.max(margin, Math.min(root.height - size - margin, root.height * topMargin));
    }

    // Linear peak to a 0..1 level on a decibel scale: -52 dBFS (room
    // noise) is empty, -6 dBFS is full. A linear scale pins normal speech
    // at the top and leaves quiet speech flat.
    function _perceptual(linear) {
        if (!(linear > 0)) return 0.0;
        return _clamp((20 * Math.log10(linear) + 52) / 46, 0.0, 1.0);
    }

    // Loudest sample in column `index` of `count`, oldest on the left.
    function _bucket(index, count) {
        const ring = samples;
        if (ring.length === 0) return 0.0;
        const capacity = 96;
        const pad = capacity - ring.length;
        const start = Math.floor(index * capacity / count) - pad;
        const end = Math.max(start + 1, Math.floor((index + 1) * capacity / count) - pad);
        let loudest = 0.0;
        for (let i = Math.max(0, start); i < Math.min(end, ring.length); i++) {
            if (ring[i] > loudest) loudest = ring[i];
        }
        return _perceptual(loudest);
    }

    function _easeLevels(levels, count, dt) {
        while (levels.length < count) levels.push(0.0);
        const live = daemonState === "recording" || daemonState === "streaming";
        const decoding = daemonState === "transcribing";
        for (let i = 0; i < count; i++) {
            let target = 0.0;
            if (live) {
                target = _bucket(i, count);
            } else if (decoding) {
                const t = i / Math.max(1, count - 1);
                target = 0.12 + 0.2 * Math.pow(0.5 + 0.5 * Math.sin(phase * 3.0 - t * Math.PI * 4.0), 2);
            }
            levels[i] = _approach(levels[i], target, target > levels[i] ? 20.0 : 7.0, dt);
        }
    }

    function _withAlpha(color, alpha) {
        const c = Qt.color(color);
        return Qt.rgba(c.r, c.g, c.b, alpha);
    }

    function _pad2(value) {
        const text = String(Math.max(0, Math.min(99, Math.round(value))));
        return text.length < 2 ? "0" + text : text;
    }

    function _deg(radians) {
        return radians * 180 / Math.PI;
    }

    Connections {
        target: root.audio
        enabled: root.audio !== null

        // Frames arrive at ~100 Hz; store them and let the tick draw.
        function onFrameReceived(framePeak, frameRms, vad, tsMs) {
            root.peak = _clamp(framePeak, 0.0, 1.0);
            root.rms = _clamp(frameRms, 0.0, 1.0);
            root.samples.push(root.peak);
            if (root.samples.length > 96) root.samples.shift();
        }

        function onDisconnected() {
            root._clearSignal();
        }
    }

    function _clearSignal() {
        peak = 0.0;
        rms = 0.0;
        vadLevel = 0.0;
        voiceLock = false;
        samples.length = 0;
    }

    onDaemonStateChanged: {
        // Audio stops when recording ends: drop the last frame's values so
        // the HUD doesn't hold them while decoding.
        if (daemonState !== "recording" && daemonState !== "streaming") {
            peak = 0.0;
            rms = 0.0;
            samples.length = 0;
        }
        if (daemonState === "idle" || daemonState === "") {
            _clearSignal();
            arcLevels.length = 0;
            stripLevels.length = 0;
        }
        live.requestPaint();
    }

    Timer {
        interval: 16
        repeat: true
        running: root.daemonState !== "idle" && root.daemonState !== ""
        onRunningChanged: root.lastTickMs = Date.now()
        onTriggered: {
            const now = Date.now();
            const dt = Math.min(0.06, Math.max(0.001, (now - root.lastTickMs) / 1000.0));
            root.lastTickMs = now;
            root.phase += dt;
            root.smoothPeak = root._approach(root.smoothPeak, root._perceptual(root.peak), 12.0, dt);
            root.smoothRms = root._approach(root.smoothRms, root._perceptual(root.rms), 8.0, dt);
            root.vadLevel = root._approach(root.vadLevel, root.audio && root.audio.vad ? 1.0 : 0.0, 6.0, dt);
            if (root.vadLevel > 0.7) root.voiceLock = true;
            else if (root.vadLevel < 0.25) root.voiceLock = false;

            root.spinSlow += dt * 0.34;
            root.spinFast -= dt * (0.72 + root.energy * 0.55);

            root._telemetryAge += dt;
            if (root._telemetryAge > 0.2) {
                root._telemetryAge = 0;
                root.shownPeak = root.smoothPeak;
                root.shownRms = root.smoothRms;
            }
            root._easeLevels(root.arcLevels, 72, dt);
            root._easeLevels(root.stripLevels, 34, dt);
            live.requestPaint();
        }
    }

    // A square canvas centered on the HUD that paints once, and again only
    // when its size or colors change. `draw(ctx, c, r)` receives the
    // canvas center and the HUD radius.
    component StaticLayer: Canvas {
        id: layer
        property real span: root.hudSize
        property var draw: null
        property var colors: []
        width: Math.ceil(span)
        height: Math.ceil(span)
        x: Math.round(root.cx - width / 2)
        y: Math.round(root.cy - height / 2)
        antialiasing: true
        onColorsChanged: requestPaint()
        onWidthChanged: requestPaint()
        onPaint: {
            const ctx = getContext("2d");
            ctx.clearRect(0, 0, width, height);
            if (draw) draw(ctx, width / 2, root.r);
        }
    }

    function _softCircle(ctx, c, radius, color, alpha) {
        const gradient = ctx.createRadialGradient(c, c, radius * 0.05, c, c, radius);
        gradient.addColorStop(0.0, _withAlpha(color, alpha));
        gradient.addColorStop(0.42, _withAlpha(color, alpha * 0.32));
        gradient.addColorStop(1.0, "rgba(0, 0, 0, 0)");
        ctx.fillStyle = gradient;
        ctx.beginPath();
        ctx.arc(c, c, radius, 0, Math.PI * 2);
        ctx.fill();
    }

    function _arc(ctx, c, radius, width, color, alpha, start, end) {
        ctx.save();
        ctx.globalAlpha = alpha;
        ctx.strokeStyle = color;
        ctx.lineWidth = width;
        ctx.lineCap = "round";
        ctx.beginPath();
        ctx.arc(c, c, radius, start, end);
        ctx.stroke();
        ctx.restore();
    }

    function _segmentedRing(ctx, c, radius, width, segments, gap, color, alpha, duty) {
        const step = Math.PI * 2 / segments;
        for (let i = 0; i < segments; i++) {
            const gate = ((i * 17) % 23) / 23.0;
            if (gate > duty) continue;
            _arc(ctx, c, radius, width, color, alpha * (0.45 + 0.55 * gate),
                 i * step + gap, (i + 1) * step - gap);
        }
    }

    function _ticks(ctx, c, radius, count, inner, color, alpha) {
        ctx.save();
        ctx.strokeStyle = color;
        ctx.lineWidth = 1;
        ctx.globalAlpha = alpha;
        ctx.beginPath();
        for (let i = 0; i < count; i++) {
            const a = i * Math.PI * 2 / count;
            const len = i % 6 === 0 ? inner * 1.8 : inner;
            ctx.moveTo(c + Math.cos(a) * (radius - len), c + Math.sin(a) * (radius - len));
            ctx.lineTo(c + Math.cos(a) * radius, c + Math.sin(a) * radius);
        }
        ctx.stroke();
        ctx.restore();
    }

    function _scanlines(ctx, c, radius, color) {
        const inner = radius * 0.74;
        const edgeFade = Math.max(12, radius * 0.16);
        ctx.save();
        ctx.beginPath();
        ctx.arc(c, c, inner, 0, Math.PI * 2);
        ctx.clip();
        ctx.lineWidth = 1;
        for (let yy = c - inner; yy <= c + inner; yy += 5) {
            const dy = yy - c;
            const half = Math.sqrt(Math.max(0, inner * inner - dy * dy));
            if (half <= 0.5) continue;
            const x0 = c - half;
            const x1 = c + half;
            const verticalAlpha = _clamp((inner - Math.abs(dy)) / edgeFade, 0.0, 1.0);
            const horizontalStop = _clamp(edgeFade / Math.max(1, half * 2), 0.0, 0.46);
            const alpha = 0.075 * verticalAlpha;
            const gradient = ctx.createLinearGradient(x0, yy, x1, yy);
            gradient.addColorStop(0.0, _withAlpha(color, 0.0));
            gradient.addColorStop(horizontalStop, _withAlpha(color, alpha));
            gradient.addColorStop(0.5, _withAlpha(color, alpha));
            gradient.addColorStop(1.0 - horizontalStop, _withAlpha(color, alpha));
            gradient.addColorStop(1.0, _withAlpha(color, 0.0));
            ctx.strokeStyle = gradient;
            ctx.beginPath();
            ctx.moveTo(x0, yy);
            ctx.lineTo(x1, yy);
            ctx.stroke();
        }
        ctx.restore();
    }

    // Outer glow, painted at its largest; energy scales and brightens it.
    StaticLayer {
        span: root.hudSize * 1.2
        colors: [root.glowColor]
        draw: function(ctx, c, r) { root._softCircle(ctx, c, r * 1.15, root.glowColor, 0.76); }
        scale: (1.03 + root.energy * 0.12) / 1.15
        opacity: (0.42 + root.energy * 0.34) / 0.76
    }

    StaticLayer {
        colors: [root.stateColor]
        draw: function(ctx, c, r) { root._softCircle(ctx, c, r * 0.54, root.stateColor, 0.38); }
        opacity: (0.16 + root.energy * 0.22) / 0.38
    }

    // Disc, scanlines, and the fixed inner grid ring.
    StaticLayer {
        colors: [root.bgColor, root.accentColor, root.gridColor]
        draw: function(ctx, c, r) {
            ctx.save();
            ctx.globalAlpha = 0.58;
            ctx.fillStyle = root.bgColor;
            ctx.beginPath();
            ctx.arc(c, c, r * 0.74, 0, Math.PI * 2);
            ctx.fill();
            ctx.restore();
            root._scanlines(ctx, c, r, root.accentColor);
            root._arc(ctx, c, r * 0.33, 1.2, root.gridColor, 0.64, 0, Math.PI * 2);
        }
    }

    StaticLayer {
        colors: [root.gridColor]
        draw: function(ctx, c, r) { root._segmentedRing(ctx, c, r * 0.46, 2.4, 28, 0.025, root.gridColor, 0.55, 0.86); }
        rotation: root._deg(root.spinSlow)
    }

    StaticLayer {
        colors: [root.stateColor]
        draw: function(ctx, c, r) { root._segmentedRing(ctx, c, r * 0.60, 3.0, 36, 0.018, root.stateColor, 1.0, 0.78); }
        rotation: root._deg(root.spinFast)
        opacity: 0.62 + root.energy * 0.24
    }

    StaticLayer {
        colors: [root.fgColor]
        draw: function(ctx, c, r) { root._segmentedRing(ctx, c, r * 0.72, 1.4, 72, 0.020, root.fgColor, 0.34, 0.58); }
        rotation: root._deg(-root.spinSlow * 0.7)
    }

    StaticLayer {
        colors: [root.stateColor]
        draw: function(ctx, c, r) { root._ticks(ctx, c, r * 0.82, 84, 8, root.stateColor, 0.52); }
        rotation: root._deg(root.phase * 0.12)
    }

    StaticLayer {
        colors: [root.fgColor]
        draw: function(ctx, c, r) { root._ticks(ctx, c, r * 0.52, 48, 5, root.fgColor, 0.24); }
        rotation: root._deg(-root.phase * 0.24)
    }

    // Voice-driven parts, repainted on each tick.
    Canvas {
        id: live
        width: Math.ceil(root.hudSize)
        height: Math.ceil(root.hudSize)
        x: Math.round(root.cx - width / 2)
        y: Math.round(root.cy - height / 2)
        antialiasing: true

        function _waveArc(ctx, c, radius, color, secondary) {
            const count = 72;
            const start = Math.PI * 0.82;
            const sweep = Math.PI * 1.36;
            ctx.save();
            ctx.lineCap = "round";
            for (let i = 0; i < count; i++) {
                const t = i / (count - 1);
                const sample = root.arcLevels.length > i ? root.arcLevels[i] : 0.0;
                const shaped = root._clamp(sample * 0.9 + root.smoothRms * 0.15, 0.025, 1.0);
                const a = start + t * sweep;
                const len = 6 + shaped * 34;
                const r0 = radius - len * 0.55;
                const r1 = radius + len;
                ctx.strokeStyle = i % 5 === 0 ? secondary : color;
                ctx.lineWidth = i % 5 === 0 ? 2.0 : 1.2;
                ctx.globalAlpha = 0.26 + shaped * 0.74;
                ctx.beginPath();
                ctx.moveTo(c + Math.cos(a) * r0, c + Math.sin(a) * r0);
                ctx.lineTo(c + Math.cos(a) * r1, c + Math.sin(a) * r1);
                ctx.stroke();
            }
            ctx.restore();
        }

        function _meterStrip(ctx, c, width, color, secondary) {
            const bars = 34;
            const gap = 2;
            const barWidth = (width - gap * (bars - 1)) / bars;
            const baseY = c + 96;
            ctx.save();
            for (let i = 0; i < bars; i++) {
                const sample = root.stripLevels.length > i ? root.stripLevels[i] : 0.0;
                const h = 4 + root._clamp(sample * 30, 0, 30);
                const x = c - width / 2 + i * (barWidth + gap);
                ctx.globalAlpha = 0.18;
                ctx.fillStyle = color;
                ctx.fillRect(x, baseY - h - 2, barWidth, h + 4);
                ctx.globalAlpha = 0.36 + sample * 0.72;
                ctx.fillStyle = i % 7 === 0 ? secondary : color;
                ctx.fillRect(x, baseY - h, barWidth, h);
            }
            ctx.restore();
        }

        onPaint: {
            const ctx = getContext("2d");
            ctx.clearRect(0, 0, width, height);
            const c = width / 2;
            const r = root.r;
            root._arc(ctx, c, r * (0.21 + root.energy * 0.045), 2.0, root.accentColor, 0.62, -Math.PI * 0.22, Math.PI * 1.35);
            root._arc(ctx, c, r * (0.26 + root.smoothRms * 0.06), 1.2, root.amberColor, 0.38 + root.vadLevel * 0.35, Math.PI * 0.72, Math.PI * 1.92);
            _waveArc(ctx, c, r * 0.69, root.stateColor, root.fgColor);
            _meterStrip(ctx, c, root.hudSize * 0.52, root.stateColor, root.amberColor);
        }
    }

    // Crosshair marks at the four compass points.
    StaticLayer {
        colors: [root.stateColor]
        draw: function(ctx, c, r) {
            ctx.save();
            ctx.strokeStyle = root.stateColor;
            ctx.lineWidth = 1;
            ctx.beginPath();
            ctx.moveTo(c - r * 0.92, c);
            ctx.lineTo(c - r * 0.77, c);
            ctx.moveTo(c + r * 0.77, c);
            ctx.lineTo(c + r * 0.92, c);
            ctx.moveTo(c, c - r * 0.92);
            ctx.lineTo(c, c - r * 0.77);
            ctx.moveTo(c, c + r * 0.77);
            ctx.lineTo(c, c + r * 0.92);
            ctx.stroke();
            ctx.restore();
        }
        opacity: 0.60 + root.energy * 0.30
    }

    // Telemetry, as text items so it isn't re-rasterized every frame.
    component Readout: Text {
        property real cxAt: 0
        property real cyAt: 0
        property int align: Text.AlignHCenter
        horizontalAlignment: align
        verticalAlignment: Text.AlignVCenter
        x: align === Text.AlignLeft ? cxAt : align === Text.AlignRight ? cxAt - implicitWidth : cxAt - implicitWidth / 2
        y: cyAt - implicitHeight / 2
        font.family: "JetBrains Mono"
        font.pixelSize: 9
        color: root.mutedColor
        opacity: 0.62
    }

    Readout {
        cxAt: root.cx; cyAt: root.cy - 22
        text: "A E G I S"
        font.pixelSize: 11
        opacity: 0.74
    }
    Readout {
        cxAt: root.cx; cyAt: root.cy + 3
        text: root._stateLabel()
        font.pixelSize: 22
        color: root.stateColor
        opacity: 0.94
    }
    Readout {
        cxAt: root.cx; cyAt: root.cy + 25
        text: root.voiceLock ? "VOICE LOCK" : "SIGNAL SEEK"
        color: root.amberColor
        opacity: 0.70 + root.vadLevel * 0.30
    }
    Readout {
        cxAt: root.cx - root.hudSize * 0.40; cyAt: root.cy - root.hudSize * 0.26
        align: Text.AlignLeft
        text: "PK " + root._pad2(root.shownPeak * 99)
    }
    Readout {
        cxAt: root.cx - root.hudSize * 0.40; cyAt: root.cy + root.hudSize * 0.30
        align: Text.AlignLeft
        text: "RMS " + root._pad2(root.shownRms * 99)
    }
    Readout {
        cxAt: root.cx + root.hudSize * 0.40; cyAt: root.cy - root.hudSize * 0.26
        align: Text.AlignRight
        text: "BUS " + (root.audio && root.audio.running ? "ON" : "WAIT")
    }
    Readout {
        cxAt: root.cx + root.hudSize * 0.40; cyAt: root.cy + root.hudSize * 0.30
        align: Text.AlignRight
        text: "QML PKG"
    }
}
