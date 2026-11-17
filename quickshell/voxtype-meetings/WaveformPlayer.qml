import QtQuick
import "MeetingText.js" as MT
import "MeetingSpeakers.js" as MS

// The kept audio of a finished meeting as a strip: you above the middle,
// the call below, a playhead while it plays here. Clicking plays from that
// point (`meeting play --at`), or moves a playback already running.
//
// With the transcript (`feed`), each bar of the call is in the colour of
// whoever was speaking then, the colour that speaker has on the chips and
// the lines. A stretch with no line keeps the neutral colour, and you keep
// the accent: the sides are told apart by position first, colour second.
Item {
  id: root

  property var theme
  property var wave: null
  // Where playback is in this meeting, or -1 when it is not playing here.
  property real positionMs: -1
  // MeetingFeed, for the speakers; without it the call is one colour.
  property var feed: null
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

  function bucketAt(x) {
    return Math.max(0, Math.min(root.count - 1, Math.floor(x / Math.max(1, root.width) * root.count)))
  }

  // The speaker of each bucket (a speaker key, "" for none), per side. One
  // mixed track has everyone in `micSpeakers`.
  property var micSpeakers: []
  property var callSpeakers: []

  function syncSpeakers() {
    if (!root.feed || root.count === 0 || root.duration <= 0) {
      root.micSpeakers = []
      root.callSpeakers = []
      return
    }
    var lines = root.feed.spans()
    // The bucket length the peaks were measured over, when the wave says.
    var bucketMs = root.wave.bucketMs > 0 ? root.wave.bucketMs : root.duration / root.count
    var mixed = root.wave.micTrack === "mixed"
    root.micSpeakers = MS.bucketSpeakers(lines, root.count, bucketMs, mixed ? "all" : "mic", root.feed.labels)
    root.callSpeakers = mixed ? [] : MS.bucketSpeakers(lines, root.count, bucketMs, "call", root.feed.labels)
  }

  onFeedChanged: root.syncSpeakers()
  onCountChanged: root.syncSpeakers()
  onDurationChanged: root.syncSpeakers()
  // Lines arriving, moved, edited or deleted, and speakers renamed: once
  // per burst, not once per line.
  Connections {
    target: root.feed ? root.feed.segments : null
    function onRowsInserted() { Qt.callLater(root.syncSpeakers) }
    function onRowsRemoved() { Qt.callLater(root.syncSpeakers) }
    function onDataChanged() { Qt.callLater(root.syncSpeakers) }
    function onModelReset() { Qt.callLater(root.syncSpeakers) }
  }
  Connections {
    target: root.feed
    function onLabelsChanged() { Qt.callLater(root.syncSpeakers) }
  }
  Connections {
    target: root.wave
    function onMicTrackChanged() { root.syncSpeakers() }
  }

  function speakerColor(key) { return root.theme.speakerColor(root.feed ? root.feed.speakerSlot(key) : -2) }

  // Who the pointer is on: above the middle your side, below it the call.
  readonly property string hoverSpeaker: {
    if (!waveMouse.containsMouse || root.count === 0) return ""
    var upper = root.wave.micTrack === "mixed" || root.wave.loopback === null || waveMouse.mouseY < root.height / 2
    var list = upper ? root.micSpeakers : root.callSpeakers
    var i = root.bucketAt(waveMouse.mouseX)
    return i < list.length ? list[i] : ""
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
        readonly property bool mixed: root.wave.micTrack === "mixed"
        readonly property string micSpeaker: index < root.micSpeakers.length ? root.micSpeakers[index] : ""
        readonly property string callSpeaker: index < root.callSpeakers.length ? root.callSpeakers[index] : ""
        Rectangle {
          objectName: "waveformMicBar"
          width: Math.max(1, parent.width - 1)
          height: Math.max(1, (parent.mixed ? bars.height : bars.height / 2) * root.frac(root.wave.mic[index]))
          // Both sides in one track (an import): centred, and neutral where
          // no line says who it is.
          anchors.bottom: parent.mixed ? undefined : parent.verticalCenter
          anchors.verticalCenter: parent.mixed ? parent.verticalCenter : undefined
          color: !parent.mixed ? root.theme.accent
            : parent.micSpeaker !== "" ? root.speakerColor(parent.micSpeaker) : root.theme.alpha(root.theme.fg, 0.75)
          opacity: parent.played ? 1.0 : 0.6
        }
        Rectangle {
          objectName: "waveformCallBar"
          visible: root.wave.loopback !== null
          width: Math.max(1, parent.width - 1)
          height: root.wave.loopback ? Math.max(1, (bars.height / 2) * root.frac(root.wave.loopback[index])) : 0
          anchors.top: parent.verticalCenter
          color: parent.callSpeaker !== "" ? root.speakerColor(parent.callSpeaker) : root.theme.fg
          // A speaker's colour at the strength of your side; the neutral
          // bars as they always were.
          opacity: parent.callSpeaker !== "" ? (parent.played ? 1.0 : 0.6) : (parent.played ? 0.8 : 0.4)
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
  // The time under the pointer and, where a line says, who is speaking
  // there. On the card's colour, so it reads over the bars.
  Rectangle {
    visible: waveMouse.containsMouse
    x: Math.max(0, Math.min(root.width - width, waveMouse.mouseX + 4))
    y: 1
    width: hoverRow.implicitWidth + root.theme.sm * 2
    height: hoverRow.implicitHeight
    radius: 3
    color: root.theme.alpha(root.theme.bg, 0.85)
    Row {
      id: hoverRow
      x: root.theme.sm
      spacing: root.theme.sm
      Text {
        objectName: "waveformHoverTime"
        text: MT.clockMs(root.msAt(waveMouse.mouseX))
        color: root.theme.fg
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.caption
      }
      Rectangle {
        visible: root.hoverSpeaker !== ""
        anchors.verticalCenter: parent.verticalCenter
        width: 6
        height: 6
        radius: 3
        color: root.hoverSpeaker !== "" ? root.speakerColor(root.hoverSpeaker) : "transparent"
      }
      Text {
        objectName: "waveformHoverSpeaker"
        visible: root.hoverSpeaker !== ""
        text: root.hoverSpeaker !== "" && root.feed ? root.feed.displayName(root.hoverSpeaker) : ""
        // A long name gives way before the label outgrows the strip.
        width: Math.min(implicitWidth, root.width / 2)
        elide: Text.ElideRight
        color: root.theme.fg
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.caption
      }
    }
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
