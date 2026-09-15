import QtQuick
import qs.Ui

BarWidget {
  id: root
  moduleName: "io.voxtype.settings"

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: "󰍬"
    tooltipText: "Voxtype settings · Audio"

    onPressed: function(mouseButton) {
      if (mouseButton !== Qt.LeftButton || !root.bar || !root.bar.shell) return
      // The shell owns the existing panel and its IPC handler, including when
      // the icon appears on more than one monitor.
      root.bar.shell.toggle(root.moduleName, JSON.stringify({ section: "Audio" }))
    }
  }
}
