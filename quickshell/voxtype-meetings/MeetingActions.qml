import QtQuick
import Quickshell.Io
import "MeetingText.js" as MT

// A meeting's action items and where they can go (`voxtype meeting actions`):
// Todoist (after a one-time `voxtype meeting todoist connect` in the
// browser) or an Omarchy reminder. Every item is ticked to start with;
// nothing is sent until Send or Remind is clicked, and an item already sent
// somewhere is skipped there.
//
//   voxtype meeting actions <id> --json
//     {meeting_id, title, summarized, todoist_connected, reminder_available,
//      items: [{number, key, text, owner, mine, due, due_time, due_said,
//               source_ms, sent: {todoist?: {id, at}, reminder?: {at}}}]}
//   voxtype meeting actions <id> --send todoist|reminder (--all | --pick 1,3)
//       [--in MINUTES] --json
//     {meeting_id, target, results: [{number, text, status: sent|already_sent|failed,
//      remote_id?, error?, minutes?}]}; exit 1 if any failed (JSON still
//      printed), or exit 1 with only a stderr line when nothing was tried
//   voxtype meeting todoist connect --json   NDJSON, blocking:
//     {"event":"authorize","url","port"} (the browser is already opened), then
//     {"event":"connected",...} exit 0 or {"event":"error","message"} exit 1
Item {
  id: root

  property string bin: "voxtype"

  property string meetingId: ""
  property var data: null
  property string error: ""
  // false once a voxtype is seen without `meeting actions`: the panel hides.
  property bool supported: true
  readonly property bool loading: listProc.running || listProc.startPending

  // Items unticked by the user, by their stable `key` (it survives a
  // regenerated summary); everything else is ticked (all by default).
  property var unticked: ({})
  // While connecting: the consent page, for when the browser did not open.
  property string connectUrl: ""

  property string busy: ""        // "todoist" | "reminder" | "connect" | ""
  property string notice: ""      // what the last action did
  property string actionError: "" // or why it failed

  readonly property var items: {
    if (!root.data) return []
    var list = root.data.items !== undefined ? root.data.items : (Array.isArray(root.data) ? root.data : [])
    return list || []
  }
  readonly property bool todoistConnected: root.data !== null && root.data.todoist_connected === true
  readonly property bool reminderAvailable: root.data !== null && root.data.reminder_available === true

  function itemId(item) { return String(item.key !== undefined && item.key !== null ? item.key : item.number) }
  // When it is due, as said or as resolved: "2026-11-13 15:00", or "next week".
  function dueText(item) {
    if (item.due) return String(item.due) + (item.due_time ? " " + String(item.due_time) : "")
    return item.due_said ? String(item.due_said) : ""
  }
  function ticked(item) { return root.unticked[root.itemId(item)] !== true }
  function sentTo(item, dest) { return item.sent !== undefined && item.sent !== null && !!item.sent[dest] }

  readonly property var tickedIds: {
    var out = []
    for (var i = 0; i < root.items.length; i++) if (root.ticked(root.items[i])) out.push(root.itemId(root.items[i]))
    return out
  }
  // What --pick takes: the ticked items' numbers.
  readonly property var tickedNumbers: {
    var out = []
    for (var i = 0; i < root.items.length; i++) if (root.ticked(root.items[i])) out.push(Number(root.items[i].number))
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
    return ["--pick", root.tickedNumbers.join(",")]
  }

  // Minutes from now for a reminder: 15, 60, or the end of the working day
  // (17:00; an hour from now if that has passed).
  function minutesFor(delay) {
    if (delay === "15m") return 15
    if (delay === "eod") {
      var now = new Date()
      var end = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 17, 0, 0)
      var mins = Math.round((end.getTime() - now.getTime()) / 60000)
      return mins >= 5 ? mins : 60
    }
    return 60
  }

  function send(dest, delay) {
    if (root.busy !== "" || root.tickedIds.length === 0) return
    root.busy = dest
    root.notice = ""
    root.actionError = ""
    var args = [root.bin, "meeting", "actions", root.meetingId, "--send", dest]
    if (dest === "reminder") args.push("--in", String(root.minutesFor(delay || "1h")))
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
    root.notice = "Opening Todoist in your browser…"
    root.actionError = ""
    root.connectUrl = ""
    connectProc.event = null
    connectProc.err = ""
    connectProc.launch([root.bin, "meeting", "todoist", "connect", "--json"])
  }

  // Leave a connect that is waiting on the browser (it saves nothing).
  function cancelConnect() {
    if (root.busy === "connect") connectProc.running = false
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
      else failed.push((r.text ? String(r.text) + ": " : "") + (r.error ? String(r.error) : "failed"))
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
    property var event: null
    property string err: ""
    stdout: SplitParser {
      onRead: function(line) {
        var ev = null
        try { ev = JSON.parse(String(line || "")) } catch (e) { return }
        if (!ev) return
        if (ev.event === "authorize") {
          root.connectUrl = ev.url ? String(ev.url) : ""
          root.notice = "Allow voxtype in the Todoist page your browser opened. This waits for you."
        } else {
          connectProc.event = ev
        }
      }
    }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: connectProc.err = String(text || "").trim() }
    onExited: function(code) {
      root.busy = ""
      root.connectUrl = ""
      var ev = connectProc.event
      if (code === 0 && ev && ev.event === "connected") {
        root.notice = "Todoist connected."
      } else {
        root.notice = ""
        root.actionError = ev && ev.event === "error" && ev.message ? String(ev.message)
          : MT.cliProblem(connectProc.err, "connecting Todoist") || (code === 0 ? "" : "Connecting Todoist was stopped.")
      }
      root.load(root.meetingId)
    }
    onFailedToStart: { root.busy = ""; root.notice = ""; root.actionError = MT.missingCli(root.bin) }
  }
}
