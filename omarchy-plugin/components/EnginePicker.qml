import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// The engine switch, pinned above the Engine section's scroller rather than
// living in it.
//
// It used to be the first card inside the form, which meant that scrolling down
// the model list — the thing you scroll to look at when you are choosing an
// engine — carried the switch off screen, and there was nothing left on screen
// saying how to change engines. Anchored here it is visible for as long as the
// section is, and the engine in effect is stated in full rather than only being
// the selected row of a dropdown.
Item {
  id: root

  property string engine: ""
  property var engineChoices: []

  // { engineName: compiled } from `voxtype info engines --json`. An engine this
  // binary was not built with cannot be selected.
  property var engineAvailability: ({})

  property string errorText: ""

  signal engineChangeRequested(string name)

  readonly property color dim: Qt.darker(ShellTheme.Color.foreground, 1.5)

  function engineAvailable(name) {
    // Absent the engines list, offer everything: `config set engine` refuses an
    // uncompiled engine with exit 2, and that message lands inline.
    return root.engineAvailability[name] !== false
  }

  readonly property var engineOptions: {
    var out = []
    for (var i = 0; i < root.engineChoices.length; i++) {
      var name = String(root.engineChoices[i])
      out.push({
        value: name,
        label: root.engineAvailable(name) ? name : name + "  ·  not in this build"
      })
    }
    return out
  }

  implicitHeight: card.implicitHeight

  BorderSurface {
    id: card
    anchors.left: parent.left
    anchors.right: parent.right
    implicitHeight: body.implicitHeight + contentTopInset + contentBottomInset
    height: implicitHeight
    color: Util.alpha(ShellTheme.Color.foreground, 0.04)
    radius: Style.cornerRadius
    borderSpec: Border.flat(Util.alpha(ShellTheme.Color.accent, 0.35), Math.max(1, Style.normalBorderWidth))
    padding: Style.spacing.rowPaddingX

    Column {
      id: body
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.topMargin: card.contentTopInset
      anchors.leftMargin: card.contentLeftInset
      anchors.rightMargin: card.contentRightInset
      spacing: Style.spacing.md

      Item {
        width: parent.width
        implicitHeight: Math.max(labels.implicitHeight, engineDropdown.implicitHeight)

        Column {
          id: labels
          anchors.left: parent.left
          anchors.verticalCenter: parent.verticalCenter
          width: parent.width - engineDropdown.width - Style.spacing.controlGap
          spacing: Style.spacing.xxs

          PanelSectionHeader { text: "TRANSCRIPTION ENGINE" }

          Text {
            text: root.engine === "" ? "unknown" : root.engine
            color: ShellTheme.Color.foreground
            font.family: Style.font.family
            font.pixelSize: Style.font.title
            font.bold: true
          }

          Text {
            width: parent.width
            text: "Switching engines validates and saves a compatible installed model together with the engine. Existing compatible choices are preserved; missing weights require an explicit download. The panel reloads after saving."
            color: root.dim
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            wrapMode: Text.WordWrap
          }
        }

        Dropdown {
          id: engineDropdown
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          width: Style.spacing.dropdownWidth
          showLabel: false
          options: root.engineOptions
          value: root.engine
          onChanged: function(next) {
            // Dropdown writes its own `value` when a row is picked, which drops
            // this binding; restore it so the schema stays the authority on
            // which engine is actually in effect.
            engineDropdown.value = Qt.binding(function() { return root.engine })
            // Nothing to write for an engine this binary lacks.
            if (!root.engineAvailable(next)) return
            root.engineChangeRequested(next)
          }
        }
      }

      Text {
        visible: root.errorText !== ""
        width: parent.width
        text: root.errorText
        color: ShellTheme.Color.urgent
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }
    }
  }
}
