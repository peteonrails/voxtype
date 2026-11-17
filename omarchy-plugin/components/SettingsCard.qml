import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// The panel's surface: a scrim over the desktop with one centered card on
// top. Children declared inside a SettingsCard land inside the card's key
// catcher, so the whole card body is one keyboard region.
//
// Escape is handled twice on purpose. PanelKeyCatcher gets it while the card
// body itself owns the keys, and the root Item below catches it once a text
// field or dropdown has focus — the catcher stands aside then (`keyBlocked`),
// and the unhandled event bubbles up the focus chain to here.
Item {
  id: root

  property int cardWidth: Style.space(900)
  property int cardHeight: Style.space(640)

  // Set by the panel while an editor owns the keyboard, so j/k/Tab go to the
  // editor instead of driving the card.
  property bool keyBlocked: false

  property color scrim: Util.alpha(ShellTheme.Color.background, 0.6)

  default property alias content: keyCatcher.data
  readonly property alias keyCatcher: keyCatcher
  readonly property alias card: card

  signal dismissed()
  signal moveRequested(int dx, int dy)
  signal activateRequested()

  Keys.onEscapePressed: root.dismissed()

  Rectangle {
    anchors.fill: parent
    color: root.scrim

    MouseArea {
      anchors.fill: parent
      onClicked: root.dismissed()
    }
  }

  BorderSurface {
    id: card
    anchors.centerIn: parent
    width: Math.min(root.cardWidth, root.width - Style.gapsOut * 2)
    height: Math.min(root.cardHeight, root.height - Style.gapsOut * 2)
    color: ShellTheme.Color.popups.background
    radius: Style.cornerRadius
    borderSpec: Border.surfaceSpec("popups", "border", ShellTheme.Color.popups.border, Math.max(1, Style.normalBorderWidth))
    padding: Style.spacing.panelPadding

    // Clicks inside the card must not reach the scrim behind it.
    MouseArea { anchors.fill: parent; onClicked: {} }

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      anchors.topMargin: card.contentTopInset
      anchors.rightMargin: card.contentRightInset
      anchors.bottomMargin: card.contentBottomInset
      anchors.leftMargin: card.contentLeftInset
      blocked: root.keyBlocked

      onCloseRequested: root.dismissed()
      onMoveRequested: function(dx, dy) { root.moveRequested(dx, dy) }
      onActivateRequested: root.activateRequested()
    }
  }
}
