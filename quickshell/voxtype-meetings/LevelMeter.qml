import QtQuick

// One track's level as a bar: dBFS mapped linearly from -60 (empty) to 0
// (full), as docs/INTEGRATIONS.md suggests, with a tick at the peak. It falls
// to empty when levels stop arriving (`at` older than `staleMs`), so a
// stopped check or a paused meeting never leaves a frozen bar.
Item {
  id: root

  property var theme
  property var db: null
  property var peakDb: null
  // When the level arrived, ms since the epoch; 0 means none yet.
  property real at: 0
  property int staleMs: 400
  property color color: root.theme.accent

  property real now: Date.now()
  readonly property bool fresh: root.at > 0 && root.now - root.at <= root.staleMs
  readonly property real fraction: root.fresh ? root.frac(root.db) : 0
  readonly property real peakFraction: root.fresh ? root.frac(root.peakDb) : 0

  function frac(v) {
    if (v === null || v === undefined || isNaN(v)) return 0
    return Math.max(0, Math.min(1, (Number(v) + 60) / 60))
  }

  implicitHeight: 6
  implicitWidth: 120

  Timer {
    interval: 100
    repeat: true
    running: root.visible && root.at > 0
    onTriggered: root.now = Date.now()
  }
  onAtChanged: root.now = Date.now()

  Rectangle {
    anchors.fill: parent
    radius: height / 2
    color: root.theme.alpha(root.theme.fg, 0.12)
  }
  Rectangle {
    objectName: "meterFill"
    height: parent.height
    radius: height / 2
    width: parent.width * root.fraction
    color: root.color
    Behavior on width { NumberAnimation { duration: 90 } }
  }
  Rectangle {
    objectName: "meterPeak"
    visible: root.peakFraction > 0
    width: 2
    height: parent.height
    x: Math.max(0, parent.width * root.peakFraction - width)
    color: root.theme.fg
    opacity: 0.7
  }
}
