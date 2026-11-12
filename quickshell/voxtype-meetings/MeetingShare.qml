import QtQuick
import Quickshell.Io
import "MeetingText.js" as MT

// Sharing a meeting's summary and notes (`voxtype meeting share`). Nothing
// leaves without an explicit click: the preview only asks what would.
//
//   voxtype meeting share <id> --preview --json [--transcript]
//     {"meeting_id", "title", "subject", "format", "transcript", "sections": [...],
//      "transcript_lines", "chars", "body",
//      "available": {"clipboard", "file", "email", "command"},
//      "why": {<destination>: "<what to do>"} (optional), "command", "folder"}
//   voxtype meeting share <id> --to clipboard|file|email|command [--transcript] --json
//     {"meeting_id", "to", "format", "transcript", "sections", "chars",
//      "path" (file; email with --transcript: the file to attach),
//      "command_status" (command)}
//   failure: exit 1, nothing on stdout, one line on stderr saying what to do
Item {
  id: root

  property string bin: "voxtype"

  property string meetingId: ""
  property bool transcript: false
  // The last preview (null while none), and its failure.
  property var preview: null
  property string previewError: ""
  readonly property bool previewing: previewProc.running || previewProc.startPending

  // The last send: where to, what came back, or why it failed.
  property string sentTo: ""
  property var result: null
  property string sendError: ""
  readonly property bool sending: sendProc.running || sendProc.startPending

  function open(id) {
    root.meetingId = id
    root.transcript = false
    root.result = null
    root.sendError = ""
    root.sentTo = ""
    root.refresh()
  }

  function setTranscript(on) {
    if (root.transcript === on) return
    root.transcript = on
    root.result = null
    root.sendError = ""
    root.refresh()
  }

  function refresh() {
    if (root.meetingId === "") return
    if (previewProc.running || previewProc.startPending) {
      previewProc.again = true
      return
    }
    root.previewError = ""
    previewProc.out = ""
    previewProc.err = ""
    var args = [root.bin, "meeting", "share", root.meetingId, "--preview", "--json"]
    if (root.transcript) args.push("--transcript")
    previewProc.launch(args)
  }

  function available(to) {
    return root.preview !== null && root.preview.available !== undefined && root.preview.available[to] === true
  }

  // Why a destination is unavailable, when the CLI says.
  function why(to) {
    return root.preview && root.preview.why && root.preview.why[to] ? String(root.preview.why[to]) : ""
  }

  function send(to) {
    if (root.sending || root.meetingId === "") return
    root.sentTo = to
    root.result = null
    root.sendError = ""
    sendProc.out = ""
    sendProc.err = ""
    var args = [root.bin, "meeting", "share", root.meetingId, "--to", to]
    if (root.transcript) args.push("--transcript")
    args.push("--json")
    sendProc.launch(args)
  }

  MeetingProcess {
    id: previewProc
    property string out: ""
    property string err: ""
    property bool again: false
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: previewProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: previewProc.err = String(text || "").trim() }
    onExited: function(code) {
      if (previewProc.again) {
        previewProc.again = false
        Qt.callLater(root.refresh)
        return
      }
      var data = null
      try { data = JSON.parse(previewProc.out) } catch (e) {}
      if (code !== 0 || !data || typeof data !== "object") {
        root.preview = null
        root.previewError = MT.cliProblem(previewProc.err, "sharing meetings") || "voxtype meeting share exited " + code
        return
      }
      root.preview = data
    }
    onFailedToStart: root.previewError = MT.missingCli(root.bin)
  }

  MeetingProcess {
    id: sendProc
    property string out: ""
    property string err: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: sendProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: sendProc.err = String(text || "").trim() }
    onExited: function(code) {
      var data = null
      try { data = JSON.parse(sendProc.out) } catch (e) {}
      if (code !== 0 || !data) {
        root.sendError = MT.cliProblem(sendProc.err, "sharing meetings") || "voxtype meeting share exited " + code
        return
      }
      root.result = data
    }
    onFailedToStart: root.sendError = MT.missingCli(root.bin)
  }
}
