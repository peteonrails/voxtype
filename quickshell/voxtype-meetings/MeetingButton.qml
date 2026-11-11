import QtQuick

// A flat button: an optional glyph and a label.
Rectangle {
  id: b

  property var theme
  property string text: ""
  property string iconText: ""
  property bool bordered: false
  property bool active: false
  property color background: "transparent"

  signal clicked()

  implicitHeight: Math.max(b.theme.controlHeight, row.implicitHeight + b.theme.sm * 2)
  implicitWidth: row.implicitWidth + b.theme.padX * 2
  radius: Math.min(b.theme.radius, 6)
  color: mouse.containsMouse && b.enabled ? b.theme.alpha(b.theme.fg, 0.12)
    : b.active ? b.theme.alpha(b.theme.accent, 0.22) : b.background
  border.width: b.bordered ? 1 : 0
  border.color: b.active ? b.theme.accent : b.theme.alpha(b.theme.fg, 0.35)
  opacity: b.enabled ? 1 : 0.5

  Row {
    id: row
    anchors.centerIn: parent
    spacing: b.theme.md

    Text {
      anchors.verticalCenter: parent.verticalCenter
      visible: b.iconText !== ""
      text: b.iconText
      color: b.theme.fg
      font.family: b.theme.iconFamily
      font.pixelSize: b.theme.title
    }

    Text {
      anchors.verticalCenter: parent.verticalCenter
      visible: b.text !== ""
      text: b.text
      color: b.theme.fg
      font.family: b.theme.fontFamily
      font.pixelSize: b.theme.body
    }
  }

  MouseArea {
    id: mouse
    anchors.fill: parent
    hoverEnabled: true
    enabled: b.enabled
    cursorShape: Qt.PointingHandCursor
    onClicked: b.clicked()
  }
}
