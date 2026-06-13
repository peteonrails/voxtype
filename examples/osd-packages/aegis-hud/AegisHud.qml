import QtQuick

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
    property real smoothPeak: 0.0
    property real smoothRms: 0.0
    property real phase: 0.0
    property real lastTickMs: Date.now()
    property var samples: []

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
        if (daemonState === "recording") return audio && audio.vad ? "VOICE" : "ARMED";
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

    function _sample(index, count) {
        const r = samples || [];
        if (r.length > 0) {
            const pos = index * (r.length - 1) / Math.max(1, count - 1);
            const lo = Math.floor(pos);
            const hi = Math.min(r.length - 1, lo + 1);
            const mix = pos - lo;
            return r[lo] * (1 - mix) + r[hi] * mix;
        }
        return 0.016 + 0.009 * Math.sin(phase * 0.9 + index * 0.37);
    }

    function _withAlpha(color, alpha) {
        if (String(color).indexOf("#") !== 0 || String(color).length !== 7) return color;
        const r = parseInt(String(color).slice(1, 3), 16);
        const g = parseInt(String(color).slice(3, 5), 16);
        const b = parseInt(String(color).slice(5, 7), 16);
        return "rgba(" + r + ", " + g + ", " + b + ", " + alpha + ")";
    }

    function _pad2(value) {
        const text = String(Math.max(0, Math.min(99, Math.round(value))));
        return text.length < 2 ? "0" + text : text;
    }

    Connections {
        target: root.audio
        enabled: root.audio !== null

        function onFrameReceived(framePeak, frameRms, vad, tsMs) {
            root.peak = _clamp(framePeak, 0.0, 1.0);
            root.rms = _clamp(frameRms, 0.0, 1.0);
            root.vadLevel = vad ? 1.0 : 0.0;

            const next = root.samples.slice();
            next.push(root.peak);
            while (next.length > 96) next.shift();
            root.samples = next;
        }

        function onDisconnected() {
            root.peak = 0.0;
            root.rms = 0.0;
            root.vadLevel = 0.0;
            root.samples = [];
        }
    }

    onDaemonStateChanged: {
        if (daemonState === "idle" || daemonState === "") {
            peak = 0.0;
            rms = 0.0;
            vadLevel = 0.0;
            samples = [];
        }
        hud.requestPaint();
    }

    Timer {
        interval: 16
        repeat: true
        running: true
        onTriggered: {
            const now = Date.now();
            const dt = Math.min(0.06, Math.max(0.001, (now - root.lastTickMs) / 1000.0));
            root.lastTickMs = now;
            root.phase += dt;
            root.smoothPeak = root._approach(root.smoothPeak, root.peak, 15.0, dt);
            root.smoothRms = root._approach(root.smoothRms, root.rms, 10.0, dt);
            root.vadLevel = root._approach(root.vadLevel, root.audio && root.audio.vad ? 1.0 : 0.0, 11.0, dt);
            hud.requestPaint();
        }
    }

    Canvas {
        id: hud
        anchors.fill: parent
        antialiasing: true

        function _drawSoftCircle(ctx, cx, cy, radius, color, alpha) {
            const gradient = ctx.createRadialGradient(cx, cy, radius * 0.05, cx, cy, radius);
            gradient.addColorStop(0.0, root._withAlpha(color, alpha));
            gradient.addColorStop(0.42, root._withAlpha(color, alpha * 0.32));
            gradient.addColorStop(1.0, "rgba(0, 0, 0, 0)");
            ctx.fillStyle = gradient;
            ctx.beginPath();
            ctx.arc(cx, cy, radius, 0, Math.PI * 2);
            ctx.fill();
        }

        function _ring(ctx, cx, cy, radius, width, color, alpha, start, end) {
            ctx.save();
            ctx.globalAlpha = alpha;
            ctx.strokeStyle = color;
            ctx.lineWidth = width;
            ctx.lineCap = "round";
            ctx.beginPath();
            ctx.arc(cx, cy, radius, start, end);
            ctx.stroke();
            ctx.restore();
        }

        function _segmentedRing(ctx, cx, cy, radius, width, segments, gap, color, alpha, spin, duty) {
            const full = Math.PI * 2;
            const step = full / segments;
            for (let i = 0; i < segments; i++) {
                const gate = ((i * 17) % 23) / 23.0;
                if (gate > duty) continue;
                const a0 = spin + i * step + gap;
                const a1 = spin + (i + 1) * step - gap;
                _ring(ctx, cx, cy, radius, width, color, alpha * (0.45 + 0.55 * gate), a0, a1);
            }
        }

        function _ticks(ctx, cx, cy, radius, count, inner, color, alpha, spin) {
            ctx.save();
            ctx.strokeStyle = color;
            ctx.lineWidth = 1;
            ctx.globalAlpha = alpha;
            for (let i = 0; i < count; i++) {
                const a = spin + i * Math.PI * 2 / count;
                const longTick = i % 6 === 0;
                const len = longTick ? inner * 1.8 : inner;
                const x0 = cx + Math.cos(a) * (radius - len);
                const y0 = cy + Math.sin(a) * (radius - len);
                const x1 = cx + Math.cos(a) * radius;
                const y1 = cy + Math.sin(a) * radius;
                ctx.beginPath();
                ctx.moveTo(x0, y0);
                ctx.lineTo(x1, y1);
                ctx.stroke();
            }
            ctx.restore();
        }

        function _waveArc(ctx, cx, cy, radius, color, secondary) {
            const count = 72;
            const start = Math.PI * 0.82;
            const sweep = Math.PI * 1.36;
            ctx.save();
            ctx.lineCap = "round";
            for (let i = 0; i < count; i++) {
                const t = count <= 1 ? 0 : i / (count - 1);
                const sample = root._sample(i, count);
                const shaped = root._clamp(sample * 4.8 + root.smoothRms * 1.35, 0.025, 1.0);
                const a = start + t * sweep;
                const len = 6 + shaped * 34;
                const r0 = radius - len * 0.55;
                const r1 = radius + len;
                ctx.strokeStyle = i % 5 === 0 ? secondary : color;
                ctx.lineWidth = i % 5 === 0 ? 2.0 : 1.2;
                ctx.globalAlpha = 0.26 + shaped * 0.74;
                ctx.beginPath();
                ctx.moveTo(cx + Math.cos(a) * r0, cy + Math.sin(a) * r0);
                ctx.lineTo(cx + Math.cos(a) * r1, cy + Math.sin(a) * r1);
                ctx.stroke();
            }
            ctx.restore();
        }

        function _meterStrip(ctx, cx, cy, width, color, secondary) {
            const bars = 34;
            const gap = 2;
            const barWidth = (width - gap * (bars - 1)) / bars;
            const baseY = cy + 96;
            ctx.save();
            for (let i = 0; i < bars; i++) {
                const sample = root._sample(i, bars);
                const h = 4 + root._clamp(sample * 56 + root.smoothRms * 28, 0, 34);
                const x = cx - width / 2 + i * (barWidth + gap);
                ctx.globalAlpha = 0.18;
                ctx.fillStyle = color;
                ctx.fillRect(x, baseY - h - 2, barWidth, h + 4);
                ctx.globalAlpha = 0.36 + sample * 0.72;
                ctx.fillStyle = i % 7 === 0 ? secondary : color;
                ctx.fillRect(x, baseY - h, barWidth, h);
            }
            ctx.restore();
        }

        function _telemetry(ctx, cx, cy, size, color, muted, amber) {
            ctx.save();
            ctx.textAlign = "center";
            ctx.textBaseline = "middle";
            ctx.font = "11px JetBrains Mono, monospace";
            ctx.fillStyle = muted;
            ctx.globalAlpha = 0.74;
            ctx.fillText("A E G I S", cx, cy - 22);
            ctx.font = "22px JetBrains Mono, monospace";
            ctx.fillStyle = color;
            ctx.globalAlpha = 0.94;
            ctx.fillText(root._stateLabel(), cx, cy + 3);
            ctx.font = "9px JetBrains Mono, monospace";
            ctx.fillStyle = amber;
            ctx.globalAlpha = 0.70 + root.vadLevel * 0.30;
            ctx.fillText(root.vadLevel > 0.5 ? "VOICE LOCK" : "SIGNAL SEEK", cx, cy + 25);

            ctx.textAlign = "left";
            ctx.fillStyle = muted;
            ctx.globalAlpha = 0.62;
            ctx.fillText("PK " + root._pad2(root.smoothPeak * 100), cx - size * 0.40, cy - size * 0.26);
            ctx.fillText("RMS " + root._pad2(root.smoothRms * 100), cx - size * 0.40, cy + size * 0.30);
            ctx.textAlign = "right";
            ctx.fillText("BUS " + (root.audio && root.audio.running ? "ON" : "WAIT"), cx + size * 0.40, cy - size * 0.26);
            ctx.fillText("QML PKG", cx + size * 0.40, cy + size * 0.30);
            ctx.restore();
        }

        function _scanlines(ctx, cx, cy, radius, color) {
            const inner = radius * 0.74;
            const edgeFade = Math.max(12, radius * 0.16);
            ctx.save();
            ctx.beginPath();
            ctx.arc(cx, cy, inner, 0, Math.PI * 2);
            ctx.clip();
            ctx.lineWidth = 1;
            for (let yy = cy - inner; yy <= cy + inner; yy += 5) {
                const dy = yy - cy;
                const half = Math.sqrt(Math.max(0, inner * inner - dy * dy));
                if (half <= 0.5) continue;
                const x0 = cx - half;
                const x1 = cx + half;
                const verticalAlpha = root._clamp((inner - Math.abs(dy)) / edgeFade, 0.0, 1.0);
                const horizontalStop = root._clamp(edgeFade / Math.max(1, half * 2), 0.0, 0.46);
                const alpha = 0.075 * verticalAlpha;
                const gradient = ctx.createLinearGradient(x0, yy, x1, yy);
                gradient.addColorStop(0.0, root._withAlpha(color, 0.0));
                gradient.addColorStop(horizontalStop, root._withAlpha(color, alpha));
                gradient.addColorStop(0.5, root._withAlpha(color, alpha));
                gradient.addColorStop(1.0 - horizontalStop, root._withAlpha(color, alpha));
                gradient.addColorStop(1.0, root._withAlpha(color, 0.0));
                ctx.strokeStyle = gradient;
                ctx.beginPath();
                ctx.moveTo(x0, yy);
                ctx.lineTo(x1, yy);
                ctx.stroke();
            }
            ctx.restore();
        }

        onPaint: {
            const ctx = getContext("2d");
            ctx.clearRect(0, 0, width, height);

            const size = root._hudSize();
            const x = root._hudX(size);
            const y = root._hudY(size);
            const cx = x + size / 2;
            const cy = y + size / 2;
            const r = size / 2;
            const state = root._stateColor();
            const accent = root._color("accent", "#38D8FF");
            const fg = root._color("foreground", "#D8FAFF");
            const muted = root._color("muted", "rgba(154, 221, 232, 0.62)");
            const grid = root._color("grid", "rgba(73, 213, 255, 0.20)");
            const glow = root._color("glow", "rgba(56, 216, 255, 0.36)");
            const amber = root._color("amber", "#FFB84A");
            const energy = root._clamp(root.smoothPeak * 1.6 + root.smoothRms * 3.0 + root.vadLevel * 0.25, 0.0, 1.0);

            _drawSoftCircle(ctx, cx, cy, r * (1.03 + energy * 0.12), glow, 0.42 + energy * 0.34);
            _drawSoftCircle(ctx, cx, cy, r * 0.54, state, 0.16 + energy * 0.22);

            ctx.save();
            ctx.globalAlpha = 0.58;
            ctx.fillStyle = root._color("background", "rgba(3, 11, 18, 0.72)");
            ctx.beginPath();
            ctx.arc(cx, cy, r * 0.74, 0, Math.PI * 2);
            ctx.fill();
            ctx.restore();

            const slowSpin = root.phase * 0.34;
            const fastSpin = -root.phase * (0.72 + energy * 0.55);
            _segmentedRing(ctx, cx, cy, r * 0.46, 2.4, 28, 0.025, grid, 0.55, slowSpin, 0.86);
            _segmentedRing(ctx, cx, cy, r * 0.60, 3.0, 36, 0.018, state, 0.62 + energy * 0.24, fastSpin, 0.78);
            _segmentedRing(ctx, cx, cy, r * 0.72, 1.4, 72, 0.020, fg, 0.34, -slowSpin * 0.7, 0.58);
            _ring(ctx, cx, cy, r * 0.33, 1.2, grid, 0.64, 0, Math.PI * 2);
            _ring(ctx, cx, cy, r * (0.21 + energy * 0.045), 2.0, accent, 0.62, -Math.PI * 0.22, Math.PI * 1.35);
            _ring(ctx, cx, cy, r * (0.26 + root.smoothRms * 0.08), 1.2, amber, 0.38 + root.vadLevel * 0.35, Math.PI * 0.72, Math.PI * 1.92);

            _ticks(ctx, cx, cy, r * 0.82, 84, 8, state, 0.52, root.phase * 0.12);
            _ticks(ctx, cx, cy, r * 0.52, 48, 5, fg, 0.24, -root.phase * 0.24);
            _waveArc(ctx, cx, cy, r * 0.69, state, fg);
            _meterStrip(ctx, cx, cy, size * 0.52, state, amber);
            _telemetry(ctx, cx, cy, size, state, muted, amber);
            _scanlines(ctx, cx, cy, r, accent);

            ctx.save();
            ctx.globalAlpha = 0.60 + energy * 0.30;
            ctx.strokeStyle = state;
            ctx.lineWidth = 1;
            ctx.beginPath();
            ctx.moveTo(cx - r * 0.92, cy);
            ctx.lineTo(cx - r * 0.77, cy);
            ctx.moveTo(cx + r * 0.77, cy);
            ctx.lineTo(cx + r * 0.92, cy);
            ctx.moveTo(cx, cy - r * 0.92);
            ctx.lineTo(cx, cy - r * 0.77);
            ctx.moveTo(cx, cy + r * 0.77);
            ctx.lineTo(cx, cy + r * 0.92);
            ctx.stroke();
            ctx.restore();
        }
    }
}
