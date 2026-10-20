// High-polish declarative voice-meter renderer for VoxType Quickshell OSD recipes.

import QtQuick

Item {
    id: root

    property var style: null
    property var ring: []
    // Signed sample range per frame, parallel to `ring` (classic card).
    property var ringMin: []
    property var ringMax: []
    // Draw the classic card: a port of the GTK4 frontend's drawing
    // (src/bin/voxtype_osd_gtk4.rs) instead of the recipe layers.
    property bool classic: false
    // Scroll clock from the host (see OsdSurface.qml): frames received
    // this recording and the wall-clock origin, 100 frames per second.
    property int framesReceived: 0
    property real clockOriginMs: -1
    property real playoutDelayFrames: 5
    property real peakDecayDbPerSec: 6.0
    property string daemonState: "idle"
    property bool vad: false
    property real currentPeakDbfs: -120
    property real rms: 0.0
    property real peak: 0.0
    property int waveformColumns: 300
    property real waveformGain: 10.0

    property real _peak: 0.0
    property real _rms: 0.0
    property real _vad: 0.0
    property real _energy: 0.0
    property real _phase: 0.0
    property var _barLevels: []
    // Absolute frame index at the waveform's right edge, advanced on every
    // display frame from the scroll clock.
    property real _scrollPosition: -1
    // Level meter on the waveform's display scale, advanced once per
    // display frame (see osd::visual::LevelMeter).
    property real _meterLevel: 0.0
    property real _meterTargetDbfs: -120
    property real _heldDbfs: -120
    property int _meterFramesSeen: 0
    // Layers sorted by order, cached so the per-frame paint loop doesn't
    // re-slice and re-sort the recipe on every frame.
    property var _sortedLayers: []

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
        _sortedLayers = layers.slice().sort(function(a, b) {
            return (a.order || 0) - (b.order || 0);
        });
    }

    // Numeric layer field with an explicit-unset check. Unset tunables are
    // omitted from the runtime JSON, so `undefined` means "use the layer
    // type's default" while explicit zeros are honored.
    function _num(value, fallback) {
        return value === undefined || value === null ? fallback : Number(value);
    }

    function _color(role, fallback) {
        const value = style && style.color ? style.color(role, fallback) : (fallback || "#ffffff");
        return _canvasColor(value);
    }

    // StyleLoader returns QML-compatible #AARRGGBB for rgba() values so
    // Rectangle.color bindings work. Canvas color stops/fills are CSS-like,
    // so convert that form back before painting recipe layers.
    function _canvasColor(value) {
        if (typeof value !== "string") return value;
        const str = String(value);
        if (/^#[0-9a-fA-F]{8}$/.test(str)) {
            const a = parseInt(str.slice(1, 3), 16) / 255;
            const r = parseInt(str.slice(3, 5), 16);
            const g = parseInt(str.slice(5, 7), 16);
            const b = parseInt(str.slice(7, 9), 16);
            return "rgba(" + r + ", " + g + ", " + b + ", " + a + ")";
        }
        return str;
    }

    // Parse a resolved color string into {r, g, b, a} components (0..1)
    // for draws that need to rebuild alpha gradients (shadow). Handles
    // #RGB, #RRGGBB, #AARRGGBB, and rgb()/rgba() strings.
    function _colorComponents(value) {
        const str = String(value);
        if (str[0] === "#") {
            const hex = str.slice(1);
            if (hex.length === 3) {
                return {
                    r: parseInt(hex[0] + hex[0], 16) / 255,
                    g: parseInt(hex[1] + hex[1], 16) / 255,
                    b: parseInt(hex[2] + hex[2], 16) / 255,
                    a: 1.0
                };
            }
            const hasAlpha = hex.length === 8;
            const off = hasAlpha ? 2 : 0;
            return {
                r: parseInt(hex.slice(off, off + 2), 16) / 255,
                g: parseInt(hex.slice(off + 2, off + 4), 16) / 255,
                b: parseInt(hex.slice(off + 4, off + 6), 16) / 255,
                a: hasAlpha ? parseInt(hex.slice(0, 2), 16) / 255 : 1.0
            };
        }
        const m = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(str);
        if (m) {
            return {
                r: Number(m[1]) / 255,
                g: Number(m[2]) / 255,
                b: Number(m[3]) / 255,
                a: m[4] === undefined ? 1.0 : Number(m[4])
            };
        }
        return { r: 0, g: 0, b: 0, a: 1.0 };
    }

    function _rgbaString(c, alpha) {
        return "rgba(" + Math.round(c.r * 255) + ", " + Math.round(c.g * 255) + ", "
            + Math.round(c.b * 255) + ", " + (c.a * alpha) + ")";
    }

    function _layerRect(layer, width, height) {
        const lx = _num(layer.x, 0);
        const ly = _num(layer.y, 0);
        const lw = _num(layer.width, 1);
        const lh = _num(layer.height, 1);
        return {
            x: lx <= 1 ? lx * width : lx,
            y: ly <= 1 ? ly * height : ly,
            w: lw <= 1 ? lw * width : lw,
            h: lh <= 1 ? lh * height : lh
        };
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

    function _signal(layer) {
        const src = layer.source || "peak";
        if (src === "rms") return _rms;
        if (src === "vad") return _vad;
        if (src === "state") return daemonState === "recording" ? 1.0 : 0.0;
        if (src === "none") return 0.0;
        return _peak;
    }

    // Mirrors osd::visual::waveform_level in the Rust frontends: absolute
    // input level on a fixed dB scale, -48 dBFS at the floor and 0 dBFS at
    // full height, shifted by `gain` (10 is neutral, each doubling +6 dB).
    // No normalization to recent input, so louder speech always draws
    // taller and normal speech (around -20 dBFS) sits near mid-height.
    readonly property real waveformFloorDbfs: -48.0

    function _displayLevel(sample, gain) {
        const magnitude = Math.abs(sample);
        if (!(magnitude > 0)) return 0.0;
        const shift = isFinite(gain) && gain > 0 ? 20 * Math.log10(gain / 10) : 0;
        const db = 20 * Math.log10(magnitude) + shift;
        return _clamp((db - waveformFloorDbfs) / -waveformFloorDbfs, 0, 1);
    }

    function _dbfsLevel(dbfs, gain) {
        if (!isFinite(dbfs)) return 0.0;
        const shift = isFinite(gain) && gain > 0 ? 20 * Math.log10(gain / 10) : 0;
        return _clamp((dbfs + shift - waveformFloorDbfs) / -waveformFloorDbfs, 0, 1);
    }

    function resetMeter() {
        _meterLevel = 0.0;
        _meterTargetDbfs = -120;
        _heldDbfs = -120;
        _meterFramesSeen = 0;
        _scrollPosition = -1;
    }

    // Advance the meter and the scroll position by one display frame.
    // Mirrors osd::visual::LevelMeter: deliveries only set the target (the
    // loudest frame since the last display frame); the meter rises with a
    // 15 ms time constant and falls with 300 ms, and the held peak falls
    // at peakDecayDbPerSec.
    function _advance(dt) {
        const r = ring || [];
        const fresh = Math.min(r.length, Math.max(0, framesReceived - _meterFramesSeen));
        _meterFramesSeen = framesReceived;
        if (fresh > 0) {
            let loudest = 0.0;
            for (let i = r.length - fresh; i < r.length; i++) loudest = Math.max(loudest, r[i]);
            _meterTargetDbfs = loudest > 0 ? 20 * Math.log10(loudest) : -120;
        }
        const step = Math.min(0.25, Math.max(0, dt));
        const target = _dbfsLevel(_meterTargetDbfs, waveformGain);
        const tau = target > _meterLevel ? 0.015 : 0.3;
        _meterLevel += (target - _meterLevel) * (1 - Math.exp(-step / tau));
        _heldDbfs = Math.max(_meterTargetDbfs, Math.max(-120, _heldDbfs - peakDecayDbPerSec * step));

        if (clockOriginMs < 0 || framesReceived === 0) {
            _scrollPosition = -1;
        } else {
            const elapsed = (Date.now() - clockOriginMs) / 10;
            _scrollPosition = Math.min(framesReceived - 1, elapsed - playoutDelayFrames);
        }
    }

    function _sampleRing(index, count, fallbackPhase) {
        const r = ring || [];
        if (r.length > 0) {
            // Map columns onto the ring's full capacity, right-aligned, so
            // a partially filled ring reads as silence on the left instead
            // of stretching early samples across the full width (which made
            // the trace shrink as history accumulated).
            const capacity = Math.max(r.length, waveformColumns);
            const pad = capacity - r.length;
            const start = Math.floor(index * capacity / Math.max(1, count)) - pad;
            const end = Math.max(start + 1, Math.floor((index + 1) * capacity / Math.max(1, count)) - pad);
            let sum = 0.0;
            let n = 0;
            for (let i = Math.max(0, start); i < Math.min(end, r.length); i++) {
                sum += _clamp(r[i], 0, 1);
                n++;
            }
            return n > 0 ? sum / n : 0.0;
        }
        const breath = 0.5 + 0.5 * Math.sin(fallbackPhase * 0.35);
        return 0.004 + breath * 0.003;
    }

    // Per-column temporal smoothing for the bars layer: rise quickly, fall
    // more slowly, which reads as intentional meter inertia.
    function _updateLevels(levels, count, rise, fall, dt) {
        const next = levels.length === count ? levels.slice() : [];
        while (next.length < count) next.push(0.0);
        for (let i = 0; i < count; i++) {
            const target = daemonState === "idle"
                ? 0.0
                : _sampleRing(i, count, _phase + i * 0.17);
            const stiffness = target > next[i] ? rise : fall;
            next[i] = _approach(next[i], target, stiffness, dt);
        }
        return next;
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

    function _barValue(index, count) {
        return _levelAt(_barLevels, index, count);
    }

    function _paintRounded(ctx, x, y, w, h, r) {
        if (w <= 0 || h <= 0) return;
        const rr = _clamp(r || 0, 0, Math.min(w, h) / 2);
        ctx.beginPath();
        ctx.moveTo(x + rr, y);
        ctx.lineTo(x + w - rr, y);
        ctx.quadraticCurveTo(x + w, y, x + w, y + rr);
        ctx.lineTo(x + w, y + h - rr);
        ctx.quadraticCurveTo(x + w, y + h, x + w - rr, y + h);
        ctx.lineTo(x + rr, y + h);
        ctx.quadraticCurveTo(x, y + h, x, y + h - rr);
        ctx.lineTo(x, y + rr);
        ctx.quadraticCurveTo(x, y, x + rr, y);
        ctx.closePath();
        ctx.fill();
    }

    function _strokeSoftLine(ctx, color, alpha, width, draw) {
        ctx.save();
        ctx.globalAlpha = alpha * 0.25;
        ctx.strokeStyle = color;
        ctx.lineWidth = width * 4.0;
        ctx.lineCap = "round";
        ctx.lineJoin = "round";
        draw();
        ctx.stroke();
        ctx.globalAlpha = alpha * 0.55;
        ctx.lineWidth = width * 2.0;
        draw();
        ctx.stroke();
        ctx.globalAlpha = alpha;
        ctx.lineWidth = width;
        draw();
        ctx.stroke();
        ctx.restore();
    }

    function _drawBackground(ctx, layer, rect) {
        const signal = _signal(layer);
        const pulse = _clamp(0.18 + signal * _num(layer.gain, 1.0) * 0.8, 0.08, 1.0);
        const alpha = _num(layer.opacity, 0.18) * pulse;
        const color = _color(layer.color || "accent", "#66c7ff");
        const gradient = ctx.createLinearGradient(rect.x, rect.y, rect.x + rect.w, rect.y + rect.h);
        gradient.addColorStop(0.0, color);
        gradient.addColorStop(0.55, _color("surface", "rgba(26, 26, 31, 0.55)"));
        gradient.addColorStop(1.0, color);
        ctx.save();
        ctx.globalAlpha = alpha;
        ctx.fillStyle = gradient;
        _paintRounded(ctx, rect.x, rect.y, rect.w, rect.h, _num(layer.radius, 12));
        ctx.restore();
    }

    function _drawShadow(ctx, layer, rect) {
        const signal = _signal(layer);
        const gain = _num(layer.gain, 1.0);
        const opacity = _num(layer.opacity, 0.9);
        const alpha = opacity * _clamp(0.34 + signal * gain * 0.34, 0.18, 0.92);
        const cx = rect.x + rect.w / 2;
        const cy = rect.y + rect.h * 0.55;
        const outer = Math.min(rect.w, rect.h) * _clamp(_num(layer.radius, 0.58), 0.25, 0.85);
        const inner = outer * 0.08;
        // Shadow defaults to black; a recipe can recolor it (e.g. the
        // theme background role) for tinted backdrops.
        const tint = _colorComponents(_color(layer.color, "#000000"));

        ctx.save();
        ctx.translate(cx, cy);
        ctx.scale(1.08, 0.92);
        const gradient = ctx.createRadialGradient(0, 0, inner, 0, 0, outer);
        gradient.addColorStop(0.0, _rgbaString(tint, alpha));
        gradient.addColorStop(0.36, _rgbaString(tint, alpha * 0.58));
        gradient.addColorStop(0.72, _rgbaString(tint, alpha * 0.24));
        gradient.addColorStop(1.0, _rgbaString(tint, 0.0));
        ctx.fillStyle = gradient;
        ctx.beginPath();
        ctx.arc(0, 0, outer, 0, Math.PI * 2);
        ctx.fill();
        ctx.restore();
    }

    function _drawPulse(ctx, layer, rect) {
        const signal = _signal(layer);
        const speed = _num(layer.speed, 1.0);
        const breath = 0.5 + 0.5 * Math.sin(_phase * 1.6 * speed);
        const alpha = _num(layer.opacity, 0.24)
            * _clamp(0.22 + signal * _num(layer.gain, 1.0) * 1.2 + breath * 0.12, 0.0, 1.0);
        const inset = 2 + breath * 5;
        ctx.save();
        ctx.globalAlpha = alpha;
        ctx.fillStyle = _color(layer.color || "accent", "#66c7ff");
        _paintRounded(
            ctx,
            rect.x + inset,
            rect.y + inset,
            rect.w - inset * 2,
            rect.h - inset * 2,
            _num(layer.radius, 12)
        );
        ctx.restore();
    }

    function _drawBars(ctx, layer, rect) {
        const gain = _num(layer.gain, 1.0) * waveformGain;
        const opacity = _num(layer.opacity, 1.0);
        const radius = _num(layer.radius, 5);
        const color = _color(layer.color || "accent", "#66c7ff");
        const bars = Math.min(28, Math.max(12, Math.floor(rect.w / 17)));
        const gap = Math.max(2.0, rect.w / bars * 0.24);
        const barW = Math.max(2.0, rect.w / bars - gap);
        const mirror = layer.mirror !== false;
        const center = (bars - 1) / 2;
        const breath = 0.97 + 0.03 * Math.sin(_phase * 0.85);
        const haloColor = _color("foreground", color);
        const baseColor = _color("accent", color);

        ctx.save();
        for (let i = 0; i < bars; i++) {
            const position = bars <= 1 ? 0.5 : i / (bars - 1);
            const weight = 0.72 + 0.28 * Math.cos((i - center) / Math.max(1, center) * Math.PI);
            const sample = _barValue(i, bars);
            const level = _clamp(_displayLevel(sample, gain) * weight * breath + _energy * 0.045, 0.028, 1.0);
            const eased = _easeOutCubic(level);
            const h = Math.max(3, rect.h * eased);
            const x = rect.x + i * (barW + gap) + gap * 0.5;
            const y = mirror ? rect.y + (rect.h - h) / 2 : rect.y + rect.h - h;

            const grad = ctx.createLinearGradient(x, y, x, y + h);
            grad.addColorStop(0.0, haloColor);
            grad.addColorStop(0.45, color);
            grad.addColorStop(1.0, baseColor);

            ctx.globalAlpha = opacity * 0.18;
            ctx.fillStyle = color;
            _paintRounded(ctx, x - 1.5, y - 1.5, barW + 3, h + 3, radius + 2);

            ctx.globalAlpha = opacity * (0.48 + 0.52 * position);
            ctx.fillStyle = grad;
            _paintRounded(ctx, x, y, barW, h, radius);
        }
        ctx.restore();
    }

    // Scrolling waveform. Every frame sits at a fixed x for its place in
    // time, offset by the scroll clock, so the trace slides at a constant
    // rate and audio already drawn never changes shape: no re-bucketing of
    // frames into columns, no per-column smoothing, no animated wobble.
    function _drawWaveform(ctx, layer, rect) {
        const color = _color(layer.color || "accent", "#66c7ff");
        const opacity = _num(layer.opacity, 1.0);
        const gain = _num(layer.gain, 1.0) * waveformGain;
        // mirror=false anchors the envelope to the bottom edge instead of
        // reflecting it around the vertical center.
        const mirror = layer.mirror !== false;
        const cy = mirror ? rect.y + rect.h / 2 : rect.y + rect.h;
        const amp = Math.max(1, rect.h * (mirror ? 0.45 : 0.9));
        const minLevel = 0.015;

        ctx.save();
        ctx.beginPath();
        ctx.rect(rect.x, rect.y, rect.w, rect.h);
        ctx.clip();

        // Baseline across the whole window, so silence and the part of the
        // window before the recording started read as a flat line.
        ctx.globalAlpha = opacity * 0.35;
        ctx.strokeStyle = color;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(rect.x, cy - minLevel * amp);
        ctx.lineTo(rect.x + rect.w, cy - minLevel * amp);
        if (mirror) {
            ctx.moveTo(rect.x, cy + minLevel * amp);
            ctx.lineTo(rect.x + rect.w, cy + minLevel * amp);
        }
        ctx.stroke();

        const r = ring || [];
        if (r.length === 0 || _scrollPosition < 0) {
            ctx.restore();
            return;
        }
        const span = Math.max(2, waveformColumns) - 1;
        const step = rect.w / span;
        const firstIndex = framesReceived - r.length;
        const right = _scrollPosition;
        const xs = [];
        const levels = [];
        for (let k = 0; k < r.length; k++) {
            const x = rect.x + rect.w - (right - (firstIndex + k)) * step;
            if (x < rect.x - step || x > rect.x + rect.w + step) continue;
            xs.push(x);
            levels.push(Math.max(minLevel, _displayLevel(r[k], gain)));
        }
        if (xs.length < 2) {
            ctx.restore();
            return;
        }

        function edge(sign) {
            ctx.moveTo(xs[0], cy - sign * levels[0] * amp);
            for (let i = 1; i < xs.length; i++) ctx.lineTo(xs[i], cy - sign * levels[i] * amp);
        }

        ctx.globalAlpha = opacity * 0.18;
        ctx.fillStyle = color;
        ctx.beginPath();
        edge(1);
        if (mirror) {
            for (let i = xs.length - 1; i >= 0; i--) ctx.lineTo(xs[i], cy + levels[i] * amp);
        } else {
            ctx.lineTo(xs[xs.length - 1], cy);
            ctx.lineTo(xs[0], cy);
        }
        ctx.closePath();
        ctx.fill();

        _strokeSoftLine(ctx, color, opacity * 0.75, 1.4, function() {
            ctx.beginPath();
            edge(1);
            if (mirror) edge(-1);
        });
        ctx.restore();
    }

    // Same scale as the waveform: a loudness that reaches half the
    // waveform's height fills half the meter.
    function _drawMeter(ctx, layer, rect) {
        const fill = _clamp(_meterLevel, 0, 1);
        const heldFill = _dbfsLevel(_heldDbfs, waveformGain);
        const opacity = _num(layer.opacity, 1.0);
        const radius = _num(layer.radius, rect.h / 2);

        ctx.save();
        ctx.globalAlpha = opacity * 0.28;
        ctx.fillStyle = _color("muted", "rgba(255, 255, 255, 0.25)");
        _paintRounded(ctx, rect.x, rect.y, rect.w, rect.h, radius);

        // layer.color sets the low-zone color; the mid/high stops keep
        // the warning/error roles so hot levels still read as hot.
        const grad = ctx.createLinearGradient(rect.x, rect.y, rect.x + rect.w, rect.y);
        grad.addColorStop(0.0, _color(layer.color || "success", "#4dd973"));
        grad.addColorStop(0.72, _color("warning", "#f2cc4d"));
        grad.addColorStop(1.0, _color("error", "#f2594d"));
        ctx.globalAlpha = opacity;
        ctx.fillStyle = grad;
        _paintRounded(ctx, rect.x, rect.y, rect.w * fill, rect.h, radius);

        if (heldFill > 0) {
            ctx.globalAlpha = opacity;
            ctx.fillStyle = _color(layer.secondary_color || "foreground", "#fcfbf8");
            _paintRounded(ctx, Math.max(rect.x, rect.x + rect.w * heldFill - 1.5), rect.y - 2, 3, rect.h + 4, 2);
        }
        ctx.restore();
    }

    function _drawRing(ctx, layer, rect) {
        const signal = _signal(layer);
        const speed = _num(layer.speed, 1.0);
        const breath = 0.5 + 0.5 * Math.sin(_phase * 2.4 * speed);
        const base = Math.min(rect.w, rect.h);
        const baseScale = _num(layer.base_scale, 0.32);
        const responseScale = _num(layer.response_scale, 0.18);
        const responseCurve = Math.max(0.01, _num(layer.response_curve, 1.0));
        const breathScale = _num(layer.breath_scale, 0.045);
        const maxScale = _num(layer.max_scale, 0.46);
        const cx = rect.x + rect.w / 2;
        const cy = rect.y + rect.h / 2;
        const color = _color(layer.color || "accent", "#66c7ff");
        const opacity = _num(layer.opacity, 1.0);
        const stroke = Math.max(2, _num(layer.radius, 3));
        const safeRadius = Math.max(1, base / 2 - Math.max(10, stroke * 2.7));
        const response = Math.pow(_clamp(signal * _num(layer.gain, 1.0), 0, 1), responseCurve);
        const targetRadius = base * (baseScale + response * responseScale + breath * breathScale);
        const radius = Math.min(safeRadius, base * maxScale, targetRadius);

        ctx.save();
        for (let pass = 0; pass < 3; pass++) {
            ctx.globalAlpha = opacity * [0.16, 0.32, 0.95][pass];
            ctx.strokeStyle = color;
            ctx.lineWidth = [10, 5, stroke][pass];
            ctx.beginPath();
            ctx.arc(cx, cy, radius + pass * 2, -Math.PI * 0.65, Math.PI * 1.35);
            ctx.stroke();
        }
        ctx.globalAlpha = opacity * 0.55;
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(cx + Math.cos(_phase * 2.2) * radius, cy + Math.sin(_phase * 2.2) * radius, 2.5, 0, Math.PI * 2);
        ctx.fill();
        ctx.restore();
    }

    function _stateIcon() {
        if (daemonState === "streaming") return "󰜟";
        if (daemonState === "transcribing") return "󰔟";
        return "󰍬";
    }

    function _stateLabel() {
        if (daemonState === "streaming") return "Streaming";
        if (daemonState === "transcribing") return "Transcribing";
        if (daemonState === "recording") return "Recording";
        return "Idle";
    }

    function _drawIcon(ctx, layer, rect) {
        const signal = _signal(layer);
        const opacity = _num(layer.opacity, 1.0);
        const size = Math.max(8, Math.min(rect.w, rect.h) * _clamp(_num(layer.gain, 1.0), 0.25, 2.5));
        const pulse = 1.0 + signal * 0.08 + Math.sin(_phase * 1.8 * _num(layer.speed, 1.0)) * 0.025;

        ctx.save();
        ctx.globalAlpha = opacity;
        ctx.fillStyle = _color(layer.color, _color(daemonState, "#66c7ff"));
        ctx.font = Math.round(size * pulse) + "px 'JetBrainsMono Nerd Font', sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(_stateIcon(), rect.x + rect.w / 2, rect.y + rect.h / 2);
        ctx.restore();
    }

    function _drawLabel(ctx, layer, rect) {
        const signal = _signal(layer);
        const opacity = _num(layer.opacity, 1.0);
        const size = Math.max(9, Math.min(rect.h * 0.62, rect.w / 5.5) * _clamp(_num(layer.gain, 1.0), 0.25, 2.5));

        ctx.save();
        ctx.globalAlpha = opacity * _clamp(0.72 + signal * 0.28, 0.0, 1.0);
        ctx.fillStyle = _color(layer.color || "foreground", "#fcfbf8");
        ctx.font = "600 " + Math.round(size) + "px sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(_stateLabel(), rect.x + rect.w / 2, rect.y + rect.h / 2, rect.w);
        ctx.restore();
    }

    // ---- Classic card: mirrors draw(), draw_waveform() and
    // draw_peak_meter() in src/bin/voxtype_osd_gtk4.rs. Keep them in step.

    function _withAlpha(role, fallback, alpha) {
        return _rgbaString(_colorComponents(_color(role, fallback)), alpha);
    }

    function _drawClassic(ctx, w, h) {
        if (w <= 0 || h <= 0) return;
        ctx.save();
        ctx.fillStyle = _color("background", "rgba(26, 26, 31, 0.85)");
        ctx.fillRect(0, 0, w, h);
        // Waveform on the left, a 1% gap, the meter on the right (7%).
        const meterW = Math.max(8, w * 0.07);
        const gap = Math.max(2, w * 0.01);
        const waveW = Math.max(0, w - meterW - gap);
        _drawClassicWaveform(ctx, 0, 0, waveW, h);
        _drawClassicMeter(ctx, waveW + gap, 0, meterW, h);
        ctx.restore();
    }

    // Mirrored envelope: the top edge follows each frame's max, the bottom
    // edge its min, on the shared dB scale, filled in the accent color.
    // One pixel column per pixel like GTK4, so the frame at absolute index
    // a sits at x + 0.5 + (n - 1) - (right - a) * (n - 1) / window.
    function _drawClassicWaveform(ctx, x, y, w, h) {
        const n = Math.floor(w);
        if (n < 2) return;
        const mid = y + h * 0.5;
        const half = h * 0.5;
        const r = ring || [];
        const rMin = ringMin || [];
        const rMax = ringMax || [];
        if (r.length > 1 && _scrollPosition >= 0 && rMin.length === r.length && rMax.length === r.length) {
            const pxPerFrame = (n - 1) / Math.max(1, waveformColumns);
            const firstIndex = framesReceived - r.length;
            const xs = [];
            const tops = [];
            const bottoms = [];
            for (let k = 0; k < r.length; k++) {
                const px = x + 0.5 + (n - 1) - (_scrollPosition - (firstIndex + k)) * pxPerFrame;
                if (px < x - pxPerFrame || px > x + w + pxPerFrame) continue;
                xs.push(px);
                tops.push(mid - _signedLevel(rMax[k]) * half);
                bottoms.push(mid - _signedLevel(rMin[k]) * half);
            }
            if (xs.length > 1) {
                ctx.save();
                ctx.beginPath();
                ctx.rect(x, y, w, h);
                ctx.clip();
                ctx.fillStyle = _color("accent", "#66c7ff");
                ctx.beginPath();
                ctx.moveTo(xs[0], tops[0]);
                for (let i = 1; i < xs.length; i++) ctx.lineTo(xs[i], tops[i]);
                for (let i = xs.length - 1; i >= 0; i--) ctx.lineTo(xs[i], bottoms[i]);
                ctx.closePath();
                ctx.fill();
                ctx.restore();
            }
        }
        // Subtle centerline.
        ctx.strokeStyle = _withAlpha("foreground", "#ebebf2", 0.15);
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(x, mid);
        ctx.lineTo(x + w, mid);
        ctx.stroke();
    }

    function _signedLevel(sample) {
        const level = _displayLevel(sample, waveformGain);
        return sample < 0 ? -level : level;
    }

    // osd::visual::MeterZone: red from -3 dBFS, yellow from -12.
    function _zoneRole(dbfs) {
        if (dbfs >= -3) return "error";
        if (dbfs >= -12) return "warning";
        return "success";
    }

    function _levelDbfs(level) {
        const shift = isFinite(waveformGain) && waveformGain > 0 ? 20 * Math.log10(waveformGain / 10) : 0;
        return waveformFloorDbfs * (1 - _clamp(level, 0, 1)) - shift;
    }

    // Ten vertical segments, lit as far as the level reaches, plus the
    // held-peak tick.
    function _drawClassicMeter(ctx, x, y, w, h) {
        if (w < 1 || h < 1) return;
        const fill = _clamp(_meterLevel, 0, 1);
        const held = _dbfsLevel(_heldDbfs, waveformGain);
        const segments = 10;
        const gap = 1.5;
        const segH = Math.max(1, (h - gap * (segments - 1)) / segments);
        const fallbacks = { "success": "#4dd973", "warning": "#f2cc4d", "error": "#f2594d" };
        for (let i = 0; i < segments; i++) {
            const fracBottom = i / segments;
            const fracTop = (i + 1) / segments;
            const role = _zoneRole(_levelDbfs(fracTop));
            const py = y + h - (i + 1) * segH - i * gap;
            ctx.fillStyle = _withAlpha(role, fallbacks[role], 0.18);
            ctx.fillRect(x, py, w, segH);
            const lit = _clamp((fill - fracBottom) / (fracTop - fracBottom), 0, 1);
            if (lit > 0) {
                ctx.fillStyle = _color(role, fallbacks[role]);
                ctx.fillRect(x, py + segH * (1 - lit), w, segH * lit);
            }
        }
        if (held > 0) {
            const py = y + h - held * h;
            ctx.strokeStyle = _withAlpha("foreground", "#ebebf2", 0.95);
            ctx.lineWidth = 1.5;
            ctx.beginPath();
            ctx.moveTo(x, py);
            ctx.lineTo(x + w, py);
            ctx.stroke();
        }
    }

    function repaint() {
        canvas.requestPaint();
    }

    // Runs once per rendered display frame, so motion follows the display's
    // refresh rate rather than a fixed timer or socket deliveries.
    FrameAnimation {
        running: root.visible && (root.classic || root.daemonState !== "idle")
        onTriggered: {
            const dt = Math.min(0.05, Math.max(0.001, frameTime));
            root._advance(dt);

            const targetPeak = root.daemonState === "idle" ? 0 : root._clamp(root.peak, 0, 1);
            const targetRms = root.daemonState === "idle" ? 0 : root._clamp(root.rms, 0, 1);
            const targetVad = root.daemonState === "idle" ? 0 : (root.vad ? 1 : 0);
            root._peak = root._approach(root._peak, targetPeak, 14.0, dt);
            root._rms = root._approach(root._rms, targetRms, 10.0, dt);
            root._vad = root._approach(root._vad, targetVad, 8.0, dt);
            root._energy = root._approach(root._energy, Math.max(root._peak, root._rms * 1.6, root._vad * 0.08), 7.0, dt);
            root._barLevels = root._updateLevels(root._barLevels, 40, 18.0, 6.5, dt);
            root._phase += dt;
            root.repaint();
        }
    }

    Canvas {
        id: canvas
        anchors.fill: parent

        onPaint: {
            const ctx = getContext("2d");
            ctx.clearRect(0, 0, width, height);
            if (root.classic) {
                root._drawClassic(ctx, width, height);
                return;
            }
            const layers = root._sortedLayers;
            for (let i = 0; i < layers.length; i++) {
                const layer = layers[i];
                const kind = layer.type || "waveform";
                const rect = root._layerRect(layer, width, height);
                if (kind === "shadow") root._drawShadow(ctx, layer, rect);
                else if (kind === "waveform") root._drawWaveform(ctx, layer, rect);
                else if (kind === "bars") root._drawBars(ctx, layer, rect);
                else if (kind === "meter") root._drawMeter(ctx, layer, rect);
                else if (kind === "pulse") root._drawPulse(ctx, layer, rect);
                else if (kind === "background") root._drawBackground(ctx, layer, rect);
                else if (kind === "ring") root._drawRing(ctx, layer, rect);
                else if (kind === "icon") root._drawIcon(ctx, layer, rect);
                else if (kind === "label") root._drawLabel(ctx, layer, rect);
            }
        }
    }
}
