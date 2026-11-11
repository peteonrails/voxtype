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

  // ------------------------------------------------------------ past meetings

  // `voxtype meeting list --json`, newest first, for the view's idle screen.
  property var pastMeetings: []
  property int listLimit: 50
  property string listError: ""
  readonly property bool listLoading: listProc.running || listProc.startPending
  readonly property bool listHasMore: root.pastMeetings.length >= root.listLimit

  function refreshList() {
    if (listProc.running || listProc.startPending) {
      listProc.again = true
      return
    }
    listProc.out = ""
    listProc.err = ""
    listProc.launch([root.bin, "meeting", "list", "--json", "--limit", String(root.listLimit)])
  }

  function loadMoreMeetings() {
    root.listLimit += 50
    root.refreshList()
  }

  // A past meeting, shown the way an ended one is.
  function openPast(meetingId) {
    root.actionError = ""
    storedProc.load(meetingId)
  }

  // Back to the list from an ended or past meeting.
  function back() {
    feed.clearEnded()
    root.refreshList()
  }

  MeetingProcess {
    id: listProc
    property string out: ""
    property string err: ""
    property bool again: false
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: listProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: listProc.err = String(text || "").trim() }
    onExited: function(code) {
      if (listProc.again) {
        listProc.again = false
        Qt.callLater(root.refreshList)
        return
      }
      if (code !== 0) {
        root.listError = listProc.err || "voxtype meeting list exited " + code
        return
      }
      var parsed = null
      try { parsed = JSON.parse(listProc.out) } catch (e) {}
      if (!parsed || parsed.length === undefined) {
        root.listError = "voxtype meeting list did not return a list"
        return
      }
      root.listError = ""
      root.pastMeetings = parsed
    }
    onFailedToStart: root.listError = "Could not run " + root.bin
  }

  Connections {
    target: feed
    // A meeting that ended belongs at the top of the list.
    function onTransitioned(from, to) { if (to === "idle" && root.shown) root.refreshList() }
    function onHasMeetingChanged() { if (!feed.hasMeeting && root.shown) root.refreshList() }
  }

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
    var keepHighlight = snap.highlight && String(text).trim() !== ""
    feed.setAnnotation(segId, text, keepHighlight)
    meetingCli.annotate(feed.meetingId, segId, text, keepHighlight, { snapshot: snap })
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
    root.refreshList()
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
      if (code !== 0 || !data || !feed.loadStored(data)) {
        root.actionError = storedProc.err || "Could not open that meeting."
        return
      }
      if (data.metadata && data.metadata.summary) assistant.setStoredSummary(feed.meetingId, data.metadata.summary)
      root.showTab("transcript")
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
      view.pastMeetings = Qt.binding(function() { return root.pastMeetings })
      view.listLoading = Qt.binding(function() { return root.listLoading })
      view.listError = Qt.binding(function() { return root.listError })
      view.listHasMore = Qt.binding(function() { return root.listHasMore })
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
    function onOpenMeetingRequested(meetingId) { root.openPast(meetingId) }
    function onMoreMeetingsRequested() { root.loadMoreMeetings() }
    function onBackRequested() { root.back() }
    function onCloseRequested() { root.closeRequested() }
    function onErrorDismissed() { root.actionError = "" }
  }
}
