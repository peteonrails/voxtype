#!/usr/bin/env python3
"""Speaker colours and the waveform's bucket-to-speaker mapping, without a window.

The functions are plain JavaScript (components/meetings/MeetingSpeakers.js);
this loads them, and MeetingTheme.qml, into a bare QML engine (PySide6) and
checks what they return:

  slots     a speaker's colour slot: first come first served, kept when its
            lines are moved away, moved with a `speaker_renamed`, shared by
            speakers given one name, and yours is always the accent
  buckets   which speaker a waveform bucket belongs to: partial overlaps,
            a speaker's lines added up, ties, empty buckets, the two sides
            and a single mixed track
  palette   the colours made from a theme: eight, told apart from each other
            and from the accent, readable on the theme's background, light on
            dark and dark on light; a ninth speaker shares the first's

--evidence DIR also saves palette.png: the eight colours on a few themes.

Needs no display and refuses to run with one in the environment.
"""
import argparse
import itertools
import json
import math
import os
from pathlib import Path
import sys

if os.environ.get("WAYLAND_DISPLAY") or os.environ.get("DISPLAY"):
    sys.exit("refusing to run with a display in the environment; use the isolated sandbox")
os.environ["QT_QPA_PLATFORM"] = "offscreen"
os.environ["QT_QUICK_BACKEND"] = "software"

from PySide6.QtCore import QUrl, qInstallMessageHandler  # noqa: E402
from PySide6.QtGui import QGuiApplication  # noqa: E402
from PySide6.QtQml import QQmlComponent, QQmlEngine  # noqa: E402
from PySide6.QtQuick import QQuickWindow  # noqa: E402

MEETINGS = Path(__file__).resolve().parents[1] / "components" / "meetings"

HARNESS = """import QtQuick
import "MeetingSpeakers.js" as MS

Rectangle {
  id: root
  width: 760
  height: parent ? parent.height : 0
  color: "#808080"

  MeetingTheme { id: theme }

  function call(name, argsJson) {
    var fns = { withSpeaker: MS.withSpeaker, renamedSlots: MS.renamedSlots, colorSlot: MS.colorSlot,
                sideOf: MS.sideOf, bucketSpeakers: MS.bucketSpeakers, palette: MS.palette }
    return JSON.stringify(fns[name].apply(null, JSON.parse(argsJson)))
  }

  // What MeetingTheme draws each slot in, for a theme's three colours.
  function themed(accent, bg, fg, slotsJson) {
    theme.accent = accent
    theme.bg = bg
    theme.fg = fg
    return JSON.stringify(JSON.parse(slotsJson).map(function(s) { return String(theme.speakerColor(s)) }))
  }

  // The picture: one row per theme, you and then the eight.
  property var themes: []
  Column {
    id: sheet
    x: 12
    y: 12
    spacing: 8
    Repeater {
      model: root.themes
      Rectangle {
        id: row
        required property var modelData
        readonly property var colors: MS.palette(modelData.accent, modelData.bg, modelData.fg, 8)
        width: root.width - 24
        height: 58
        radius: 6
        color: modelData.bg
        Text { x: 10; y: 6; text: row.modelData.name; color: row.modelData.fg; font.pixelSize: 11 }
        Row {
          x: 10
          y: 26
          spacing: 14
          Repeater {
            model: 9
            Row {
              id: who
              required property int index
              readonly property color tint: index === 0 ? row.modelData.accent
                : Qt.rgba(row.colors[index - 1][0], row.colors[index - 1][1], row.colors[index - 1][2], 1)
              spacing: 4
              Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4; color: who.tint }
              Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 3; height: 16; color: who.tint }
              Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 3; height: 16; color: who.tint; opacity: 0.6 }
              Text { text: who.index === 0 ? "You" : "Sp " + who.index; color: who.tint; font.pixelSize: 11; font.bold: true }
            }
          }
        }
      }
    }
  }
}
"""

