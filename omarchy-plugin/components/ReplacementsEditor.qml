import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// The text.replacements table, which is a map rather than a typed key and so
// cannot be a form row. Each entry is one `voxtype config set
// text.replacements.<spoken> "<written>"`; deleting one is `config unset`.
//
// The delete confirmation reparents itself onto `overlayHost` (the panel
// card) so its scrim covers the whole card instead of one scrolled row.
Item {
  id: root

  property var replacements: ({})
  property string errorText: ""
  property Item overlayHost: null

  signal setRequested(string from, string to)
  signal unsetRequested(string from)
  signal editorFocusChanged(bool active)

  readonly property color dim: Qt.darker(ShellTheme.Color.foreground, 1.5)

  readonly property var entries: {
    var keys = []
    for (var k in root.replacements) keys.push(k)
    keys.sort()
    var out = []
    for (var i = 0; i < keys.length; i++)
      out.push({ from: keys[i], to: String(root.replacements[keys[i]]) })
    return out
  }

  property string pendingDelete: ""

  implicitHeight: body.implicitHeight

  Column {
    id: body
    anchors.left: parent.left
    anchors.right: parent.right
    spacing: Style.spacing.lg

    PanelSectionHeader { text: "SPOKEN REPLACEMENTS" }

    Text {
      width: parent.width
      text: "Each row rewrites a spoken phrase in the transcript before it is typed."
      color: root.dim
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      wrapMode: Text.WordWrap
    }

    Text {
      visible: root.errorText !== ""
      width: parent.width
      text: root.errorText
      color: ShellTheme.Color.urgent
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      wrapMode: Text.WordWrap
    }

    Text {
      visible: root.entries.length === 0
      width: parent.width
      text: "No replacements configured."
      color: root.dim
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
    }

    Column {
      width: parent.width
      spacing: Style.spacing.sm

      Repeater {
        model: root.entries

        Item {
          id: row
          required property var modelData

          width: parent.width
          implicitHeight: Math.max(Style.space(34), toField.implicitHeight)
          height: implicitHeight

          Text {
            id: fromText
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            width: Style.space(180)
            text: row.modelData.from
            color: ShellTheme.Color.foreground
            font.family: Style.font.family
            font.pixelSize: Style.font.body
            elide: Text.ElideRight
          }

          Text {
            id: arrow
            anchors.left: fromText.right
            anchors.leftMargin: Style.spacing.controlGap
            anchors.verticalCenter: parent.verticalCenter
            text: "→"
            color: root.dim
            font.family: Style.font.family
            font.pixelSize: Style.font.body
          }

          TextField {
            id: toField
            anchors.left: arrow.right
            anchors.leftMargin: Style.spacing.controlGap
            anchors.right: deleteButton.left
            anchors.rightMargin: Style.spacing.controlGap
            anchors.verticalCenter: parent.verticalCenter
            text: row.modelData.to
            onEditingFinished: {
              if (text === row.modelData.to) return
              root.setRequested(row.modelData.from, text)
            }
            onActiveFocusChanged: root.editorFocusChanged(activeFocus)
          }

          PanelActionButton {
            id: deleteButton
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            iconText: "󰆴"
            tooltipText: "Delete replacement"
            hoverColor: ShellTheme.Color.urgent
            onClicked: {
              root.pendingDelete = row.modelData.from
              confirm.opened = true
            }
          }
        }
      }
    }

    PanelSeparator {}

    PanelSectionHeader { text: "ADD A REPLACEMENT" }

    Item {
      width: parent.width
      implicitHeight: Math.max(Style.space(34), addFrom.implicitHeight)
      height: implicitHeight

      TextField {
        id: addFrom
        anchors.left: parent.left
        anchors.verticalCenter: parent.verticalCenter
        width: Style.space(180)
        placeholderText: "spoken"
        onActiveFocusChanged: root.editorFocusChanged(activeFocus)
      }

      TextField {
        id: addTo
        anchors.left: addFrom.right
        anchors.leftMargin: Style.spacing.controlGap
        anchors.right: addButton.left
        anchors.rightMargin: Style.spacing.controlGap
        anchors.verticalCenter: parent.verticalCenter
        placeholderText: "written"
        onActiveFocusChanged: root.editorFocusChanged(activeFocus)
        onAccepted: addButton.clicked()
      }

      Button {
        id: addButton
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        text: "Add"
        iconText: ""
        bordered: true
        enabled: addFrom.text.trim() !== "" && addTo.text.trim() !== ""
        opacity: enabled ? 1.0 : 0.45
        onClicked: {
          if (!enabled) return
          root.setRequested(addFrom.text.trim(), addTo.text)
          addFrom.text = ""
          addTo.text = ""
        }
      }
    }
  }

  ConfirmDialog {
    id: confirm
    parent: root.overlayHost ? root.overlayHost : root
    anchors.fill: parent
    message: "Delete the replacement for \"" + root.pendingDelete + "\"?"
    confirmText: "Delete"
    onCanceled: {
      confirm.opened = false
      root.pendingDelete = ""
    }
    onConfirmed: {
      confirm.opened = false
      var target = root.pendingDelete
      root.pendingDelete = ""
      if (target !== "") root.unsetRequested(target)
    }
  }
}
