import QtQuick
import QtQuick.Controls.Basic as C

// A single-line field in the view's theme.
C.TextField {
  id: f
  property var theme

  color: f.theme.fg
  placeholderTextColor: f.theme.dim
  selectionColor: f.theme.accent
  selectedTextColor: f.theme.bg
  font.family: f.theme.fontFamily
  font.pixelSize: f.theme.body
  leftPadding: f.theme.padX
  rightPadding: f.theme.padX
  topPadding: f.theme.md
  bottomPadding: f.theme.md
  InputDemand { theme: f.theme; active: f.visible && f.enabled && (f.hovered || f.activeFocus) }

  background: Rectangle {
    radius: Math.min(f.theme.radius, 6)
    color: f.theme.alpha(f.theme.fg, 0.05)
    border.width: 1
    border.color: f.activeFocus ? f.theme.accent : f.theme.alpha(f.theme.fg, 0.25)
  }
}