# name, accent, background, foreground: the view's own default, a few of the
# themes Omarchy ships (dark and light), and two with a grey accent.
THEMES = [
    ("voxtype default", "#6e89c2", "#1a1a1f", "#dcdfe4"),
    ("tokyo night", "#7aa2f7", "#1a1b26", "#c0caf5"),
    ("catppuccin", "#89b4fa", "#1e1e2e", "#cdd6f4"),
    ("gruvbox", "#7daea3", "#282828", "#d4be98"),
    ("nord", "#81a1c1", "#2e3440", "#d8dee9"),
    ("everforest", "#7fbbb3", "#2d353b", "#d3c6aa"),
    ("ristretto", "#f38d70", "#2c2525", "#e6d9db"),
    ("matte black", "#e68e0d", "#121212", "#bebebe"),
    ("grey accent, dark", "#bbbbbb", "#101010", "#eeeeee"),
    ("rose pine dawn", "#286983", "#faf4ed", "#575279"),
    ("catppuccin latte", "#1e66f5", "#eff1f5", "#4c4f69"),
    ("grey accent, light", "#333333", "#ffffff", "#111111"),
]

failures = []


def check(ok, what):
    if not ok:
        failures.append(what)


def rgb(color):
    """A colour as three floats in 0..1, from "#rrggbb" or [r, g, b]."""
    if isinstance(color, str):
        return [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    return list(color)


def linear(v):
    return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4


def luminance(color):
    r, g, b = (linear(v) for v in rgb(color))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a, b):
    la, lb = luminance(a), luminance(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)


def oklab(color):
    r, g, b = (linear(v) for v in rgb(color))
    l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b) ** (1 / 3)
    m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b) ** (1 / 3)
    s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b) ** (1 / 3)
    return (0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
            1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
            0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s)


