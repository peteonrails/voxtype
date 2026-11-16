import QtQuick
import Quickshell
import Quickshell.Io
import "voxtype-meetings"

// A separate application entrypoint: history works even when the OSD is off.
ShellRoot {
  id: root
  property bool opened: false
  property var hosts: ({})
  property var activeHost: null

  function openFor(path) {
    var host = root.hosts[path]
    if (!host) {
      host = hostFactory.createObject(root, { configPath: path })
      if (!host) return "Could not create the history window"
      var next = Object.assign({}, root.hosts)
      next[path] = host
      root.hosts = next
    }
    root.activeHost = host
    root.opened = true
    host.window.refresh()
    return "ok"
  }

  Component {
    id: hostFactory
    Item {
      id: host
      required property string configPath
      readonly property alias window: historyWindow
      HistoryCli {
        id: client
        bin: Quickshell.env("VOXTYPE_HISTORY_BIN") || "voxtype"
        configPath: host.configPath
      }
      HistoryWindow {
        id: historyWindow
        client: client
        visible: root.opened && root.activeHost === host
        onPasteRequested: if (root.activeHost === host) root.opened = false
        onPasteFailed: if (root.activeHost === host) root.opened = true
        onVisibleChanged: {
          if (!visible && root.opened && root.activeHost === host) root.opened = false
        }
      }
    }
  }

  Component.onCompleted: root.openFor(Quickshell.env("VOXTYPE_HISTORY_CONFIG") || "")
  IpcHandler {
    target: "history"
    function open(): string { return root.openFor("") }
    function openConfig(path: string): string { return root.openFor(path) }
    function close(): string { root.opened = false; return "ok" }
  }
}
