import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// Shown when this panel was installed by hand (a plain directory under
// ~/.config/omarchy/plugins) and voxtype's package ships its own copy. The
// hand-installed one never updates with voxtype, so it drifts from the CLI it
// drives. The button runs `voxtype setup omarchy --replace` in a terminal,
// which swaps the directory for a link to the packaged copy.
Item {
  id: root

  property bool active: false

  signal replaceRequested()

  objectName: "pluginCopyBanner"
  visible: active
  implicitHeight: active ? banner.implicitHeight : 0
  height: implicitHeight

  readonly property int pad: Style.spacing.controlPaddingX

  BorderSurface {
    id: banner
    anchors.left: parent.left
    anchors.right: parent.right
    implicitHeight: Math.max(bannerText.implicitHeight, switchButton.implicitHeight) + root.pad * 2
    height: implicitHeight
    color: Util.alpha(ShellTheme.Color.accent, 0.12)
    radius: Style.cornerRadius
    borderSpec: Border.flat(Util.alpha(ShellTheme.Color.accent, 0.45), Math.max(1, Style.normalBorderWidth))

    Text {
      id: bannerText
      objectName: "pluginCopyText"
      anchors.left: parent.left
      anchors.leftMargin: root.pad
      anchors.right: switchButton.left
      anchors.rightMargin: Style.spacing.controlGap
      anchors.verticalCenter: parent.verticalCenter
      text: "This is an older copy of Voxtype Settings. Switch to the one that updates with voxtype?"
      color: ShellTheme.Color.foreground
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      wrapMode: Text.WordWrap
    }

    Button {
      id: switchButton
      objectName: "pluginCopySwitch"
      anchors.right: parent.right
      anchors.rightMargin: root.pad
      anchors.verticalCenter: parent.verticalCenter
      text: "Switch"
      iconText: "󰑓"
      bordered: true
      onClicked: root.replaceRequested()
    }
  }
}
