import QtQuick
import Quickshell.Io

// The canonical history JSON transport for portable application hosts.
Item {
  id: root
  property string bin: "voxtype"
  property string configPath: ""
  property var queue: []
  signal historyReply(string token, bool ok, var value, string message)
  function historyRequest(token, args) {
    var command = [root.bin]
    if (root.configPath !== "") command.push("--config", root.configPath)
    command.push("history", "--json")
    for (var i = 0; i < args.length; ++i) command.push(String(args[i]))
    var next = root.queue.slice()
    next.push({ token: String(token), command: command })
    root.queue = next
    pump()
  }
  function meetingRequest(token, id) {
    var command = [root.bin]
    if (root.configPath !== "") command.push("--config", root.configPath)
    command.push("meeting", "view", String(id))
    var next = root.queue.slice()
    next.push({token:String(token), command:command, meetingLaunch:true})
    root.queue = next
    pump()
  }
  function pump() {
    if (proc.running || proc.startPending || root.queue.length === 0) return
    var request = root.queue[0]
    root.queue = root.queue.slice(1)
    proc.token = request.token
    proc.meetingLaunch = !!request.meetingLaunch
    proc.out = ""
    proc.err = ""
    proc.launch(request.command)
  }
  HistoryProcess {
    id: proc
    property bool meetingLaunch: false
    property string token: ""
    property string out: ""
    property string err: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: proc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: proc.err = String(text || "").trim() }
    onExited: function(code) {
      if (proc.meetingLaunch) {
        root.historyReply(proc.token, code === 0, null, proc.err || (code === 0 ? "" : "Meeting launch did not complete"))
        Qt.callLater(root.pump)
        return
      }
      var value = null
      var message = proc.err
      try { value = JSON.parse(proc.out) }
      catch (e) { message = message || "History command did not return JSON" }
      if (value && value.error) message = String(value.error)
      root.historyReply(proc.token, code === 0 && value !== null, value, message)
      Qt.callLater(root.pump)
    }
    onFailedToStart: {
      root.historyReply(proc.token, false, null, "Could not run " + root.bin)
      Qt.callLater(root.pump)
    }
  }
}
