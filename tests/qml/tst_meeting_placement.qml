// Run through tests/quickshell-meeting-placement.sh: offscreen, no
// compositor, state written under a scratch XDG_STATE_HOME.
import QtQuick
import Quickshell
import "voxtype-meetings" as M

ShellRoot {
    id: tests
    property int passed: 0

    function check(condition, message) {
        if (!condition) throw new Error(message);
        passed++;
    }

    function at(p) {
        return JSON.stringify({ x: p.x, left: p.left, cardLeft: p.cardLeft, pillRight: p.pillRight, minimized: p.minimized });
    }

    M.MeetingPlacement {
        id: p
        host: "standalone"
        screenName: "TEST-1"
        screenWidth: 1600
        screenHeight: 1000
    }

    // Drag the window by `dx` in steps, the way pointer events arrive.
    function drag(dx) {
        const step = dx > 0 ? 40 : -40;
        for (let moved = 0; Math.abs(moved) < Math.abs(dx); moved += step) p.moveBy(step, 0);
        p.endMove();
    }

    Timer {
        interval: 100
        running: true
        onTriggered: {
            try {
                // A card left in its default place, minimized: the pill keeps
                // its right edge, against the screen's right.
                tests.check(p.x === -1, "starts unplaced: " + tests.at(p));
                p.minimized = true;
                tests.check(p.pillRight, "a card on the right minimizes to a right-anchored pill");
                const pillW = p.pillWidth;
                tests.check(p.left === 1600 - pillW - p.gap, "pill against the right edge: " + tests.at(p));

                // #7: dragged all the way left, the pill stays there.
                tests.drag(-2000);
                tests.check(p.left === p.gap, "pill dragged to the far left lands a gap in: " + tests.at(p));
                tests.check(p.x >= 0, "the card's x is placed, not the unplaced -1: " + tests.at(p));
                tests.check(!p.pillRight, "a pill on the left keeps its left edge: " + tests.at(p));

                // Expanding puts the card where the pill is, not at the right.
                p.minimized = false;
                tests.check(p.cardLeft === p.gap, "the card expands at the left edge: " + tests.at(p));
                tests.check(p.cardLeft + p.cardWidth <= p.screenWidth, "the card fits on screen: " + tests.at(p));

                // And back: a pill dragged to the right keeps its right edge.
                p.minimized = true;
                tests.drag(2000);
                tests.check(p.left === 1600 - p.pillWidth - p.gap, "pill dragged to the far right: " + tests.at(p));
                tests.check(p.pillRight, "a pill on the right keeps its right edge");
                p.minimized = false;
                tests.check(p.cardLeft + p.cardWidth === 1600 - p.gap, "the card expands against the right edge: " + tests.at(p));

                // A card wider than half the screen dragged left as a pill.
                p.width = 800;
                p.minimized = true;
                tests.drag(-2000);
                tests.check(p.x >= 0 && p.left === p.gap, "a wide card's pill stays left: " + tests.at(p));
                p.minimized = false;
                tests.check(p.cardLeft === p.gap, "and the wide card expands there: " + tests.at(p));

                console.info("MEETING_PLACEMENT_PASS " + tests.passed);
                Qt.exit(0);
            } catch (e) {
                console.error("MEETING_PLACEMENT_FAIL " + e.message);
                Qt.exit(1);
            }
        }
    }
}
