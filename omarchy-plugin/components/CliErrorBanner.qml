import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// A voxtype reader ran and failed. The rest of the panel still works, so this
// is a strip above the form rather than the degraded state, but the failure is
// said out loud: the list it would have filled is empty, not stale.
Item {
  id: root

  property bool active: false
  property string message: ""

  objectName: "cliErrorBanner"
  visible: active
  implicitHeight: active ? banner.implicitHeight : 0
  height: implicitHeight

  readonly property int pad: Style.spacing.controlPaddingX

  BorderSurface {
    id: banner
    anchors.left: parent.left
    anchors.right: parent.right
    implicitHeight: bannerText.implicitHeight + root.pad * 2
    height: implicitHeight
    color: Util.alpha(ShellTheme.Color.urgent, 0.10)
    radius: Style.cornerRadius
    borderSpec: Border.flat(Util.alpha(ShellTheme.Color.urgent, 0.45), Math.max(1, Style.normalBorderWidth))

    // Selectable, so the error can be copied into a bug report.
    TextEdit {
      id: bannerText
      objectName: "cliErrorText"
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.margins: root.pad
      anchors.verticalCenter: parent.verticalCenter
      text: root.message
      readOnly: true
      selectByMouse: true
      color: ShellTheme.Color.urgent
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      wrapMode: TextEdit.Wrap
    }
  }
}
