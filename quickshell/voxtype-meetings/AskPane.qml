import QtQuick
import QtQuick.Layouts
import "MeetingText.js" as MT

// The Ask tab: questions about this meeting and their answers, streamed in as
// the agent writes them. Quick chips cover the common ones. [mm:ss] citations
// in an answer jump to that line of the transcript.
Item {
  id: root

  property var theme
  property var assistant: null
  property var feed: null

  signal agentMenuRequested(var anchor)
  signal citationActivated(real ms)

  function submit() {
    var q = askField.text.trim()
    if (q === "" || !root.assistant) return
    root.assistant.ask(q, "")
    askField.text = ""
  }

  readonly property string lastRemote: root.feed && root.feed.segments.count >= 0 ? root.feed.lastRemoteSpeaker() : ""

  ColumnLayout {
    anchors.fill: parent
    spacing: root.theme.lg

    RowLayout {
      Layout.fillWidth: true
      Text {
        Layout.fillWidth: true
        text: "Ask about this meeting"
        color: root.theme.fg
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.body
        font.bold: true
      }
      AgentChip {
        theme: root.theme
        assistant: root.assistant
        onMenuRequested: function(anchor) { root.agentMenuRequested(anchor) }
      }
    }

    ListView {
      id: thread
      objectName: "askThread"
      Layout.fillWidth: true
      Layout.fillHeight: true
      clip: true
      spacing: root.theme.lg
      model: root.assistant ? root.assistant.thread : null
      boundsBehavior: Flickable.StopAtBounds
      onCountChanged: Qt.callLater(thread.positionViewAtEnd)

      Text {
        anchors.centerIn: parent
        width: parent.width
        visible: thread.count === 0
        horizontalAlignment: Text.AlignHCenter
        text: "Ask anything about what has been said, or pick a question below. Answers cite the time of the lines they use."
        color: root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.bodySmall
        wrapMode: Text.WordWrap
      }

      delegate: Item {
        id: entry
        required property int index
        required property string role
        required property string text
        required property string agent
        required property bool pending
        required property string error
        objectName: entry.role === "q" ? "askQuestion" : "askAnswer"
        width: ListView.view.width
        implicitHeight: entry.role === "q" ? bubble.height : answer.implicitHeight

        Rectangle {
          id: bubble
          visible: entry.role === "q"
          anchors.right: parent.right
          width: Math.min(parent.width * 0.85, qText.implicitWidth + root.theme.padX * 2)
          height: qText.implicitHeight + root.theme.md * 2
          radius: Math.min(root.theme.radius, 8)
          color: root.theme.alpha(root.theme.accent, 0.18)
          Text {
            id: qText
            anchors.fill: parent
            anchors.margins: root.theme.md
            anchors.leftMargin: root.theme.padX
            anchors.rightMargin: root.theme.padX
            text: entry.text
            color: root.theme.fg
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.body
            wrapMode: Text.WordWrap
          }
        }

        Column {
          id: answer
          visible: entry.role === "a"
          width: parent.width
          spacing: root.theme.xs

          Text {
            objectName: "answerText"
            width: parent.width
            visible: entry.text !== ""
            text: MT.linkify(entry.text, root.theme.accent)
            textFormat: Text.MarkdownText
            color: root.theme.fg
            linkColor: root.theme.accent
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.body
            wrapMode: Text.WordWrap
            onLinkActivated: function(link) {
              var ms = MT.linkMs(link)
              if (ms >= 0) root.citationActivated(ms)
            }
            MouseArea {
              anchors.fill: parent
              acceptedButtons: Qt.NoButton
              cursorShape: parent.hoveredLink !== "" ? Qt.PointingHandCursor : Qt.ArrowCursor
            }
          }

          Text {
            objectName: "answerPending"
            visible: entry.pending
            text: entry.text === "" ? "Thinking…" : "…"
            color: root.theme.dim
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.bodySmall
          }

          Text {
            objectName: "answerError"
            width: parent.width
            visible: entry.error !== ""
            text: entry.error
            color: root.theme.urgent
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.bodySmall
            wrapMode: Text.WordWrap
          }

          Text {
            objectName: "answerBy"
            visible: !entry.pending && entry.error === "" && entry.agent !== ""
            text: "Answered by " + entry.agent
            color: root.theme.dim
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.caption
          }
        }
      }
    }

    Flow {
      objectName: "askChips"
      Layout.fillWidth: true
      spacing: root.theme.md

      MeetingButton {
        objectName: "askChip_last5"
        theme: root.theme
        text: "Last 5 minutes"
        bordered: true
        enabled: root.assistant !== null
        onClicked: root.assistant.askChip("last-5m")
      }
      MeetingButton {
        objectName: "askChip_lastSpeaker"
        theme: root.theme
        visible: root.lastRemote !== ""
        text: "What did " + root.lastRemote + " just say?"
        bordered: true
        enabled: root.assistant !== null
        onClicked: root.assistant.askChip("last-speaker")
      }
      MeetingButton {
        objectName: "askChip_actions"
        theme: root.theme
        text: "Action items so far"
        bordered: true
        enabled: root.assistant !== null
        onClicked: root.assistant.askChip("actions")
      }
    }

    RowLayout {
      Layout.fillWidth: true
      spacing: root.theme.gap

      MeetingTextField {
        id: askField
        objectName: "askField"
        theme: root.theme
        Layout.fillWidth: true
        placeholderText: "Ask a question…"
        onAccepted: root.submit()
      }

      MeetingButton {
        objectName: "askButton"
        theme: root.theme
        text: "Ask"
        bordered: true
        enabled: askField.text.trim() !== ""
        onClicked: root.submit()
      }
    }
  }
}
