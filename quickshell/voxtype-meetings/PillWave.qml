import QtQuick

// The pill's live waveform: the last few seconds of the meeting's levels as a
// short strip, your mic above the middle line and the call below it, moving
// at about 10 Hz. It samples the feed's latest `levels` (MeetingFeed), and a
// sample taken while none are arriving is silence, so the strip falls flat
// when they stop (paused, or a quiet daemon).
Item {
  id: root

  property var theme
  property var feed: null
  // Sampling only while this is true (a recording meeting, pill shown).
  property bool active: false
  property int samples: 28
  property int barWidth: 2
  property int barGap: 1
  property int staleMs: 400

  // Oldest first, 0 to 1 each.
  property var micSamples: []
  property var callSamples: []
  readonly property bool hasCall: root.feed !== null && root.feed.loopbackSeen

  implicitWidth: root.samples * (root.barWidth + root.barGap) - root.barGap
  implicitHeight: 22

  function frac(db) {
    if (db === null || db === undefined || isNaN(db)) return 0
    return Math.max(0, Math.min(1, (Number(db) + 60) / 60))
  }

  function sample() {
    var fresh = root.feed !== null && root.feed.levelsAt > 0 && Date.now() - root.feed.levelsAt <= root.staleMs
    var mic = root.micSamples.slice(-(root.samples - 1))
    var call = root.callSamples.slice(-(root.samples - 1))
    mic.push(fresh ? root.frac(root.feed.micDb) : 0)
    call.push(fresh ? root.frac(root.feed.loopbackDb) : 0)
    root.micSamples = mic
    root.callSamples = call
  }

  function clear() {
    var flat = []
    for (var i = 0; i < root.samples; i++) flat.push(0)
    root.micSamples = flat
    root.callSamples = flat.slice()
  }

  Component.onCompleted: root.clear()

  Timer {
    interval: 100
    repeat: true
    running: root.active && root.visible
    onTriggered: root.sample()
  }

  Rectangle {
    anchors.verticalCenter: parent.verticalCenter
    width: parent.width
    height: 1
    color: root.theme.alpha(root.theme.fg, 0.25)
  }

  Row {
    anchors.fill: parent
    spacing: root.barGap
    Repeater {
      model: root.samples
      Item {
        required property int index
        width: root.barWidth
        height: root.height
        Rectangle {
          width: parent.width
          height: Math.round((root.height / 2 - 1) * (root.micSamples[index] || 0))
          anchors.bottom: parent.verticalCenter
          radius: 1
          color: root.theme.accent
        }
        Rectangle {
          visible: root.hasCall
          width: parent.width
          height: Math.round((root.height / 2 - 1) * (root.callSamples[index] || 0))
          anchors.top: parent.verticalCenter
          radius: 1
          color: root.theme.alpha(root.theme.fg, 0.7)
        }
      }
    }
  }
}
