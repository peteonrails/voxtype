import QtQuick
import QtQuick.Layouts

// The meeting view: a header with the meeting's state and controls, then four
// tabs: Transcript, Notes, Ask, and Summary (once the meeting has stopped).
// Everything shown comes from `feed` (MeetingFeed) and `assistant`
// (MeetingAssistant); everything done goes out as a signal, which Meetings.qml
// turns into a `voxtype meeting` command.
//
// A meeting that has ended stays on screen ("Ended") so its lines can still be
// fixed: chip renames, per-line reassignment and notes work the same as during
// it. Tabs keep their own scroll position: switching never rebuilds one.
//
// Overlays (the reassign chooser, the note editor, the agent menu and the
// consent dialog) are plain items, not Popups: a layer-shell surface has no
// window overlay for a Popup to render into.
//
// No level meters: meeting.sock carries no levels yet. No countdown either: it
// carries no chunk timing, so between updates the view says "Listening" and
// how long ago the last lines arrived, rather than guessing at a boundary.
Item {
  id: root

  property var theme: MeetingTheme {}
  property var feed: null
  property var assistant: null
  // The action running now, from MeetingCli.busy.
  property string busy: ""
  property string actionError: ""
  property bool canOpenSettings: false
  // Past meetings for the idle screen (Meetings.qml reads them).
  property var pastMeetings: []
  property bool listLoading: false
  property string listError: ""
  property bool listHasMore: false

  signal pauseRequested()
  signal resumeRequested()
  signal stopRequested()
  signal startRequested(string title)
  signal renameRequested(var keys, string name)
  // Move one line: `speaker` goes to `voxtype meeting reassign`, `key` is the
  // speaker key to show until it answers.
  signal reassignRequested(int segId, string speaker, string key)
  signal restoreRequested(int segId)
  // A note on one line; an empty `text` removes it.
  signal annotateRequested(int segId, string text)
  signal settingsRequested()
  signal openMeetingRequested(string meetingId)
  signal moreMeetingsRequested()
  // From an ended or past meeting back to the list.
  signal backRequested()
  signal closeRequested()
  signal errorDismissed()

  readonly property string status: feed ? feed.status : ""
  readonly property bool connected: feed ? feed.connected : false
  readonly property bool live: feed ? feed.active : false
  readonly property bool ended: feed ? feed.ended : false
  // Something to show: a meeting running, or one that ended and is kept.
  readonly property bool hasMeeting: live || ended

  // ------------------------------------------------------------- tabs

  readonly property var tabs: root.ended ? ["transcript", "notes", "ask", "summary"] : ["transcript", "notes", "ask"]
  property string tab: "transcript"
  readonly property string currentTab: root.tabs.indexOf(root.tab) === -1 ? "transcript" : root.tab

  function selectTab(name) { root.tab = String(name) }

  function tabTitle(name) {
    return name === "transcript" ? "Transcript" : name === "notes" ? "Notes" : name === "ask" ? "Ask" : "Summary"
  }

  // A citation or chapter link: show that line of the transcript.
  function jumpTo(ms) {
    root.tab = "transcript"
    Qt.callLater(function() { transcriptPane.jumpTo(ms) })
  }

  // Ticks once a second while it matters, so elapsed and "ago" stay current.
  property real now: Date.now()
  Timer {
    interval: 1000
    repeat: true
    running: root.visible && root.live
    onTriggered: root.now = Date.now()
  }

  focus: true
  Keys.onEscapePressed: {
    if (consent.visible) consent.dismiss()
    else if (agentMenu.visible) agentMenu.close()
    else if (reassignPopup.visible) reassignPopup.close()
    else if (noteEditor.visible) noteEditor.close()
    else root.closeRequested()
  }

  // The title typed for a start is kept until the meeting is actually up, so
  // a refused start (meeting mode off, say) does not throw it away.
  onLiveChanged: {
    if (root.live) {
      titleField.text = ""
      endedTitleField.text = ""
    }
  }

  Connections {
    target: root.feed
    function onTransitioned(from, to) {
      root.now = Date.now()
      // A new meeting opens on its transcript.
      if (to === "recording" && from !== "paused") root.tab = "transcript"
      // Its summary is what a stopped meeting is for.
      if (to === "summarizing") root.tab = "summary"
    }
  }

  Connections {
    target: root.assistant
    function onConsentNeeded() { consent.visible = true }
  }
  onAssistantChanged: if (root.assistant && root.assistant.pending !== null) consent.visible = true

  // chrono writes nanoseconds; JavaScript dates take milliseconds at most.
  function parseTime(iso) {
    if (!iso) return NaN
    return Date.parse(String(iso).replace(/(\.\d{3})\d+/, "$1"))
  }

  function clock(ms) {
    var total = Math.max(0, Math.floor(ms / 1000))
    var h = Math.floor(total / 3600)
    var m = Math.floor((total % 3600) / 60)
    var s = total % 60
    var mm = (h > 0 && m < 10 ? "0" : "") + m
    return (h > 0 ? h + ":" : "") + mm + ":" + (s < 10 ? "0" : "") + s
  }

  readonly property string elapsedText: {
    var started = root.parseTime(root.feed ? root.feed.startedAt : "")
    if (isNaN(started)) return ""
    return root.clock((root.ended ? root.feed.endedAt : root.now) - started)
  }

  // Today's meeting needs only the time; an older one its date too.
  readonly property string endedAtText: {
    if (!root.ended || root.feed.endedAt <= 0) return ""
    var end = new Date(root.feed.endedAt)
    if (end.toDateString() === new Date(root.now).toDateString()) return "ended at " + Qt.formatTime(end, "hh:mm")
    var start = root.parseTime(root.feed.startedAt)
    return Qt.formatDateTime(isNaN(start) ? end : new Date(start), "ddd d MMM yyyy, hh:mm")
  }

  readonly property string stateLabel: {
    if (root.busy === "pause") return "Pausing…"
    if (root.busy === "resume") return "Resuming…"
    if (root.busy === "stop") return "Stopping…"
    if (root.status === "recording") return "Recording"
    if (root.status === "paused") return "Paused"
    if (root.status === "finalizing") return "Finalizing"
    if (root.status === "summarizing") return "Summarizing"
    if (root.ended) return "Ended"
    return "Idle"
  }

  readonly property string listeningText: {
    if (root.ended) return "Click a line's speaker to move it, or a chip to rename a speaker."
    if (!root.feed || !root.connected) return ""
    if (root.status === "paused") return "Paused. Nothing is being recorded."
    if (root.status === "finalizing") return "Transcribing the last of the audio…"
    if (root.status !== "recording") return ""
    var last = root.feed.lastSegmentsAt
    if (last <= 0) return root.feed.segments.count > 0 ? "Listening…" : "Listening for the first chunk…"
    var ago = Math.max(0, Math.round((root.now - last) / 1000))
    return "Listening… last update " + (ago < 5 ? "just now" : ago + " s ago") + "."
  }

  // ------------------------------------------------------- reassignment

  function openReassign(line, anchor) {
    reassignPopup.segId = line.segId
    reassignPopup.speaker = line.speaker
    reassignPopup.originalSpeaker = line.originalSpeaker
    reassignPopup.lineText = line.text
    newSpeakerField.text = ""
    root.place(reassignPopup, anchor)
    reassignPopup.visible = true
    // The keyboard is already this window's (the click gave it focus); taking
    // it here only makes Escape reach the chooser.
    reassignPopup.forceActiveFocus()
  }

  // A pick from the chooser: an existing chip, or a typed name that may be one.
  function chooseSpeaker(name, keys) {
    var segId = reassignPopup.segId
    var current = reassignPopup.speaker
    reassignPopup.close()
    if (keys.indexOf(current) !== -1) return
    root.reassignRequested(segId, name, keys[0])
  }

  function chooseNewSpeaker(name) {
    var wanted = String(name).trim()
    if (wanted === "") return
    var fold = wanted.toLowerCase()
    var chips = root.feed.chips
    for (var i = 0; i < chips.length; i++)
      if (chips[i].name.toLowerCase() === fold) return root.chooseSpeaker(chips[i].name, chips[i].keys)
    root.chooseSpeaker(wanted, [wanted])
  }

  function openNoteEditor(line, anchor) {
    noteEditor.segId = line.segId
    noteEditor.existing = line.annotation
    noteEditor.lineText = line.text
    noteField.text = line.annotation
    root.place(noteEditor, anchor)
    noteEditor.visible = true
    noteField.forceActiveFocus()
  }

  // Under `anchor`, or above it if there is no room.
  function place(popup, anchor) {
    var p = anchor.mapToItem(root, 0, 0)
    popup.anchorTop = p.y
    popup.anchorBottom = p.y + anchor.height
    popup.x = Math.max(root.theme.lg, Math.min(p.x - root.theme.lg, root.width - popup.width - root.theme.lg))
  }

  // ------------------------------------------------------------- surface

  MeetingSurface {
    id: card
    theme: root.theme
    anchors.fill: parent

    // Focus for Escape once the card is clicked.
    MouseArea { anchors.fill: parent; onClicked: root.forceActiveFocus() }

    ColumnLayout {
      anchors.fill: parent
      anchors.margins: root.theme.padding
      spacing: root.theme.xxl

      // ------------------------------------------------------- header

      RowLayout {
        Layout.fillWidth: true
        spacing: root.theme.gap

        MeetingButton {
          objectName: "backButton"
          theme: root.theme
          visible: root.ended
          text: "‹ Meetings"
          onClicked: root.backRequested()
        }

        Text {
          objectName: "meetingTitle"
          Layout.fillWidth: true
          text: root.hasMeeting
            ? (root.feed.title !== "" ? root.feed.title : "Untitled meeting")
            : "Meetings"
          color: root.theme.fg
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.heading
          font.bold: true
          elide: Text.ElideRight
        }

        MeetingButton {
          objectName: "closeButton"
          theme: root.theme
          text: "Close"
          onClicked: root.closeRequested()
        }
      }

      RowLayout {
        Layout.fillWidth: true
        visible: root.hasMeeting
        spacing: root.theme.gap

        Rectangle {
          id: stateDot
          implicitWidth: 9
          implicitHeight: 9
          radius: 4.5
          color: root.status === "recording" ? root.theme.urgent : root.theme.dim
          SequentialAnimation on opacity {
            running: root.status === "recording" && root.visible
            loops: Animation.Infinite
            NumberAnimation { to: 0.35; duration: 900 }
            NumberAnimation { to: 1.0; duration: 900 }
            onRunningChanged: if (!running) stateDot.opacity = 1
          }
        }

        Text {
          objectName: "meetingState"
          text: root.stateLabel
          color: root.theme.fg
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.body
        }

        Text {
          objectName: "meetingElapsed"
          text: root.elapsedText
          visible: text !== ""
          color: root.theme.dim
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.body
        }

        Text {
          objectName: "meetingEndedAt"
          text: root.endedAtText
          visible: text !== ""
          color: root.theme.dim
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.body
        }

        Item { Layout.fillWidth: true }

        MeetingButton {
          objectName: "pauseButton"
          theme: root.theme
          visible: root.live
          text: root.status === "paused" ? "Resume" : "Pause"
          bordered: true
          enabled: root.connected && root.busy === "" && (root.status === "recording" || root.status === "paused")
          onClicked: root.status === "paused" ? root.resumeRequested() : root.pauseRequested()
        }

        MeetingButton {
          objectName: "stopButton"
          theme: root.theme
          visible: root.live
          text: "Stop"
          bordered: true
          enabled: root.connected && root.busy === "" && (root.status === "recording" || root.status === "paused")
          onClicked: root.stopRequested()
        }
      }

      // Listening, paused or finalizing, between chunks.
      RowLayout {
        objectName: "listeningIndicator"
        Layout.fillWidth: true
        visible: root.live && root.connected || root.ended
        spacing: root.theme.md

        Row {
          spacing: root.theme.xs
          visible: root.status === "recording"
          Repeater {
            model: 3
            Rectangle {
              required property int index
              width: 4
              height: 4
              radius: 2
              color: root.theme.accent
              opacity: 0.3
              SequentialAnimation on opacity {
                running: root.status === "recording" && root.visible
                loops: Animation.Infinite
                PauseAnimation { duration: index * 220 }
                NumberAnimation { to: 1.0; duration: 380 }
                NumberAnimation { to: 0.3; duration: 380 }
                PauseAnimation { duration: (2 - index) * 220 }
              }
            }
          }
        }

        Text {
          objectName: "listeningText"
          Layout.fillWidth: true
          text: root.listeningText
          color: root.theme.dim
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.bodySmall
          elide: root.ended ? Text.ElideNone : Text.ElideRight
          wrapMode: root.ended ? Text.WordWrap : Text.NoWrap
        }
      }

      // Ended: a new meeting can start from here, replacing this one.
      RowLayout {
        objectName: "endedStartRow"
        Layout.fillWidth: true
        visible: root.ended
        spacing: root.theme.gap

        MeetingTextField {
          id: endedTitleField
          objectName: "endedTitleField"
          theme: root.theme
          Layout.fillWidth: true
          placeholderText: "New meeting title (optional)"
          enabled: root.connected
          onAccepted: endedStartButton.clicked()
        }

        MeetingButton {
          id: endedStartButton
          objectName: "endedStartButton"
          theme: root.theme
          text: root.busy === "start" ? "Starting…" : "Start meeting"
          bordered: true
          enabled: root.connected && root.busy === ""
          onClicked: {
            if (!endedStartButton.enabled) return
            root.startRequested(endedTitleField.text)
          }
        }
      }

      // ------------------------------------------------------- notices

      Text {
        objectName: "reconnectNotice"
        Layout.fillWidth: true
        visible: !root.connected && (root.live || root.feed && root.feed.everConnected) && !root.ended
        text: "Lost the connection to voxtype. Reconnecting…"
        color: root.theme.urgent
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.bodySmall
        wrapMode: Text.WordWrap
      }

      Rectangle {
        objectName: "actionError"
        Layout.fillWidth: true
        visible: root.actionError !== "" || (root.feed && root.feed.lastError !== "")
        implicitHeight: errorText.implicitHeight + root.theme.padX * 2
        radius: Math.min(root.theme.radius, 6)
        color: root.theme.alpha(root.theme.urgent, 0.12)
        border.width: 1
        border.color: root.theme.alpha(root.theme.urgent, 0.45)

        Text {
          id: errorText
          anchors.left: parent.left
          anchors.right: dismissError.left
          anchors.verticalCenter: parent.verticalCenter
          anchors.leftMargin: root.theme.padX
          anchors.rightMargin: root.theme.gap
          text: root.actionError !== "" ? root.actionError : (root.feed ? root.feed.lastError : "")
          color: root.theme.fg
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.bodySmall
          wrapMode: Text.WordWrap
        }

        MeetingButton {
          id: dismissError
          theme: root.theme
          anchors.right: parent.right
          anchors.rightMargin: root.theme.sm
          anchors.verticalCenter: parent.verticalCenter
          text: "Dismiss"
          onClicked: {
            if (root.feed) root.feed.lastError = ""
            root.errorDismissed()
          }
        }
      }

      // ------------------------------------------------------- tabs

      Row {
        objectName: "tabBar"
        Layout.fillWidth: true
        visible: root.hasMeeting
        spacing: root.theme.sm

        Repeater {
          model: root.tabs
          MeetingButton {
            required property string modelData
            objectName: "tab_" + modelData
            theme: root.theme
            text: root.tabTitle(modelData)
            active: root.currentTab === modelData
            onClicked: root.tab = modelData
          }
        }
      }

      StackLayout {
        objectName: "tabStack"
        Layout.fillWidth: true
        Layout.fillHeight: true
        visible: root.hasMeeting
        currentIndex: ["transcript", "notes", "ask", "summary"].indexOf(root.currentTab)

        TranscriptPane {
          id: transcriptPane
          objectName: "transcriptPane"
          theme: root.theme
          feed: root.feed
          ended: root.ended
          onRenameRequested: function(keys, name) { root.renameRequested(keys, name) }
          onSpeakerClicked: function(line, anchor) { root.openReassign(line, anchor) }
          onNoteClicked: function(line, anchor) { root.openNoteEditor(line, anchor) }
        }

        NotesPane {
          objectName: "notesPane"
          theme: root.theme
          assistant: root.assistant
        }

        AskPane {
          objectName: "askPane"
          theme: root.theme
          assistant: root.assistant
          feed: root.feed
          onAgentMenuRequested: function(anchor) { agentMenu.openAt(anchor) }
          onCitationActivated: function(ms) { root.jumpTo(ms) }
        }

        SummaryPane {
          id: summaryPane
          objectName: "summaryPane"
          theme: root.theme
          assistant: root.assistant
          feed: root.feed
          onAgentMenuRequested: function(anchor) { agentMenu.openAt(anchor) }
          onCitationActivated: function(ms) { root.jumpTo(ms) }
        }
      }

      // ------------------------------------------------------- idle
      //
      // No meeting on screen: start one, or pick a past one to read and fix.

      ColumnLayout {
        objectName: "idleState"
        Layout.fillWidth: true
        Layout.fillHeight: true
        visible: !root.hasMeeting
        spacing: root.theme.xxl

        Text {
          objectName: "idleTitle"
          Layout.fillWidth: true
          text: root.connected ? "No meeting in progress" : "Voxtype is not running"
          color: root.theme.fg
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.title
        }

        Text {
          objectName: "idleDetail"
          Layout.fillWidth: true
          visible: !root.connected
          text: "Waiting for the voxtype daemon to start a meeting. Past meetings are still here."
          color: root.theme.dim
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.bodySmall
          wrapMode: Text.WordWrap
        }

        RowLayout {
          Layout.fillWidth: true
          spacing: root.theme.gap

          MeetingTextField {
            id: titleField
            objectName: "startTitleField"
            theme: root.theme
            Layout.fillWidth: true
            placeholderText: "Meeting title (optional)"
            enabled: root.connected
            onAccepted: startButton.clicked()
          }

          MeetingButton {
            id: startButton
            objectName: "startButton"
            theme: root.theme
            text: root.busy === "start" ? "Starting…" : "Start meeting"
            bordered: true
            enabled: root.connected && root.busy === ""
            onClicked: {
              if (!startButton.enabled) return
              root.startRequested(titleField.text)
            }
          }
        }

        Rectangle {
          Layout.fillWidth: true
          implicitHeight: 1
          color: root.theme.alpha(root.theme.fg, 0.12)
        }

        MeetingList {
          objectName: "meetingListPane"
          Layout.fillWidth: true
          Layout.fillHeight: true
          theme: root.theme
          meetings: root.pastMeetings
          loading: root.listLoading
          error: root.listError
          hasMore: root.listHasMore
          onOpenRequested: function(meetingId) { root.openMeetingRequested(meetingId) }
          onMoreRequested: root.moreMeetingsRequested()
        }
      }
    }
  }

  // ------------------------------------------------------------- overlays

  // Behind any open overlay: a click outside closes it.
  MouseArea {
    objectName: "overlayScrim"
    anchors.fill: parent
    visible: reassignPopup.visible || noteEditor.visible || agentMenu.visible || consent.visible
    z: 9
    onClicked: {
      if (consent.visible) return
      reassignPopup.close()
      noteEditor.close()
      agentMenu.close()
    }
  }

  MeetingSurface {
    id: reassignPopup
    objectName: "reassignPopup"
    theme: root.theme
    visible: false
    z: 10
    width: Math.min(root.width - root.theme.lg * 2, 340)
    height: popupColumn.implicitHeight + root.theme.padX * 2
    // Below the speaker it was opened from, or above it if there is no room.
    y: anchorBottom + root.theme.xs + height <= root.height - root.theme.lg
      ? anchorBottom + root.theme.xs
      : Math.max(root.theme.lg, anchorTop - root.theme.xs - height)

    property int segId: -1
    property string speaker: ""
    property string originalSpeaker: ""
    property string lineText: ""
    property real anchorTop: 0
    property real anchorBottom: 0

    function close() {
      reassignPopup.visible = false
      root.forceActiveFocus()
    }

    Keys.onEscapePressed: reassignPopup.close()

    // Clicks inside must not reach the scrim.
    MouseArea { anchors.fill: parent; onClicked: reassignPopup.forceActiveFocus() }

    Column {
      id: popupColumn
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.margins: root.theme.padX
      spacing: root.theme.lg

      Text {
        width: parent.width
        text: "Move this line to"
        color: root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.caption
      }

      Text {
        objectName: "reassignLineText"
        width: parent.width
        text: "“" + reassignPopup.lineText + "”"
        color: root.theme.fg
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.bodySmall
        elide: Text.ElideRight
      }

      Flow {
        objectName: "reassignChips"
        width: parent.width
        spacing: root.theme.md

        Repeater {
          model: reassignPopup.visible && root.feed ? root.feed.chips : []

          delegate: Rectangle {
            id: pick
            required property var modelData
            required property int index
            objectName: "reassignChip_" + index
            readonly property string name: modelData.name
            readonly property bool current: modelData.keys.indexOf(reassignPopup.speaker) !== -1
            implicitWidth: pickRow.implicitWidth + root.theme.padX * 2
            implicitHeight: root.theme.controlHeight
            radius: height / 2
            color: pick.current ? root.theme.alpha(transcriptPane.chipColor(pick.index), 0.25)
              : pickMouse.containsMouse ? root.theme.alpha(root.theme.fg, 0.12) : root.theme.alpha(root.theme.fg, 0.06)
            border.width: 1
            border.color: root.theme.alpha(transcriptPane.chipColor(pick.index), pick.current ? 1.0 : 0.6)

            Row {
              id: pickRow
              anchors.centerIn: parent
              spacing: root.theme.md
              Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: 8
                height: 8
                radius: 4
                color: transcriptPane.chipColor(pick.index)
              }
              Text {
                anchors.verticalCenter: parent.verticalCenter
                text: pick.current ? pick.name + " (now)" : pick.name
                color: root.theme.fg
                font.family: root.theme.fontFamily
                font.pixelSize: root.theme.bodySmall
              }
            }

            MouseArea {
              id: pickMouse
              anchors.fill: parent
              hoverEnabled: true
              enabled: !pick.current
              cursorShape: Qt.PointingHandCursor
              onClicked: root.chooseSpeaker(pick.name, pick.modelData.keys)
            }
          }
        }
      }

      RowLayout {
        width: parent.width
        spacing: root.theme.gap

        MeetingTextField {
          id: newSpeakerField
          objectName: "newSpeakerField"
          theme: root.theme
          Layout.fillWidth: true
          placeholderText: "New speaker…"
          font.pixelSize: root.theme.bodySmall
          onAccepted: root.chooseNewSpeaker(newSpeakerField.text)
          Keys.onEscapePressed: reassignPopup.close()
        }

        MeetingButton {
          objectName: "newSpeakerButton"
          theme: root.theme
          text: "Move"
          bordered: true
          enabled: newSpeakerField.text.trim() !== ""
          onClicked: root.chooseNewSpeaker(newSpeakerField.text)
        }
      }

      MeetingButton {
        objectName: "restoreButton"
        theme: root.theme
        visible: reassignPopup.originalSpeaker !== ""
        text: "Restore original (" + (root.feed ? root.feed.displayName(reassignPopup.originalSpeaker) : "") + ")"
        bordered: true
        onClicked: {
          var segId = reassignPopup.segId
          reassignPopup.close()
          root.restoreRequested(segId)
        }
      }
    }
  }

  MeetingSurface {
    id: noteEditor
    objectName: "noteEditor"
    theme: root.theme
    visible: false
    z: 10
    width: Math.min(root.width - root.theme.lg * 2, 340)
    height: noteColumn.implicitHeight + root.theme.padX * 2
    y: anchorBottom + root.theme.xs + height <= root.height - root.theme.lg
      ? anchorBottom + root.theme.xs
      : Math.max(root.theme.lg, anchorTop - root.theme.xs - height)

    property int segId: -1
    property string existing: ""
    property string lineText: ""
    property real anchorTop: 0
    property real anchorBottom: 0

    function close() {
      noteEditor.visible = false
      root.forceActiveFocus()
    }

    function save() {
      var segId = noteEditor.segId
      var text = noteField.text.trim()
      noteEditor.close()
      if (text === noteEditor.existing) return
      root.annotateRequested(segId, text)
    }

    MouseArea { anchors.fill: parent; onClicked: noteField.forceActiveFocus() }

    Column {
      id: noteColumn
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.margins: root.theme.padX
      spacing: root.theme.lg

      Text {
        width: parent.width
        text: "Note on “" + noteEditor.lineText + "”"
        color: root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.caption
        elide: Text.ElideRight
      }

      MeetingTextField {
        id: noteField
        objectName: "noteField"
        theme: root.theme
        width: parent.width
        placeholderText: "Add a note…"
        onAccepted: noteEditor.save()
        Keys.onEscapePressed: noteEditor.close()
      }

      Row {
        spacing: root.theme.gap
        MeetingButton {
          objectName: "noteSaveButton"
          theme: root.theme
          text: "Save note"
          bordered: true
          onClicked: noteEditor.save()
        }
        MeetingButton {
          objectName: "noteRemoveButton"
          theme: root.theme
          visible: noteEditor.existing !== ""
          text: "Remove"
          bordered: true
          onClicked: {
            noteField.text = ""
            noteEditor.save()
          }
        }
      }
    }
  }

  // The agent chip's menu: agents to pick from, the local-only switch, and
  // the way to the rest of the assistant settings.
  MeetingSurface {
    id: agentMenu
    objectName: "agentMenu"
    theme: root.theme
    visible: false
    z: 10
    width: 260
    height: menuColumn.implicitHeight + root.theme.padX * 2

    function openAt(anchor) {
      if (root.assistant) root.assistant.fetchStatus()
      var p = anchor.mapToItem(root, 0, 0)
      agentMenu.x = Math.max(root.theme.lg, Math.min(p.x + anchor.width - agentMenu.width, root.width - agentMenu.width - root.theme.lg))
      agentMenu.y = p.y + anchor.height + root.theme.xs
      agentMenu.visible = true
      agentMenu.forceActiveFocus()
    }
    function close() {
      agentMenu.visible = false
      root.forceActiveFocus()
    }
    Keys.onEscapePressed: agentMenu.close()

    MouseArea { anchors.fill: parent; onClicked: {} }

    Column {
      id: menuColumn
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.margins: root.theme.padX
      spacing: root.theme.sm

      Text {
        width: parent.width
        text: "Answers and summaries from"
        color: root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.caption
      }

      Repeater {
        model: root.assistant ? root.assistant.candidates : []
        MeetingButton {
          required property var modelData
          objectName: "agentOption_" + modelData.id
          theme: root.theme
          width: menuColumn.width
          text: (modelData.label || modelData.id)
          active: root.assistant && !root.assistant.localOnly && root.assistant.status
            && root.assistant.status.agent === modelData.id
          onClicked: {
            root.assistant.chooseAgent(modelData.id)
            agentMenu.close()
          }
        }
      }

      MeetingButton {
        objectName: "localOnlyToggle"
        theme: root.theme
        width: menuColumn.width
        text: (root.assistant && root.assistant.localOnly ? "✓ " : "") + "Local model only"
        active: root.assistant !== null && root.assistant.localOnly
        onClicked: {
          root.assistant.setLocalOnly(!root.assistant.localOnly)
          agentMenu.close()
        }
      }

      MeetingButton {
        objectName: "openSettings"
        theme: root.theme
        width: menuColumn.width
        visible: root.canOpenSettings
        text: "Change in Settings"
        onClicked: {
          agentMenu.close()
          root.settingsRequested()
        }
      }
    }
  }

  // The first time an agent is needed: send this meeting to it (once, for
  // good), use a local model, or not now.
  MeetingSurface {
    id: consent
    objectName: "consentDialog"
    theme: root.theme
    visible: false
    z: 11
    anchors.centerIn: parent
    width: Math.min(root.width - root.theme.lg * 4, 360)
    height: consentColumn.implicitHeight + root.theme.padding * 2

    readonly property bool choosing: root.assistant !== null && root.assistant.blocker === "choose"
    readonly property var agents: {
      if (!root.assistant || !root.assistant.status) return []
      if (consent.choosing) return root.assistant.candidates
      var s = root.assistant.status
      return [{ id: s.agent, label: s.agent_label || s.agent }]
    }
    readonly property string agentNames: {
      var names = []
      for (var i = 0; i < consent.agents.length; i++) names.push(consent.agents[i].label || consent.agents[i].id)
      return names.join(" or ")
    }

    function dismiss() {
      if (root.assistant) root.assistant.declinePending()
      consent.visible = false
      root.forceActiveFocus()
    }
    onVisibleChanged: if (consent.visible) consent.forceActiveFocus()
    Keys.onEscapePressed: consent.dismiss()

    MouseArea { anchors.fill: parent; onClicked: {} }

    Column {
      id: consentColumn
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.margins: root.theme.padding
      spacing: root.theme.lg

      Text {
        objectName: "consentTitle"
        width: parent.width
        text: "Send meeting transcripts to " + consent.agentNames + "?"
        color: root.theme.fg
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.title
        font.bold: true
        wrapMode: Text.WordWrap
      }

      Text {
        width: parent.width
        text: "Questions and summaries send this meeting's transcript, speaker names and notes to "
          + consent.agentNames + ", through your own account with it. You are asked once per agent. "
          + "A local model keeps everything on this computer."
        color: root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.bodySmall
        wrapMode: Text.WordWrap
      }

      Repeater {
        model: consent.agents
        MeetingButton {
          required property var modelData
          objectName: "consentAllow_" + modelData.id
          theme: root.theme
          text: "Send to " + (modelData.label || modelData.id)
          bordered: true
          active: true
          onClicked: {
            consent.visible = false
            root.forceActiveFocus()
            root.assistant.allowAndContinue(modelData.id)
          }
        }
      }

      MeetingButton {
        objectName: "consentLocal"
        theme: root.theme
        text: "Use a local model instead"
        bordered: true
        onClicked: {
          consent.visible = false
          root.forceActiveFocus()
          root.assistant.useLocalAndContinue()
        }
      }

      MeetingButton {
        objectName: "consentNotNow"
        theme: root.theme
        text: "Not now"
        onClicked: consent.dismiss()
      }
    }
  }
}
