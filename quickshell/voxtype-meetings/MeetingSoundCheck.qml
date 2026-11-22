import QtQuick
import Quickshell
import Quickshell.Io
import "MeetingText.js" as MT

// A pre-call sound check (`voxtype meeting soundcheck`): the meeting's mic
// and loopback opened as a meeting would open them, nothing recorded, and
// both tracks' levels about ten times a second.
//
//   voxtype meeting soundcheck --seconds N --json     NDJSON on stdout:
//     {"event":"soundcheck","state":"running","seconds":30,
//      "echo_cancel":"webrtc"|"gtcrn"|"none","mic":"<device>","loopback":true}
//     {"event":"levels","meeting_id":null,"soundcheck":true,"t_ms":100,
//      "mic":-23.0,"mic_peak":-20.0,"loopback":-9.0,"loopback_peak":-6.0}
//     {"event":"soundcheck","state":"stopped",
//      "reason":"done"|"stopped"|"meeting"|"error","message":"..."}
//   voxtype meeting soundcheck --stop
//
// The command's own output drives this, not the socket: it works with or
// without the daemon (without one, the command opens the audio itself).
// Levels are dBFS with a -100 floor; null means no audio in that window, and
// loopback is null throughout when no loopback device is configured.
Item {
  id: root

  property string bin: "voxtype"
  property int seconds: 30

  // "" (never run), "starting", "running" or "stopped".
  property string checkState: ""
  readonly property bool running: root.checkState === "starting" || root.checkState === "running"
  property string echoCancel: ""
  property string mic: ""
  property bool loopback: false
  property real startedAt: 0
  property int total: 0

  property var micDb: null
  property var micPeakDb: null
  property var loopbackDb: null
  property var loopbackPeakDb: null
  // When the last levels arrived (ms since the epoch): a meter falls once
  // they stop.
  property real levelsAt: 0

  // Why it stopped: "done", "stopped", "meeting" or "error", with a message.
  property string reason: ""
  property string message: ""

  function start() {
    if (checkProc.running || checkProc.startPending) return
    root.checkState = "starting"
    root.reason = ""
    root.message = ""
    root.echoCancel = ""
    root.mic = ""
    root.loopback = false
    root.micDb = root.micPeakDb = root.loopbackDb = root.loopbackPeakDb = null
    root.levelsAt = 0
    checkProc.err = ""
    checkProc.sawStop = false
    checkProc.launch([root.bin, "meeting", "soundcheck", "--seconds", String(root.seconds), "--json"])
  }

  function stop() {
    if (!root.running) return
    if (!stopProc.running && !stopProc.startPending)
      stopProc.launch([root.bin, "meeting", "soundcheck", "--stop"])
  }

  // Back to the never-run state (leaving the panel).
  function reset() {
    root.stop()
    if (!root.running) root.checkState = ""
  }

  function handle(ev) {
    if (!ev || typeof ev !== "object") return
    if (ev.event === "soundcheck") {
      if (ev.state === "running") {
        root.checkState = "running"
        root.echoCancel = String(ev.echo_cancel || "")
        root.mic = String(ev.mic || "")
        root.loopback = ev.loopback === true
        root.total = Number(ev.seconds) || root.seconds
        root.startedAt = Date.now()
      } else if (ev.state === "stopped") {
        checkProc.sawStop = true
        root.finish(String(ev.reason || "done"), String(ev.message || ""))
      }
    } else if (ev.event === "levels" && ev.soundcheck === true) {
      root.micDb = ev.mic === null || ev.mic === undefined ? null : Number(ev.mic)
      root.micPeakDb = ev.mic_peak === null || ev.mic_peak === undefined ? null : Number(ev.mic_peak)
      root.loopbackDb = ev.loopback === null || ev.loopback === undefined ? null : Number(ev.loopback)
      root.loopbackPeakDb = ev.loopback_peak === null || ev.loopback_peak === undefined ? null : Number(ev.loopback_peak)
      root.levelsAt = Date.now()
    }
  }

  function finish(reason, message) {
    root.checkState = "stopped"
    root.reason = reason
    root.message = message
    root.levelsAt = 0
  }

  MeetingProcess {
    id: checkProc
    property string err: ""
    property bool sawStop: false
    stdout: SplitParser {
      onRead: function(line) {
        var text = String(line || "").trim()
        if (text === "") return
        try { root.handle(JSON.parse(text)) } catch (e) {}
      }
    }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: checkProc.err = String(text || "").trim() }
    onExited: function(code) {
      if (checkProc.sawStop) return
      // Ended without saying why: the command failed before it could, or
      // this voxtype has no sound check.
      if (code === 0) root.finish("done", "")
      else root.finish("error", MT.cliProblem(checkProc.err, "the sound check")
                                || "voxtype meeting soundcheck exited " + code)
    }
    onFailedToStart: root.finish("error", MT.missingCli(root.bin))
  }

  MeetingProcess { id: stopProc }

  Component.onDestruction: if (root.running) Quickshell.execDetached([root.bin, "meeting", "soundcheck", "--stop"])
}
