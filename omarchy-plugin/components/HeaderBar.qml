import QtQuick
import QtQuick.Layouts
import Quickshell
import qs.Commons
import qs.Ui

// Title row: what this panel is, whether the GPU is actually being used, and the
// four actions that are not settings (restart, drop to the TUI, edit the config
// file by hand, close).
//
// The daemon's own state is deliberately *not* here — it is reported once, on the
// runtime facts line below. It used to appear in both places, which meant reading
// "Daemon idle" twice on one screen.
Item {
  id: root

  property string title: "Voxtype Settings"
  property string voxtypeVersion: ""
  // What the running daemon reports, which is not always what the CLI that
  // rendered this panel is. An upgrade installed but never restarted leaves
  // the two apart, and the daemon's number is the one that describes the
  // behaviour the user is about to configure.
  property string daemonVersionLabel: ""
  property bool daemonVersionDiffers: false
  property string configPath: ""
  property bool restarting: false

  // From `voxtype info accel --json`; the badge draws itself or nothing.
  property string accelState: ""
  property string accelBackend: ""
  property string accelEvidence: ""

  signal restartRequested()
  signal tuiRequested()
  signal editConfigRequested()
  signal closeRequested()

  implicitHeight: layout.implicitHeight

  readonly property color dim: Qt.darker(Color.foreground, 1.5)

  // A Button sizes itself to its content, so one without an icon comes out
  // shorter than its neighbours — which is exactly how "Open TUI" ended up
  // visibly smaller than the buttons either side of it. Rather than trust every
  // button to have matching content forever, the row takes the tallest natural
  // height and gives it to all of them, with the control-height token as the
  // floor. Nothing here is a pixel measurement, so it follows the theme's
  // spacing tokens and cannot drift again.
  readonly property int actionHeight: Math.max(
    Style.spacing.controlHeight,
    restartAction.implicitHeight,
    tuiAction.implicitHeight,
    editAction.implicitHeight,
    closeAction.implicitHeight)
  // Display only — the path handed to the editor stays absolute. Writing the
  // home directory as ~ is both how users refer to it and nine characters the
  // subtitle no longer has to elide away.
  readonly property string displayConfigPath: {
    var path = String(root.configPath)
    if (path === "") return ""
    var home = String(Quickshell.env("HOME") || "")
    if (home !== "" && path.indexOf(home + "/") === 0)
      return "~" + path.substring(home.length)
    return path
  }

  readonly property string subtitle: {
    var parts = []
    if (root.daemonVersionLabel !== "") parts.push("daemon " + root.daemonVersionLabel)
    else if (root.voxtypeVersion !== "") parts.push("voxtype " + root.voxtypeVersion)
    if (root.displayConfigPath !== "") parts.push(root.displayConfigPath)
    return parts.join("  ·  ")
  }

  RowLayout {
    id: layout
    anchors.left: parent.left
    anchors.right: parent.right
    anchors.verticalCenter: parent.verticalCenter
    spacing: Style.spacing.controlGap

    ColumnLayout {
      Layout.fillWidth: true
      spacing: Style.spacing.xxs

      Text {
        text: root.title
        color: Color.foreground
        font.family: Style.font.family
        font.pixelSize: Style.font.heading
        font.bold: true
      }

      Text {
        visible: root.subtitle !== ""
        Layout.fillWidth: true
        text: root.subtitle
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        elide: Text.ElideMiddle
      }
    }

    // Where the daemon-state dot and label used to be. This answers a question
    // the buttons beside it cannot: whether the acceleration the user configured
    // is the acceleration they are getting.
    AccelBadge {
      Layout.alignment: Qt.AlignVCenter
      // The same height the buttons take, so the row is one line of chips rather
      // than a short badge next to tall buttons.
      Layout.preferredHeight: root.actionHeight
      accelState: root.accelState
      backend: root.accelBackend
      evidence: root.accelEvidence
    }

    Button {
      id: restartAction
      Layout.alignment: Qt.AlignVCenter
      Layout.preferredHeight: root.actionHeight
      text: root.restarting ? "Restarting…" : "Restart daemon"
      iconText: "󰑐"
      bordered: true
      focusable: true
      iconSpinning: root.restarting
      onClicked: root.restartRequested()
    }

    Button {
      id: tuiAction
      Layout.alignment: Qt.AlignVCenter
      Layout.preferredHeight: root.actionHeight
      text: "Open TUI"
      iconText: "󰆍"
      bordered: true
      focusable: true
      onClicked: root.tuiRequested()
    }

    Button {
      id: editAction
      Layout.alignment: Qt.AlignVCenter
      Layout.preferredHeight: root.actionHeight
      // Nothing to point an editor at until the schema has reported the path.
      visible: root.configPath !== ""
      text: "Edit config"
      iconText: "󰈙"
      bordered: true
      focusable: true
      onClicked: root.editConfigRequested()
    }

    PanelActionButton {
      id: closeAction
      Layout.alignment: Qt.AlignVCenter
      Layout.preferredHeight: root.actionHeight
      Layout.preferredWidth: root.actionHeight
      iconText: "󰅖"
      tooltipText: "Close (Esc)"
      bordered: true
      focusable: true
      onClicked: root.closeRequested()
    }
  }
}
