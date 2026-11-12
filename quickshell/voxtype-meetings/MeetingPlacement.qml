import QtQuick
import Quickshell
import Quickshell.Io

// Where the meeting view sits and how big it is: position, width, full or
// compact height, minimized. Remembered per host and per output in
// $XDG_STATE_HOME/voxtype/meeting-view.json, so the card comes back where it
// was left.
//
// Layer-shell has no window position, only anchors and margins: the window
// anchors top and left and these are its margins (plus bottom too in full
// height, so the card fills the height the bar leaves). `x` and `y` are the
// card's left and top in screen coordinates.
QtObject {
  id: root

  // "omarchy" or "standalone": positions differ between shells.
  property string host: "standalone"
  // The output, for the per-output position; set from the window's screen.
  property string screenName: ""
  property real screenWidth: 1920
  property real screenHeight: 1080

  // Space kept from a screen edge.
  property int gap: 10
  // A drag that ends this close to an edge lands on it.
  property int snap: 24
  property int minWidth: 280
  property int defaultWidth: 460
  // The pill a minimized card becomes: as wide as what it says (the view
  // sets pillContentWidth), within these bounds.
  property int pillMinWidth: 140
  property int pillMaxWidth: 380
  property real pillContentWidth: 200
  readonly property real pillWidth: Math.max(root.pillMinWidth, Math.min(root.pillMaxWidth, Math.round(root.pillContentWidth)))
  property int pillHeight: 40
  // The pill keeps the card's outer edge: its right edge for a card on the
  // right half of the screen, its left edge otherwise. Decided when it
  // minimizes, so the pill does not jump as its text grows.
  property bool pillRight: true
  onMinimizedChanged: {
    if (root.minimized) {
      root.pillRight = root.cardLeft + root.cardWidth / 2 > root.screenWidth / 2
    } else {
      // Back to a card that fits the screen wherever the pill went.
      root.x = Math.max(0, Math.min(root.screenWidth - root.cardWidth, root.cardLeft))
    }
  }

  // -1 until placed: the card starts against the right edge.
  property real x: -1
  property real y: -1
  property real width: root.defaultWidth
  property bool compact: false
  property bool minimized: false

  readonly property real maxWidth: Math.max(root.minWidth, Math.round(root.screenWidth / 2))
  readonly property real cardWidth: root.clampWidth(root.width)
  readonly property real compactHeight: Math.round(root.screenHeight * 0.4)
  // Full height is the anchors' business; this is only the estimate the
  // math needs (snapping, the pill's place).
  readonly property real cardHeight: root.compact ? root.compactHeight : root.screenHeight - 2 * root.gap
  readonly property real cardLeft: root.x < 0 ? root.screenWidth - root.cardWidth - root.gap : root.x
  readonly property real pillLeft: root.cardLeft + (root.pillRight ? root.cardWidth - root.pillWidth : 0)
  readonly property real left: root.minimized ? root.pillLeft : root.cardLeft
  readonly property real top: root.compact || root.minimized ? (root.y < 0 ? root.gap : root.y) : root.gap
  // Full height anchors the bottom too; compact and the pill do not.
  readonly property bool fullHeight: !root.compact && !root.minimized
  readonly property real windowWidth: root.minimized ? root.pillWidth : root.cardWidth
  readonly property real windowHeight: root.minimized ? root.pillHeight : root.cardHeight

  // The edge the resize handle sits on: the one facing the screen's middle.
  readonly property bool handleOnLeft: root.cardLeft + root.cardWidth / 2 > root.screenWidth / 2

  function clampWidth(w) { return Math.max(root.minWidth, Math.min(root.maxWidth, Math.round(w))) }

  // Within `snap` of an edge, onto it (a `gap` in).
  function snapX(x, w) {
    if (x < root.gap + root.snap) return root.gap
    if (root.screenWidth - (x + w) < root.gap + root.snap) return root.screenWidth - w - root.gap
    return Math.round(x)
  }
  function snapY(y, h) {
    if (y < root.gap + root.snap) return root.gap
    if (root.screenHeight - (y + h) < root.gap + root.snap) return root.screenHeight - h - root.gap
    return Math.round(y)
  }

  // Move by the pointer's travel since the last event (the window follows the
  // margins, so the pointer is back where it was pressed in it). Clamped to
  // the screen; snapped when the drag ends.
  function moveBy(dx, dy) {
    var w = root.windowWidth
    var h = root.windowHeight
    root.setLeft(Math.max(0, Math.min(root.screenWidth - w, root.left + dx)))
    if (!root.fullHeight) root.y = Math.max(0, Math.min(root.screenHeight - h, root.top + dy))
  }

  function endMove() {
    root.setLeft(root.snapX(root.left, root.windowWidth))
    if (!root.fullHeight) root.y = root.snapY(root.top, root.windowHeight)
    root.save()
  }

  // Put the window's left edge here: the card's own, or the pill's, which
  // sits at a fixed offset from the card's.
  function setLeft(left) {
    root.x = root.minimized && root.pillRight ? left + root.pillWidth - root.cardWidth : left
  }

  // A drag of the inner edge: on the left edge the card keeps its right side.
  function resizeBy(dx) {
    var before = root.cardWidth
    if (root.handleOnLeft) {
      var right = root.cardLeft + before
      root.width = root.clampWidth(before - dx)
      root.x = right - root.cardWidth
    } else {
      root.width = root.clampWidth(before + dx)
    }
  }

  function endResize() {
    root.x = Math.max(0, Math.min(root.screenWidth - root.cardWidth, root.cardLeft))
    root.save()
  }

  function toggleCompact() {
    root.compact = !root.compact
    if (root.compact && root.y < 0) root.y = root.gap
    root.save()
  }

  function reset() {
    root.x = -1
    root.y = -1
    root.width = root.defaultWidth
    root.compact = false
    root.save()
  }

  // ------------------------------------------------------------ persistence

  readonly property string key: root.host + ":" + (root.screenName !== "" ? root.screenName : "default")
  readonly property string statePath: {
    var dir = Quickshell.env("XDG_STATE_HOME")
    return (dir ? String(dir) : String(Quickshell.env("HOME")) + "/.local/state") + "/voxtype/meeting-view.json"
  }
  property var saved: ({})
  property bool loaded: false

  function apply() {
    var p = root.saved[root.key]
    if (!p) return
    if (p.x !== undefined) root.x = Number(p.x)
    if (p.y !== undefined) root.y = Number(p.y)
    if (p.width !== undefined) root.width = Number(p.width)
    if (p.compact !== undefined) root.compact = p.compact === true
  }

  function save() {
    var all = ({})
    for (var k in root.saved) all[k] = root.saved[k]
    all[root.key] = { x: root.x, y: root.y, width: root.cardWidth, compact: root.compact }
    root.saved = all
    saveDebounce.restart()
  }

  onKeyChanged: if (root.loaded) root.apply()

  property Timer saveDebounce: Timer {
    interval: 300
    onTriggered: {
      stateDir.running = true
    }
  }

  // The directory may not exist on a first run.
  property Process stateDir: Process {
    command: ["mkdir", "-p", root.statePath.replace(/\/[^\/]*$/, "")]
    onExited: stateFile.setText(JSON.stringify(root.saved, null, 2) + "\n")
  }

  property FileView stateFile: FileView {
    path: root.statePath
    printErrors: false
    atomicWrites: true
    onLoaded: {
      try {
        var parsed = JSON.parse(text())
        if (parsed && typeof parsed === "object") root.saved = parsed
      } catch (e) {}
      root.loaded = true
      root.apply()
    }
    onLoadFailed: root.loaded = true
  }
}
