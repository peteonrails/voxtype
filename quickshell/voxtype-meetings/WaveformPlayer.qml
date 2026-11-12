import QtQuick
import "MeetingText.js" as MT

// The kept audio of a finished meeting as a strip: you above the middle,
// the call below, a playhead while it plays here. Clicking plays from that
// point (`meeting play --at`), or moves a playback already running.
Item {
  id: root

  property var theme
  property var wave: null
  // Where playback is in this meeting, or -1 when it is not playing here.
  property real positionMs: -1
  signal seekRequested(real ms)

  readonly property int count: root.wave ? root.wave.mic.length : 0
  readonly property real duration: root.wave ? root.wave.durationMs : 0

  implicitHeight: 34

  function frac(db) {
    if (db === null || db === undefined || isNaN(db)) return 0
    return Math.max(0, Math.min(1, (Number(db) + 60) / 60))
  }

  function msAt(x) {
    return Math.max(0, Math.min(root.duration, x / Math.max(1, root.width) * root.duration))
  }

  Rectangle {
    anchors.fill: parent
    radius: Math.min(root.theme.radius, 6)
    color: root.theme.alpha(root.theme.fg, 0.05)
  }

  Row {
    id: bars
    anchors.fill: parent
    anchors.topMargin: 3
    anchors.bottomMargin: 3
    Repeater {
      model: root.count
      Item {
        required property int index
        width: root.width / Math.max(1, root.count)
        height: bars.height
        readonly property bool played: root.positionMs >= 0 && (index + 1) / root.count * root.duration <= root.positionMs
        Rectangle {
          width: Math.max(1, parent.width - 1)
          height: Math.max(1, (root.wave.micTrack === "mixed" ? bars.height : bars.height / 2) * root.frac(root.wave.mic[index]))
          // Both sides in one track (an import): centred, in a neutral colour.
          anchors.bottom: root.wave.micTrack === "mixed" ? undefined : parent.verticalCenter
          anchors.verticalCenter: root.wave.micTrack === "mixed" ? parent.verticalCenter : undefined
          color: root.wave.micTrack === "mixed" ? root.theme.alpha(root.theme.fg, 0.75) : root.theme.accent
          opacity: parent.played ? 1.0 : 0.6
        }
        Rectangle {
          visible: root.wave.loopback !== null
          width: Math.max(1, parent.width - 1)
          height: root.wave.loopback ? Math.max(1, (bars.height / 2) * root.frac(root.wave.loopback[index])) : 0
          anchors.top: parent.verticalCenter
          color: root.theme.fg
          opacity: parent.played ? 0.8 : 0.4
        }
      }
    }
  }

  Rectangle {
    objectName: "waveformPlayhead"
    visible: root.positionMs >= 0 && root.duration > 0
    width: 2
    height: parent.height
    x: Math.min(root.width - width, root.positionMs / Math.max(1, root.duration) * root.width)
    color: root.theme.fg
  }

  // Where a click would play from, while the pointer is on the strip.
  Rectangle {
    visible: waveMouse.containsMouse
    width: 1
    height: parent.height
    x: waveMouse.mouseX
    color: root.theme.alpha(root.theme.fg, 0.5)
  }
  Text {
    objectName: "waveformHoverTime"
    visible: waveMouse.containsMouse
    text: MT.clockMs(root.msAt(waveMouse.mouseX))
    x: Math.min(root.width - width - 4, waveMouse.mouseX + 4)
    y: 1
    color: root.theme.fg
    font.family: root.theme.fontFamily
    font.pixelSize: root.theme.caption
  }

  MouseArea {
    id: waveMouse
    objectName: "waveformArea"
    anchors.fill: parent
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    onClicked: function(mouse) { root.seekRequested(root.msAt(mouse.x)) }
  }
}
