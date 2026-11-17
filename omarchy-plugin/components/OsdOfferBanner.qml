import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// Offers the Quickshell OSD to someone whose config pins another frontend,
// the way UpgradeOfferBanner offers a better model. Whether to offer is the
// CLI's call (`setup osd --status --json` reports `offer`), never the panel's.
//
// Switch runs `voxtype setup osd --use-auto` and then the panel's restart flow.
// Not now records the choice with `voxtype setup osd --dismiss-offer`.
Item {
  id: root

  property bool active: false
  // What the config names today ("gtk4" or "native"), for the sentence.
  property string configured: ""
  property bool switching: false
  property bool restarting: false
  property bool dismissing: false
  property string errorText: ""

  signal switchRequested()
  signal notNowRequested()

  visible: active
  implicitHeight: active ? banner.implicitHeight : 0
  height: implicitHeight

  readonly property int pad: Style.spacing.controlPaddingX
  readonly property bool busy: switching || restarting || dismissing

  function frontendName(name) {
    if (name === "gtk4") return "GTK4"
    if (name === "native") return "native"
    return name
  }

  BorderSurface {
    id: banner
    anchors.left: parent.left
    anchors.right: parent.right
    implicitHeight: Math.max(texts.implicitHeight, switchButton.implicitHeight) + root.pad * 2
    height: implicitHeight
    color: Util.alpha(ShellTheme.Color.accent, 0.12)
    radius: Style.cornerRadius
    borderSpec: Border.flat(Util.alpha(ShellTheme.Color.accent, 0.45), Math.max(1, Style.normalBorderWidth))

    Column {
      id: texts
      anchors.left: parent.left
      anchors.leftMargin: root.pad
      anchors.right: notNowButton.left
      anchors.rightMargin: Style.spacing.controlGap
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.spacing.xxs

      Text {
        objectName: "osdOfferText"
        width: parent.width
        text: "The Quickshell OSD is available on this desktop. Your OSD is set to "
          + (root.configured !== "" ? root.frontendName(root.configured) : "another frontend")
          + ". Switch to the Quickshell OSD and its styles?"
        color: ShellTheme.Color.foreground
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
        wrapMode: Text.WordWrap
      }

      Text {
        width: parent.width
        text: "Switch sets the OSD frontend to auto and restarts voxtype."
        color: Qt.darker(ShellTheme.Color.foreground, 1.5)
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }

      Text {
        objectName: "osdOfferError"
        width: parent.width
        visible: root.errorText !== ""
        text: root.errorText
        color: ShellTheme.Color.urgent
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }
    }

    Button {
      id: notNowButton
      objectName: "osdOfferNotNow"
      anchors.right: switchButton.left
      anchors.rightMargin: Style.spacing.controlGap
      anchors.verticalCenter: parent.verticalCenter
      text: "Not now"
      bordered: true
      enabled: !root.busy
      onClicked: root.notNowRequested()
    }

    Button {
      id: switchButton
      objectName: "osdOfferSwitch"
      anchors.right: parent.right
      anchors.rightMargin: root.pad
      anchors.verticalCenter: parent.verticalCenter
      text: root.switching || root.restarting ? "Switching…" : "Switch"
      iconText: "󰑓"
      iconSpinning: root.switching || root.restarting
      bordered: true
      enabled: !root.busy
      onClicked: root.switchRequested()
    }
  }
}
