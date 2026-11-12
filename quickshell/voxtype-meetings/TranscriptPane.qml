import QtQuick
import QtQuick.Layouts
import QtQuick.Controls.Basic as C

// The Transcript tab: speaker chips (click to rename), then one row per
// utterance with its time, speaker and text. A line's speaker opens the
// reassign chooser; hovering a line offers Edit (its text, inline: Enter
// saves, Escape cancels), Add note and Delete; a note or highlight shows as a
// marker in the margin and the note under the text.
Item {
  id: root

  property var theme
  property var feed: null
  property bool ended: false

  signal renameRequested(var keys, string name)
  signal speakerClicked(var line, var anchor)
  signal noteClicked(var line, var anchor)
  signal editRequested(int segId, string text)
  signal resetTextRequested(int segId)
  signal playRequested(int segId)
  signal settingsRequested()
  // ▶ on lines unless the meeting is known to have kept no audio.
  property bool canPlay: true
  property bool audioKnownMissing: false
  property bool canOpenSettings: false
  // The line being heard while the meeting's audio plays.
  property int playingSegId: -1
  property bool audioHintDismissed: false
  // The how-to under the chips, until dismissed (once per shell session:
  // the controller remembers it across reopening the view).
  property bool speakerHintDismissed: false
  signal speakerHintDismissRequested()
  onAudioKnownMissingChanged: if (!root.audioKnownMissing) root.audioHintDismissed = false
  signal deleteRequested(int segId)

  // A small text action that shows on a line's hover.
  component LineAction: Text {
    id: act
    signal activated()
    readonly property bool hovered: actMouse.containsMouse
    color: act.hovered ? root.theme.fg : root.theme.dim
    font.family: root.theme.fontFamily
    font.pixelSize: root.theme.caption
    font.underline: act.hovered
    MouseArea {
      id: actMouse
      anchors.fill: parent
      anchors.margins: -root.theme.sm
      hoverEnabled: true
      cursorShape: Qt.PointingHandCursor
      onClicked: act.activated()
    }
  }

  readonly property alias list: transcript

  // Show the line at `ms` (a citation or a chapter) and flash it.
  function jumpTo(ms) {
    var row = root.feed ? root.feed.rowAt(ms) : -1
    if (row < 0) return
    transcript.follow = false
    transcript.positionViewAtIndex(row, ListView.Center)
    root.flashSegId = root.feed.segments.get(row).segId
    flashTimer.restart()
  }

  property int flashSegId: -1
  Timer { id: flashTimer; interval: 1600; onTriggered: root.flashSegId = -1 }

  function chipColor(index) {
    var hues = [0.58, 0.08, 0.33, 0.83, 0.15, 0.5, 0.0, 0.7]
    return Qt.hsla(hues[index % hues.length], 0.55, 0.62, 1.0)
  }

  function clock(ms) {
    var total = Math.max(0, Math.floor(ms / 1000))
    var h = Math.floor(total / 3600)
    var m = Math.floor((total % 3600) / 60)
    var s = total % 60
    var mm = (h > 0 && m < 10 ? "0" : "") + m
    return (h > 0 ? h + ":" : "") + mm + ":" + (s < 10 ? "0" : "") + s
  }

  ColumnLayout {
    anchors.fill: parent
    spacing: root.theme.lg

    Flow {
      objectName: "speakerChips"
      Layout.fillWidth: true
      visible: root.feed && root.feed.chips.length > 0
      spacing: root.theme.md

      Repeater {
        model: root.feed ? root.feed.chips : []

        delegate: Item {
          id: chip
          required property var modelData
          required property int index
          property bool editing: false
          objectName: "speakerChip_" + index
          readonly property string name: modelData.name
          implicitWidth: editing ? 170 : chipRow.implicitWidth + root.theme.padX * 2
          implicitHeight: root.theme.controlHeight

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
            color: chipMouse.containsMouse ? root.theme.alpha(root.theme.fg, 0.12) : root.theme.alpha(root.theme.fg, 0.06)
            border.width: 1
            border.color: root.theme.alpha(root.chipColor(chip.index), 0.6)

            Row {
              id: chipRow
              anchors.centerIn: parent
              spacing: root.theme.md
              Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: 8
                height: 8
                radius: 4
                color: root.chipColor(chip.index)
              }
              Text {
                objectName: "speakerChipName"
                anchors.verticalCenter: parent.verticalCenter
                text: chip.name
                color: root.theme.fg
                font.family: root.theme.fontFamily
                font.pixelSize: root.theme.bodySmall
              }
            }

            MouseArea {
              id: chipMouse
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: chip.beginEdit()
              InputDemand { theme: root.theme; active: chipMouse.containsMouse }
            }
          }

          MeetingTextField {
            id: renameField
            objectName: "speakerRenameField"
            theme: root.theme
            anchors.fill: parent
            visible: chip.editing
            font.pixelSize: root.theme.bodySmall
            topPadding: 0
            bottomPadding: 0
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

    // Once, not per line: why there is no ▶, and where it is turned on.
    RowLayout {
      objectName: "audioHint"
      Layout.fillWidth: true
      visible: root.audioKnownMissing && !root.audioHintDismissed
      spacing: root.theme.md
      Text {
        Layout.fillWidth: true
        text: "Audio not kept for this meeting, so lines cannot be played. "
          + (root.canOpenSettings ? "Keep audio is in Settings, under Meeting." : "Set [meeting] retain_audio = true to keep it next time.")
        color: root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.caption
        wrapMode: Text.WordWrap
      }
      MeetingButton {
        objectName: "audioHintSettings"
        theme: root.theme
        visible: root.canOpenSettings
        text: "Settings"
        onClicked: root.settingsRequested()
      }
      MeetingButton {
        objectName: "audioHintDismiss"
        theme: root.theme
        text: "✕"
        onClicked: root.audioHintDismissed = true
      }
    }

    RowLayout {
      objectName: "speakerHint"
      Layout.fillWidth: true
      visible: root.feed !== null && root.feed.chips.length > 0 && !root.speakerHintDismissed
      spacing: root.theme.md
      Text {
        objectName: "speakerHintText"
        Layout.fillWidth: true
        text: "Click a chip to name a speaker; two given the same name are merged. Click a line's speaker to move just that line."
        color: root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.caption
        wrapMode: Text.WordWrap
      }
      MeetingButton {
        objectName: "speakerHintDismiss"
        theme: root.theme
        text: "✕"
        onClicked: root.speakerHintDismissRequested()
      }
    }

    Rectangle {
      Layout.fillWidth: true
      implicitHeight: 1
      color: root.theme.alpha(root.theme.fg, 0.12)
    }

    Item {
      Layout.fillWidth: true
      Layout.fillHeight: true

      Text {
        anchors.centerIn: parent
        width: parent.width
        visible: transcript.count === 0
        horizontalAlignment: Text.AlignHCenter
        text: root.ended ? "Nothing was transcribed in this meeting." : "Lines appear here as each chunk is transcribed."
        color: root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.bodySmall
        wrapMode: Text.WordWrap
      }

      ListView {
        id: transcript
        objectName: "transcriptList"
        anchors.fill: parent
        clip: true
        spacing: root.theme.lg
        model: root.feed ? root.feed.segments : null
        boundsBehavior: Flickable.StopAtBounds

        // Follow new lines until the user scrolls up; scrolling back to the
        // bottom (or Jump to latest) resumes following.
        property bool follow: true
        onMovementEnded: transcript.follow = transcript.atYEnd
        onCountChanged: if (transcript.follow) Qt.callLater(transcript.positionViewAtEnd)
        onHeightChanged: if (transcript.follow && transcript.visible) Qt.callLater(transcript.positionViewAtEnd)

        delegate: Item {
          id: line
          required property int index
          required property string text
          required property string speaker
          required property real startMs
          required property int segId
          required property string originalSpeaker
          required property string originalText
          required property string annotation
          required property bool highlight
          objectName: "transcriptLine"
          width: ListView.view.width
          implicitHeight: lineColumn.implicitHeight
          readonly property string speakerName: root.feed.displayName(line.speaker)
          readonly property int chipIndex: root.feed.chipIndexFor(line.speaker)
          readonly property bool marked: line.annotation !== "" || line.highlight
          property bool editing: false

          function beginEdit() {
            line.editing = true
            editField.text = line.text
            editField.forceActiveFocus()
            editField.cursorPosition = editField.length
          }

          function cancelEdit() {
            line.editing = false
            root.forceActiveFocus()
          }

          function commitEdit() {
            var next = editField.text.trim()
            line.editing = false
            root.forceActiveFocus()
            if (next !== "" && next !== line.text) root.editRequested(line.segId, next)
          }

          Rectangle {
            anchors.fill: parent
            anchors.margins: -root.theme.xs
            radius: 4
            color: root.theme.alpha(root.theme.accent, 0.18)
            visible: root.flashSegId === line.segId
          }

          // The line being heard.
          Rectangle {
            objectName: "linePlaying"
            anchors.fill: parent
            anchors.margins: -root.theme.xs
            radius: 4
            color: root.theme.alpha(root.theme.accent, 0.12)
            border.width: 1
            border.color: root.theme.alpha(root.theme.accent, 0.5)
            visible: root.playingSegId === line.segId
          }

          // Hover shows "Add note" without taking clicks from anything.
          MouseArea {
            id: lineHover
            anchors.fill: parent
            hoverEnabled: true
            acceptedButtons: Qt.NoButton
          }

          Rectangle {
            objectName: "lineMarker"
            visible: line.marked
            width: 3
            radius: 1
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            anchors.left: parent.left
            color: line.highlight && line.annotation === "" ? "#e5c07b" : root.theme.accent
          }

          Text {
            id: stamp
            objectName: "lineTime"
            x: root.theme.md
            width: 40
            text: root.clock(line.startMs)
            // Over the time, read from the line's own hover area: anything
            // here that took hover would take the line's actions with it.
            readonly property bool hot: root.canPlay && lineHover.containsMouse
              && lineHover.mouseX >= stamp.x && lineHover.mouseX < stamp.x + stamp.contentWidth + 4
              && lineHover.mouseY < stamp.y + stamp.height
            color: stamp.hot ? root.theme.accent : root.theme.dim
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.caption
            font.underline: stamp.hot
            topPadding: 2
            // The time seeks: plays the kept audio from this line, or moves
            // a playback that is running here.
            MouseArea {
              objectName: "lineSeek"
              anchors.fill: parent
              enabled: root.canPlay
              cursorShape: Qt.PointingHandCursor
              onClicked: root.playRequested(line.segId)
            }
          }

          Column {
            id: lineColumn
            anchors.left: stamp.right
            anchors.right: parent.right
            spacing: root.theme.xxs

            Item {
              width: parent.width
              implicitHeight: speakerLabel.implicitHeight

              Row {
                spacing: root.theme.md

                // The speaker is the handle for moving this line elsewhere.
                Text {
                  id: speakerLabel
                  objectName: "lineSpeaker"
                  text: line.speakerName
                  color: root.chipColor(Math.max(0, line.chipIndex))
                  font.family: root.theme.fontFamily
                  font.pixelSize: root.theme.bodySmall
                  font.bold: true
                  font.underline: speakerMouse.containsMouse

                  MouseArea {
                    id: speakerMouse
                    objectName: "lineSpeakerButton"
                    anchors.fill: parent
                    anchors.margins: -root.theme.xs
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.speakerClicked(line, speakerLabel)
                    // The chooser has a "New speaker…" field.
                    InputDemand { theme: root.theme; active: speakerMouse.containsMouse }
                  }
                }

                Text {
                  objectName: "lineMoved"
                  visible: line.originalSpeaker !== ""
                  anchors.baseline: speakerLabel.baseline
                  text: "moved from " + root.feed.displayName(line.originalSpeaker)
                  color: root.theme.dim
                  font.family: root.theme.fontFamily
                  font.pixelSize: root.theme.caption
                  font.italic: true
                }
              }

              Row {
                objectName: "lineActions"
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                spacing: root.theme.lg
                visible: !line.editing
                  && (lineHover.containsMouse || playAction.hovered || editAction.hovered
                    || noteAction.hovered || deleteAction.hovered)

                LineAction {
                  id: playAction
                  objectName: "linePlayButton"
                  visible: root.canPlay
                  text: "▶"
                  onActivated: root.playRequested(line.segId)
                }

                // These open a text field, so they ask for the keyboard on
                // hover: the click that opens the field then focuses it.
                LineAction {
                  id: editAction
                  objectName: "lineEditButton"
                  text: "Edit"
                  onActivated: line.beginEdit()
                  InputDemand { theme: root.theme; active: editAction.hovered }
                }
                LineAction {
                  id: noteAction
                  objectName: "lineNoteButton"
                  text: line.marked ? "Edit note" : "Add note"
                  onActivated: root.noteClicked(line, noteAction)
                  InputDemand { theme: root.theme; active: noteAction.hovered }
                }
                LineAction {
                  id: deleteAction
                  objectName: "lineDeleteButton"
                  text: "Delete"
                  onActivated: root.deleteRequested(line.segId)
                }
              }
            }

            C.TextArea {
              id: editField
              objectName: "lineEditField"
              width: parent.width
              visible: line.editing
              wrapMode: TextEdit.Wrap
              color: root.theme.fg
              selectionColor: root.theme.accent
              selectedTextColor: root.theme.bg
              font.family: root.theme.fontFamily
              font.pixelSize: root.theme.body
              padding: root.theme.sm
              background: Rectangle {
                radius: Math.min(root.theme.radius, 4)
                color: root.theme.alpha(root.theme.fg, 0.05)
                border.width: 1
                border.color: root.theme.accent
              }
              // Enter saves; Shift+Enter is a new line; Escape cancels.
              Keys.onReturnPressed: function(event) {
                if (event.modifiers & Qt.ShiftModifier) event.accepted = false
                else line.commitEdit()
              }
              Keys.onEnterPressed: function(event) { line.commitEdit() }
              Keys.onEscapePressed: line.cancelEdit()
              onActiveFocusChanged: if (!activeFocus && line.editing) line.cancelEdit()
              InputDemand { theme: root.theme; active: editField.visible && (editField.hovered || editField.activeFocus) }
            }

            Text {
              objectName: "lineEditHint"
              visible: line.editing
              text: "Enter saves · Escape cancels"
              color: root.theme.dim
              font.family: root.theme.fontFamily
              font.pixelSize: root.theme.caption
            }

            Text {
              objectName: "lineText"
              width: parent.width
              visible: !line.editing
              text: line.text
              color: root.theme.fg
              font.family: root.theme.fontFamily
              font.pixelSize: root.theme.body
              wrapMode: Text.WordWrap
            }

            // An edited line says so under its text, with the way back. Its
            // own row: the speaker's row has the hover actions on the right.
            Row {
              visible: line.originalText !== ""
              spacing: root.theme.md
              Text {
                objectName: "lineEdited"
                text: "edited ·"
                color: root.theme.dim
                font.family: root.theme.fontFamily
                font.pixelSize: root.theme.caption
                font.italic: true
              }
              LineAction {
                id: restoreAction
                objectName: "lineRestoreTextButton"
                text: "Restore original text"
                onActivated: root.resetTextRequested(line.segId)
              }
            }

            Text {
              objectName: "lineAnnotation"
              width: parent.width
              visible: line.annotation !== ""
              text: "Note: " + line.annotation
              color: root.theme.accent
              font.family: root.theme.fontFamily
              font.pixelSize: root.theme.bodySmall
              font.italic: true
              wrapMode: Text.WordWrap
            }
          }
        }
      }

      MeetingButton {
        objectName: "jumpToLatest"
        theme: root.theme
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: root.theme.lg
        visible: !transcript.follow && transcript.count > 0
        text: "Jump to latest"
        bordered: true
        background: root.theme.bg
        onClicked: {
          transcript.follow = true
          transcript.positionViewAtEnd()
        }
      }
    }
  }
}
