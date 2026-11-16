import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Shared normal-window content. The host supplies the canonical CLI transport;
// the component never opens a microphone, compositor socket or history file.
Rectangle {
  id: root
  color: Qt.rgba(root.bg.r, root.bg.g, root.bg.b, 1)
  property var client: null
  // A MeetingTheme from the host (the Omarchy shell's colours and font, or
  // the active Omarchy theme under the standalone shell). Without one the
  // panel uses its own dark palette.
  property var theme: null
  property string stateText: "Loading history"
  readonly property int entryCount: rows.count
  property string errorText: ""
  property string listErrorText: ""
  property string noticeText: ""
  property int sequence: 0
  property string pending: ""
  property string listPending: ""
  property string query: ""
  property string renderedQuery: ""
  property string pendingKind: "rows"
  property bool historyEnabled: false
  property bool historyPaused: false
  property bool stateKnown: false
  // Days of history kept; 0 keeps everything.
  property int retentionDays: 30
  readonly property var retentionChoices: [7, 30, 90, 365, 0]
  property string expandedId: ""
  property string correctionId: ""
  // Where the caret goes when a correction starts; -1 puts it at the end.
  property int correctionCursor: -1
  property string deleteId: ""
  property string pasteId: ""
  // The host sets this to the window's visibility, so a hidden window stops
  // asking for new dictations.
  property bool live: true
  signal pasteRequested()
  signal pasteFailed()
  signal closeRequested()

  readonly property color fg: theme ? theme.fg : "#eceff4"
  readonly property color bg: theme ? theme.bg : "#151820"
  readonly property color accent: theme ? theme.accent : "#85c7d0"
  readonly property color bad: theme ? theme.urgent : "#ffb4ab"
  // Everything else is the foreground at a strength, so any theme, light or
  // dark, gets surfaces that sit on its own background.
  readonly property color dim: tint(0.62)
  readonly property color faint: tint(0.38)
  readonly property color card: tint(0.05)
  readonly property color cardHover: tint(0.09)
  readonly property color field: tint(0.04)
  readonly property color line: tint(0.20)
  readonly property color raised: Qt.tint(Qt.rgba(bg.r, bg.g, bg.b, 1), tint(0.10))
  readonly property string fontFamily: theme && theme.fontFamily ? theme.fontFamily : Qt.application.font.family
  function tint(strength) { return Qt.rgba(fg.r, fg.g, fg.b, strength) }

  // A small flat button. It stays a Controls Button so it keeps click(),
  // keyboard activation and its accessible role.
  component Chip: Button {
    id: chip
    property bool danger: false
    property bool outlined: false
    font.pixelSize: 12
    topPadding: 3
    bottomPadding: 3
    leftPadding: 8
    rightPadding: 8
    implicitHeight: 24
    hoverEnabled: true
    contentItem: Text {
      text: chip.text
      font: chip.font
      color: !chip.enabled ? root.faint
        : chip.danger && chip.hovered ? root.bad
        : chip.hovered ? root.fg : root.tint(0.74)
      horizontalAlignment: Text.AlignHCenter
      verticalAlignment: Text.AlignVCenter
    }
    background: Rectangle {
      radius: 4
      color: chip.down ? root.tint(0.20) : chip.hovered && chip.enabled ? root.tint(0.12) : "transparent"
      border.width: chip.outlined ? 1 : 0
      border.color: root.line
    }
  }

  function bestText(entry) {
    if (entry.corrected_text !== undefined && entry.corrected_text !== null)
      return String(entry.corrected_text)
    return String(entry.text || entry.raw_text || "")
  }

  function beginCorrection(id, cursor) {
    if (pending !== "") return
    errorText = ""
    correctionCursor = cursor === undefined ? -1 : cursor
    correctionId = String(id)
  }

  function cancelCorrection() {
    if (pending !== "") return
    correctionId = ""
    errorText = ""
  }

  function saveCorrection(text, original) {
    if (correctionId === "" || pending !== "") return
    // Nothing changed: leave the dictation unmarked.
    if (String(text) === String(original)) { cancelCorrection(); return }
    requestAction("correct", ["correct", correctionId, "--", String(text)])
  }

  function requestAction(kind, args) {
    if (!client || pending !== "") return
    errorText = ""
    pendingKind = kind
    pending = "history-action-" + (++sequence)
    client.historyRequest(pending, args)
  }

  function openMeeting(id) {
    if (!client || pending !== "") return
    errorText = ""
    pendingKind = "open-meeting"
    pending = "history-meeting-" + (++sequence)
    client.meetingRequest(pending, String(id))
  }

  function seconds(ms) {
    var n = Number(ms)
    if (!(n > 0)) return ""
    return n < 60000 ? (n / 1000).toFixed(1) + " s"
      : Math.floor(n / 60000) + " min " + Math.round((n % 60000) / 1000) + " s"
  }

  function joined(parts) {
    var kept = []
    for (var i = 0; i < parts.length; ++i)
      if (parts[i] !== undefined && parts[i] !== null && String(parts[i]) !== "") kept.push(String(parts[i]))
    return kept.join("  ·  ")
  }

  // The expanded view of one dictation, as label and value pairs.
  function detailRows(serialized) {
    var entry = JSON.parse(serialized)
    var out = []
    var raw = String(entry.raw_text || "")
    var text = String(entry.text || "")
    var corrected = entry.corrected_text !== undefined && entry.corrected_text !== null
    out.push({ k: "Raw:", v: raw === "" ? "(nothing heard)" : raw, mono: false })
    if (text !== raw || corrected) out.push({ k: "Output:", v: text, mono: false })
    if (corrected) out.push({ k: "Correction:", v: String(entry.corrected_text) === "" ? "(empty)" : String(entry.corrected_text), mono: false })
    var edits = entry.edits || []
    if (edits.length > 0) {
      var diff = []
      for (var i = 0; i < edits.length; ++i) {
        if (String(edits[i].from || "") !== "") diff.push("- " + edits[i].from)
        if (String(edits[i].to || "") !== "") diff.push("+ " + edits[i].to)
      }
      out.push({ k: "Automatic edits:", v: diff.join("\n"), mono: true })
    }
    out.push({ k: "Engine:", v: joined([entry.engine, entry.model, entry.variant, entry.backend, entry.language]), mono: false })
    var timing = joined([
      seconds(entry.audio_ms) === "" ? "" : seconds(entry.audio_ms) + " of audio",
      entry.transcribe_ms === undefined || entry.transcribe_ms === null ? "" : entry.transcribe_ms + " ms to transcribe",
      entry.model_load_ms ? entry.model_load_ms + " ms to load the model" : "",
      entry.streaming ? "streaming" : ""])
    if (timing !== "") out.push({ k: "Timing:", v: timing, mono: false })
    var session = joined([
      entry.trigger ? "started by " + entry.trigger : "",
      entry.ended_by ? "ended by " + entry.ended_by : "",
      entry.profile ? "profile " + entry.profile : "",
      entry.input_device ? "mic " + entry.input_device : "",
      entry.auto_submit ? "auto-submit" : ""])
    if (session !== "") out.push({ k: "Session:", v: session, mono: false })
    var result = joined([entry.outcome, entry.error])
    if (result !== "") out.push({ k: "Result:", v: result, mono: false })
    out.push({ k: "ID:", v: String(entry.id), mono: true })
    return out
  }

  function dayLabel(day) {
    var date = new Date(day + "T00:00:00")
    if (isNaN(date.getTime())) return day
    var today = new Date()
    today.setHours(0, 0, 0, 0)
    var days = Math.round((today.getTime() - date.getTime()) / 86400000)
    var name = date.toLocaleDateString(Qt.locale(), "dddd, MMMM d")
    return days === 0 ? "Today  ·  " + name : days === 1 ? "Yesterday  ·  " + name : name
  }

  function applyState(value) {
    historyEnabled = !!value.enabled
    historyPaused = !!value.paused
    if (value.retention_days !== undefined && value.retention_days !== null)
      retentionDays = Number(value.retention_days)
    stateKnown = true
    stateText = !historyEnabled ? "History off" : historyPaused ? "History paused" : "History on"
  }

  function changeState(action) {
    requestAction(action, [action])
  }

  function retentionLabel(days) {
    return days === 0 ? "forever" : days === 365 ? "1 year" : days === 1 ? "1 day" : days + " days"
  }

  function refresh() {
    if (!client) return
    listPending = "history-list-" + (++sequence)
    client.historyRequest(listPending, query === "" ? ["list"] : ["search", query])
  }

  // Nothing the user is in the middle of may be disturbed by a background
  // refresh: a correction being typed, a confirmation, a running action.
  function idle() {
    return listPending === "" && pending === "" && correctionId === ""
      && !pasteDialog.visible && !deleteDialog.visible
  }

  // Bring the list model to `items` without rebuilding rows that are still
  // there, so the view keeps its place and open rows stay open.
  function syncRows(items) {
    var seen = {}
    var duplicate = false
    for (var i = 0; i < items.length; ++i) {
      if (seen[items[i].entryId]) duplicate = true
      seen[items[i].entryId] = true
    }
    // Search can return one meeting twice; those lists are simply rebuilt.
    if (duplicate || root.renderedQuery !== root.query || root.query !== "") {
      rows.clear()
      for (var a = 0; a < items.length; ++a) rows.append(items[a])
      root.renderedQuery = root.query
      return
    }
    for (var r = rows.count - 1; r >= 0; --r)
      if (!seen[rows.get(r).entryId]) rows.remove(r)
    for (var n = 0; n < items.length; ++n) {
      var found = -1
      for (var k = n; k < rows.count; ++k)
        if (rows.get(k).entryId === items[n].entryId) { found = k; break }
      if (found === -1) rows.insert(n, items[n])
      else {
        if (found !== n) rows.move(found, n, 1)
        rows.set(n, items[n])
      }
    }
    root.renderedQuery = root.query
  }

  ListModel { id: rows }

  // New dictations appear on their own while the window is showing.
  Timer {
    interval: 2000
    repeat: true
    running: root.live && root.client !== null
    onTriggered: if (root.idle()) root.refresh()
  }
  Timer {
    id: searchDelay
    interval: 300
    onTriggered: { root.query = search.text.trim(); root.refresh() }
  }
  Timer {
    id: noticeFade
    interval: 3500
    onTriggered: root.noticeText = ""
  }
  onNoticeTextChanged: if (noticeText !== "") noticeFade.restart()

  Connections {
    target: root.client
    function onHistoryReply(token, ok, value, message) {
      var kind
      if (token === root.listPending) {
        kind = "rows"
        root.listPending = ""
      } else if (token === root.pending) {
        kind = root.pendingKind
        root.pending = ""
      } else return
      if (kind !== "rows") {
        if (kind === "open-meeting") {
          if (!ok) root.errorText = message || "Meeting launch did not complete"
          else root.noticeText = "Opening the meeting."
          return
        }
        if (kind === "paste") {
          if (!ok) {
            root.errorText = (value && value.error) || message || "Replay did not complete"
            root.pasteFailed()
          } else root.noticeText = "Sent again."
          return
        }
        if (kind === "copy") {
          if (!ok) root.errorText = (value && value.error) || message || "Copy did not complete"
          else root.noticeText = "Copied."
          return
        }
        if (kind === "correct") {
          if (!ok) {
            root.errorText = (value && value.error) || message || "Correction did not complete"
            return
          }
          root.correctionId = ""
          root.refresh()
          return
        }
        if (kind === "delete") {
          if (!ok) {
            root.errorText = (value && value.error) || message || "Deletion did not complete"
            return
          }
          deleteDialog.close()
          root.refresh()
          return
        }
        if (!ok || !value || value.enabled === undefined) {
          root.stateText = "History state unavailable"
          root.errorText = (value && value.error) || message || "History change did not complete"
          return
        }
        root.applyState(value)
        root.refresh()
        return
      }
      var entries = root.query === "" ? (value ? value.entries : null) : value
      if (!ok || !entries || typeof entries.length !== "number") {
        root.listErrorText = message || "Could not read history"
        return
      }
      root.listErrorText = ""
      if (root.query === "")
        root.applyState(value)
      var items = []
      for (var i = 0; i < entries.length; ++i) {
        var hit = entries[i]
        if (hit.source === "meeting") {
          items.push({
            entryId: String(hit.meeting_id),
            day: String(hit.date).substring(0, 10),
            time: "Meeting",
            app: String(hit.title || "Meeting"),
            modelName: "meeting",
            length: "",
            failedMark: "",
            correctedMark: "",
            transcript: String(hit.snippet_plain || ""),
            detailsJson: "{}"
          })
          continue
        }
        var entry = hit.entry || hit
        var corrected = entry.corrected_text !== undefined && entry.corrected_text !== null
        items.push({
          entryId: String(entry.id),
          day: String(entry.ts_start).substring(0, 10),
          time: String(entry.ts_start).substring(11, 16),
          app: String(entry.app || "unknown app"),
          modelName: String(entry.model || "unknown model"),
          length: root.seconds(entry.audio_ms),
          failedMark: entry.delivered ? "" : "[failed]",
          correctedMark: corrected ? "[corrected]" : "",
          transcript: root.bestText(entry),
          detailsJson: JSON.stringify(entry)
        })
      }
      var atTop = list.atYBeginning
      root.syncRows(items)
      if (atTop) Qt.callLater(list.positionViewAtBeginning)
    }
  }

  Control {
   id: surface
   anchors.fill: parent
   padding: 16
   font.family: root.fontFamily
   background: null
   contentItem: ColumnLayout {
    spacing: 10

    RowLayout {
      Layout.fillWidth: true
      spacing: 10
      Label {
        text: "Voxtype"
        textFormat: Text.PlainText
        color: root.accent
        font.pixelSize: 20
        font.bold: true
      }
      Label {
        text: "History"
        textFormat: Text.PlainText
        color: root.fg
        font.pixelSize: 20
      }
      Rectangle {
        visible: root.stateKnown
        radius: 9
        implicitHeight: 18
        implicitWidth: stateChip.implicitWidth + 16
        color: "transparent"
        border.width: 1
        border.color: stateChip.color
        Label {
          id: stateChip
          anchors.centerIn: parent
          text: !root.historyEnabled ? "Off" : root.historyPaused ? "Paused" : "On"
          textFormat: Text.PlainText
          font.pixelSize: 11
          color: !root.historyEnabled ? root.dim : root.historyPaused ? root.fg : root.accent
        }
      }
      TextField {
        id: search
        objectName: "history-search"
        Layout.fillWidth: true
        Layout.leftMargin: 6
        implicitHeight: 28
        font.pixelSize: 13
        color: root.fg
        placeholderText: "Search dictations and meetings"
        placeholderTextColor: root.faint
        selectByMouse: true
        leftPadding: 10
        rightPadding: 10
        background: Rectangle {
          radius: 6
          color: root.field
          border.width: 1
          border.color: search.activeFocus ? root.accent : root.line
        }
        onTextEdited: searchDelay.restart()
        onAccepted: { searchDelay.stop(); root.query = text.trim(); root.refresh() }
        Keys.onEscapePressed: { text = ""; searchDelay.stop(); root.query = ""; root.refresh() }
      }
      Chip {
        id: retentionChip
        objectName: "history-retention"
        outlined: true
        visible: root.stateKnown
        enabled: root.pending === ""
        text: "Keep " + root.retentionLabel(root.retentionDays) + "  ▾"
        onClicked: retentionMenu.popup(retentionChip, 0, retentionChip.height + 2)
        Menu {
          id: retentionMenu
          font.family: root.fontFamily
          palette.window: root.raised
          palette.windowText: root.fg
          palette.text: root.fg
          palette.base: root.raised
          palette.button: root.raised
          palette.buttonText: root.fg
          palette.dark: root.fg
          palette.mid: root.line
          palette.midlight: root.tint(0.12)
          palette.light: root.tint(0.12)
          palette.highlight: root.tint(0.16)
          palette.highlightedText: root.fg
          MenuItem { enabled: false; text: "Keep dictations for" }
          Repeater {
            model: root.retentionChoices
            delegate: MenuItem {
              required property int modelData
              objectName: "history-retention-" + modelData
              text: (root.retentionDays === modelData ? "●  " : "     ") + root.retentionLabel(modelData)
              onTriggered: if (root.retentionDays !== modelData) root.requestAction("retention", ["retention", String(modelData)])
            }
          }
        }
      }
      Chip {
        objectName: "history-pause"
        outlined: true
        visible: root.historyEnabled
        enabled: root.pending === ""
        text: root.historyPaused ? "Resume" : "Pause"
        onClicked: root.changeState(root.historyPaused ? "resume" : "pause")
      }
      Chip {
        objectName: "history-enabled"
        outlined: true
        enabled: root.pending === ""
        text: root.historyEnabled ? "Turn off" : "Turn on"
        onClicked: root.changeState(root.historyEnabled ? "disable" : "enable")
      }
      Chip {
        objectName: "history-close"
        text: "✕"
        font.pixelSize: 15
        leftPadding: 7
        rightPadding: 7
        onClicked: root.closeRequested()
      }
    }
    Label {
      Layout.fillWidth: true
      visible: (root.errorText !== "" && root.correctionId === "") || root.listErrorText !== ""
      text: root.errorText || root.listErrorText
      textFormat: Text.PlainText
      wrapMode: Text.Wrap
      color: root.bad
      font.pixelSize: 13
    }
    Label {
      Layout.fillWidth: true
      visible: root.noticeText !== ""
      text: root.noticeText
      textFormat: Text.PlainText
      wrapMode: Text.Wrap
      color: root.accent
      font.pixelSize: 13
    }
    Label {
      visible: rows.count === 0 && root.listPending === "" && root.listErrorText === ""
      text: root.query === "" ? "No dictations stored" : "No matches"
      color: root.dim
      font.pixelSize: 13
    }
    ListView {
      id: list
      Layout.fillWidth: true
      Layout.fillHeight: true
      model: rows
      clip: true
      spacing: 3
      boundsBehavior: Flickable.StopAtBounds
      section.property: "day"
      section.delegate: Label {
        required property string section
        text: root.dayLabel(section)
        textFormat: Text.PlainText
        color: root.accent
        font.pixelSize: 12
        font.bold: true
        topPadding: 10
        bottomPadding: 4
        leftPadding: 4
      }
      delegate: Rectangle {
        id: entryCard
        required property var model
        readonly property string entryId: model.entryId
        readonly property string transcript: model.transcript
        readonly property bool isMeeting: model.detailsJson === "{}"
        readonly property bool editing: !isMeeting && root.correctionId === entryId
        readonly property bool expanded: !isMeeting && root.expandedId === entryId
        width: list.width
        height: content.implicitHeight + 12
        radius: 6
        color: hover.hovered || editing || expanded ? root.cardHover : root.card
        border.width: editing ? 1 : 0
        border.color: root.accent

        onTranscriptChanged: if (!editing) body.text = transcript
        onEditingChanged: {
          if (editing) {
            var caret = root.correctionCursor < 0 ? body.length
              : Math.min(root.correctionCursor, body.length)
            body.forceActiveFocus()
            // Focus and the read-only switch both move the caret, so place it last.
            Qt.callLater(function() { if (entryCard.editing) body.cursorPosition = caret })
          } else {
            body.deselect()
            body.text = transcript
          }
        }
        HoverHandler { id: hover }

        RowLayout {
          id: content
          anchors.left: parent.left
          anchors.right: parent.right
          anchors.top: parent.top
          anchors.leftMargin: 10
          anchors.rightMargin: 6
          anchors.topMargin: 6
          spacing: 8

          Label {
            Layout.alignment: Qt.AlignTop
            Layout.topMargin: 5
            Layout.preferredWidth: 52
            text: entryCard.model.time
            textFormat: Text.PlainText
            color: root.dim
            font.pixelSize: 12
          }
          ColumnLayout {
            Layout.fillWidth: true
            spacing: 1
            RowLayout {
              Layout.fillWidth: true
              spacing: 2
              TextArea {
                id: body
                objectName: entryCard.editing ? "history-correction-text" : "history-text-" + entryCard.entryId
                Layout.fillWidth: true
                readOnly: !entryCard.editing
                wrapMode: TextEdit.Wrap
                textFormat: TextEdit.PlainText
                selectByMouse: entryCard.editing
                color: root.fg
                font.pixelSize: 15
                placeholderText: entryCard.model.failedMark !== "" ? "No text" : ""
                placeholderTextColor: root.faint
                topPadding: 3
                bottomPadding: 3
                leftPadding: 4
                rightPadding: 4
                background: Rectangle {
                  radius: 4
                  color: entryCard.editing ? root.field : "transparent"
                }
                Component.onCompleted: text = entryCard.transcript
                Keys.onPressed: function(event) {
                  if (!entryCard.editing) return
                  if (event.key === Qt.Key_Escape) {
                    root.cancelCorrection()
                    event.accepted = true
                  } else if ((event.key === Qt.Key_Return || event.key === Qt.Key_Enter)
                      && !(event.modifiers & Qt.ShiftModifier)) {
                    root.saveCorrection(body.text, entryCard.transcript)
                    event.accepted = true
                  }
                }
                // Click the words to correct them, with the caret where you clicked.
                MouseArea {
                  anchors.fill: parent
                  enabled: !entryCard.editing && !entryCard.isMeeting
                  visible: enabled
                  cursorShape: Qt.IBeamCursor
                  onClicked: function(mouse) {
                    root.beginCorrection(entryCard.entryId, body.positionAt(mouse.x, mouse.y))
                  }
                }
              }
              Row {
                Layout.alignment: Qt.AlignTop
                spacing: 0
                opacity: hover.hovered || entryCard.editing || entryCard.expanded ? 1 : 0.45
                Chip {
                  objectName: "history-meeting-" + entryCard.entryId
                  visible: entryCard.isMeeting
                  enabled: root.pending === ""
                  text: "Open meeting"
                  onClicked: root.openMeeting(entryCard.entryId)
                }
                Chip {
                  objectName: "history-correct-" + entryCard.entryId
                  visible: !entryCard.isMeeting && !entryCard.editing
                  enabled: root.pending === ""
                  text: "✎ Edit"
                  onClicked: root.beginCorrection(entryCard.entryId, -1)
                }
                Chip {
                  objectName: "history-copy-" + entryCard.entryId
                  visible: !entryCard.isMeeting && !entryCard.editing
                  enabled: root.pending === "" && entryCard.transcript !== ""
                  text: "Copy"
                  onClicked: root.requestAction("copy", ["copy", entryCard.entryId])
                }
                Chip {
                  objectName: "history-paste-" + entryCard.entryId
                  visible: !entryCard.isMeeting && !entryCard.editing
                  enabled: root.pending === "" && entryCard.transcript !== ""
                  text: "Paste again"
                  onClicked: { root.pasteId = entryCard.entryId; root.errorText = ""; pasteDialog.open() }
                }
                Chip {
                  objectName: "history-details-" + entryCard.entryId
                  visible: !entryCard.isMeeting && !entryCard.editing
                  text: entryCard.expanded ? "Hide details" : "Details"
                  onClicked: root.expandedId = entryCard.expanded ? "" : entryCard.entryId
                }
                Chip {
                  objectName: "history-delete-" + entryCard.entryId
                  visible: !entryCard.isMeeting && !entryCard.editing
                  enabled: root.pending === ""
                  danger: true
                  text: "Delete"
                  onClicked: { root.deleteId = entryCard.entryId; root.errorText = ""; deleteDialog.open() }
                }
                Chip {
                  objectName: "history-correction-cancel"
                  visible: entryCard.editing
                  enabled: root.pending === ""
                  text: "Cancel"
                  onClicked: root.cancelCorrection()
                }
                Chip {
                  objectName: "history-correction-save"
                  visible: entryCard.editing
                  enabled: root.pending === ""
                  outlined: true
                  text: "Save"
                  onClicked: root.saveCorrection(body.text, entryCard.transcript)
                }
              }
            }
            Label {
              Layout.fillWidth: true
              Layout.leftMargin: 4
              visible: entryCard.editing
              text: root.errorText !== "" ? root.errorText
                : "Enter saves  ·  Esc cancels  ·  Shift+Enter starts a new line. The original wording is kept."
              textFormat: Text.PlainText
              wrapMode: Text.Wrap
              color: root.errorText !== "" ? root.bad : root.dim
              font.pixelSize: 11
            }
            RowLayout {
              Layout.fillWidth: true
              Layout.leftMargin: 4
              spacing: 8
              Label {
                Layout.fillWidth: true
                text: entryCard.isMeeting ? entryCard.model.app
                  : root.joined([entryCard.model.app, entryCard.model.modelName, entryCard.model.length])
                textFormat: Text.PlainText
                elide: Text.ElideRight
                color: root.dim
                font.pixelSize: 11
              }
              Label {
                visible: text !== ""
                text: entryCard.model.failedMark
                textFormat: Text.PlainText
                color: root.bad
                font.pixelSize: 11
              }
              Label {
                visible: text !== ""
                text: entryCard.model.correctedMark
                textFormat: Text.PlainText
                color: root.accent
                font.pixelSize: 11
              }
            }
            ColumnLayout {
              Layout.fillWidth: true
              Layout.leftMargin: 4
              Layout.topMargin: 6
              Layout.bottomMargin: 4
              visible: entryCard.expanded
              spacing: 3
              Repeater {
                model: entryCard.expanded ? root.detailRows(entryCard.model.detailsJson) : []
                delegate: RowLayout {
                  id: detailRow
                  required property var modelData
                  Layout.fillWidth: true
                  spacing: 8
                  Label {
                    Layout.alignment: Qt.AlignTop
                    Layout.preferredWidth: 112
                    text: detailRow.modelData.k
                    textFormat: Text.PlainText
                    color: root.dim
                    font.pixelSize: 12
                  }
                  Label {
                    Layout.fillWidth: true
                    text: detailRow.modelData.v
                    textFormat: Text.PlainText
                    wrapMode: Text.Wrap
                    color: root.tint(0.86)
                    font.pixelSize: 12
                    font.family: detailRow.modelData.mono ? "monospace" : root.fontFamily
                  }
                }
              }
            }
          }
        }
      }
      ScrollBar.vertical: ScrollBar {}
    }
   }
  }
  Component.onCompleted: refresh()
  Dialog {
    id: pasteDialog
    title: "Send this transcript again?"
    modal: true
    anchors.centerIn: parent
    width: Math.min(root.width - 32, 560)
    font.family: root.fontFamily
    palette.window: root.raised
    palette.windowText: root.fg
    palette.text: root.fg
    palette.base: root.raised
    palette.button: root.tint(0.16)
    palette.buttonText: root.fg
    palette.dark: root.fg
    palette.mid: root.line
    palette.light: root.tint(0.12)
    contentItem: ColumnLayout {
      Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        text: "Continue hides history. Switch to the target application: the transcript is sent after 1.5 seconds, using your current output settings."
      }
      RowLayout {
        Button { text: "Cancel"; onClicked: pasteDialog.close() }
        Button {
          objectName: "history-paste-confirm"
          text: "Continue"
          onClicked: {
            pasteDialog.close()
            root.pasteRequested()
            root.requestAction("paste", ["paste", root.pasteId, "--delay-ms", "1500"])
          }
        }
      }
    }
  }
  Dialog {
    id: deleteDialog
    title: "Delete this dictation?"
    modal: true
    anchors.centerIn: parent
    width: Math.min(root.width - 32, 560)
    palette: pasteDialog.palette
    font.family: root.fontFamily
    closePolicy: root.pending === "" ? Popup.CloseOnEscape : Popup.NoAutoClose
    contentItem: ColumnLayout {
      Label {
        Layout.fillWidth: true
        wrapMode: Text.Wrap
        text: "Permanently remove this dictation from its day file and the shared search index. This cannot be undone."
      }
      Label {
        Layout.fillWidth: true
        visible: root.errorText !== ""
        text: root.errorText
        textFormat: Text.PlainText
        wrapMode: Text.Wrap
        color: root.bad
      }
      RowLayout {
        Button {
          objectName: "history-delete-cancel"
          text: "Cancel"
          enabled: root.pending === ""
          onClicked: deleteDialog.close()
        }
        Button {
          objectName: "history-delete-confirm"
          text: "Delete permanently"
          enabled: root.pending === ""
          onClicked: root.requestAction("delete", ["delete", root.deleteId])
        }
      }
    }
  }
}