def apart(a, b):
    """How different two colours look (distance in OKLab; 0.02 is just visible)."""
    return math.dist(oklab(a), oklab(b))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--evidence", type=Path, help="save palette.png here")
    args = parser.parse_args()

    warnings = []
    qInstallMessageHandler(lambda kind, context, message: warnings.append(message))
    app = QGuiApplication([])
    engine = QQmlEngine()
    component = QQmlComponent(engine)
    component.setData(HARNESS.encode(), QUrl.fromLocalFile(str(MEETINGS / "speakers-harness.qml")))
    root = component.create()
    if root is None:
        sys.exit("harness did not load: " + "; ".join(e.toString() for e in component.errors()))

    def call(name, *arguments):
        return json.loads(root.call(name, json.dumps(arguments)))

    # ------------------------------------------------------------ slots
    slots = {}
    for key in ["You", "Remote 1", "Remote 2", "You", "Remote 1", "Remote 3"]:
        slots = call("withSpeaker", slots, key)
    check(slots == {"Remote 1": 0, "Remote 2": 1, "Remote 3": 2}, "slots in order of appearance, none for You: %s" % slots)
    check(call("withSpeaker", slots, "Remote 2") == slots, "a known speaker changes nothing")
    check(call("withSpeaker", slots, "") == slots, "no key, no slot")
    labels = {}
    check([call("colorSlot", slots, labels, k) for k in ["You", "Remote 1", "Remote 2", "Remote 3"]] == [-1, 0, 1, 2],
          "each speaker its own slot, You the accent's")
    check(call("colorSlot", slots, labels, "Nobody") == -2, "an unknown speaker has no slot")

    # A rename is a label: the key, and so the slot, stays.
    named = {"Remote 2": "Priya"}
    check(call("colorSlot", slots, named, "Remote 2") == 1, "a renamed speaker keeps its slot")
    # The feed never takes a slot back: a speaker whose lines all moved away
    # keeps it, and a new speaker gets the next one.
    more = call("withSpeaker", slots, "Guest")
    check(more["Guest"] == 3 and more["Remote 2"] == 1, "a new speaker takes a new slot: %s" % more)
    # Two keys given one name are one speaker, in the earlier one's slot.
    merged = {"Remote 1": "Dana", "Remote 3": "dana"}
    check(call("colorSlot", slots, merged, "Remote 3") == 0 and call("colorSlot", slots, merged, "Remote 1") == 0,
          "speakers named alike share the lower slot")
    check(call("colorSlot", slots, merged, "Remote 2") == 1, "and the others keep theirs")
    # Named as you are: yours.
    mine = {"You": "Pete", "Remote 1": "pete"}
    check(call("colorSlot", slots, mine, "Remote 1") == -1, "a speaker named as you are is the accent")
    check(call("colorSlot", slots, {"Remote 1": "You"}, "Remote 1") == -1, "also by your key's own name")

    # speaker_renamed: the daemon changed a key.
    moved = call("renamedSlots", slots, "Remote 2", "Speaker B")
    check(moved == {"Remote 1": 0, "Speaker B": 1, "Remote 3": 2}, "the slot goes with the new key: %s" % moved)
    onto = call("renamedSlots", slots, "Remote 2", "Remote 3")
    check(onto == {"Remote 1": 0, "Remote 3": 2}, "a key that had its own keeps it: %s" % onto)
    check(call("withSpeaker", onto, "Late")["Late"] == 1, "a slot nobody holds is taken first")
    check(call("renamedSlots", slots, "Nobody", "X") == slots and call("renamedSlots", slots, "Remote 1", "Remote 1") == slots,
          "nothing to rename, nothing changed")
    check(call("renamedSlots", slots, "Remote 1", "You") == {"Remote 2": 1, "Remote 3": 2}, "renamed to You: no slot")

    # More speakers than colours: every one still has its own slot...
    many = {}
    for n in range(1, 11):
        many = call("withSpeaker", many, "Remote %d" % n)
    check(sorted(many.values()) == list(range(10)), "ten speakers, ten slots: %s" % many)

    # ------------------------------------------------------------ buckets
    def line(start, end, speaker, source="loopback"):
        return {"startMs": start, "endMs": end, "speaker": speaker, "source": source}

    def buckets(lines, count=4, bucket_ms=1000, side="call", names=None):
        return call("bucketSpeakers", lines, count, bucket_ms, side, names or {})

    check(buckets([line(0, 600, "A"), line(600, 1000, "B")]) == ["A", "", "", ""], "the larger share of a bucket wins")
    check(buckets([line(0, 400, "A"), line(400, 1000, "B")]) == ["B", "", "", ""], "whichever side of the bucket it is on")
    check(buckets([line(0, 300, "A"), line(300, 700, "B"), line(700, 1000, "A")]) == ["A", "", "", ""],
          "a speaker's lines in a bucket add up")
    check(buckets([line(0, 500, "A"), line(500, 1000, "B")]) == ["A", "", "", ""], "even shares: the one who spoke first")
    check(buckets([line(500, 1000, "B"), line(0, 500, "A")]) == ["A", "", "", ""], "whatever order the lines come in")
    check(buckets([line(900, 1000, "A")]) == ["A", "", "", ""], "a little is enough when nobody else is there")
    check(buckets([line(0, 400, "B"), line(500, 2500, "A")]) == ["A", "A", "A", ""], "a line across buckets marks each")
    check(buckets([line(0, 1000, "A")]) == ["A", "", "", ""], "a line ending on a boundary stays out of the next bucket")
    check(buckets([line(1000, 2000, "A"), line(3200, 3900, "B")]) == ["", "A", "", "B"], "no line, no speaker")
    check(buckets([]) == ["", "", "", ""], "no lines at all")
    check(buckets([line(2500, 2500, "A"), line(1200, 0, "B")]) == ["", "B", "A", ""], "a line with no length marks where it is")
    check(buckets([line(3500, 9000, "A"), line(7000, 8000, "B")]) == ["", "", "", "A"], "past the last bucket is left out")
    check(buckets([line(0, 4000, "A")], bucket_ms=0) == ["", "", "", ""], "no bucket length, nothing to say")
    check(buckets([line(0, 1500, "A")], count=3, bucket_ms=500.5) == ["A", "A", "A"], "a bucket length that is not a whole number")

    # The two sides: your lines are not the call's, and the other way round.
    both = [line(0, 1000, "You", "microphone"), line(200, 900, "Remote 1"), line(1000, 2000, "Remote 2", "loopback"),
            line(2000, 3000, "You", ""), line(3000, 4000, "Speaker 1", "")]
    check(buckets(both, side="call") == ["Remote 1", "Remote 2", "", "Speaker 1"], "the call side")
    check(buckets(both, side="mic") == ["You", "", "You", ""], "your side; a line with no source goes by its speaker")
    check(buckets(both, side="all") == ["You", "Remote 2", "You", "Speaker 1"], "one mixed track has everyone")
    check(call("sideOf", line(0, 1, "Remote 1", "microphone")) == "mic", "a line's source decides its side")

    # Two keys with one name count as one speaker.
    same = {"Remote 2": "Ana", "Remote 3": "ana"}
    shared = [line(0, 300, "Remote 3"), line(300, 700, "Remote 4"), line(700, 1000, "Remote 2")]
    check(buckets(shared, names=same) == ["Remote 3", "", "", ""], "named alike: one speaker, the key heard first")
    check(buckets(shared) == ["Remote 4", "", "", ""], "and apart without the name")

    # ------------------------------------------------------------ palette
    for name, accent, bg, fg in THEMES:
        colors = call("palette", accent, bg, fg, 8)
        check(len(colors) == 8 and all(len(c) == 3 and all(0 <= v <= 1 for v in c) for c in colors), name + ": eight colours")
        closest = min(apart(a, b) for a, b in itertools.combinations(colors, 2))
        check(closest >= 0.08, "%s: two speakers too alike (%.3f)" % (name, closest))
        if "grey" not in name:
            nearest = min(apart(accent, c) for c in colors)
            check(nearest >= 0.08, "%s: a speaker too like you (%.3f)" % (name, nearest))
        weakest = min(contrast(c, bg) for c in colors)
        check(weakest >= 3.5, "%s: a colour too faint on the background (%.2f)" % (name, weakest))
        dark = luminance(bg) < luminance(fg)
        check(all((luminance(c) > luminance(bg)) == dark for c in colors), name + ": light on dark, dark on light")
        check(call("palette", accent, bg, fg, 8) == colors, name + ": the same every time")
        check(call("palette", accent, bg, fg, 20) == colors, name + ": never more than eight")
        check(call("palette", accent, bg, fg, 3) == colors[:3], name + ": the first three are the first three")

        # What the theme draws: you in the accent, the ninth as the first.
        drawn = json.loads(root.themed(accent, bg, fg, json.dumps([-1, 0, 1, 7, 8, 9, 15, 16, -2])))
        check(drawn[0] == accent, "%s: you are the accent (%s)" % (name, drawn[0]))
        check(len(set(drawn[1:4])) == 3 and drawn[0] not in drawn[1:4], name + ": speakers differ")
        check(drawn[4] == drawn[1] and drawn[5] == drawn[2] and drawn[6] == drawn[3] and drawn[7] == drawn[1],
              "%s: past eight the colours start over (%s)" % (name, drawn))
        check(drawn[8] not in drawn[:4], name + ": no slot, the dim colour")
        for want, got in zip(colors[:2], drawn[1:3]):
            check(all(abs(a - b) < 1 / 255 for a, b in zip(want, rgb(got))), name + ": the theme draws the palette")

    # A light and a dark theme get different colours from the same accent.
    check(call("palette", "#7aa2f7", "#1a1b26", "#c0caf5", 8) != call("palette", "#7aa2f7", "#fafafa", "#202020", 8),
          "the palette follows the background")

    if args.evidence:
        args.evidence.mkdir(parents=True, exist_ok=True)
        root.setProperty("themes", [{"name": n, "accent": a, "bg": b, "fg": f} for n, a, b, f in THEMES])
        window = QQuickWindow()
        root.setParentItem(window.contentItem())
        # 58 px a row and 8 between, inside 12 px: the Column has no height
        # of its own until a window has laid it out.
        window.resize(int(root.width()), 24 + len(THEMES) * 66 - 8)
        window.show()
        for _ in range(20):
            app.processEvents()
        check(window.grabWindow().save(str(args.evidence / "palette.png")), "palette.png saved")

    for message in warnings:
        if any(mark in message for mark in ("TypeError", "ReferenceError", "Unable to assign", "is not a type")):
            failures.append("QML: " + message)
    for failure in failures:
        print("FAIL " + failure)
    print("FAIL meeting-speakers" if failures else "PASS meeting-speakers")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
