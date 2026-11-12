import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// The calendar's private iCal link, which names meetings that start without
// a title. It is not a config key: `voxtype meeting calendar set-ics` keeps
// it in voxtype's state directory, and nothing ever prints it back.
//
// The field is masked and is emptied once the link is saved, so the link is
// on screen only while it is being pasted. The panel hands it to the CLI on
// stdin, never in argv (which `ps` shows). What is shown afterwards comes from
// `meeting calendar status --json`: the host, when it was fetched, the last
// error and the next event.
Item {
  id: root

  // `meeting calendar status --json`, or null when it could not be read.
  property var status: null
  // "set", "clear" or "" while nothing runs.
  property string busy: ""
  // The last Set/Clear failure (one line naming only the host), and what the
  // last Set reported.
  property string errorText: ""
  property string noticeText: ""

  property bool highlighted: false

  signal setRequested(string link)
  signal clearRequested()
  signal editorFocusChanged(bool active)
  signal editorEscaped()

  readonly property var link: root.status && root.status.calendar_link ? root.status.calendar_link : null
  readonly property bool linkSet: root.link !== null && root.link.set === true
  readonly property string host: root.linkSet && root.link.host ? String(root.link.host) : ""
  readonly property string source: root.status && root.status.source ? String(root.status.source) : ""
  readonly property string helpText: root.status && root.status.link_help ? String(root.status.link_help) : ""
  readonly property color dim: Qt.darker(ShellTheme.Color.foreground, 1.5)

  // For the keyboard cursor: Enter on this row goes to the field.
  function activate() {
    field.forceActiveFocus()
  }

  // Bumped by the panel each time a link is saved: the field is emptied so
  // the link is not kept on screen. A refused link stays, to be corrected.
  property int savedCount: 0
  onSavedCountChanged: field.text = ""

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
    if (root.status === null) return ""
    if (root.linkSet) {
      var parts = ["Linked to " + (root.host || "your calendar") + "."]
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
      return parts.join(" ")
    }
    if (root.source === "command") return "No link set. The calendar command below names untitled meetings."
    if (root.source === "plugin") return "No link set. The Next Meeting widget on the bar names untitled meetings."
    return "No link set. Meetings that start without a title keep their app's name, such as \"Zoom call\"."
  }

  readonly property string lastError: root.linkSet && root.link.last_error ? String(root.link.last_error) : ""

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
        objectName: "calendarLinkLabel"
        text: "Calendar (iCal link)"
        color: ShellTheme.Color.foreground
        font.family: Style.font.family
        font.pixelSize: Style.font.body
      }

      Text {
        width: parent.width
        text: "Meetings that start without a title take it from your calendar. Paste the calendar's private iCal address; it is stored by voxtype and never shown again."
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
          objectName: "calendarLinkField"
          anchors.left: parent.left
          anchors.right: setButton.left
          anchors.rightMargin: Style.spacing.controlGap
          anchors.verticalCenter: parent.verticalCenter
          password: true
          hasCursor: root.highlighted
          enabled: root.busy === ""
          placeholderText: root.linkSet
            ? "Saved (" + (root.host || "link") + "). Paste a new link to replace it"
            : "https://calendar.google.com/calendar/ical/…/basic.ics"
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
          objectName: "calendarLinkSet"
          anchors.right: clearButton.visible ? clearButton.left : parent.right
          anchors.rightMargin: clearButton.visible ? Style.spacing.controlGap : 0
          anchors.verticalCenter: parent.verticalCenter
          text: root.busy === "set" ? "Checking…" : (root.linkSet ? "Replace" : "Set")
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
          objectName: "calendarLinkClear"
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          visible: root.linkSet
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
        objectName: "calendarLinkNotice"
        width: parent.width
        visible: text !== ""
        text: root.noticeText
        color: ShellTheme.Color.foreground
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }

      Text {
        objectName: "calendarLinkStatus"
        width: parent.width
        visible: text !== ""
        text: root.statusLine
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }

      Text {
        objectName: "calendarLinkError"
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
        objectName: "calendarLinkHelpToggle"
        visible: root.helpText !== ""
        text: (root.showHelp ? "▾ " : "▸ ") + "Where to find the link"
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
        objectName: "calendarLinkHelp"
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
