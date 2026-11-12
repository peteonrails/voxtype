import QtQuick
import Quickshell.Io
import "MeetingText.js" as MT

// A meeting's action items and where they can go (`voxtype meeting actions`):
// Todoist (after a one-time `--connect todoist` in the browser) or an
// Omarchy reminder. Every item is ticked to start with; nothing is sent
// until Send or Remind is clicked, and an item already sent somewhere is
// not sent there twice.
//
//   voxtype meeting actions <id> --json
//   voxtype meeting actions <id> --send todoist (--all | --pick <ids...>) --json
//   voxtype meeting actions <id> --send reminder --in <15m|1h|eod> (--all | --pick ...) --json
//   voxtype meeting actions --connect todoist --json
//
// The shapes are read through items()/connected()/results() below, so a
// contract change lands in one place.
Item {
  id: root

  property string bin: "voxtype"

  property string meetingId: ""
  property var data: null
  property string error: ""
  // false once a voxtype is seen without `meeting actions`: the panel hides.
  property bool supported: true
  readonly property bool loading: listProc.running || listProc.startPending

  // Ids unticked by the user; everything else is ticked (all by default).
  property var unticked: ({})

  property string busy: ""        // "todoist" | "reminder" | "connect" | ""
  property string notice: ""      // what the last action did
  property string actionError: "" // or why it failed

  readonly property var items: {
    if (!root.data) return []
    var list = root.data.items !== undefined ? root.data.items : (Array.isArray(root.data) ? root.data : [])
    return list || []
  }
  readonly property bool todoistConnected: root.data !== null && root.data.todoist !== undefined
    && root.data.todoist !== null && root.data.todoist.connected === true
  readonly property bool reminderAvailable: root.data !== null && root.data.reminder !== undefined
    && root.data.reminder !== null && root.data.reminder.available === true

  function itemId(item) { return String(item.id) }
  function ticked(item) { return root.unticked[root.itemId(item)] !== true }
  function sentTo(item, dest) { return item.sent !== undefined && item.sent !== null && !!item.sent[dest] }

  readonly property var tickedIds: {
    var out = []
    for (var i = 0; i < root.items.length; i++) if (root.ticked(root.items[i])) out.push(root.itemId(root.items[i]))
    return out
  }

  function toggle(item) {
    var next = ({})
    for (var k in root.unticked) next[k] = root.unticked[k]
    var id = root.itemId(item)
    if (next[id] === true) delete next[id]
    else next[id] = true
    root.unticked = next
  }

  function load(id) {
    if (id !== root.meetingId) {
      root.data = null
      root.unticked = ({})
      root.notice = ""
      root.actionError = ""
    }
    root.meetingId = id
    if (id === "" || !root.supported) return
    if (listProc.running || listProc.startPending) {
      listProc.again = true
      return
    }
    root.error = ""
    listProc.out = ""
    listProc.err = ""
    listProc.launch([root.bin, "meeting", "actions", id, "--json"])
  }

  function pickArgs() {
    if (root.tickedIds.length === root.items.length) return ["--all"]
    return ["--pick"].concat(root.tickedIds)
  }

  function send(dest, delay) {
    if (root.busy !== "" || root.tickedIds.length === 0) return
    root.busy = dest
    root.notice = ""
    root.actionError = ""
    var args = [root.bin, "meeting", "actions", root.meetingId, "--send", dest]
    if (dest === "reminder") args.push("--in", delay || "1h")
    args = args.concat(root.pickArgs())
    args.push("--json")
    sendProc.dest = dest
    sendProc.out = ""
    sendProc.err = ""
    sendProc.launch(args)
  }

  function connect() {
    if (root.busy !== "") return
    root.busy = "connect"
    root.notice = "Finish connecting in your browser. This waits for you."
    root.actionError = ""
    connectProc.out = ""
    connectProc.err = ""
    connectProc.launch([root.bin, "meeting", "actions", "--connect", "todoist", "--json"])
  }

  // What a send did, in words: per-item results if the CLI gives them.
  function describe(dest, out) {
    var results = out && out.results && out.results.length !== undefined ? out.results : null
    if (!results) return dest === "todoist" ? "Sent to Todoist." : "Reminder set."
    var sent = 0, already = 0, failed = []
    for (var i = 0; i < results.length; i++) {
      var r = results[i]
      if (r.status === "sent") sent++
      else if (r.status === "already_sent") already++
      else failed.push(r.reason ? String(r.reason) : "failed")
    }
    var parts = []
    if (sent > 0) parts.push(dest === "todoist" ? sent + (sent === 1 ? " item" : " items") + " sent to Todoist"
                                                 : sent + (sent === 1 ? " reminder" : " reminders") + " set")
    if (already > 0) parts.push(already + " already " + (dest === "todoist" ? "in Todoist" : "set"))
    var text = parts.length > 0 ? parts.join(", ") + "." : ""
    if (failed.length > 0) root.actionError = failed.join(" ")
    return text
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
        Qt.callLater(function() { root.load(root.meetingId) })
        return
      }
      if (code !== 0) {
        // A voxtype without the command keeps the summary as it was.
        if (/unrecognized subcommand/i.test(listProc.err)) { root.supported = false; root.data = null; return }
        root.error = MT.cliProblem(listProc.err, "action items") || "voxtype meeting actions exited " + code
        return
      }
      var parsed = null
      try { parsed = JSON.parse(listProc.out) } catch (e) {}
      root.data = parsed
    }
    onFailedToStart: root.error = MT.missingCli(root.bin)
  }

  MeetingProcess {
    id: sendProc
    property string dest: ""
    property string out: ""
    property string err: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: sendProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: sendProc.err = String(text || "").trim() }
    onExited: function(code) {
      root.busy = ""
      var parsed = null
      try { parsed = JSON.parse(sendProc.out) } catch (e) {}
      if (parsed) root.notice = root.describe(sendProc.dest, parsed)
      if (code !== 0 && root.actionError === "")
        root.actionError = MT.cliProblem(sendProc.err, "action items") || "voxtype meeting actions exited " + code
      root.load(root.meetingId)
    }
    onFailedToStart: { root.busy = ""; root.actionError = MT.missingCli(root.bin) }
  }

  MeetingProcess {
    id: connectProc
    property string out: ""
    property string err: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: connectProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: connectProc.err = String(text || "").trim() }
    onExited: function(code) {
      root.busy = ""
      if (code === 0) {
        root.notice = "Todoist connected."
      } else {
        root.notice = ""
        root.actionError = MT.cliProblem(connectProc.err, "connecting Todoist") || "Connecting Todoist failed."
      }
      root.load(root.meetingId)
    }
    onFailedToStart: { root.busy = ""; root.notice = ""; root.actionError = MT.missingCli(root.bin) }
  }
}
