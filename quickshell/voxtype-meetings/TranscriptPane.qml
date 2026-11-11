import QtQuick
import QtQuick.Layouts

// The Transcript tab: speaker chips (click to rename), then one row per
// utterance with its time, speaker and text. A line's speaker opens the
// reassign chooser; hovering a line offers "Add note"; a note or highlight
// shows as a marker in the margin and the note under the text.
Item {
  id: root

  property var theme
  property var feed: null
  property bool ended: false

  signal renameRequested(var keys, string name)
  signal speakerClicked(var line, var anchor)
  signal noteClicked(var line, var anchor)

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

    Text {
      Layout.fillWidth: true
      visible: root.feed && root.feed.chips.length > 0
      text: "Click a chip to name a speaker; two given the same name are merged. Click a line's speaker to move just that line."
      color: root.theme.dim
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.caption
      wrapMode: Text.WordWrap
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
          required property string annotation
          required property bool highlight
          objectName: "transcriptLine"
          width: ListView.view.width
          implicitHeight: lineColumn.implicitHeight
          readonly property string speakerName: root.feed.displayName(line.speaker)
          readonly property int chipIndex: root.feed.chipIndexFor(line.speaker)
          readonly property bool marked: line.annotation !== "" || line.highlight

          Rectangle {
            anchors.fill: parent
            anchors.margins: -root.theme.xs
            radius: 4
            color: root.theme.alpha(root.theme.accent, 0.18)
            visible: root.flashSegId === line.segId
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
            color: root.theme.dim
            font.family: root.theme.fontFamily
            font.pixelSize: root.theme.caption
            topPadding: 2
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

              Text {
                id: noteAction
                objectName: "lineNoteButton"
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                visible: lineHover.containsMouse || noteMouse.containsMouse
                text: line.marked ? "Edit note" : "Add note"
                color: noteMouse.containsMouse ? root.theme.fg : root.theme.dim
                font.family: root.theme.fontFamily
                font.pixelSize: root.theme.caption
                font.underline: noteMouse.containsMouse

                MouseArea {
                  id: noteMouse
                  anchors.fill: parent
                  anchors.margins: -root.theme.sm
                  hoverEnabled: true
                  cursorShape: Qt.PointingHandCursor
                  onClicked: root.noteClicked(line, noteAction)
                }
              }
            }

            Text {
              objectName: "lineText"
              width: parent.width
              text: line.text
              color: root.theme.fg
              font.family: root.theme.fontFamily
              font.pixelSize: root.theme.body
              wrapMode: Text.WordWrap
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
