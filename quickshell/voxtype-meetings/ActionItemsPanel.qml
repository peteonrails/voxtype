import QtQuick
import QtQuick.Layouts
import "MeetingText.js" as MT

// The Summary tab's action items: one row each with a tick (all ticked to
// start), the owner, the due date when one was said, and the time it was
// said, which jumps to the line. Send to Todoist (or Connect Todoist… first)
// and Remind me… act on the ticked rows; a row already sent shows ✓ there.
ColumnLayout {
  id: root

  property var theme
  property var actions: null
  signal citationActivated(real ms)

  spacing: root.theme.md
  visible: root.actions !== null && root.actions.supported && root.actions.items.length > 0

  RowLayout {
    Layout.fillWidth: true
    spacing: root.theme.gap
    Text {
      Layout.fillWidth: true
      text: "Action items"
      color: root.theme.fg
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.body
      font.bold: true
    }
    Text {
      objectName: "actionsCount"
      text: root.actions ? root.actions.tickedIds.length + " of " + root.actions.items.length + " ticked" : ""
      color: root.theme.dim
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.caption
    }
  }

  Repeater {
    model: root.actions ? root.actions.items : []
    delegate: RowLayout {
      id: item
      required property var modelData
      required property int index
      objectName: "actionRow_" + index
      Layout.fillWidth: true
      spacing: root.theme.md

      MeetingButton {
        objectName: "actionTick"
        theme: root.theme
        text: root.actions.ticked(item.modelData) ? "☑" : "☐"
        Layout.alignment: Qt.AlignTop
        onClicked: root.actions.toggle(item.modelData)
      }

      ColumnLayout {
        Layout.fillWidth: true
        spacing: root.theme.xxs
        Text {
          objectName: "actionText"
          Layout.fillWidth: true
          text: String(item.modelData.text || "")
          color: root.theme.fg
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.bodySmall
          wrapMode: Text.WordWrap
        }
        Flow {
          Layout.fillWidth: true
          spacing: root.theme.md
          Rectangle {
            objectName: "actionOwner"
            visible: !!item.modelData.owner
            implicitWidth: ownerText.implicitWidth + root.theme.md * 2
            implicitHeight: ownerText.implicitHeight + root.theme.xxs * 2
            radius: height / 2
            color: root.theme.alpha(root.theme.accent, 0.18)
            Text {
              id: ownerText
              anchors.centerIn: parent
              text: String(item.modelData.owner || "")
              color: root.theme.fg
              font.family: root.theme.fontFamily
              font.pixelSize: root.theme.caption
            }
          }
          Text {
            objectName: "actionDue"
            visible: text !== ""
            text: root.actions.dueText(item.modelData) !== "" ? "due " + root.actions.dueText(item.modelData) : ""
            color: root.theme.dim
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.caption
          }
          Text {
            objectName: "actionTime"
            visible: item.modelData.source_ms !== undefined && item.modelData.source_ms !== null
            text: MT.clockMs(item.modelData.source_ms)
            color: root.theme.accent
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.caption
            font.underline: timeMouse.containsMouse
            MouseArea {
              id: timeMouse
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: root.citationActivated(Number(item.modelData.source_ms))
            }
          }
          Text {
            objectName: "actionSent"
            visible: text !== ""
            text: {
              var where = []
              if (root.actions.sentTo(item.modelData, "todoist")) where.push("Todoist")
              if (root.actions.sentTo(item.modelData, "reminder")) where.push("reminder")
              return where.length > 0 ? "✓ " + where.join(", ") : ""
            }
            color: root.theme.accent
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.caption
          }
        }
      }
    }
  }

  Flow {
    Layout.fillWidth: true
    spacing: root.theme.gap
    MeetingButton {
      objectName: "actionsTodoist"
      theme: root.theme
      bordered: true
      visible: root.actions !== null && root.actions.todoistConnected
      enabled: root.actions !== null && root.actions.busy === "" && root.actions.tickedIds.length > 0
      text: root.actions && root.actions.busy === "todoist" ? "Sending…" : "Send to Todoist"
      onClicked: root.actions.send("todoist")
    }
    MeetingButton {
      objectName: "actionsConnect"
      theme: root.theme
      bordered: true
      visible: root.actions !== null && !root.actions.todoistConnected
      enabled: root.actions !== null && root.actions.busy === ""
      text: root.actions && root.actions.busy === "connect" ? "Waiting for the browser…" : "Connect Todoist…"
      onClicked: root.actions.connect()
    }
    MeetingButton {
      id: remindButton
      objectName: "actionsRemind"
      theme: root.theme
      bordered: true
      visible: root.actions !== null && root.actions.reminderAvailable
      enabled: root.actions !== null && root.actions.busy === "" && root.actions.tickedIds.length > 0
      text: root.actions && root.actions.busy === "reminder" ? "Setting…" : "Remind me…"
      onClicked: remindMenu.visible = !remindMenu.visible
    }
  }

  // The delay for Remind me….
  Flow {
    id: remindMenu
    objectName: "remindMenu"
    Layout.fillWidth: true
    visible: false
    spacing: root.theme.gap
    Repeater {
      model: [{ key: "15m", label: "In 15 min" }, { key: "1h", label: "In 1 hour" }, { key: "eod", label: "End of day (5 pm)" }]
      MeetingButton {
        required property var modelData
        objectName: "remind_" + modelData.key
        theme: root.theme
        text: modelData.label
        onClicked: {
          remindMenu.visible = false
          root.actions.send("reminder", modelData.key)
        }
      }
    }
  }

  // The consent page, should the browser not have opened it.
  Text {
    objectName: "actionsConnectUrl"
    Layout.fillWidth: true
    visible: root.actions !== null && root.actions.connectUrl !== ""
    text: root.actions ? '<a href="' + root.actions.connectUrl + '" style="color:' + root.theme.accent + '">Open the Todoist page</a> if it did not open.' : ""
    textFormat: Text.StyledText
    color: root.theme.dim
    font.family: root.theme.fontFamily
    font.pixelSize: root.theme.caption
    onLinkActivated: function(link) { Qt.openUrlExternally(link) }
  }
  MeetingButton {
    objectName: "actionsCancelConnect"
    theme: root.theme
    visible: root.actions !== null && root.actions.busy === "connect"
    text: "Cancel"
    onClicked: root.actions.cancelConnect()
  }

  Text {
    objectName: "actionsNotice"
    Layout.fillWidth: true
    visible: text !== ""
    text: root.actions ? root.actions.notice : ""
    color: root.theme.fg
    font.family: root.theme.fontFamily
    font.pixelSize: root.theme.bodySmall
    wrapMode: Text.WordWrap
  }
  Text {
    objectName: "actionsError"
    Layout.fillWidth: true
    visible: text !== ""
    text: root.actions ? (root.actions.actionError !== "" ? root.actions.actionError : root.actions.error) : ""
    color: root.theme.urgent
    font.family: root.theme.fontFamily
    font.pixelSize: root.theme.bodySmall
    wrapMode: Text.WordWrap
  }
}
