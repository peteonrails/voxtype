import QtQuick

// The Meetings view of the Voxtype plugin, everything but its registration in
// Panel.qml: the live feed from meeting.sock, the `voxtype meeting` commands,
// the window, and opening it when a meeting starts.
//
// The plugin is keepLoaded, so this exists for as long as the shell does and
// the feed stays connected while the view is closed. That is how the view
// opens on a meeting started anywhere (the CLI, the detection prompt's Record
// button, the tray): the daemon announces every start on the socket, so there
// is nothing to wire into each of those paths.
Item {
  id: root

  property string bin: "voxtype"
  // The view is up.
  property bool shown: false
  // The plugin is open in any view. A meeting start never takes over an open
  // Settings panel.
  property bool panelOpen: false

  signal openRequested()
  signal closeRequested()

  readonly property alias feed: feed
  readonly property alias cli: meetingCli
  readonly property var window: windowLoader.item

  property string actionError: ""

  MeetingFeed { id: feed }

  MeetingCli {
    id: meetingCli
    bin: root.bin
    onLabelApplied: function(key, name) { feed.applyLabel([key], name) }
    onActionFinished: function(action, ok, message) {
      if (!ok) root.actionError = message
    }
    // The line was moved on screen before the CLI ran: keep it if it took
    // (with the daemon's version of the segment when --json gave one), put it
    // back if not.
    onReassignFinished: function(extra, ok, message, segment) {
      if (!ok) {
        feed.restoreSnapshot(extra ? extra.snapshot : null)
        return
      }
      if (segment) feed.updateSegment(segment, extra && extra.restore === true)
    }
    // `[meeting] show_view_on_start`, read at the moment it is needed so a
    // change made with `voxtype config set` applies to the next meeting.
    onShowOnStartRead: function(show) {
      if (show && !root.panelOpen) root.openRequested()
    }
  }

  Connections {
    target: feed
    function onMeetingStarted() {
      if (!root.panelOpen) meetingCli.readShowOnStart()
    }
  }

  // Move one transcript line to a speaker: `speaker` is what the CLI is given
  // (a display name, or a new name), `key` the speaker key to show meanwhile.
  function reassign(segId, speaker, key) {
    var snap = feed.snapshot(segId)
    if (!snap) return
    root.actionError = ""
    feed.applyReassign(segId, key)
    meetingCli.reassign(feed.meetingId, segId, speaker, { snapshot: snap, restore: false })
  }

  function restoreSpeaker(segId) {
    var snap = feed.snapshot(segId)
    if (!snap) return
    root.actionError = ""
    feed.applyRestore(segId)
    meetingCli.restoreSpeaker(feed.meetingId, segId, { snapshot: snap, restore: true })
  }

  onShownChanged: {
    // A finished meeting stays on screen until the view is closed.
    if (!root.shown) {
      feed.clearEnded()
      return
    }
    root.actionError = ""
    feed.connectNow()
  }

  // Loaded by URL, not as an inline component: a PanelWindow type needs the
  // Wayland backend just to compile, and the Settings panel's offscreen tests
  // (and any session without layer-shell) must still load the plugin.
  Loader {
    id: windowLoader
    active: root.shown
    source: active ? Qt.resolvedUrl("MeetingWindow.qml") : ""
    onLoaded: {
      var view = windowLoader.item.view
      view.feed = feed
      view.busy = Qt.binding(function() { return meetingCli.busy })
      view.actionError = Qt.binding(function() { return root.actionError })
      windowLoader.item.visible = true
    }
  }

  Connections {
    target: windowLoader.item ? windowLoader.item.view : null
    function onPauseRequested() { root.actionError = ""; meetingCli.pause() }
    function onResumeRequested() { root.actionError = ""; meetingCli.resume() }
    function onStopRequested() { root.actionError = ""; meetingCli.stop() }
    function onStartRequested(title) { root.actionError = ""; meetingCli.start(title) }
    function onRenameRequested(keys, name) {
      root.actionError = ""
      var list = []
      for (var i = 0; i < keys.length; i++) list.push(String(keys[i]))
      meetingCli.label(feed.meetingId, list, name)
    }
    function onReassignRequested(segId, speaker, key) { root.reassign(segId, speaker, key) }
    function onRestoreRequested(segId) { root.restoreSpeaker(segId) }
    function onCloseRequested() { root.closeRequested() }
    function onErrorDismissed() { root.actionError = "" }
  }
}
