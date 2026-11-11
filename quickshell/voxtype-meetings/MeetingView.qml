import QtQuick
import QtQuick.Layouts
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// The live meeting view: a header with the meeting's state and controls,
// speaker chips, and the transcript as the daemon streams it. Everything shown
// comes from `feed` (MeetingFeed); everything done goes out as a signal, which
// Meetings.qml turns into a `voxtype meeting` command.
//
// A meeting that has ended stays on screen ("Ended") so its lines can still be
// fixed: chip renames and per-line reassignment work the same as during it.
// Clicking a line's speaker opens a small chooser to move just that line.
//
// No level meters: meeting.sock carries no levels yet. No countdown either: it
// carries no chunk timing, so between updates the view says "Listening" and
// how long ago the last lines arrived, rather than guessing at a boundary.
Item {
  id: root

  property var feed: null
  // The action running now, from MeetingCli.busy.
  property string busy: ""
  property string actionError: ""

  signal pauseRequested()
  signal resumeRequested()
  signal stopRequested()
  signal startRequested(string title)
  signal renameRequested(var keys, string name)
  // Move one line: `speaker` goes to `voxtype meeting reassign`, `key` is the
  // speaker key to show until it answers.
  signal reassignRequested(int segId, string speaker, string key)
  signal restoreRequested(int segId)
  signal closeRequested()
  signal errorDismissed()

  readonly property string status: feed ? feed.status : ""
  readonly property bool connected: feed ? feed.connected : false
  readonly property bool live: feed ? feed.active : false
  readonly property bool ended: feed ? feed.ended : false
  // Something to show: a meeting running, or one that ended and is kept.
  readonly property bool hasMeeting: live || ended
  readonly property color fg: ShellTheme.Color.foreground
  readonly property color dim: Qt.darker(ShellTheme.Color.foreground, 1.5)
  readonly property color accent: ShellTheme.Color.accent
  readonly property color urgent: ShellTheme.Color.urgent

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
    if (reassignPopup.visible) reassignPopup.close()
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
    function onTransitioned(from, to) { root.now = Date.now() }
  }

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

  readonly property string endedAtText: root.ended && root.feed.endedAt > 0
    ? "ended at " + Qt.formatTime(new Date(root.feed.endedAt), "hh:mm") : ""

  readonly property string stateLabel: {
    if (root.busy === "pause") return "Pausing…"
    if (root.busy === "resume") return "Resuming…"
    if (root.busy === "stop") return "Stopping…"
    if (root.status === "recording") return "Recording"
    if (root.status === "paused") return "Paused"
    if (root.status === "finalizing") return "Finalizing"
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
    var p = anchor.mapToItem(root, 0, 0)
    reassignPopup.anchorTop = p.y
    reassignPopup.anchorBottom = p.y + anchor.height
    reassignPopup.x = Math.max(Style.spacing.lg, Math.min(p.x - Style.spacing.lg,
                                 root.width - reassignPopup.width - Style.spacing.lg))
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

  function chipColor(index) {
    var hues = [0.58, 0.08, 0.33, 0.83, 0.15, 0.5, 0.0, 0.7]
    return Qt.hsla(hues[index % hues.length], 0.55, 0.62, 1.0)
  }

  // ------------------------------------------------------------- surface

  BorderSurface {
    id: card
    anchors.fill: parent
    color: ShellTheme.Color.popups.background
    radius: Style.cornerRadius
    borderSpec: Border.surfaceSpec("popups", "border", ShellTheme.Color.popups.border, Math.max(1, Style.normalBorderWidth))
    padding: Style.spacing.panelPadding

    // Focus for Escape once the card is clicked.
    MouseArea { anchors.fill: parent; onClicked: root.forceActiveFocus() }

    ColumnLayout {
      anchors.fill: parent
      anchors.topMargin: card.contentTopInset
      anchors.rightMargin: card.contentRightInset
      anchors.bottomMargin: card.contentBottomInset
      anchors.leftMargin: card.contentLeftInset
      spacing: Style.spacing.xxl

      // ------------------------------------------------------- header

      RowLayout {
        Layout.fillWidth: true
        spacing: Style.spacing.controlGap

        Text {
          text: "󰍬"
          color: root.hasMeeting ? root.accent : root.dim
          font.family: Style.font.family
          font.pixelSize: Style.font.iconLarge
        }

        Text {
          objectName: "meetingTitle"
          Layout.fillWidth: true
          text: root.hasMeeting
            ? (root.feed.title !== "" ? root.feed.title : "Untitled meeting")
            : "Meetings"
          color: root.fg
          font.family: Style.font.family
          font.pixelSize: Style.font.heading
          font.bold: true
          elide: Text.ElideRight
        }

        Button {
          objectName: "closeButton"
          iconText: "󰅖"
          onClicked: root.closeRequested()
        }
      }

      RowLayout {
        Layout.fillWidth: true
        visible: root.hasMeeting
        spacing: Style.spacing.controlGap

        Rectangle {
          id: stateDot
          implicitWidth: Style.space(9)
          implicitHeight: Style.space(9)
          radius: width / 2
          color: root.status === "recording" ? root.urgent : root.dim
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
          color: root.fg
          font.family: Style.font.family
          font.pixelSize: Style.font.body
        }

        Text {
          objectName: "meetingElapsed"
          text: root.elapsedText
          visible: text !== ""
          color: root.dim
          font.family: Style.font.family
          font.pixelSize: Style.font.body
        }

        Text {
          objectName: "meetingEndedAt"
          text: root.endedAtText
          visible: text !== ""
          color: root.dim
          font.family: Style.font.family
          font.pixelSize: Style.font.body
        }

        Item { Layout.fillWidth: true }

        Button {
          objectName: "pauseButton"
          visible: root.live
          text: root.status === "paused" ? "Resume" : "Pause"
          iconText: root.status === "paused" ? "󰐊" : "󰏤"
          bordered: true
          enabled: root.connected && root.busy === "" && (root.status === "recording" || root.status === "paused")
          opacity: enabled ? 1 : 0.5
          onClicked: root.status === "paused" ? root.resumeRequested() : root.pauseRequested()
        }

        Button {
          objectName: "stopButton"
          visible: root.live
          text: "Stop"
          iconText: "󰓛"
          bordered: true
          enabled: root.connected && root.busy === "" && (root.status === "recording" || root.status === "paused")
          opacity: enabled ? 1 : 0.5
          onClicked: root.stopRequested()
        }
      }

      // Listening, paused or finalizing, between chunks.
      RowLayout {
        objectName: "listeningIndicator"
        Layout.fillWidth: true
        visible: root.live && root.connected || root.ended
        spacing: Style.spacing.md

        Row {
          spacing: Style.spacing.xs
          visible: root.status === "recording"
          Repeater {
            model: 3
            Rectangle {
              required property int index
              width: Style.space(4)
              height: Style.space(4)
              radius: width / 2
              color: root.accent
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
          color: root.dim
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          elide: root.ended ? Text.ElideNone : Text.ElideRight
          wrapMode: root.ended ? Text.WordWrap : Text.NoWrap
        }
      }

      // Ended: a new meeting can start from here, replacing this one.
      RowLayout {
        objectName: "endedStartRow"
        Layout.fillWidth: true
        visible: root.ended
        spacing: Style.spacing.controlGap

        TextField {
          id: endedTitleField
          objectName: "endedTitleField"
          Layout.fillWidth: true
          placeholderText: "New meeting title (optional)"
          enabled: root.connected
          onAccepted: endedStartButton.clicked()
        }

        Button {
          id: endedStartButton
          objectName: "endedStartButton"
          text: root.busy === "start" ? "Starting…" : "Start meeting"
          iconText: "󰑊"
          bordered: true
          enabled: root.connected && root.busy === ""
          opacity: enabled ? 1 : 0.5
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
        color: root.urgent
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
        wrapMode: Text.WordWrap
      }

      BorderSurface {
        objectName: "actionError"
        Layout.fillWidth: true
        visible: root.actionError !== "" || (root.feed && root.feed.lastError !== "")
        implicitHeight: errorText.implicitHeight + Style.spacing.controlPaddingX * 2
        color: Util.alpha(root.urgent, 0.12)
        radius: Style.cornerRadius
        borderSpec: Border.flat(Util.alpha(root.urgent, 0.45), Math.max(1, Style.normalBorderWidth))

        Text {
          id: errorText
          anchors.left: parent.left
          anchors.right: dismissError.left
          anchors.verticalCenter: parent.verticalCenter
          anchors.leftMargin: Style.spacing.controlPaddingX
          anchors.rightMargin: Style.spacing.controlGap
          text: root.actionError !== "" ? root.actionError : (root.feed ? root.feed.lastError : "")
          color: root.fg
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WordWrap
        }

        Button {
          id: dismissError
          anchors.right: parent.right
          anchors.rightMargin: Style.spacing.sm
          anchors.verticalCenter: parent.verticalCenter
          iconText: "󰅖"
          onClicked: {
            if (root.feed) root.feed.lastError = ""
            root.errorDismissed()
          }
        }
      }

      // ------------------------------------------------------- speakers

      Flow {
        objectName: "speakerChips"
        Layout.fillWidth: true
        visible: root.hasMeeting && root.feed.chips.length > 0
        spacing: Style.spacing.md

        Repeater {
          model: root.feed ? root.feed.chips : []

          delegate: Item {
            id: chip
            required property var modelData
            required property int index
            property bool editing: false
            objectName: "speakerChip_" + index
            readonly property string name: modelData.name
            implicitWidth: editing ? Style.space(170) : chipRow.implicitWidth + Style.spacing.controlPaddingX * 2
            implicitHeight: Style.spacing.controlHeight

            function beginEdit() {
              chip.editing = true
              renameField.text = chip.name
              renameField.forceActiveFocus()
              renameField.selectAll()
            }

            function commit() {
              var next = renameField.text.trim()
              chip.editing = false
              root.forceActiveFocus()
              if (next !== "" && next !== chip.name) root.renameRequested(chip.modelData.keys, next)
            }

            Rectangle {
              anchors.fill: parent
              visible: !chip.editing
              radius: height / 2
              color: chipMouse.containsMouse ? Util.alpha(root.fg, 0.12) : Util.alpha(root.fg, 0.06)
              border.width: 1
              border.color: Util.alpha(root.chipColor(chip.index), 0.6)

              Row {
                id: chipRow
                anchors.centerIn: parent
                spacing: Style.spacing.md
                Rectangle {
                  anchors.verticalCenter: parent.verticalCenter
                  width: Style.space(8)
                  height: Style.space(8)
                  radius: width / 2
                  color: root.chipColor(chip.index)
                }
                Text {
                  objectName: "speakerChipName"
                  anchors.verticalCenter: parent.verticalCenter
                  text: chip.name
                  color: root.fg
                  font.family: Style.font.family
                  font.pixelSize: Style.font.bodySmall
                }
              }

              MouseArea {
                id: chipMouse
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: chip.beginEdit()
              }
            }

            TextField {
              id: renameField
              objectName: "speakerRenameField"
              anchors.fill: parent
              visible: chip.editing
              font.pixelSize: Style.font.bodySmall
              verticalPadding: 0
              placeholderText: "Name"
              onAccepted: chip.commit()
              Keys.onEscapePressed: {
                chip.editing = false
                root.forceActiveFocus()
              }
              onActiveFocusChanged: if (!activeFocus && chip.editing) chip.editing = false
            }
          }
        }
      }

      Text {
        Layout.fillWidth: true
        visible: root.hasMeeting && root.feed.chips.length > 0
        text: "Click a chip to name a speaker; two given the same name are merged. Click a line's speaker to move just that line."
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }

      Rectangle {
        Layout.fillWidth: true
        implicitHeight: 1
        visible: root.hasMeeting
        color: Util.alpha(root.fg, 0.12)
      }

      // ------------------------------------------------------- transcript

      Item {
        Layout.fillWidth: true
        Layout.fillHeight: true
        visible: root.hasMeeting

        Text {
          anchors.centerIn: parent
          width: parent.width
          visible: transcript.count === 0
          horizontalAlignment: Text.AlignHCenter
          text: root.ended ? "Nothing was transcribed in this meeting." : "Lines appear here as each chunk is transcribed."
          color: root.dim
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WordWrap
        }

        ListView {
          id: transcript
          objectName: "transcriptList"
          anchors.fill: parent
          clip: true
          spacing: Style.spacing.lg
          model: root.feed ? root.feed.segments : null
          boundsBehavior: Flickable.StopAtBounds

          // Follow new lines until the user scrolls up; scrolling back to the
          // bottom (or Jump to latest) resumes following.
          property bool follow: true
          onMovementEnded: transcript.follow = transcript.atYEnd
          onCountChanged: if (transcript.follow) Qt.callLater(transcript.positionViewAtEnd)
          onHeightChanged: if (transcript.follow) Qt.callLater(transcript.positionViewAtEnd)


          delegate: Item {
            id: line
            required property int index
            required property string text
            required property string speaker
            required property real startMs
            required property int segId
            required property string originalSpeaker
            objectName: "transcriptLine"
            width: ListView.view.width
            implicitHeight: lineColumn.implicitHeight
            readonly property string speakerName: root.feed.displayName(line.speaker)
            readonly property int chipIndex: root.feed.chipIndexFor(line.speaker)

            Text {
              id: stamp
              objectName: "lineTime"
              width: Style.space(46)
              text: root.clock(line.startMs)
              color: root.dim
              font.family: Style.font.family
              font.pixelSize: Style.font.caption
                  topPadding: 2
            }

            Column {
              id: lineColumn
              anchors.left: stamp.right
              anchors.right: parent.right
              spacing: Style.spacing.xxs

              Row {
                spacing: Style.spacing.md

                // The speaker is the handle for moving this line elsewhere.
                Text {
                  id: speakerLabel
                  objectName: "lineSpeaker"
                  text: line.speakerName
                  color: root.chipColor(Math.max(0, line.chipIndex))
                  font.family: Style.font.family
                  font.pixelSize: Style.font.bodySmall
                  font.bold: true
                  font.underline: speakerMouse.containsMouse

                  MouseArea {
                    id: speakerMouse
                    objectName: "lineSpeakerButton"
                    anchors.fill: parent
                    anchors.margins: -Style.spacing.xs
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.openReassign(line, speakerLabel)
                  }
                }

                Text {
                  objectName: "lineMoved"
                  visible: line.originalSpeaker !== ""
                  anchors.baseline: speakerLabel.baseline
                  text: "moved from " + root.feed.displayName(line.originalSpeaker)
                  color: root.dim
                  font.family: Style.font.family
                  font.pixelSize: Style.font.caption
                  font.italic: true
                }
              }
              Text {
                objectName: "lineText"
                width: parent.width
                text: line.text
                color: root.fg
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                wrapMode: Text.WordWrap
              }
            }
          }
        }

        Button {
          objectName: "jumpToLatest"
          anchors.horizontalCenter: parent.horizontalCenter
          anchors.bottom: parent.bottom
          anchors.bottomMargin: Style.spacing.lg
          visible: !transcript.follow && transcript.count > 0
          text: "Jump to latest"
          iconText: "󰁅"
          bordered: true
          background: ShellTheme.Color.popups.background
          onClicked: {
            transcript.follow = true
            transcript.positionViewAtEnd()
          }
        }
      }

      // ------------------------------------------------------- idle

      Item {
        objectName: "idleState"
        Layout.fillWidth: true
        Layout.fillHeight: true
        visible: !root.hasMeeting

        Column {
          anchors.centerIn: parent
          width: Math.min(parent.width, Style.space(340))
          spacing: Style.spacing.xxl

          Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: "󰍬"
            color: root.dim
            font.family: Style.font.family
            font.pixelSize: Style.font.displayLarge
          }

          Text {
            objectName: "idleTitle"
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: root.connected ? "No meeting in progress" : "Voxtype is not running"
            color: root.fg
            font.family: Style.font.family
            font.pixelSize: Style.font.title
          }

          Text {
            objectName: "idleDetail"
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: root.connected
              ? "Start one here, from the tray, or with voxtype meeting start. This view follows it live."
              : "Waiting for the voxtype daemon. This view connects as soon as it is running."
            color: root.dim
            font.family: Style.font.family
            font.pixelSize: Style.font.bodySmall
            wrapMode: Text.WordWrap
          }

          TextField {
            id: titleField
            objectName: "startTitleField"
            width: parent.width
            placeholderText: "Meeting title (optional)"
            enabled: root.connected
            onAccepted: startButton.clicked()
          }

          Button {
            id: startButton
            objectName: "startButton"
            anchors.horizontalCenter: parent.horizontalCenter
            text: root.busy === "start" ? "Starting…" : "Start meeting"
            iconText: "󰑊"
            bordered: true
            enabled: root.connected && root.busy === ""
            opacity: enabled ? 1 : 0.5
            onClicked: {
              if (!startButton.enabled) return
              root.startRequested(titleField.text)
            }
          }
        }
      }
    }
  }

  // ------------------------------------------------------------- chooser
  //
  // A plain item, not a Popup: Qt Quick Controls popups need a window overlay
  // that a layer-shell surface does not provide (see the plugin README).

  MouseArea {
    objectName: "reassignScrim"
    anchors.fill: parent
    visible: reassignPopup.visible
    z: 9
    onClicked: reassignPopup.close()
  }

  BorderSurface {
    id: reassignPopup
    objectName: "reassignPopup"
    visible: false
    z: 10
    width: Math.min(root.width - Style.spacing.lg * 2, Style.space(340))
    implicitHeight: popupColumn.implicitHeight + Style.spacing.controlPaddingX * 2
    height: implicitHeight
    // Below the speaker it was opened from, or above it if there is no room.
    y: anchorBottom + Style.spacing.xs + height <= root.height - Style.spacing.lg
      ? anchorBottom + Style.spacing.xs
      : Math.max(Style.spacing.lg, anchorTop - Style.spacing.xs - height)
    color: ShellTheme.Color.popups.background
    radius: Style.cornerRadius
    borderSpec: Border.surfaceSpec("popups", "border", ShellTheme.Color.popups.border, Math.max(1, Style.normalBorderWidth))

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
      anchors.margins: Style.spacing.controlPaddingX
      spacing: Style.spacing.lg

      Text {
        width: parent.width
        text: "Move this line to"
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }

      Text {
        objectName: "reassignLineText"
        width: parent.width
        text: "“" + reassignPopup.lineText + "”"
        color: root.fg
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
        elide: Text.ElideRight
      }

      Flow {
        objectName: "reassignChips"
        width: parent.width
        spacing: Style.spacing.md

        Repeater {
          model: reassignPopup.visible && root.feed ? root.feed.chips : []

          delegate: Rectangle {
            id: pick
            required property var modelData
            required property int index
            objectName: "reassignChip_" + index
            readonly property string name: modelData.name
            readonly property bool current: modelData.keys.indexOf(reassignPopup.speaker) !== -1
            implicitWidth: pickRow.implicitWidth + Style.spacing.controlPaddingX * 2
            implicitHeight: Style.spacing.controlHeight
            radius: height / 2
            color: pick.current ? Util.alpha(root.chipColor(pick.index), 0.25)
              : pickMouse.containsMouse ? Util.alpha(root.fg, 0.12) : Util.alpha(root.fg, 0.06)
            border.width: 1
            border.color: Util.alpha(root.chipColor(pick.index), pick.current ? 1.0 : 0.6)

            Row {
              id: pickRow
              anchors.centerIn: parent
              spacing: Style.spacing.md
              Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: Style.space(8)
                height: Style.space(8)
                radius: width / 2
                color: root.chipColor(pick.index)
              }
              Text {
                anchors.verticalCenter: parent.verticalCenter
                text: pick.current ? pick.name + " (now)" : pick.name
                color: root.fg
                font.family: Style.font.family
                font.pixelSize: Style.font.bodySmall
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
        spacing: Style.spacing.controlGap

        TextField {
          id: newSpeakerField
          objectName: "newSpeakerField"
          Layout.fillWidth: true
          placeholderText: "New speaker…"
          font.pixelSize: Style.font.bodySmall
          onAccepted: root.chooseNewSpeaker(newSpeakerField.text)
          Keys.onEscapePressed: reassignPopup.close()
        }

        Button {
          objectName: "newSpeakerButton"
          text: "Move"
          bordered: true
          enabled: newSpeakerField.text.trim() !== ""
          opacity: enabled ? 1 : 0.5
          onClicked: root.chooseNewSpeaker(newSpeakerField.text)
        }
      }

      Button {
        objectName: "restoreButton"
        visible: reassignPopup.originalSpeaker !== ""
        text: "Restore original (" + root.feed.displayName(reassignPopup.originalSpeaker) + ")"
        iconText: "󰕌"
        bordered: true
        onClicked: {
          var segId = reassignPopup.segId
          reassignPopup.close()
          root.restoreRequested(segId)
        }
      }
    }
  }
}
