import QtQuick
import Quickshell

// A normal xdg-shell window, shared by integrated and standalone hosts.
// Hosts keep it alive while replay is pending and map the replay signals to
// hide/reopen, rather than quitting and killing the CLI before delivery.
FloatingWindow {
  id: root
  title: "VoxType History"
  implicitWidth: 1000
  implicitHeight: 760
  minimumSize: Qt.size(640, 420)
  color: "#151820"
  property var client: null
  readonly property int entryCount: history.entryCount

  signal pasteRequested()
  signal pasteFailed()

  function refresh() { history.refresh() }


  HistoryPanel {
    id: history
    anchors.fill: parent
    client: root.client
    onPasteRequested: root.pasteRequested()
    onPasteFailed: root.pasteFailed()
  }
}
