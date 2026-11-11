import QtQuick
import Quickshell
import Quickshell.Io
import "MeetingText.js" as MT

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
//   {"meeting_id": "<id>"|null, "ask_id": "<id>"|null,
//    "tab": "transcript"|"notes"|"ask"|"summary"|null,
//    "ask": "last-5m"|"last-speaker"|"actions"|null, "open": bool,
//    "requested_at": <unix ms>}
// Every running host reads it, so the CLI needs to know none of them. As
// `[meeting.assistant] open_command`, plain `voxtype meeting view` picks the
// meeting and answer up from VOXTYPE_MEETING_ID and VOXTYPE_ASK_ID.
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
  // "omarchy" or "standalone": named in meeting-view.state, and positions
  // are remembered per host.
  property string host: "standalone"

  // Where the card is, how big, and whether it is the pill. Kept here, not in
  // the window, so it outlives closing the view.
  readonly property alias placement: placement
  MeetingPlacement {
    id: placement
    host: root.host
    gap: root.theme.margin
  }

  // Collapse to the pill, or back to the card (opening the view if it is
  // not up). One window either way.
  function minimize() {
    if (root.window) root.window.view.closeOverlays()
    placement.minimized = true
  }
  function expand() {
    placement.minimized = false
    if (!root.shown) root.openRequested()
  }

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
  // An answer to show once its meeting is loaded (a notification's Open).
  property string pendingAskId: ""

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
    root.commitDelete()
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
        root.listError = MT.cliProblem(listProc.err, "the meetings list") || "voxtype meeting list exited " + code
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
    onFailedToStart: root.listError = MT.missingCli(root.bin)
  }

  // ------------------------------------------------------------ daemon

  // While meeting.sock cannot be reached: is the daemon down, or running
  // without the socket (a voxtype from before it, or one that failed to bind
  // it)? `voxtype status` exits 0 when a daemon answers. The view says which.
  property bool daemonRunning: false
  Timer {
    interval: 4000
    repeat: true
    triggeredOnStart: true
    running: root.shown && !feed.connected
    onTriggered: if (!daemonProc.running && !daemonProc.startPending) daemonProc.launch([root.bin, "status", "--format", "json"])
  }
  MeetingProcess {
    id: daemonProc
    onExited: function(code) { root.daemonRunning = code === 0 }
    onFailedToStart: root.daemonRunning = false
  }

  // ------------------------------------------------------------ copy

  // The command a Copy runs, text on stdin. wl-clipboard's, which Omarchy and
  // most Wayland desktops ship.
  property string copyCommand: "wl-copy"
  signal copied(string key)

  function copyText(text, key) {
    if (copyProc.running || copyProc.startPending) return
    copyProc.text = String(text || "")
    copyProc.key = key
    copyProc.err = ""
    copyProc.stdinEnabled = true
    copyProc.launch([root.copyCommand])
  }

  MeetingProcess {
    id: copyProc
    property string text: ""
    property string key: ""
    property string err: ""
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: copyProc.err = String(text || "").trim() }
    onExited: function(code) {
      if (code === 0) root.copied(copyProc.key)
      else root.actionError = "Could not copy: " + (copyProc.err || root.copyCommand + " exited " + code)
    }
    onFailedToStart: root.actionError = "Copying needs " + root.copyCommand + " (the wl-clipboard package), which is not installed."
  }
  Connections {
    target: copyProc
    function onStarted() {
      copyProc.write(copyProc.text)
      copyProc.stdinEnabled = false
    }
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
    onSegmentActionFinished: function(action, extra, ok, message, segment) {
      if (action === "edit") {
        if (!ok && extra && feed.textOf(extra.segId) !== null) {
          feed.setText(extra.segId, extra.before)
          feed.setOriginalText(extra.segId, extra.original)
        } else if (ok && segment) {
          feed.updateSegment(segment)
        }
      } else if (action === "delete-line" && !ok && extra && extra.meeting === feed.meetingId) {
        feed.restoreRow(extra.row)
      }
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
      if (!show) return
      // A pill expands; a closed view opens, unless the host has something
      // else open.
      if (root.shown) placement.minimized = false
      else if (!root.panelOpen) root.openRequested()
    }
  }

  Connections {
    target: root
    function onCopied(key) { if (root.window) root.window.view.noteCopied(key) }
  }

  Connections {
    target: feed
    function onMeetingStarted() {
      if (root.shown ? placement.minimized : !root.panelOpen) meetingCli.readShowOnStart()
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

  function annotate(segId, text, highlight) {
    var snap = feed.annotationSnapshot(segId)
    if (!snap) return
    root.actionError = ""
    feed.setAnnotation(segId, text, highlight)
    meetingCli.annotate(feed.meetingId, segId, text, highlight, { snapshot: snap })
  }

  // ------------------------------------------------------------ line edits

  // A line's text, shown at once and put back if the CLI refuses it.
  function editText(segId, text) {
    var before = feed.textOf(segId)
    var original = feed.originalTextOf(segId)
    var next = String(text).trim()
    if (before === null || next === "" || next === before) return
    root.actionError = ""
    feed.setText(segId, next)
    // The first correction keeps what was transcribed.
    if (original === "") feed.setOriginalText(segId, before)
    meetingCli.editText(feed.meetingId, segId, next, { segId: segId, before: before, original: original })
  }

  // "Restore original text": the line's transcribed text back.
  function resetText(segId) {
    var before = feed.textOf(segId)
    var original = feed.originalTextOf(segId)
    if (before === null || !original) return
    root.actionError = ""
    feed.setText(segId, original)
    feed.setOriginalText(segId, "")
    meetingCli.resetText(feed.meetingId, segId, { segId: segId, before: before, original: original })
  }

  // Delete with an undo window: the line goes at once, and the CLI only runs
  // when the window closes (or another delete, Back, or closing the view
  // accepts it). Undo inside the window needs no command at all.
  property var pendingDelete: null
  readonly property int undoMs: 5000

  function deleteLine(segId) {
    root.commitDelete()
    var row = feed.removeRow(segId)
    if (!row) return
    root.actionError = ""
    root.pendingDelete = { segId: segId, row: row, meeting: feed.meetingId }
    undoTimer.restart()
  }

  function undoDelete() {
    undoTimer.stop()
    var p = root.pendingDelete
    root.pendingDelete = null
    if (p && p.meeting === feed.meetingId) feed.restoreRow(p.row)
  }

  function commitDelete() {
    undoTimer.stop()
    var p = root.pendingDelete
    root.pendingDelete = null
    if (p) meetingCli.deleteLine(p.meeting, p.segId, p)
  }

  Timer { id: undoTimer; interval: root.undoMs; onTriggered: root.commitDelete() }

  Connections {
    target: feed
    function onMeetingIdChanged() { if (root.pendingDelete && root.pendingDelete.meeting !== feed.meetingId) root.commitDelete() }
  }

  // ------------------------------------------------------------ export, folders

  // Exports go to the user's documents, where they will look for them, not
  // the meeting's own folder under ~/.local/share.
  readonly property string exportDir: {
    var docs = Quickshell.env("XDG_DOCUMENTS_DIR")
    return (docs ? String(docs) : String(Quickshell.env("HOME")) + "/Documents") + "/Voxtype"
  }
  // The last export: {path, folder}, for the notice and its Open folder.
  property var lastExport: null

  function exportName(format) {
    var started = Date.parse(String(feed.startedAt || "").replace(/(\.\d{3})\d+/, "$1"))
    var day = isNaN(started) ? Qt.formatDate(new Date(), "yyyy-MM-dd") : Qt.formatDate(new Date(started), "yyyy-MM-dd")
    var title = String(feed.title || "Meeting").replace(/[\/\\:*?"<>|\x00-\x1f]+/g, "-").trim().slice(0, 80)
    var ext = { markdown: "md", text: "txt", json: "json", srt: "srt", vtt: "vtt" }[format]
    return day + " " + (title !== "" ? title : "Meeting") + "." + ext
  }

  function exportMeeting(format) {
    if (feed.meetingId === "" || exportProc.running || exportProc.startPending) return
    root.actionError = ""
    root.lastExport = null
    var path = root.exportDir + "/" + root.exportName(format)
    var args = [root.bin, "meeting", "export", feed.meetingId, "--format", format, "--output", path]
    // The readable formats carry who said what, and when.
    if (format === "markdown" || format === "text") args.push("--timestamps", "--speakers", "--metadata")
    exportProc.path = path
    exportProc.err = ""
    exportProc.launch(["sh", "-c", 'mkdir -p "$1" && shift && exec "$@"', "sh", root.exportDir].concat(args))
  }

  MeetingProcess {
    id: exportProc
    property string path: ""
    property string err: ""
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: exportProc.err = String(text || "").trim() }
    onExited: function(code) {
      if (code === 0) {
        var home = String(Quickshell.env("HOME") || "")
        var shown = home !== "" && exportProc.path.indexOf(home + "/") === 0 ? "~" + exportProc.path.slice(home.length) : exportProc.path
        root.lastExport = { path: exportProc.path, folder: root.exportDir, shown: shown }
      }
      else root.actionError = MT.cliProblem(exportProc.err, "exports") || "voxtype meeting export exited " + code
    }
    onFailedToStart: root.actionError = MT.missingCli(root.bin)
  }

  // xdg-open, which opens a folder in the desktop's file manager.
  property string openCommand: "xdg-open"
  function openFolder(path) {
    if (!path || openProc.running || openProc.startPending) return
    openProc.launch([root.openCommand, String(path)])
  }
  MeetingProcess {
    id: openProc
    onExited: function(code) { if (code !== 0) root.actionError = "Could not open the folder (" + root.openCommand + " exited " + code + ")." }
    onFailedToStart: root.actionError = "Opening folders needs " + root.openCommand + " (the xdg-utils package), which is not installed."
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
    var loading = !!(req.meeting_id && req.meeting_id !== "latest" && req.meeting_id !== feed.meetingId && !feed.active)
    root.select(req)
    if (req.ask_id) {
      root.showTab("ask")
      // A meeting still loading shows the answer once it is in.
      if (loading) root.pendingAskId = String(req.ask_id)
      else assistant.showAsk(String(req.ask_id))
    }
    if (req.ask) {
      root.showTab("ask")
      assistant.askChip(String(req.ask))
    }
    if (req.window === "minimize") {
      root.minimize()
      if (!root.shown) root.openRequested()
      return
    }
    if (req.window === "expand") {
      root.expand()
      return
    }
    if (req.open !== false) {
      // Opening a minimized view means wanting to see it.
      placement.minimized = false
      if (!root.shown) root.openRequested()
    }
  }

  onShownChanged: {
    root.writeViewState()
    // A finished meeting stays on screen until the view is closed.
    if (!root.shown) {
      root.commitDelete()
      feed.clearEnded()
      placement.minimized = false
      return
    }
    root.actionError = ""
    feed.connectNow()
    assistant.fetchStatus()
    root.refreshList()
    opacityProc.read()
  }

  // ------------------------------------------------------------ view state
  //
  // $XDG_RUNTIME_DIR/voxtype/meeting-view.state tells the daemon the view is
  // on screen, so it can leave out its meeting notifications:
  //   {"visible": bool, "minimized": bool, "host": "omarchy"|"standalone",
  //    "pid": <this shell's pid>, "updated_at": <unix seconds>}
  // Rewritten whole (tmp and rename) on every show, hide and minimize, and
  // every `stateHeartbeatMs` while visible; a reader treats a dead pid or an
  // `updated_at` older than 30 s as not visible, which covers a crash. A
  // minimized pill is visible.

  property int stateHeartbeatMs: 10000
  // The shell's pid: the parent of a `sh` this process starts.
  property int shellPid: 0
  property bool stateReady: false

  function writeViewState() {
    if (!root.stateReady) return
    var state = { visible: root.shown, minimized: root.shown && placement.minimized, host: root.host,
                  pid: root.shellPid, updated_at: Math.floor(Date.now() / 1000) }
    stateFile.setText(JSON.stringify(state) + "\n")
  }

  Connections {
    target: placement
    function onMinimizedChanged() { root.writeViewState() }
  }

  Timer {
    interval: root.stateHeartbeatMs
    repeat: true
    running: root.shown && root.stateReady
    onTriggered: root.writeViewState()
  }

  FileView {
    id: stateFile
    path: root.runtimeDir + "/meeting-view.state"
    printErrors: false
    atomicWrites: true
  }

  Process {
    id: pidProc
    running: true
    command: ["sh", "-c", 'mkdir -p "$1"; echo $PPID', "sh", root.runtimeDir]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.shellPid = Number(String(text || "").trim()) || 0
    }
    onExited: {
      root.stateReady = true
      root.writeViewState()
    }
  }

  // ------------------------------------------------------------ opacity

  // `[meeting] view_opacity`, read each time the view opens; a voxtype
  // without the key leaves the default.
  property real viewOpacity: 0.8
  MeetingProcess {
    id: opacityProc
    property string out: ""
    function read() {
      if (opacityProc.running || opacityProc.startPending) return
      opacityProc.out = ""
      opacityProc.launch([root.bin, "config", "get", "meeting.view_opacity"])
    }
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: opacityProc.out = String(text || "").trim() }
    onExited: function(code) {
      var v = Number(opacityProc.out)
      if (code === 0 && isFinite(v) && v > 0) root.viewOpacity = Math.max(0.3, Math.min(1.0, v))
    }
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
    onFailedToStart: root.actionError = MT.missingCli(root.bin)
    onExited: function(code) {
      var data = null
      try { data = JSON.parse(storedProc.out) } catch (e) {}
      if (code !== 0 || !data || !feed.loadStored(data)) {
        root.actionError = MT.cliProblem(storedProc.err, "opening past meetings") || "Could not open that meeting."
        return
      }
      if (data.metadata && data.metadata.summary) assistant.setStoredSummary(feed.meetingId, data.metadata.summary)
      if (root.pendingAskId !== "") {
        assistant.showAsk(root.pendingAskId)
        root.pendingAskId = ""
        return
      }
      root.showTab("transcript")
    }
  }

  // ------------------------------------------------------------ window

  // Loaded by URL, not as an inline component: a PanelWindow type needs the
  // Wayland backend just to compile, and the Settings panel's offscreen tests
  // (and any session without layer-shell) must still load the plugin.
  // The window gets the theme and the placement as it is created, so its
  // first layout already has them.
  Loader {
    id: windowLoader
    active: root.shown
    onActiveChanged: if (windowLoader.active) windowLoader.load()
    Component.onCompleted: if (windowLoader.active) windowLoader.load()
    function load() {
      var props = { theme: root.theme, placement: placement }
      if (root.screen) props.screen = root.screen
      windowLoader.setSource(Qt.resolvedUrl("MeetingWindow.qml"), props)
    }
    onLoaded: {
      var w = windowLoader.item
      if (root.screen) w.screen = Qt.binding(function() { return root.screen })
      // The output's size and name, for clamping, snapping and the
      // per-output position.
      placement.screenWidth = Qt.binding(function() { return w.screen ? w.screen.width : 1920 })
      placement.screenHeight = Qt.binding(function() { return w.screen ? w.screen.height : 1080 })
      placement.screenName = Qt.binding(function() { return w.screen && w.screen.name ? String(w.screen.name) : "" })
      var view = w.view
      view.restingOpacity = Qt.binding(function() { return root.viewOpacity })
      view.feed = feed
      view.assistant = assistant
      view.canOpenSettings = Qt.binding(function() { return root.canOpenSettings })
      view.busy = Qt.binding(function() { return meetingCli.busy })
      view.actionError = Qt.binding(function() { return root.actionError })
      view.pastMeetings = Qt.binding(function() { return root.pastMeetings })
      view.listLoading = Qt.binding(function() { return root.listLoading })
      view.listError = Qt.binding(function() { return root.listError })
      view.listHasMore = Qt.binding(function() { return root.listHasMore })
      view.daemonRunning = Qt.binding(function() { return root.daemonRunning })
      view.pendingDelete = Qt.binding(function() { return root.pendingDelete !== null })
      view.lastExport = Qt.binding(function() { return root.lastExport })
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
    function onAnnotateRequested(segId, text, highlight) { root.annotate(segId, text, highlight) }
    function onSettingsRequested() { root.settingsRequested() }
    function onOpenMeetingRequested(meetingId) { root.openPast(meetingId) }
    function onMoreMeetingsRequested() { root.loadMoreMeetings() }
    function onBackRequested() { root.back() }
    function onMinimizeRequested() { root.minimize() }
    function onExpandRequested() { root.expand() }
    function onCopyRequested(text, key) { root.copyText(text, key) }
    function onEditTextRequested(segId, text) { root.editText(segId, text) }
    function onResetTextRequested(segId) { root.resetText(segId) }
    function onDeleteLineRequested(segId) { root.deleteLine(segId) }
    function onUndoDeleteRequested() { root.undoDelete() }
    function onExportRequested(format) { root.exportMeeting(format) }
    function onOpenFolderRequested(path) { root.openFolder(path) }
    function onExportNoticeDismissed() { root.lastExport = null }
    function onCloseRequested() { root.closeRequested() }
    function onErrorDismissed() { root.actionError = "" }
  }
}
