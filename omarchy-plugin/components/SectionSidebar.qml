import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// Left rail: the search box, then one row per section in schema order.
// While a search is active the section rows dim, because the form is showing
// matches from every section rather than the selected one.
Item {
  id: root

  property var sections: []
  property string activeSection: ""
  property bool searching: false

  readonly property alias searchField: search
  readonly property bool searchActive: search.activeFocus

  signal sectionSelected(string section)
  signal searchChanged(string text)
  signal dismissRequested()

  // The search box is giving up the keyboard so the form's row cursor can have
  // it. Emitted for Down and Tab, the two keys that mean "on to the settings".
  signal traverseRequested()

  implicitWidth: Style.space(180)

  function focusSearch() {
    search.forceActiveFocus()
    search.selectAll()
  }

  Column {
    id: column
    anchors.left: parent.left
    anchors.right: parent.right
    anchors.top: parent.top
    anchors.bottom: parent.bottom
    spacing: Style.spacing.lg

    TextField {
      id: search
      width: parent.width
      placeholderText: "Search settings"
      onTextChanged: root.searchChanged(text)

      // The card's key catcher steps aside while this field has focus, so
      // Escape has to be handled here too.
      Keys.onEscapePressed: {
        if (search.text !== "") {
          search.text = ""
          return
        }
        root.dismissRequested()
      }

      // While this field holds the keyboard, j and k are letters. Down (or Tab)
      // is the way out of it and into the form.
      Keys.onDownPressed: function(event) {
        root.traverseRequested()
        event.accepted = true
      }

      Keys.onTabPressed: function(event) {
        root.traverseRequested()
        event.accepted = true
      }
    }

    Column {
      width: parent.width
      spacing: Style.spacing.xxs
      opacity: root.searching ? 0.45 : 1.0

      Repeater {
        model: root.sections

        Button {
          required property var modelData
          width: parent.width
          text: String(modelData)
          leftAlign: true
          selected: !root.searching && String(modelData) === root.activeSection
          onClicked: root.sectionSelected(String(modelData))
        }
      }
    }

    Text {
      visible: root.searching
      width: parent.width
      text: "Showing matches from every section."
      color: Qt.darker(ShellTheme.Color.foreground, 1.6)
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      wrapMode: Text.WordWrap
    }
  }
}
