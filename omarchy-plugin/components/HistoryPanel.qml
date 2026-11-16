import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Shared normal-window content. The host supplies the canonical CLI transport;
// the component never opens a microphone, compositor socket or history file.
Rectangle {
  id: root
  color: "#151820"
  property var client: null
  property string stateText: "Loading history"
  readonly property int entryCount: rows.count
  property string errorText: ""
  property string listErrorText: ""
  property string noticeText: ""
  property int sequence: 0
  property string pending: ""
  property string listPending: ""
  property string query: ""
  property string pendingKind: "rows"
  property bool historyEnabled: false
  property bool historyPaused: false
  property string expandedId: ""
  property string correctionId: ""
  property string deleteId: ""
  property string pasteId: ""
  signal pasteRequested()
  signal pasteFailed()

  function bestText(entry) {
    if (entry.corrected_text !== undefined && entry.corrected_text !== null)
      return String(entry.corrected_text)
    return String(entry.text || entry.raw_text || "")
  }

  function beginCorrection(serialized) {
    var entry = JSON.parse(serialized)
    correctionId = String(entry.id)
    correctionText.text = root.bestText(entry)
    errorText = ""
    correctionDialog.open()
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

  function detailText(serialized) {
    var entry = JSON.parse(serialized)
    var diff = ""
    var edits = entry.edits || []
    for (var i = 0; i < edits.length; ++i)
      diff += "- " + edits[i].from + "\n+ " + edits[i].to + "\n"
    return "Raw:\n" + (entry.raw_text || "") + "\n\nOutput:\n" + (entry.text || "")
      + "\n\nCorrection:\n" + (entry.corrected_text === undefined || entry.corrected_text === null ? "(none)" : entry.corrected_text)
      + "\n\nAutomatic edits:\n" + (diff || "(none)")
      + "\nMetadata:\n" + JSON.stringify({id:entry.id, engine:entry.engine, model:entry.model, backend:entry.backend, variant:entry.variant, language:entry.language, input_device:entry.input_device, audio_ms:entry.audio_ms, transcribe_ms:entry.transcribe_ms, model_load_ms:entry.model_load_ms, streaming:entry.streaming, trigger:entry.trigger, ended_by:entry.ended_by, profile:entry.profile, auto_submit:entry.auto_submit, outcome:entry.outcome, error:entry.error}, null, 2)
  }

  function applyState(value) {
    historyEnabled = !!value.enabled
    historyPaused = !!value.paused
    stateText = !historyEnabled ? "History off" : historyPaused ? "History paused" : "History on"
  }

  function changeState(action) {
    requestAction(action, [action])
  }

  function refresh() {
    if (!client) return
    listPending = "history-list-" + (++sequence)
    client.historyRequest(listPending, query === "" ? ["list"] : ["search", query])
  }

  ListModel { id: rows }
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
          else root.noticeText = "Meeting launch requested."
          return
        }
        if (kind === "paste") {
          if (!ok) {
            root.errorText = (value && value.error) || message || "Replay did not complete"
            root.pasteFailed()
          } else root.noticeText = "Sent transcript again."
          return
        }
        if (kind === "copy") {
          if (!ok) root.errorText = (value && value.error) || message || "Copy did not complete"
          else root.noticeText = "Copied transcript."
          return
        }
        if (kind === "correct") {
          if (!ok) {
            root.errorText = (value && value.error) || message || "Correction did not complete"
            return
          }
          correctionDialog.close()
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
      rows.clear()
      for (var i = 0; i < entries.length; ++i) {
        var hit = entries[i]
        if (hit.source === "meeting") {
          rows.append({entryId:String(hit.meeting_id), day:String(hit.date).substring(0,10), time:"Meeting", app:String(hit.title || "Meeting"), modelName:"meeting", markers:"", transcript:String(hit.snippet_plain || ""), detailsJson:"{}"})
          continue
        }
        var entry = hit.entry || hit
        var corrected = entry.corrected_text !== undefined && entry.corrected_text !== null
        rows.append({
          entryId: String(entry.id),
          day: String(entry.ts_start).substring(0, 10),
          time: String(entry.ts_start).substring(11, 19),
          app: String(entry.app || "unknown app"),
          modelName: String(entry.model || "unknown model"),
          markers: (entry.delivered ? "" : "[failed] ") + (corrected ? "[corrected]" : ""),
          transcript: root.bestText(entry),
          detailsJson: JSON.stringify(entry)
        })
      }
    }
  }

  ColumnLayout {
    anchors.fill: parent
    anchors.margins: 20
    spacing: 14
    RowLayout {
      Layout.fillWidth: true
      Label {
        Layout.fillWidth: true
        text: root.stateText
        textFormat: Text.PlainText
        color: "#eceff4"
        font.pixelSize: 24
      }
      Button {
        objectName: "history-pause"
        visible: root.historyEnabled
        enabled: root.pending === ""
        text: root.historyPaused ? "Resume history" : "Pause history"
        onClicked: root.changeState(root.historyPaused ? "resume" : "pause")
      }
      Button {
        objectName: "history-enabled"
        enabled: root.pending === ""
        text: root.historyEnabled ? "Disable history" : "Enable history"
        onClicked: root.changeState(root.historyEnabled ? "disable" : "enable")
      }
    }
    RowLayout {
      Layout.fillWidth: true
      TextField {
        id: search
        objectName: "history-search"
        Layout.fillWidth: true
        placeholderText: "Search dictations and meetings"
        onAccepted: { root.query = text.trim(); root.refresh() }
      }
      Button {
        objectName: "history-search-submit"
        text: "Search"
        onClicked: { root.query = search.text.trim(); root.refresh() }
      }
    }
    Label {
      Layout.fillWidth: true
      visible: root.errorText !== "" || root.listErrorText !== ""
      text: root.errorText || root.listErrorText
      textFormat: Text.PlainText
      wrapMode: Text.Wrap
      color: "#ffb4ab"
    }
    Label {
      Layout.fillWidth: true
      visible: root.noticeText !== ""
      text: root.noticeText
      textFormat: Text.PlainText
      wrapMode: Text.Wrap
      color: "#85c7d0"
    }
    Label {
      visible: rows.count === 0 && root.listPending === "" && root.listErrorText === ""
      text: root.query === "" ? "No dictations stored" : "No matches"
      color: "#c0c6d4"
    }
    ListView {
      id: list
      Layout.fillWidth: true
      Layout.fillHeight: true
      model: rows
      clip: true
      spacing: 8
      section.property: "day"
      section.delegate: Label {
        required property string section
        text: section
        color: "#85c7d0"
        font.pixelSize: 18
        padding: 8
      }
      delegate: Rectangle {
        required property var model
        width: list.width
        height: content.implicitHeight + 24
        radius: 6
        color: "#222733"
        ColumnLayout {
          id: content
          anchors.left: parent.left
          anchors.right: parent.right
          anchors.top: parent.top
          anchors.margins: 12
          Label {
            Layout.fillWidth: true
            text: model.time + "  " + model.markers
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            color: "#e7cb80"
          }
          Label {
            Layout.fillWidth: true
            text: model.app + "  |  " + model.modelName
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            color: "#c0c6d4"
          }
          Label {
            Layout.fillWidth: true
            text: model.transcript
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            color: "#eceff4"
          }
          Button {
            objectName: "history-meeting-" + model.entryId
            visible: model.detailsJson === "{}"
            enabled: root.pending === ""
            text: "Open meeting"
            onClicked: root.openMeeting(model.entryId)
          }
          Button {
            objectName: "history-details-" + model.entryId
            visible: model.detailsJson !== "{}"
            text: root.expandedId === model.entryId ? "Hide details" : "Details and automatic edits"
            onClicked: root.expandedId = root.expandedId === model.entryId ? "" : model.entryId
          }
          Button {
            objectName: "history-correct-" + model.entryId
            visible: model.detailsJson !== "{}"
            enabled: root.pending === ""
            text: "Correct transcript"
            onClicked: root.beginCorrection(model.detailsJson)
          }
          Button {
            objectName: "history-delete-" + model.entryId
            visible: model.detailsJson !== "{}"
            enabled: root.pending === ""
            text: "Delete"
            onClicked: { root.deleteId = model.entryId; root.errorText = ""; deleteDialog.open() }
          }
          Button {
            objectName: "history-copy-" + model.entryId
            visible: model.detailsJson !== "{}"
            enabled: root.pending === "" && model.transcript !== ""
            text: "Copy"
            onClicked: root.requestAction("copy", ["copy", model.entryId])
          }
          Button {
            objectName: "history-paste-" + model.entryId
            visible: model.detailsJson !== "{}"
            enabled: root.pending === "" && model.transcript !== ""
            text: "Paste again"
            onClicked: { root.pasteId = model.entryId; root.errorText = ""; pasteDialog.open() }
          }
          Label {
            Layout.fillWidth: true
            visible: root.expandedId === model.entryId
            text: visible ? root.detailText(model.detailsJson) : ""
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            font.family: "monospace"
            color: "#eceff4"
          }
        }
      }
      ScrollBar.vertical: ScrollBar {}
    }
  }
  Component.onCompleted: refresh()
  Dialog {
    id: pasteDialog
    title: "Send this transcript again?"
    modal: true
    anchors.centerIn: parent
    width: Math.min(root.width - 32, 560)
    palette: correctionDialog.palette
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
    palette: correctionDialog.palette
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
        color: "#ffb4ab"
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
  Dialog {
    id: correctionDialog
    title: "Correct transcript"
    modal: true
    anchors.centerIn: parent
    width: Math.min(root.width - 32, 680)
    closePolicy: root.pending === "" ? Popup.CloseOnEscape : Popup.NoAutoClose
    palette.window: "#222733"
    palette.windowText: "#eceff4"
    palette.text: "#eceff4"
    palette.base: "#151820"
    palette.button: "#333b4a"
    palette.buttonText: "#eceff4"
    contentItem: ColumnLayout {
      Label {
        text: "Original engine and processed text are preserved. An empty correction is valid."
        Layout.fillWidth: true
        wrapMode: Text.Wrap
      }
      ScrollView {
        Layout.fillWidth: true
        Layout.preferredHeight: 180
        TextArea {
          id: correctionText
          objectName: "history-correction-text"
          wrapMode: TextEdit.Wrap
          selectByMouse: true
        }
      }
      Label {
        Layout.fillWidth: true
        visible: root.errorText !== ""
        text: root.errorText
        textFormat: Text.PlainText
        wrapMode: Text.Wrap
        color: "#ffb4ab"
      }
      RowLayout {
        Button {
          objectName: "history-correction-cancel"
          text: "Cancel"
          enabled: root.pending === ""
          onClicked: correctionDialog.close()
        }
        Button {
          objectName: "history-correction-save"
          text: "Save correction"
          enabled: root.pending === ""
          onClicked: root.requestAction("correct", ["correct", root.correctionId, "--", correctionText.text])
        }
      }
    }
  }
}
