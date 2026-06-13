// Resolved Quickshell OSD style loader.
//
// `voxtype-osd-quickshell` writes a JSON file and exposes it through
// VOXTYPE_OSD_STYLE_FILE. This component parses it and falls back to the
// built-in defaults when the file is missing or invalid.

import QtQuick
import Quickshell
import Quickshell.Io

QtObject {
    id: root

    property string styleFile: Quickshell.env("VOXTYPE_OSD_STYLE_FILE") || ""
    property var config: _defaultConfig()

    // Degraded-path fallback used only when VOXTYPE_OSD_STYLE_FILE is
    // missing or invalid. Mirrors the Rust defaults; keep in sync with
    // src/osd/style.rs (semantic_colors + Palette::fallback) and
    // src/osd/config.rs (default_visual_layers). Layer tunables omitted
    // here use the renderer's per-layer-type defaults, same as the
    // runtime JSON.
    function _defaultConfig() {
        return {
            "style": "default",
            "palette": "omarchy",
            "layout": "compact",
            "position": "bottom-center",
            "margin_px": 24,
            "top_margin": 0.85,
            "package_dir": null,
            "asset_root": null,
            "custom_qml": null,
            "colors": {
                "accent": "#66C7FF",
                "background": "rgba(26, 26, 31, 0.850)",
                "surface": "rgba(26, 26, 31, 0.850)",
                "foreground": "#EBEBF2",
                "muted": "rgba(235, 235, 242, 0.650)",
                "success": "#4DD973",
                "warning": "#F2CC4D",
                "error": "#F2594D",
                "recording": "#F2594D",
                "streaming": "#66C7FF",
                "transcribing": "#F2CC4D",
                "idle": "rgba(235, 235, 242, 0.750)"
            },
            "frame": {
                "background": "background",
                "border": "state",
                "glow": true,
                "halo": true
            },
            "visual": {
                "layers": [
                    {
                        "type": "waveform",
                        "source": "peak",
                        "color": "accent",
                        "order": 10,
                        "height": 0.82,
                        "mirror": true
                    },
                    {
                        "type": "meter",
                        "source": "peak",
                        "color": "success",
                        "secondary_color": "foreground",
                        "order": 20,
                        "y": 0.86,
                        "height": 0.14
                    }
                ]
            }
        };
    }

    // QML color properties accept "#RRGGBB"/"#AARRGGBB" and named colors
    // but not CSS "rgb()/rgba()" strings, which the Rust palette and
    // package manifests can emit. Convert those to "#AARRGGBB" so the
    // result works for both QML color properties and Canvas styles.
    function _normalizeColor(value) {
        if (typeof value !== "string") return value;
        const m = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(value);
        if (!m) return value;
        function hex2(n) {
            const clamped = Math.max(0, Math.min(255, Math.round(n)));
            return (clamped < 16 ? "0" : "") + clamped.toString(16);
        }
        const a = m[4] === undefined ? 1.0 : Math.max(0, Math.min(1, Number(m[4])));
        return "#" + hex2(a * 255) + hex2(Number(m[1])) + hex2(Number(m[2])) + hex2(Number(m[3]));
    }

    function color(role, fallback) {
        const colors = config && config.colors ? config.colors : {};
        if (!role || role.length === 0) return fallback !== undefined ? fallback : "#ffffff";
        if (role[0] === "#") return role;
        if (role.indexOf("rgb(") === 0 || role.indexOf("rgba(") === 0) {
            return _normalizeColor(role);
        }
        const resolved = colors[role];
        if (resolved !== undefined) return _normalizeColor(resolved);
        return fallback !== undefined ? fallback : role;
    }

    function customQmlUrl() {
        if (!config || !config.custom_qml || config.custom_qml.length === 0) {
            return "";
        }
        if (config.custom_qml.indexOf("file://") === 0) {
            return config.custom_qml;
        }
        return "file://" + config.custom_qml;
    }

    function _loadText(text) {
        try {
            const parsed = JSON.parse(text || "{}");
            if (parsed && typeof parsed === "object") {
                config = parsed;
            }
        } catch (e) {
            console.warn("voxtype OSD style: invalid JSON:", e);
            config = _defaultConfig();
        }
    }

    property FileView _fileView: FileView {
        path: root.styleFile
        watchChanges: true
        printErrors: false

        onLoaded: root._loadText(text())
        onLoadFailed: root.config = root._defaultConfig()
        onFileChanged: reload()
    }
}
