import QtQuick
import QtQuick.Layouts
import "MeetingText.js" as MT

// The Summary tab, once the meeting has stopped: the summary as written
// (overview, decisions, action items, chapters), "Summarizing…" while it is
// being written, and Regenerate after speakers or lines have been fixed.
// Chapter times and other [mm:ss] links jump into the transcript.
Item {
  id: root

  property var theme
  property var assistant: null
  property var feed: null

  signal agentMenuRequested(var anchor)
  signal citationActivated(real ms)

  readonly property string summaryState: root.assistant ? root.assistant.summaryState : ""
  readonly property alias flick: flick

  ColumnLayout {
    anchors.fill: parent
    spacing: root.theme.lg

    RowLayout {
      Layout.fillWidth: true
      spacing: root.theme.gap
      Text {
        Layout.fillWidth: true
        text: "Summary"
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
      MeetingButton {
        objectName: "regenerateButton"
        theme: root.theme
        text: root.summaryState === "ready" || root.summaryState === "error" ? "Regenerate" : "Summarize"
        bordered: true
        enabled: root.summaryState !== "summarizing" && root.feed && root.feed.meetingId !== ""
        onClicked: root.assistant.regenerate()
      }
    }

    Text {
      objectName: "summarizing"
      Layout.fillWidth: true
      visible: root.summaryState === "summarizing"
      text: "Summarizing… this takes a minute for a long meeting."
      color: root.theme.dim
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.bodySmall
      wrapMode: Text.WordWrap
    }

    Text {
      objectName: "summaryError"
      Layout.fillWidth: true
      visible: root.summaryState === "error"
      text: root.assistant ? root.assistant.summaryError : ""
      color: root.theme.urgent
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.bodySmall
      wrapMode: Text.WordWrap
    }

    Text {
      objectName: "summaryEmpty"
      Layout.fillWidth: true
      visible: root.summaryState === ""
      text: "No summary yet. Summarize writes one from the transcript, speaker names and your notes."
      color: root.theme.dim
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.bodySmall
      wrapMode: Text.WordWrap
    }

    Flickable {
      id: flick
      objectName: "summaryFlick"
      Layout.fillWidth: true
      Layout.fillHeight: true
      clip: true
      contentWidth: width
      contentHeight: summaryText.implicitHeight
      boundsBehavior: Flickable.StopAtBounds
      // An old summary stays readable while a new one is written.
      visible: root.assistant && root.assistant.summaryMarkdown !== ""

      Text {
        id: summaryText
        objectName: "summaryText"
        width: flick.width
        text: root.assistant ? MT.linkify(root.assistant.summaryMarkdown, root.theme.accent) : ""
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
    }
  }
}
