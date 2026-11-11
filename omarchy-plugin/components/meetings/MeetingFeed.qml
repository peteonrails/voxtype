import QtQuick
import Quickshell
import Quickshell.Io

// The live meeting, as the daemon publishes it on
// $XDG_RUNTIME_DIR/voxtype/meeting.sock (docs/INTEGRATIONS.md, "Live transcript
// and status"). Non-visual: the view binds to it, and the plugin keeps one
// alive whether or not the view is open, so it can tell when a meeting starts.
//
// Every connection begins with a `hello` snapshot, so the model is rebuilt from
// scratch on each one and a reconnect can never duplicate a line. Between
// snapshots it follows `segments`, `speaker_renamed` and `status`.
//
// A `status` event carries no title, id or start time. When one says a meeting
// is running and this feed has no meeting yet, it reconnects, and the new
// connection's `hello` supplies them.
Item {
  id: root

  property string socketPath: {
    var dir = Quickshell.env("XDG_RUNTIME_DIR")
    return (dir ? String(dir) : "/tmp") + "/voxtype/meeting.sock"
  }

  // Off for tests that want to drive the socket by hand.
  property bool autoConnect: true

  readonly property bool connected: sock.connected
  // At least one hello has been read since the plugin loaded. Until then the
  // status is not known, and a meeting already running is not a "start".
  property bool everConnected: false

  // idle, recording, paused or finalizing; "" until the first hello.
  property string status: ""
  property string meetingId: ""
  property string title: ""
  property string startedAt: ""
  property int chunkCount: 0
  property bool diarization: false
  // Wall-clock ms of the last `segments` event, for the listening indicator.
  property real lastSegmentsAt: 0
  property string lastError: ""

  // speaker key → human name, from segments' speaker_label and from renames
  // made here. Replaced (never mutated) so bindings see each change.
  property var labels: ({})
  // Speaker keys in order of first appearance.
  property var speakerOrder: []

  readonly property bool active: status === "recording" || status === "paused" || status === "finalizing"

  // One row per utterance, kept in start_ms order.
  readonly property alias segments: model
  ListModel { id: model }

  // `from` is the status before this one: "" when the feed had none.
  signal transitioned(string from, string to)
  // A meeting began while this feed was watching: an idle (or ended) meeting
  // gave way to a recording one. Never fired for a meeting already running
  // when the feed first connected.
  signal meetingStarted()

  function speakerKey(segment) {
    if (segment.speaker_id !== undefined && segment.speaker_id !== null && segment.speaker_id !== "")
      return String(segment.speaker_id)
    var source = String(segment.source || "")
    if (source === "microphone") return "You"
    if (source === "loopback") return "Remote"
    return "Unknown"
  }

  function displayName(key) {
    var name = root.labels[key]
    return name !== undefined && name !== "" ? String(name) : String(key)
  }

  // Speaker chips: one per distinct display name, in order of first
  // appearance. Two keys renamed alike collapse onto one chip, matched the way
  // `meeting label` matches names (case-insensitively).
  readonly property var chips: {
    var out = []
    var byName = ({})
    for (var i = 0; i < root.speakerOrder.length; i++) {
      var key = root.speakerOrder[i]
      var name = root.displayName(key)
      var fold = name.toLowerCase()
      if (byName[fold] !== undefined) {
        out[byName[fold]].keys.push(key)
        continue
      }
      byName[fold] = out.length
      out.push({ name: name, keys: [key], index: out.length })
    }
    return out
  }

  function chipIndexFor(key) {
    var fold = root.displayName(key).toLowerCase()
    for (var i = 0; i < root.chips.length; i++)
      if (root.chips[i].name.toLowerCase() === fold) return i
    return -1
  }

  function noteSpeaker(key) {
    if (root.speakerOrder.indexOf(key) !== -1) return
    var next = root.speakerOrder.slice()
    next.push(key)
    root.speakerOrder = next
  }

  function setLabels(next) { root.labels = next }

  function copyLabels() {
    var next = ({})
    for (var k in root.labels) next[k] = root.labels[k]
    return next
  }

  // A rename made here and accepted by `meeting label`: the daemon does not
  // announce it on the socket, so the feed applies it itself. Segments that
  // arrive later carry it in speaker_label anyway.
  function applyLabel(keys, name) {
    var next = root.copyLabels()
    for (var i = 0; i < keys.length; i++) next[keys[i]] = name
    root.labels = next
  }

  function insertSegment(segment) {
    var key = root.speakerKey(segment)
    var row = {
      segId: Number(segment.id),
      startMs: Number(segment.start_ms) || 0,
      endMs: Number(segment.end_ms) || 0,
      text: String(segment.text || "").trim(),
      speaker: key,
      source: String(segment.source || ""),
      chunkId: Number(segment.chunk_id) || 0
    }
    if (row.text === "") return
    for (var d = model.count - 1; d >= 0; d--) {
      if (model.get(d).segId === row.segId) {
        model.set(d, row)
        return
      }
    }
    // Usually the newest line belongs at the end; walk back from there. Equal
    // start times keep arrival order, as the daemon's backlog does.
    var at = model.count
    while (at > 0 && model.get(at - 1).startMs > row.startMs) at--
    model.insert(at, row)
    root.noteSpeaker(key)
    if (segment.speaker_label !== undefined && segment.speaker_label !== null && segment.speaker_label !== ""
        && root.labels[key] !== String(segment.speaker_label)) {
      var next = root.copyLabels()
      next[key] = String(segment.speaker_label)
      root.labels = next
    }
  }

  function setStatus(next) {
    var from = root.status
    if (from === next) return
    root.status = next
    root.transitioned(from, next)
  }

  function reset() {
    model.clear()
    root.labels = ({})
    root.speakerOrder = []
    root.meetingId = ""
    root.title = ""
    root.startedAt = ""
    root.chunkCount = 0
    root.diarization = false
    root.lastSegmentsAt = 0
  }

  function handleHello(ev) {
    var before = root.status
    var beforeId = root.meetingId
    var known = root.everConnected
    root.everConnected = true
    root.reset()
    root.meetingId = ev.meeting_id ? String(ev.meeting_id) : ""
    root.title = ev.title ? String(ev.title) : ""
    root.startedAt = ev.started_at ? String(ev.started_at) : ""
    root.chunkCount = Number(ev.chunk_count) || 0
    root.diarization = !!(ev.diarization && ev.diarization.enabled)
    var list = ev.segments && ev.segments.length !== undefined ? ev.segments : []
    for (var i = 0; i < list.length; i++) if (list[i]) root.insertSegment(list[i])
    var next = String(ev.status || "idle")
    root.setStatus(next)
    // A status event already announced this meeting and asked for this hello;
    // or the daemon restarted into a new one. Either way it is a start only if
    // the feed saw the previous state.
    if (known && next === "recording" && root.meetingId !== "" && root.meetingId !== beforeId
        && (before === "idle" || before === "recording" && beforeId === ""))
      root.meetingStarted()
  }

  function handleEvent(ev) {
    var kind = String(ev.event || "")
    if (kind === "hello") {
      root.handleHello(ev)
    } else if (kind === "segments") {
      var list = ev.segments && ev.segments.length !== undefined ? ev.segments : []
      for (var i = 0; i < list.length; i++) if (list[i]) root.insertSegment(list[i])
      if (ev.chunk_id !== undefined) root.chunkCount = Math.max(root.chunkCount, Number(ev.chunk_id) + 1)
      root.lastSegmentsAt = Date.now()
    } else if (kind === "speaker_renamed") {
      root.renameSpeaker(String(ev.from || ""), String(ev.to || ""))
    } else if (kind === "status") {
      var next = String(ev.status || "")
      if (next === "") return
      if (next === "idle") {
        root.setStatus("idle")
        // The meeting is over; the daemon's next hello is empty too.
        root.reset()
        return
      }
      root.setStatus(next)
      // A start: fetch the title, id and start time this event lacks.
      if (root.meetingId === "") root.reconnect()
    } else if (kind === "error") {
      root.lastError = String(ev.message || "")
    }
  }

  function renameSpeaker(from, to) {
    if (from === "" || to === "" || from === to) return
    for (var i = 0; i < model.count; i++)
      if (model.get(i).speaker === from) model.setProperty(i, "speaker", to)
    var order = root.speakerOrder.slice()
    var at = order.indexOf(from)
    if (at !== -1) {
      if (order.indexOf(to) === -1) order[at] = to
      else order.splice(at, 1)
      root.speakerOrder = order
    }
    // A name given to the old ID moves to the new one, unless the new one
    // already has its own (the daemon's rule for the saved labels too).
    if (root.labels[from] !== undefined) {
      var next = root.copyLabels()
      if (next[to] === undefined) next[to] = next[from]
      delete next[from]
      root.labels = next
    }
  }

  // ------------------------------------------------------------ connection

  function connectNow() {
    retry.attempt = 0
    if (!sock.connected) sock.connected = true
  }

  function reconnect() {
    sock.connected = false
    retry.attempt = 0
    retry.interval = 50
    retry.restart()
  }

  Socket {
    id: sock
    path: root.socketPath
    parser: SplitParser {
      onRead: function(line) {
        var text = String(line).trim()
        if (text === "") return
        var ev = null
        try { ev = JSON.parse(text) } catch (e) {
          console.warn("Voxtype meetings: unreadable meeting.sock line: " + text.slice(0, 80))
          return
        }
        if (ev && typeof ev === "object") root.handleEvent(ev)
      }
    }
    onConnectedChanged: {
      if (sock.connected) {
        retry.attempt = 0
        root.lastError = ""
      } else {
        retry.interval = 1000
        retry.restart()
      }
    }
  }

  // Backs off while the daemon is away (1, 2, 4, 8 s, then every 8 s): each
  // failed attempt costs a warning in the shell's log, and nobody is waiting.
  // Opening the view retries at once (connectNow).
  Timer {
    id: retry
    property int attempt: 0
    interval: 1000
    repeat: false
    running: false
    onTriggered: {
      if (!root.autoConnect || sock.connected) return
      sock.connected = true
      retry.attempt++
      retry.interval = Math.min(8000, 1000 * Math.pow(2, Math.min(3, retry.attempt)))
      retry.restart()
    }
  }

  Component.onCompleted: {
    if (root.autoConnect) {
      retry.interval = 50
      retry.restart()
    }
  }
}
