import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// One entry of the schema's `secrets` list: a value voxtype keeps outside
// config.toml and never prints back, today the calendar's private iCal link.
//
// The field is masked and is emptied once the value is saved, so it is on
// screen only while it is being pasted. The panel hands it to the entry's
// set_command on stdin, never in argv (which `ps` shows). Afterwards the row
// shows what the entry and its status_command report: for the calendar link,
// the host, when it was fetched, the next event and the last error.
Item {
  id: root

  // The schema's entry: {key, label, description, help, set, host,
  // last_error, set_command, clear_command, status_command}.
  property var spec: null
  // The status_command's answer, or null.
  property var status: null
  // "set", "clear" or "" while nothing runs for this entry.
  property string busy: ""
  // The last Set/Clear failure (one line naming no secret), and what the last
  // Set or Clear reported.
  property string errorText: ""
  property string noticeText: ""
  // Bumped by the panel each time the value is saved: the field is emptied so
  // it is not kept on screen. A refused value stays, to be corrected.
  property int savedCount: 0

  property bool highlighted: false

  signal setRequested(string value)
  signal clearRequested()
  signal editorFocusChanged(bool active)
  signal editorEscaped()

  onSavedCountChanged: field.text = ""

  // For the keyboard cursor: Enter on this row goes to the field.
  function activate() {
    field.forceActiveFocus()
  }

  // The calendar's status_command answer carries more than the entry: when
  // it was fetched, the next event, and what names meetings without a link.
  readonly property var link: root.status && root.status.calendar_link ? root.status.calendar_link : null
  readonly property bool valueSet: root.link !== null ? root.link.set === true : (root.spec !== null && root.spec.set === true)
  readonly property string host: {
    var h = root.link !== null ? root.link.host : (root.spec ? root.spec.host : null)
    return root.valueSet && h ? String(h) : ""
  }
  readonly property string lastError: {
    if (!root.valueSet) return ""
    var e = root.link !== null ? root.link.last_error : (root.spec ? root.spec.last_error : null)
    return e ? String(e) : ""
  }
  readonly property string label: root.spec && root.spec.label ? String(root.spec.label) : (root.spec ? String(root.spec.key) : "")
  readonly property string description: root.spec && root.spec.description ? String(root.spec.description) : ""
  readonly property string helpText: root.spec && root.spec.help ? String(root.spec.help) : ""
  readonly property bool canClear: root.spec !== null && root.spec.clear_command !== undefined && root.spec.clear_command !== null
  readonly property color dim: Qt.darker(ShellTheme.Color.foreground, 1.5)

  function clockTime(date) {
    return Qt.formatTime(date, "hh:mm")
  }

  function dayAndTime(date) {
    var today = new Date()
    var sameDay = date.getFullYear() === today.getFullYear() && date.getMonth() === today.getMonth()
      && date.getDate() === today.getDate()
    return (sameDay ? "" : Qt.formatDate(date, "ddd") + " ") + root.clockTime(date)
  }

  readonly property string statusLine: {
    if (root.spec === null) return ""
    if (root.valueSet) {
      var parts = [root.host !== "" ? "Linked to " + root.host + "." : "Saved."]
      if (root.link !== null) {
        var fetched = root.link.fetched_at
        if (fetched !== null && fetched !== undefined)
          parts.push("Fetched at " + root.clockTime(new Date(Number(fetched) * 1000)) + ".")
        else
          parts.push("Not fetched yet.")
        var next = root.link.next
        if (next && next.title) {
          var start = new Date(String(next.start))
          parts.push("Next: " + String(next.title) + (isNaN(start.getTime()) ? "" : ", " + root.dayAndTime(start)) + ".")
        } else if (root.link.events === 0) {
          parts.push("Nothing in the next 48 hours.")
        }
      }
      return parts.join(" ")
    }
    if (root.link === null) return "Not set."
    var source = root.status.source ? String(root.status.source) : ""
    if (source === "command") return "No link set. The calendar command below names untitled meetings."
    if (source === "plugin") return "No link set. The Next Meeting widget on the bar names untitled meetings."
    return "No link set. Meetings that start without a title keep their app's name, such as \"Zoom call\"."
  }

  property bool showHelp: false

  implicitHeight: surface.implicitHeight

  CursorSurface {
    id: surface
    anchors.left: parent.left
    anchors.right: parent.right
    hasCursor: root.highlighted
    implicitHeight: body.implicitHeight + Style.spacing.md * 2

    Column {
      id: body
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      anchors.leftMargin: Style.spacing.md
      anchors.rightMargin: Style.spacing.md
      spacing: Style.spacing.sm

      Text {
        objectName: "secretLabel"
        text: root.label
        color: ShellTheme.Color.foreground
        font.family: Style.font.family
        font.pixelSize: Style.font.body
      }

      Text {
        objectName: "secretDescription"
        width: parent.width
        text: (root.description !== "" ? root.description + " " : "")
          + "Voxtype keeps it outside config.toml and never shows it again."
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }

      Item {
        width: parent.width
        implicitHeight: Math.max(Style.space(34), field.implicitHeight)
        height: implicitHeight

        TextField {
          id: field
          objectName: "secretField"
          anchors.left: parent.left
          anchors.right: setButton.left
          anchors.rightMargin: Style.spacing.controlGap
          anchors.verticalCenter: parent.verticalCenter
          password: true
          hasCursor: root.highlighted
          enabled: root.busy === ""
          placeholderText: root.valueSet
            ? "Saved" + (root.host !== "" ? " (" + root.host + ")" : "") + ". Paste a new one to replace it"
            : "Paste it here"
          onActiveFocusChanged: root.editorFocusChanged(activeFocus)
          onAccepted: setButton.clicked()
          Keys.onEscapePressed: function(event) {
            field.focus = false
            root.editorEscaped()
            event.accepted = true
          }
        }

        Button {
          id: setButton
          objectName: "secretSet"
          anchors.right: clearButton.visible ? clearButton.left : parent.right
          anchors.rightMargin: clearButton.visible ? Style.spacing.controlGap : 0
          anchors.verticalCenter: parent.verticalCenter
          text: root.busy === "set" ? "Checking…" : (root.valueSet ? "Replace" : "Set")
          iconText: ""
          bordered: true
          enabled: root.busy === "" && field.text.trim() !== ""
          opacity: enabled || root.busy === "set" ? 1.0 : 0.45
          onClicked: {
            if (!enabled) return
            root.setRequested(field.text.trim())
          }
        }

        Button {
          id: clearButton
          objectName: "secretClear"
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          visible: root.valueSet && root.canClear
          text: root.busy === "clear" ? "Clearing…" : "Clear"
          iconText: ""
          bordered: true
          enabled: root.busy === ""
          opacity: enabled || root.busy === "clear" ? 1.0 : 0.45
          onClicked: {
            if (!enabled) return
            root.clearRequested()
          }
        }
      }

      Text {
        objectName: "secretNotice"
        width: parent.width
        visible: text !== ""
        text: root.noticeText
        color: ShellTheme.Color.foreground
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }

      Text {
        objectName: "secretStatus"
        width: parent.width
        visible: text !== ""
        text: root.statusLine
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }

      Text {
        objectName: "secretError"
        width: parent.width
        visible: text !== ""
        text: root.errorText !== "" ? root.errorText
          : (root.lastError !== "" ? "Last fetch failed: " + root.lastError : "")
        color: ShellTheme.Color.urgent
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }

      Text {
        objectName: "secretHelpToggle"
        visible: root.helpText !== ""
        text: (root.showHelp ? "▾ " : "▸ ") + "Where to find it"
        color: helpMouse.containsMouse ? ShellTheme.Color.foreground : root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        MouseArea {
          id: helpMouse
          anchors.fill: parent
          hoverEnabled: true
          cursorShape: Qt.PointingHandCursor
          onClicked: root.showHelp = !root.showHelp
        }
      }

      Text {
        objectName: "secretHelp"
        width: parent.width
        visible: root.showHelp && root.helpText !== ""
        text: root.helpText
        textFormat: Text.PlainText
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }
    }
  }
}
