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
  signal copyRequested(string text, string key)
  signal settingsRequested()
  property bool canOpenSettings: false
  property string copiedKey: ""
  // MeetingActions: the action items as rows to tick and send.
  property var actions: null
  readonly property bool actionsShown: actionsPanel.visible

  // The summary as written, without its action items section while the
  // panel above it shows them.
  function shownMarkdown(md) {
    if (!root.actionsShown) return md
    return md.replace(/(^|\n\n)## Action items\n\n[\s\S]*?(?=\n\n## |$)/, "")
  }

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
        label: root.assistant ? root.assistant.summaryLabel : ""
        onMenuRequested: function(anchor) { root.agentMenuRequested(anchor) }
      }
      MeetingButton {
        objectName: "summaryCopy"
        theme: root.theme
        visible: root.assistant && root.assistant.summaryMarkdown !== ""
        text: root.copiedKey === "summary" ? "Copied" : "Copy"
        bordered: true
        onClicked: root.copyRequested(root.assistant.summaryMarkdown, "summary")
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
      objectName: "assistantProblem"
      Layout.fillWidth: true
      visible: text !== "" && root.summaryState !== "ready"
      text: root.assistant ? root.assistant.problem : ""
      color: root.theme.urgent
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.bodySmall
      wrapMode: Text.WordWrap
    }

    // Writing it: at the top, naming who is writing it.
    RowLayout {
      objectName: "summarizingRow"
      Layout.fillWidth: true
      visible: root.summaryState === "summarizing"
      spacing: root.theme.md
      Row {
        spacing: root.theme.xs
        Repeater {
          model: 3
          Rectangle {
            required property int index
            width: 5
            height: 5
            radius: 2.5
            color: root.theme.accent
            opacity: 0.3
            SequentialAnimation on opacity {
              running: root.summaryState === "summarizing" && root.visible
              loops: Animation.Infinite
              PauseAnimation { duration: index * 200 }
              NumberAnimation { to: 1.0; duration: 350 }
              NumberAnimation { to: 0.3; duration: 350 }
              PauseAnimation { duration: (2 - index) * 200 }
            }
          }
        }
      }
      Text {
        objectName: "summarizing"
        Layout.fillWidth: true
        text: "Summarizing with " + (root.assistant && root.assistant.summaryWith !== "" ? root.assistant.summaryWith : "the assistant") + "…"
        color: root.theme.fg
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.bodySmall
        wrapMode: Text.WordWrap
      }
    }

    // A failed summary: what happened and the ways on, at the top.
    Rectangle {
      objectName: "summaryErrorCard"
      Layout.fillWidth: true
      visible: root.summaryState === "error"
      implicitHeight: errorColumn.implicitHeight + root.theme.padX * 2
      radius: Math.min(root.theme.radius, 6)
      color: root.theme.alpha(root.theme.urgent, 0.12)
      border.width: 1
      border.color: root.theme.alpha(root.theme.urgent, 0.5)

      Column {
        id: errorColumn
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: root.theme.padX
        spacing: root.theme.md

        Text {
          objectName: "summaryErrorTitle"
          width: parent.width
          text: "Couldn't write the summary"
          color: root.theme.fg
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.body
          font.bold: true
        }
        Text {
          objectName: "summaryError"
          width: parent.width
          text: root.assistant ? root.assistant.summaryError : ""
          color: root.theme.fg
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.bodySmall
          wrapMode: Text.WordWrap
        }
        Flow {
          width: parent.width
          spacing: root.theme.gap
          MeetingButton {
            objectName: "summaryRetry"
            theme: root.theme
            text: "Retry"
            bordered: true
            onClicked: root.assistant.regenerate()
          }
          MeetingButton {
            objectName: "summaryUseAgent"
            theme: root.theme
            visible: root.assistant !== null && root.assistant.summaryFailedLocal && root.assistant.allowedAgent !== null
            text: root.assistant && root.assistant.allowedAgent ? "Use " + root.assistant.allowedAgent.name + " instead" : ""
            bordered: true
            onClicked: root.assistant.useAgentInstead()
          }
          MeetingButton {
            objectName: "summarySettings"
            theme: root.theme
            visible: root.canOpenSettings
            text: "Settings"
            bordered: true
            onClicked: root.settingsRequested()
          }
        }
      }
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
      contentHeight: summaryColumn.implicitHeight
      boundsBehavior: Flickable.StopAtBounds
      // An old summary stays readable while a new one is written.
      visible: root.assistant && root.assistant.summaryMarkdown !== ""

      Column {
        id: summaryColumn
        width: flick.width
        spacing: root.theme.lg

      ActionItemsPanel {
        id: actionsPanel
        objectName: "actionsPanel"
        width: flick.width
        theme: root.theme
        actions: root.actions
        onCitationActivated: function(ms) { root.citationActivated(ms) }
      }

      Text {
        id: summaryText
        objectName: "summaryText"
        width: flick.width
        text: root.assistant ? MT.linkify(root.shownMarkdown(root.assistant.summaryMarkdown), root.theme.accent) : ""
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

    // With no summary text the layout would spread the rows above down the
    // pane, leaving a state or an error stranded at the bottom: keep them up.
    Item {
      Layout.fillHeight: true
      visible: !flick.visible
    }
  }
}
