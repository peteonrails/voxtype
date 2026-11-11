import QtQuick
import QtQuick.Layouts
import QtQuick.Controls.Basic as C

// The Notes tab: a pad for this meeting's notes.md. It saves itself (debounced,
// through `voxtype meeting notes --set-file`) and picks up edits made
// elsewhere unless you have unsaved typing of your own.
Item {
  id: root

  property var theme
  property var assistant: null
  onAssistantChanged: if (root.assistant) area.text = root.assistant.notes

  ColumnLayout {
    anchors.fill: parent
    spacing: root.theme.md

    RowLayout {
      Layout.fillWidth: true
      Text {
        Layout.fillWidth: true
        text: "Your notes go into the summary as guidance."
        color: root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.caption
        wrapMode: Text.WordWrap
      }
      Text {
        objectName: "notesStatus"
        text: !root.assistant ? ""
          : root.assistant.notesError !== "" ? "Not saved"
          : root.assistant.notesSaving ? "Saving…"
          : root.assistant.notesDirty ? "Edited"
          : root.assistant.notes !== "" ? "Saved" : ""
        color: root.assistant && root.assistant.notesError !== "" ? root.theme.urgent : root.theme.dim
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.caption
      }
    }

    Text {
      objectName: "notesError"
      Layout.fillWidth: true
      visible: root.assistant && root.assistant.notesError !== ""
      text: root.assistant ? root.assistant.notesError : ""
      color: root.theme.urgent
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.caption
      wrapMode: Text.WordWrap
    }

    C.ScrollView {
      Layout.fillWidth: true
      Layout.fillHeight: true

      C.TextArea {
        id: area
        objectName: "notesArea"
        wrapMode: TextEdit.Wrap
        placeholderText: "Notes for this meeting…"
        placeholderTextColor: root.theme.dim
        color: root.theme.fg
        selectionColor: root.theme.accent
        selectedTextColor: root.theme.bg
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.body
        padding: root.theme.padX
        background: Rectangle {
          radius: Math.min(root.theme.radius, 6)
          color: root.theme.alpha(root.theme.fg, 0.04)
          border.width: 1
          border.color: area.activeFocus ? root.theme.accent : root.theme.alpha(root.theme.fg, 0.2)
        }
        onTextChanged: if (root.assistant) root.assistant.editNotes(area.text)
        InputDemand { theme: root.theme; active: area.visible && (area.hovered || area.activeFocus) }
        Component.onCompleted: if (root.assistant) area.text = root.assistant.notes
      }
    }
  }

  // Text that arrives from the daemon replaces the pad's, which only happens
  // when the pad has nothing unsaved (the assistant checks).
  Connections {
    target: root.assistant
    function onNotesChanged() {
      if (area.text !== root.assistant.notes) area.text = root.assistant.notes
    }
  }
}
