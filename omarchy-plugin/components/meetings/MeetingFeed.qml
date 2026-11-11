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
//
// A meeting that ends is kept, marked `ended`, so its lines can still be
// fixed after the call: the daemon resets to an empty idle snapshot, but the
// feed holds on to the transcript until a new meeting starts or the view is
// closed (clearEnded).
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
  // The last meeting is over but still on screen for editing.
  property bool ended: false
  // Wall-clock ms the meeting was seen to end.
  property real endedAt: 0
  readonly property bool hasMeeting: active || ended

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
      chunkId: Number(segment.chunk_id) || 0,
      originalSpeaker: root.originalOf(segment)
    }
    if (row.text === "") return
    for (var d = model.count - 1; d >= 0; d--) {
      if (model.get(d).segId === row.segId) {
        model.set(d, row)
        root.noteLabel(key, segment)
        return
      }
    }
    // Usually the newest line belongs at the end; walk back from there. Equal
    // start times keep arrival order, as the daemon's backlog does.
    var at = model.count
    while (at > 0 && model.get(at - 1).startMs > row.startMs) at--
    model.insert(at, row)
    root.noteSpeaker(key)
    root.noteLabel(key, segment)
  }

  function noteLabel(key, segment) {
    if (segment.speaker_label === undefined || segment.speaker_label === null || segment.speaker_label === "") return
    if (root.labels[key] === String(segment.speaker_label)) return
    var next = root.copyLabels()
    next[key] = String(segment.speaker_label)
    root.labels = next
  }

  // The pipeline's own speaker key for a line someone moved. The daemon puts
  // `original_speaker` on every segment with an override (hello, segments,
  // segment_updated, `show --json`) and leaves it off otherwise, so it alone
  // says whether a line is moved, and the mark survives a reconnect.
  function originalOf(segment) {
    var o = segment.original_speaker
    return o !== undefined && o !== null && o !== "" ? String(o) : ""
  }

  function rowIndex(segId) {
    for (var i = model.count - 1; i >= 0; i--) if (model.get(i).segId === segId) return i
    return -1
  }

  // Everything about a line that a reassignment changes, to put back if the
  // CLI refuses it.
  function snapshot(segId) {
    var i = root.rowIndex(segId)
    if (i === -1) return null
    var r = model.get(i)
    return { segId: segId, speaker: r.speaker, originalSpeaker: r.originalSpeaker,
             speakerOrder: root.speakerOrder.slice() }
  }

  function restoreSnapshot(snap) {
    if (!snap) return
    var i = root.rowIndex(snap.segId)
    if (i === -1) return
    model.setProperty(i, "speaker", snap.speaker)
    model.setProperty(i, "originalSpeaker", snap.originalSpeaker)
    root.speakerOrder = snap.speakerOrder
  }

  // A line moved to `key`, before the CLI has confirmed it.
  function applyReassign(segId, key) {
    var i = root.rowIndex(segId)
    if (i === -1) return
    var r = model.get(i)
    var original = r.originalSpeaker !== "" ? r.originalSpeaker : r.speaker
    model.setProperty(i, "speaker", key)
    model.setProperty(i, "originalSpeaker", key === original ? "" : original)
    root.noteSpeaker(key)
    root.pruneSpeakers()
  }

  // A line put back on its original speaker, before the CLI has confirmed it.
  // Without a known original there is nothing to show until it answers.
  function applyRestore(segId) {
    var i = root.rowIndex(segId)
    if (i === -1) return
    var r = model.get(i)
    if (r.originalSpeaker === "") return
    model.setProperty(i, "speaker", r.originalSpeaker)
    model.setProperty(i, "originalSpeaker", "")
    root.noteSpeaker(r.speaker)
    root.pruneSpeakers()
  }

  // A changed segment, from `segment_updated` or the CLI's --json answer.
  // It is the daemon's word on the line, override and all.
  function updateSegment(segment) {
    if (!segment || segment.id === undefined) return
    var i = root.rowIndex(Number(segment.id))
    if (i === -1) {
      root.insertSegment(segment)
      return
    }
    var r = model.get(i)
    var key = root.speakerKey(segment)
    var original = root.originalOf(segment)
    model.setProperty(i, "speaker", key)
    if (segment.text !== undefined && String(segment.text).trim() !== "") model.setProperty(i, "text", String(segment.text).trim())
    model.setProperty(i, "originalSpeaker", original)
    root.noteSpeaker(key)
    root.noteLabel(key, segment)
    root.pruneSpeakers()
  }

  // Chips are the speakers the transcript has lines for; one emptied by a
  // reassignment goes away.
  function pruneSpeakers() {
    var present = ({})
    for (var i = 0; i < model.count; i++) present[model.get(i).speaker] = true
    var next = []
    for (var j = 0; j < root.speakerOrder.length; j++)
      if (present[root.speakerOrder[j]]) next.push(root.speakerOrder[j])
    if (next.length !== root.speakerOrder.length) root.speakerOrder = next
  }

  // The view closed: an ended meeting has nothing left to show.
  function clearEnded() {
    if (!root.ended) return
    root.reset()
  }

  // The meeting on screen is over; keep it for editing.
  function endMeeting() {
    root.ended = model.count > 0 || root.meetingId !== ""
    root.endedAt = Date.now()
    if (!root.ended) root.reset()
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
    root.ended = false
    root.endedAt = 0
  }

  function handleHello(ev) {
    var before = root.status
    var beforeId = root.meetingId
    var known = root.everConnected
    var wasActive = root.active
    root.everConnected = true
    var next = String(ev.status || "idle")
    // An idle hello never carries a transcript. If the feed holds a meeting
    // (ended, or running until the daemon went away), keep it for editing.
    if (next === "idle" && (root.ended || wasActive)) {
      if (!root.ended) root.endMeeting()
      root.setStatus("idle")
      return
    }
    root.reset()
    root.meetingId = ev.meeting_id ? String(ev.meeting_id) : ""
    root.title = ev.title ? String(ev.title) : ""
    root.startedAt = ev.started_at ? String(ev.started_at) : ""
    root.chunkCount = Number(ev.chunk_count) || 0
    root.diarization = !!(ev.diarization && ev.diarization.enabled)
    var list = ev.segments && ev.segments.length !== undefined ? ev.segments : []
    for (var i = 0; i < list.length; i++) if (list[i]) root.insertSegment(list[i])
    root.setStatus(next)
    // A status event already announced this meeting and asked for this hello;
    // or the daemon restarted into a new one. Either way it is a start only if
    // the feed saw the previous state.
    if (known && next === "recording" && root.meetingId !== "" && root.meetingId !== beforeId
        && (before === "idle" || before === "recording" && beforeId === ""))
      root.meetingStarted()
  }

  function handleEvent(ev) {
    // Every event is tagged `event`. `type` is accepted too: an early draft
    // of the segment_updated contract used it.
    var kind = String(ev.event || ev.type || "")
    if (kind === "hello") {
      root.handleHello(ev)
    } else if (kind === "segments") {
      var list = ev.segments && ev.segments.length !== undefined ? ev.segments : []
      for (var i = 0; i < list.length; i++) if (list[i]) root.insertSegment(list[i])
      if (ev.chunk_id !== undefined) root.chunkCount = Math.max(root.chunkCount, Number(ev.chunk_id) + 1)
      root.lastSegmentsAt = Date.now()
    } else if (kind === "segment_updated") {
      root.updateSegment(ev.segment)
    } else if (kind === "speaker_renamed") {
      root.renameSpeaker(String(ev.from || ""), String(ev.to || ""))
    } else if (kind === "status") {
      var next = String(ev.status || "")
      if (next === "") return
      if (next === "idle") {
        // The meeting is over (the daemon's next hello is empty), but its
        // lines stay on screen to be fixed.
        if (root.active) root.endMeeting()
        else if (!root.ended) root.reset()
        root.setStatus("idle")
        return
      }
      // A start: fetch the title, id and start time this event lacks. An
      // ended meeting gives way to the new one now.
      var starting = root.ended || root.meetingId === ""
      if (root.ended) root.reset()
      root.setStatus(next)
      if (starting) root.reconnect()
    } else if (kind === "error") {
      root.lastError = String(ev.message || "")
    }
  }

  function renameSpeaker(from, to) {
    if (from === "" || to === "" || from === to) return
    for (var i = 0; i < model.count; i++) {
      if (model.get(i).speaker === from) model.setProperty(i, "speaker", to)
      if (model.get(i).originalSpeaker === from) model.setProperty(i, "originalSpeaker", to)
    }
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
