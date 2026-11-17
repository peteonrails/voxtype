import QtQuick
import QtQuick.Layouts

// The pre-call sound check, reached from the idle screen: both tracks'
// levels from MeetingSoundCheck, what the next meeting will use (the mic,
// whether the call is captured, the echo handling), and how long is left.
ColumnLayout {
  id: root

  property var theme
  property var check: null
  signal closeRequested()

  spacing: root.theme.lg

  readonly property string checkState: root.check ? root.check.checkState : ""
  readonly property bool running: root.check !== null && root.check.running
  property real now: Date.now()
  readonly property int secondsLeft: {
    if (!root.check || root.checkState !== "running" || root.check.startedAt <= 0) return 0
    return Math.max(0, Math.ceil(root.check.total - (root.now - root.check.startedAt) / 1000))
  }

  Timer {
    interval: 250
    repeat: true
    running: root.visible && root.checkState === "running"
    onTriggered: root.now = Date.now()
  }

  function echoText(mode) {
    if (mode === "webrtc") return "Echo cancellation: PipeWire's WebRTC canceller."
    if (mode === "gtcrn") return "Echo cancellation: voxtype's GTCRN model."
    if (mode === "none") return "No echo cancellation: on speakers, the other side's voice can end up in your lines. Headphones avoid that."
    return ""
  }

  function stoppedText() {
    if (!root.check) return ""
    var r = root.check.reason
    if (r === "done") return "Done. Run it again any time before a call."
    if (r === "stopped") return "Stopped."
    if (r === "meeting") return "A meeting started, so the check gave it the microphone."
    if (r === "error") return root.check.message !== "" ? root.check.message : "The sound check failed."
    return ""
  }

  RowLayout {
    Layout.fillWidth: true
    spacing: root.theme.gap

    Text {
      Layout.fillWidth: true
      text: "Sound check"
      color: root.theme.fg
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.title
    }

    MeetingButton {
      objectName: "soundCheckClose"
      theme: root.theme
      text: "‹ Meetings"
      onClicked: root.closeRequested()
    }
  }

  Text {
    Layout.fillWidth: true
    text: "Opens the microphone and the call audio the way a meeting would, and records nothing. "
      + "Say something, and play a bit of the call: both bars should move."
    color: root.theme.dim
    font.family: root.theme.fontFamily
    font.pixelSize: root.theme.bodySmall
    wrapMode: Text.WordWrap
  }

  // ---- your microphone
  ColumnLayout {
    Layout.fillWidth: true
    spacing: root.theme.xs

    RowLayout {
      Layout.fillWidth: true
      Text {
        text: "Your microphone"
        color: root.theme.fg
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.body
      }
      Text {
        objectName: "soundCheckMic"
        Layout.fillWidth: true
        horizontalAlignment: Text.AlignRight
        text: root.check ? root.check.mic : ""
        elide: Text.ElideMiddle
        color: root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.caption
      }
    }
    LevelMeter {
      objectName: "soundCheckMicMeter"
      Layout.fillWidth: true
      theme: root.theme
      db: root.check ? root.check.micDb : null
      peakDb: root.check ? root.check.micPeakDb : null
      at: root.check ? root.check.levelsAt : 0
    }
  }

  // ---- the call
  ColumnLayout {
    Layout.fillWidth: true
    spacing: root.theme.xs

    Text {
      text: "The call"
      color: root.theme.fg
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.body
    }
    LevelMeter {
      objectName: "soundCheckLoopbackMeter"
      Layout.fillWidth: true
      visible: !(root.checkState === "running" && root.check && !root.check.loopback)
      theme: root.theme
      color: root.theme.alpha(root.theme.fg, 0.75)
      db: root.check ? root.check.loopbackDb : null
      peakDb: root.check ? root.check.loopbackPeakDb : null
      at: root.check ? root.check.levelsAt : 0
    }
    Text {
      objectName: "soundCheckNoLoopback"
      Layout.fillWidth: true
      visible: root.checkState === "running" && root.check !== null && !root.check.loopback
      text: "No call audio is captured, so a meeting records only your side. "
        + "Set meeting.audio.loopback_device to the call's output monitor."
      color: root.theme.dim
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.bodySmall
      wrapMode: Text.WordWrap
    }
  }

  Text {
    objectName: "soundCheckEcho"
    Layout.fillWidth: true
    visible: text !== ""
    text: root.check ? root.echoText(root.check.echoCancel) : ""
    color: root.theme.dim
    font.family: root.theme.fontFamily
    font.pixelSize: root.theme.bodySmall
    wrapMode: Text.WordWrap
  }

  RowLayout {
    Layout.fillWidth: true
    spacing: root.theme.gap

    Text {
      objectName: "soundCheckStatus"
      Layout.fillWidth: true
      text: root.checkState === "starting" ? "Opening the audio…"
        : root.checkState === "running" ? root.secondsLeft + " s left"
        : root.stoppedText()
      color: root.check && root.checkState === "stopped" && root.check.reason === "error" ? root.theme.urgent : root.theme.dim
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.bodySmall
      wrapMode: Text.WordWrap
    }

    MeetingButton {
      objectName: "soundCheckStop"
      theme: root.theme
      visible: root.running
      text: "Stop"
      bordered: true
      onClicked: root.check.stop()
    }

    MeetingButton {
      objectName: "soundCheckStart"
      theme: root.theme
      visible: !root.running
      text: root.checkState === "stopped" ? "Check again" : "Start"
      bordered: true
      onClicked: root.check.start()
    }
  }

  Item { Layout.fillHeight: true }
}
