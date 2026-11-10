import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// Voxtype reads its config at startup, so a written setting is not a live
// setting. The banner appears after the first successful write and stays until
// a restart succeeds.
Item {
  id: root

  property bool active: false
  property bool restarting: false
  property string message: "Changes take effect after restart."

  signal restartRequested()

  visible: active
  implicitHeight: active ? banner.implicitHeight : 0
  height: implicitHeight

  // The banner is a padded box, not a fixed-height strip: its height comes from
  // whichever of the message or the button is taller, plus the same padding on
  // every side. The button used to sit against the right edge on a hardcoded
  // 38px band that took no account of how tall the button actually was.
  readonly property int pad: Style.spacing.controlPaddingX

  BorderSurface {
    id: banner
    anchors.left: parent.left
    anchors.right: parent.right
    implicitHeight: Math.max(bannerText.implicitHeight, restartButton.implicitHeight)
      + root.pad * 2
    height: implicitHeight
    color: Util.alpha(ShellTheme.Color.accent, 0.12)
    radius: Style.cornerRadius
    borderSpec: Border.flat(Util.alpha(ShellTheme.Color.accent, 0.45), Math.max(1, Style.normalBorderWidth))

    Text {
      id: bannerText
      anchors.left: parent.left
      anchors.leftMargin: root.pad
      anchors.right: restartButton.left
      anchors.rightMargin: Style.spacing.controlGap
      anchors.verticalCenter: parent.verticalCenter
      text: root.message
      color: ShellTheme.Color.foreground
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      wrapMode: Text.WordWrap
    }

    Button {
      id: restartButton
      anchors.right: parent.right
      anchors.rightMargin: root.pad
      anchors.verticalCenter: parent.verticalCenter
      text: root.restarting ? "Restarting…" : "Restart voxtype"
      iconText: "󰑐"
      iconSpinning: root.restarting
      bordered: true
      onClicked: root.restartRequested()
    }
  }
}
