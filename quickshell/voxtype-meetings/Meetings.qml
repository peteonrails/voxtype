import QtQuick
import Quickshell
import Quickshell.Io

// The meeting view, everything but the host: the live feed from meeting.sock,
// the `voxtype meeting` commands, the assistant (notes, ask, summary), the
// window, and opening it when a meeting starts or something asks for it.
//
// One implementation for every host. The Omarchy plugin wraps it in
// OmarchyMeetingsHost.qml and the standalone Quickshell shell in
// MeetingControls.qml; a host supplies the theme, says whether the view is
// `shown`, and turns openRequested/closeRequested into its own way of opening
// a window (shell.summon, or a property).
//
// A host keeps this alive for as long as it runs, so the feed stays connected
// while the view is closed. That is how the view opens on a meeting started
// anywhere: the daemon announces every start on the socket.
//
// `voxtype meeting view` (and a notification's Open action, which runs it)
// writes $XDG_RUNTIME_DIR/voxtype/meeting-view.request:
//   {"meeting_id": "<id>"|null, "tab": "transcript"|"notes"|"ask"|"summary"|null,
//    "ask": "last-5m"|"last-speaker"|"actions"|null, "open": bool,
//    "requested_at": <unix ms>}
// Every running host reads it, so the CLI needs to know none of them.
Item {
  id: root

  property string bin: "voxtype"
  property var theme: MeetingTheme {}
  // The view is up.
  property bool shown: false
  // The host has something else open (the Omarchy Settings panel). A meeting
  // start never takes that over.
  property bool panelOpen: false
  // The output to open on, or null for the host's default.
  property var screen: null
  // The host can open voxtype's settings ("Change in Settings").
  property bool canOpenSettings: false

  signal openRequested()
  signal closeRequested()
  signal settingsRequested()

  readonly property alias feed: feed
  readonly property alias cli: meetingCli
  readonly property alias assistant: assistant
  readonly property var window: windowLoader.item

  property string actionError: ""
  // The tab to show when the view next opens, from a request.
  property string pendingTab: ""

  readonly property string runtimeDir: {
    var dir = Quickshell.env("XDG_RUNTIME_DIR")
    return (dir ? String(dir) : "/tmp") + "/voxtype"
  }

  MeetingFeed { id: feed }

  MeetingAssistant {
    id: assistant
    bin: root.bin
    feed: feed
    viewShown: root.shown
    onAssistantError: function(message) { root.actionError = message }
  }

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
      if (segment) feed.updateSegment(segment)
    }
    onAnnotateFinished: function(extra, ok, message, segment) {
      if (!ok) {
        var snap = extra ? extra.snapshot : null
        if (snap) feed.setAnnotation(snap.segId, snap.annotation, snap.highlight)
        return
      }
      if (segment) feed.updateSegment(segment)
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
      if (!root.panelOpen && !root.shown) meetingCli.readShowOnStart()
    }
  }

  // Move one transcript line to a speaker: `speaker` is what the CLI is given
  // (a display name, or a new name), `key` the speaker key to show meanwhile.
  function reassign(segId, speaker, key) {
    var snap = feed.snapshot(segId)
    if (!snap) return
    root.actionError = ""
    feed.applyReassign(segId, key)
    meetingCli.reassign(feed.meetingId, segId, speaker, { snapshot: snap })
  }

  function restoreSpeaker(segId) {
    var snap = feed.snapshot(segId)
    if (!snap) return
    root.actionError = ""
    feed.applyRestore(segId)
    meetingCli.restoreSpeaker(feed.meetingId, segId, { snapshot: snap })
  }

  function annotate(segId, text) {
    var snap = feed.annotationSnapshot(segId)
    if (!snap) return
    root.actionError = ""
    feed.setAnnotation(segId, text, snap.highlight && String(text).trim() !== "")
    meetingCli.annotate(feed.meetingId, segId, text, { snapshot: snap })
  }

  // A host's own open request with a payload ({"tab", "meeting_id"}), such as
  // Omarchy's `{"view":"meetings","tab":"ask"}`.
  function select(payload) {
    if (!payload) return
    var id = payload.meeting_id ? String(payload.meeting_id) : ""
    if (id !== "" && id !== "latest" && id !== feed.meetingId && !feed.active) storedProc.load(id)
    if (payload.tab) root.showTab(String(payload.tab))
  }

  function showTab(tab) {
    root.pendingTab = tab
    if (root.window) root.window.view.selectTab(tab)
  }

  function handleRequest(req) {
    var age = Date.now() - (Number(req.requested_at) || 0)
    if (!(age >= -5000 && age < 30000)) return
    root.select(req)
    if (req.ask) {
      root.showTab("ask")
      assistant.askChip(String(req.ask))
    }
    if (req.open !== false && !root.shown) root.openRequested()
  }

  onShownChanged: {
    // A finished meeting stays on screen until the view is closed.
    if (!root.shown) {
      feed.clearEnded()
      return
    }
    root.actionError = ""
    feed.connectNow()
    assistant.fetchStatus()
  }

  // ------------------------------------------------------------ requests

  FileView {
    id: requestFile
    path: root.runtimeDir + "/meeting-view.request"
    watchChanges: true
    printErrors: false
    onLoaded: {
      var req = null
      try { req = JSON.parse(requestFile.text()) } catch (e) {}
      consumeRequest.running = true
      if (req && typeof req === "object") root.handleRequest(req)
    }
    onFileChanged: reload()
  }

  Process {
    id: consumeRequest
    command: ["rm", "-f", root.runtimeDir + "/meeting-view.request"]
  }

  MeetingProcess {
    id: storedProc
    property string out: ""
    property string err: ""
    function load(id) {
      if (storedProc.running) return
      storedProc.out = ""
      storedProc.err = ""
      storedProc.launch([root.bin, "meeting", "show", id, "--json"])
    }
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: storedProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: storedProc.err = String(text || "").trim() }
    onExited: function(code) {
      var data = null
      try { data = JSON.parse(storedProc.out) } catch (e) {}
      if (code !== 0 || !data || !feed.loadStored(data))
        root.actionError = storedProc.err || "Could not open that meeting."
    }
  }

  // ------------------------------------------------------------ window

  // Loaded by URL, not as an inline component: a PanelWindow type needs the
  // Wayland backend just to compile, and the Settings panel's offscreen tests
  // (and any session without layer-shell) must still load the plugin.
  Loader {
    id: windowLoader
    active: root.shown
    source: active ? Qt.resolvedUrl("MeetingWindow.qml") : ""
    onLoaded: {
      var w = windowLoader.item
      w.theme = root.theme
      if (root.screen) w.screen = Qt.binding(function() { return root.screen })
      var view = w.view
      view.feed = feed
      view.assistant = assistant
      view.canOpenSettings = Qt.binding(function() { return root.canOpenSettings })
      view.busy = Qt.binding(function() { return meetingCli.busy })
      view.actionError = Qt.binding(function() { return root.actionError })
      if (root.pendingTab !== "") view.selectTab(root.pendingTab)
      root.pendingTab = ""
      w.visible = true
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
    function onAnnotateRequested(segId, text) { root.annotate(segId, text) }
    function onSettingsRequested() { root.settingsRequested() }
    function onCloseRequested() { root.closeRequested() }
    function onErrorDismissed() { root.actionError = "" }
  }
}
