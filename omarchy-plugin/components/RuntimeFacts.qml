import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// Saved selection plus measured daemon/unit state. The selection is explicitly
// labelled configured: the daemon may still run its previous startup snapshot.
//
// Every fact here is measured, never inferred. A reading the panel could not
// take is left out rather than filled in with a plausible number, so an absent
// item means "not known", not "zero".
//
// The unit state is deliberately separate from the daemon state: `voxtype
// status` answers what the daemon is doing, and a unit sitting in `failed`
// answers nothing at all. Collapsing the two loses the state the user has to
// act on, so `failed` gets the urgent color and its own segment.
Item {
  id: root

  property string engine: ""
  property string model: ""

  property string daemonState: ""
  property bool statusKnown: false

  // From `systemctl --user show voxtype`. Empty means the query failed.
  property string unitState: ""
  property string mainPid: ""

  // From nvidia-smi or rocm-smi. Empty text hides the reading.
  property string vramLabel: ""
  property string vramText: ""

  readonly property color dim: Qt.darker(ShellTheme.Color.foreground, 1.5)
  readonly property color separatorColor: Qt.darker(ShellTheme.Color.foreground, 2.2)
  readonly property bool unitFailed: root.unitState === "failed"

  // Built as a list so the separators stay right however many items are
  // present, and so a missing reading closes its own gap instead of leaving a
  // dangling dot.
  readonly property var facts: {
    var out = []

    if (root.engine !== "") {
      out.push({
        text: "configured: " + (root.model !== "" ? (root.engine + " · " + root.model) : root.engine),
        urgent: false
      })
    }

    if (root.statusKnown && root.daemonState !== "")
      out.push({ text: "daemon " + root.daemonState, urgent: false })

    if (root.unitState !== "") {
      var unit = "unit " + root.unitState
      if (root.mainPid !== "") unit += " (pid " + root.mainPid + ")"
      out.push({ text: unit, urgent: root.unitFailed })
    }

    if (root.vramText !== "" && root.vramLabel !== "")
      out.push({ text: root.vramLabel + " " + root.vramText, urgent: false })

    return out
  }

  // One Text rather than a Row of them, because only a single Text can elide.
  // A Row sizes itself to its children and keeps growing past the card's edge,
  // which is how a long model name pushed the later readings outside the
  // container. Markup is what keeps the failed-unit segment its own colour
  // inside that one Text.
  function hex(c) {
    function pair(v) {
      var s = Math.round(v * 255).toString(16)
      return s.length < 2 ? "0" + s : s
    }
    return "#" + pair(c.r) + pair(c.g) + pair(c.b)
  }

  // The facts are voxtype's own strings (model names, unit states, byte counts),
  // but they end up inside markup, so they are escaped rather than trusted to
  // contain no angle bracket.
  function escaped(text) {
    return String(text)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
  }

  readonly property string line: {
    var separator = "<font color=\"" + root.hex(root.separatorColor) + "\">&nbsp;&nbsp;·&nbsp;&nbsp;</font>"
    var parts = []
    for (var i = 0; i < root.facts.length; i++) {
      var fact = root.facts[i]
      var body = root.escaped(fact.text)
      if (fact.urgent === true)
        body = "<b><font color=\"" + root.hex(ShellTheme.Color.urgent) + "\">" + body + "</font></b>"
      parts.push(body)
    }
    return parts.join(separator)
  }

  visible: facts.length > 0
  // Height comes from the font, not from which facts happen to be known, so a
  // reading arriving on a later poll does not shove the form down.
  implicitHeight: visible ? Math.ceil(metrics.height) : 0

  TextMetrics {
    id: metrics
    font.family: Style.font.family
    font.pixelSize: Style.font.caption
    text: "Ag"
  }

  Text {
    anchors.left: parent.left
    anchors.right: parent.right
    anchors.verticalCenter: parent.verticalCenter
    text: root.line
    textFormat: Text.StyledText
    color: root.dim
    font.family: Style.font.family
    font.pixelSize: Style.font.caption
    // Single line, elided at the right: when the readings do not fit, the last
    // of them is cut off rather than the row spilling out of the card.
    maximumLineCount: 1
    elide: Text.ElideRight
  }
}
