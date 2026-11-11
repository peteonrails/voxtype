import QtQuick
import Quickshell.Io


// The meeting view's subprocesses: `voxtype meeting …` and the one config read
// it needs. Kept apart from VoxtypeCli so the Settings panel's process plumbing
// is untouched, but built the same way: every call is a CliProcess, so a
// binary that cannot start is reported instead of silently doing nothing.
//
// Actions run one at a time, in order. A rename of a merged speaker chip is
// one `meeting label` per speaker key, and a pause queued behind a label must
// still happen.
Item {
  id: root

  // The binary to run: VoxtypeCli.bin, so dev.json applies here too.
  property string bin: "voxtype"

  // The action running now ("pause", "label", …), or "".
  readonly property string busy: actionProc.running || actionProc.startPending ? actionProc.action : ""
  property var queue: []

  signal actionFinished(string action, bool ok, string message)
  signal showOnStartRead(bool show)

  function enqueue(action, args, extra) {
    var next = root.queue.slice()
    next.push({ action: action, args: [root.bin].concat(args), extra: extra || null })
    root.queue = next
    root.pump()
  }

  function pump() {
    if (actionProc.running || actionProc.startPending || root.queue.length === 0) return
    var op = root.queue[0]
    root.queue = root.queue.slice(1)
    actionProc.action = op.action
    actionProc.extra = op.extra
    actionProc.err = ""
    actionProc.out = ""
    actionProc.launch(op.args)
  }

  function pause() { root.enqueue("pause", ["meeting", "pause"]) }
  function resume() { root.enqueue("resume", ["meeting", "resume"]) }
  function stop() { root.enqueue("stop", ["meeting", "stop"]) }

  function start(title) {
    var args = ["meeting", "start"]
    var t = String(title || "").trim()
    if (t !== "") args.push("--title", t)
    root.enqueue("start", args)
  }

  // One label per key, each reported on its own.
  function label(meeting, keys, name) {
    for (var i = 0; i < keys.length; i++)
      root.enqueue("label", ["meeting", "label", meeting || "latest", String(keys[i]), String(name)],
                   { key: String(keys[i]), name: String(name) })
  }

  // Move one line to another speaker: an existing ID or display name, or a new
  // name. `extra` comes back with reassignFinished, so the caller can roll its
  // optimistic change back.
  function reassign(meeting, segId, speaker, extra) {
    root.enqueue("reassign", ["meeting", "reassign", meeting || "latest", String(segId), String(speaker), "--json"], extra)
  }

  function restoreSpeaker(meeting, segId, extra) {
    root.enqueue("reassign", ["meeting", "reassign", meeting || "latest", String(segId), "--reset", "--json"], extra)
  }

  // `segment` is the updated segment from --json, or null if it printed none.
  signal reassignFinished(var extra, bool ok, string message, var segment)

  // A note and/or highlight on one line; neither removes the annotation.
  // Annotating replaces what was there, so both are always sent.
  function annotate(meeting, segId, text, highlight, extra) {
    var t = String(text || "").trim()
    var args = ["meeting", "annotate", meeting || "latest", String(segId)]
    if (t === "" && !highlight) args.push("--remove")
    else {
      if (t !== "") args.push(t)
      if (highlight) args.push("--highlight")
    }
    args.push("--json")
    root.enqueue("annotate", args, extra)
  }

  signal annotateFinished(var extra, bool ok, string message, var segment)

  // `[meeting] show_view_on_start`. A CLI that predates the key exits non-zero
  // on it; that reads as the default, which is on.
  function readShowOnStart() {
    if (configProc.running) return
    configProc.out = ""
    configProc.launch([root.bin, "config", "get", "meeting.show_view_on_start"])
  }

  signal labelApplied(string key, string name)

  MeetingProcess {
    id: actionProc
    property string action: ""
    property var extra: null
    property string err: ""
    property string out: ""

    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: actionProc.out = String(text || "").trim()
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: actionProc.err = String(text || "").trim()
    }
    onExited: function(exitCode) {
      var action = actionProc.action
      var extra = actionProc.extra
      var ok = exitCode === 0
      var message = ok ? "" : (actionProc.err || ("voxtype meeting " + action + " exited " + exitCode))
      actionProc.action = ""
      if (ok && action === "label" && extra) root.labelApplied(extra.key, extra.name)
      if (action === "reassign" || action === "annotate") {
        var segment = null
        if (ok) {
          try { segment = JSON.parse(actionProc.out) } catch (e) { segment = null }
          if (segment && segment.segment && segment.id === undefined) segment = segment.segment
          if (!segment || typeof segment !== "object" || segment.id === undefined) segment = null
        }
        if (action === "reassign") root.reassignFinished(extra, ok, message, segment)
        else root.annotateFinished(extra, ok, message, segment)
      }
      root.actionFinished(action, ok, message)
      Qt.callLater(root.pump)
    }
    onFailedToStart: {
      var action = actionProc.action
      var extra = actionProc.extra
      var message = "Could not run " + root.bin + ". Is voxtype installed?"
      actionProc.action = ""
      if (action === "reassign") root.reassignFinished(extra, false, message, null)
      if (action === "annotate") root.annotateFinished(extra, false, message, null)
      root.actionFinished(action, false, message)
      Qt.callLater(root.pump)
    }
  }

  MeetingProcess {
    id: configProc
    property string out: ""

    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: configProc.out = String(text || "").trim()
    }
    onExited: function(exitCode) {
      root.showOnStartRead(exitCode !== 0 || configProc.out !== "false")
    }
    onFailedToStart: root.showOnStartRead(true)
  }
}
