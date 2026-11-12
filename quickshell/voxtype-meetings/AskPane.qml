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
  signal copyRequested(string text, string key)
  property string copiedKey: ""

  // Show one answer (a notification's Open) and flash it.
  property int flashRow: -1
  function focusRow(row) {
    thread.positionViewAtIndex(row, ListView.Center)
    root.flashRow = row
    flashTimer.restart()
  }
  Timer { id: flashTimer; interval: 1600; onTriggered: root.flashRow = -1 }

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

    // Why nothing can answer (no agent, an old voxtype, the CLI missing).
    Text {
      objectName: "assistantProblem"
      Layout.fillWidth: true
      visible: text !== ""
      text: root.assistant ? root.assistant.problem : ""
      color: root.theme.urgent
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.bodySmall
      wrapMode: Text.WordWrap
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
      // Follow the newest answer as it streams, fails or finishes, unless the
      // user has scrolled up to read something older.
      property bool following: true
      onMovementEnded: thread.following = thread.atYEnd
      onCountChanged: { thread.following = true; Qt.callLater(thread.positionViewAtEnd) }
      onContentHeightChanged: if (thread.following) Qt.callLater(thread.positionViewAtEnd)

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
        required property string citations
        required property string askId
        objectName: entry.role === "q" ? "askQuestion" : "askAnswer"
        width: ListView.view.width
        implicitHeight: entry.role === "q" ? bubble.height : answer.implicitHeight

        Rectangle {
          anchors.fill: parent
          anchors.margins: -root.theme.xs
          radius: 4
          color: root.theme.alpha(root.theme.accent, 0.18)
          visible: root.flashRow === entry.index
        }

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
            text: MT.linkCitations(entry.text, entry.citations, root.theme.accent)
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

          Rectangle {
            objectName: "answerErrorCard"
            width: parent.width
            visible: entry.error !== ""
            implicitHeight: answerErrorColumn.implicitHeight + root.theme.padX * 2
            height: implicitHeight
            radius: Math.min(root.theme.radius, 6)
            color: root.theme.alpha(root.theme.urgent, 0.12)
            border.width: 1
            border.color: root.theme.alpha(root.theme.urgent, 0.5)
            Column {
              id: answerErrorColumn
              anchors.left: parent.left
              anchors.right: parent.right
              anchors.top: parent.top
              anchors.margins: root.theme.padX
              spacing: root.theme.xs
              Text {
                width: parent.width
                text: "Couldn't answer"
                color: root.theme.fg
                font.family: root.theme.fontFamily
                font.pixelSize: root.theme.bodySmall
                font.bold: true
              }
              Text {
                objectName: "answerError"
                width: parent.width
                text: entry.error
                color: root.theme.fg
                font.family: root.theme.fontFamily
                font.pixelSize: root.theme.bodySmall
                wrapMode: Text.WordWrap
              }
            }
          }

          Row {
            spacing: root.theme.lg
            visible: !entry.pending && entry.error === ""

            Text {
              objectName: "answerBy"
              visible: entry.agent !== ""
              text: "Answered by " + entry.agent
              color: root.theme.dim
              font.family: root.theme.fontFamily
              font.pixelSize: root.theme.caption
            }

            Text {
              id: copyAnswer
              objectName: "answerCopy"
              visible: entry.text !== ""
              readonly property string key: "answer:" + entry.index
              text: root.copiedKey === copyAnswer.key ? "Copied" : "Copy"
              color: copyMouse.containsMouse ? root.theme.fg : root.theme.dim
              font.family: root.theme.fontFamily
              font.pixelSize: root.theme.caption
              font.underline: copyMouse.containsMouse
              MouseArea {
                id: copyMouse
                anchors.fill: parent
                anchors.margins: -root.theme.sm
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: root.copyRequested(entry.text, copyAnswer.key)
              }
            }
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
