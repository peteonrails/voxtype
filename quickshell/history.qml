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

  // The active Omarchy theme, when Omarchy is installed: its colours file is
  // read here so the window matches the desktop and follows a theme switch.
  // Anywhere else the file is missing and the panel keeps its own palette.
  readonly property string home: Quickshell.env("HOME") || ""
  readonly property var themePaths: [
    home + "/.local/state/omarchy/current/theme/colors.toml",
    home + "/.config/omarchy/current/theme/colors.toml"
  ]
  property int themePathIndex: 0
  property string themeText: ""
  property var themeColors: null

  function parseColors(text) {
    var found = {}
    var lines = String(text).split("\n")
    for (var i = 0; i < lines.length; ++i) {
      var match = /^\s*([A-Za-z0-9_]+)\s*=\s*"(#[0-9A-Fa-f]{6}(?:[0-9A-Fa-f]{2})?)"/.exec(lines[i])
      if (match) found[match[1]] = match[2]
    }
    return found.foreground && found.background ? found : null
  }

  FileView {
    id: themeFile
    path: root.home === "" ? "" : root.themePaths[root.themePathIndex]
    watchChanges: true
    printErrors: false
    onLoaded: {
      var content = text()
      if (content === root.themeText) return
      root.themeText = content
      root.themeColors = root.parseColors(content)
    }
    onLoadFailed: {
      if (root.themePathIndex + 1 < root.themePaths.length) root.themePathIndex += 1
      else { root.themeText = ""; root.themeColors = null }
    }
    onFileChanged: reload()
  }
  // Omarchy switches themes by re-pointing a symlink, which a watch on the
  // old file does not always see.
  Timer {
    interval: 5000
    repeat: true
    running: root.opened && root.home !== ""
    onTriggered: themeFile.reload()
  }

  MeetingTheme {
    id: omarchyTheme
    fg: root.themeColors ? root.themeColors.foreground : "#dcdfe4"
    bg: root.themeColors ? root.themeColors.background : "#1a1a1f"
    accent: root.themeColors ? (root.themeColors.accent || root.themeColors.color4 || root.themeColors.foreground) : "#6e89c2"
    border: accent
    urgent: root.themeColors && root.themeColors.color1 ? root.themeColors.color1 : "#e06c75"
  }

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
        theme: root.themeColors ? omarchyTheme : null
        visible: root.opened && root.activeHost === host
        onPasteRequested: if (root.activeHost === host) root.opened = false
        onPasteFailed: if (root.activeHost === host) root.opened = true
        onCloseRequested: if (root.activeHost === host) root.opened = false
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
