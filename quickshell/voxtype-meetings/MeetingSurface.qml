import QtQuick

// A card: the view's background, the chooser, menus and dialogs. Opaque, so
// an overlay never shows the transcript through it.
Rectangle {
  property var theme
  color: Qt.rgba(theme.bg.r, theme.bg.g, theme.bg.b, 1)
  radius: theme.radius
  border.width: 1
  border.color: theme.border
}
