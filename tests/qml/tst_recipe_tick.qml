// Run through tests/quickshell-recipe-tick.sh: offscreen, no compositor.
// #1078: the OSD renderer must tick on the frame clock, not a 16 ms Timer.
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

    function clockOf(item) {
        for (let i = 0; i < item.data.length; i++) {
            const child = item.data[i];
            if (child instanceof FrameAnimation) return child;
        }
        return null;
    }

    function timersOf(item) {
        let count = 0;
        for (let i = 0; i < item.data.length; i++) {
            if (item.data[i] instanceof Timer) count++;
        }
        return count;
    }

    FloatingWindow {
        id: host
        implicitWidth: 200
        implicitHeight: 40
        visible: true
        color: "transparent"

        VT.StyleLoader { id: style }
        VT.RecipeRenderer {
            id: renderer
            anchors.fill: parent
            style: style
            active: true
            daemonState: "idle"
        }
    }

    property int framesAtStart: 0

    Timer {
        id: start
        interval: 150
        running: true
        onTriggered: {
            try {
                const clock = tests.clockOf(renderer);
                tests.check(clock !== null, "The renderer ticks from a FrameAnimation");
                tests.check(tests.timersOf(renderer) === 0, "No interval Timer drives the renderer");
                tests.check(!clock.running, "The frame clock is stopped while idle");

                // A tick advances animation time by the frame time it is
                // given, not by wall-clock time between calls.
                const phase = renderer._phase;
                renderer._tick(0.008);
                renderer._tick(0.008);
                tests.check(Math.abs(renderer._phase - phase - 0.016) < 1e-9,
                    "Two 8 ms frames advance the animation by 16 ms");
                renderer._tick(5);
                tests.check(Math.abs(renderer._phase - phase - 0.066) < 1e-9,
                    "A long stall is clamped to one 50 ms step");

                renderer.daemonState = "recording";
                tests.check(clock.running, "The frame clock runs while recording");
                tests.framesAtStart = renderer._frame;
                settle.start();
            } catch (e) {
                console.error("RECIPE_TICK_FAIL " + e.message);
                Qt.exit(1);
            }
        }
    }

    Timer {
        id: settle
        interval: 500
        onTriggered: {
            try {
                tests.check(renderer._frame > tests.framesAtStart,
                    "Rendered frames advance the renderer");
                renderer.daemonState = "idle";
                const clock = tests.clockOf(renderer);
                tests.check(!clock.running, "The frame clock stops at idle");
                console.info("RECIPE_TICK_PASS " + tests.passed);
                Qt.exit(0);
            } catch (e) {
                console.error("RECIPE_TICK_FAIL " + e.message);
                Qt.exit(1);
            }
        }
    }
}
