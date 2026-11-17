import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// What a model row shows while it is downloading: a slim bar, the percentage,
// and a cancel.
//
// `indeterminate` is a real state, not a placeholder. A voxtype build whose
// `setup --download` does not speak the NDJSON progress contract yet reports no
// percentage at all, and the honest display for that is a bar that moves
// without claiming a number.
Item {
  id: root

  // Below zero means "no percentage known".
  property real pct: -1
  readonly property bool indeterminate: pct < 0

  // The file currently being fetched. Progress is per file, so the bar runs
  // 0→100 once per file a model is made of; naming the file is what stops the
  // restart reading as the bar going backwards.
  property string file: ""

  property bool cancelling: false

  signal cancelRequested()

  implicitHeight: Math.max(cancel.implicitHeight, bar.height, label.implicitHeight)
  implicitWidth: row.implicitWidth

  Row {
    id: row
    anchors.right: parent.right
    anchors.verticalCenter: parent.verticalCenter
    spacing: Style.spacing.controlGap

    Text {
      anchors.verticalCenter: parent.verticalCenter
      visible: root.file !== "" && !root.cancelling
      width: Style.space(150)
      horizontalAlignment: Text.AlignRight
      text: root.file
      color: Qt.darker(ShellTheme.Color.foreground, 2.0)
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      elide: Text.ElideMiddle
    }

    Item {
      id: bar
      anchors.verticalCenter: parent.verticalCenter
      width: Style.space(120)
      height: Style.space(6)

      Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: Util.alpha(ShellTheme.Color.foreground, 0.15)
      }

      Rectangle {
        id: fill
        height: parent.height
        radius: height / 2
        color: ShellTheme.Color.accent
        // A third of the track, swept back and forth, when there is no figure
        // to fill to.
        width: root.indeterminate
          ? Math.round(parent.width / 3)
          : Math.round(parent.width * Math.max(0, Math.min(100, root.pct)) / 100)

        Behavior on width {
          enabled: !root.indeterminate
          NumberAnimation { duration: 120 }
        }

        SequentialAnimation on x {
          running: root.indeterminate && root.visible
          loops: Animation.Infinite
          NumberAnimation { from: 0; to: bar.width - fill.width; duration: 900; easing.type: Easing.InOutQuad }
          NumberAnimation { from: bar.width - fill.width; to: 0; duration: 900; easing.type: Easing.InOutQuad }
        }

        // The sweep animation owns x while indeterminate; put it back at the
        // left edge once a real percentage starts arriving.
        onXChanged: if (!root.indeterminate && x !== 0) x = 0
      }
    }

    Text {
      id: label
      anchors.verticalCenter: parent.verticalCenter
      width: Style.space(46)
      horizontalAlignment: Text.AlignRight
      text: root.cancelling
        ? "stopping"
        : (root.indeterminate ? "working" : Math.round(root.pct) + "%")
      color: Qt.darker(ShellTheme.Color.foreground, 1.5)
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
    }

    PanelActionButton {
      id: cancel
      anchors.verticalCenter: parent.verticalCenter
      enabled: !root.cancelling
      opacity: root.cancelling ? 0.45 : 1.0
      iconText: "󰅖"
      tooltipText: "Cancel download"
      onClicked: root.cancelRequested()
    }
  }
}
