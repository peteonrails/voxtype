// Run through tests/quickshell-shell-style.sh: Quickshell's Process and
// FileView types need the qs application, not the plain Qt test runner.
import QtQuick
import Quickshell
import "voxtype-shared" as VT

ShellRoot {
    id: tests
    property int passed: 0

    function check(condition, message) {
        if (!condition) throw new Error(message);
        passed++;
    }

    VT.ShellMetrics { id: metrics; enabled: false }
    VT.StyleLoader { id: style }
    VT.RecipeRenderer {
        id: renderer
        width: 184
        height: 28
        style: style
        active: false
    }
    VT.ShellBorder {
        id: edge
        width: 320
        height: 64
        colors: ["#739fd2", "#fa40a9"]
        cornerRadius: 8
        thickness: 2
        angle: 45
    }

    Timer {
        interval: 100
        running: true
        onTriggered: {
            try {
                tests.check(!metrics._query.running, "Inactive metrics start no processes");
                metrics.applyRounding('{"int":8}');
                tests.check(metrics.cornerRadius === 8, "Hyprland rounding is applied");
                metrics.applyRounding('{"int":0}');
                tests.check(metrics.cornerRadius === 0, "Square corners remain square");
                metrics.applyRounding('{"int":8}');
                const invalid = ["not json", "{}", '{"int":null}', '{"int":-1}', '{"int":"12"}'];
                for (let i = 0; i < invalid.length; i++) {
                    metrics.applyRounding(invalid[i]);
                    tests.check(metrics.cornerRadius === 8, "Malformed rounding keeps the previous radius");
                }
                edge.colors = ["#739fd2", "#fa40a9", "#e8dac7"];
                tests.check(edge._stops.length === 3, "All gradient stops are loaded");
                tests.check(edge._stops[1].position === 0.5, "Gradient stops are evenly spaced");
                edge.colors = ["#b6d572"];
                tests.check(edge._stops.length === 1, "Theme reload replaces old stops");
                edge.thickness = 0;
                tests.check(!edge.visible, "A zero-width border is hidden");
                edge.thickness = 100;
                tests.check(edge.inset === 32, "A thick border cannot invert its inner shape");
                edge.thickness = 2;

                style.config = { layout: "shell", shell: { spacing_scale: 1, font_size: 12, border_width: 2,
                    border_colors: ["#739fd2", "#fa40a9"], border_angle: 45, text: "foreground" } };
                tests.check(style.shellCornerRadius === 0, "No Hyprland means square shell corners");
                style._shellMetrics.applyRounding('{"int":8}');
                tests.check(style.shellCornerRadius === 8, "Rounding changes propagate through the style loader");
                style._shellMetrics.applyRounding('{"int":0}');
                tests.check(style.shellCornerRadius === 0, "Square rounding propagates without a restart");
                tests.check(style.color("#ff739fd2") === "#ff739fd2", "Resolved border colors retain Qt alpha ordering");
                style.config = { layout: "shell", visual: { layers: [
                    { type: "dots", source: "peak", color: "accent", secondary_color: "foreground", order: 10 }
                ] } };
                tests.check(renderer._wantsDots && !renderer._wantsLine, "Shell Dots selects the existing dots renderer");
                tests.check(renderer._sortedLayers.length === 1, "Shell Dots does not add waveform or meter layers");
                tests.check(renderer._sortedLayers[0].secondary_color === "foreground", "Dots retain semantic theme roles");
                style.config = style._defaultConfig();
                tests.check(style.config.layout === "compact", "Existing default layout is unchanged");
                console.info("SHELL_STYLE_PASS " + tests.passed + " checks");
                Qt.quit();
            } catch (e) {
                console.error("SHELL_STYLE_FAIL " + e);
                Qt.exit(1);
            }
        }
    }
}
