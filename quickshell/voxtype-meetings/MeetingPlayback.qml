import QtQuick
import Quickshell.Io
import "MeetingText.js" as MT

// Playing a meeting's kept audio from a line (`voxtype meeting play`).
//
//   voxtype meeting play <id> --segment <seg> [--track mixed|mic|loopback] --json
//       {"meeting_id", "track", "from_ms", "pid"}; starts in the background
//   voxtype meeting play --status --json
//       {"state": "playing"|"stopped", "meeting_id", "track", "from_ms", "pid",
//        "started_at", "position_ms"}; all null before anything has played
//   voxtype meeting play --stop --json
//   socket: {"event":"playback", meeting_id, track, state, from_ms}, coalesced
//
// A failure exits 1 with one line on stderr, shown as it is. While playing,
// --status is polled every 500 ms for `position_ms`, which is what highlights
// the line being heard; the socket event only says to look.
Item {
  id: root

  property string bin: "voxtype"

  property string playState: "stopped"
  property string meetingId: ""
  property string track: "mixed"
  property real positionMs: -1
  readonly property bool playing: root.playState === "playing"

  // The last failure's stderr; `noAudio` when it says the meeting kept none.
  property string error: ""
  readonly property bool busy: playProc.running || playProc.startPending

  signal failed(string message, bool noAudio)

  function play(meeting, segId, track) {
    if (root.busy) return
    root.error = ""
    playProc.err = ""
    playProc.launch([root.bin, "meeting", "play", String(meeting), "--segment", String(segId),
                     "--track", track || "mixed", "--json"])
  }

  // From a point in the meeting (the waveform player): `--at` whole seconds.
  function playAt(meeting, ms, track) {
    if (root.busy) return
    root.error = ""
    playProc.err = ""
    playProc.launch([root.bin, "meeting", "play", String(meeting), "--at", String(Math.floor(Math.max(0, ms) / 1000)),
                     "--track", track || "mixed", "--json"])
  }

  function stop() {
    if (stopProc.running || stopProc.startPending) return
    stopProc.out = ""
    stopProc.launch([root.bin, "meeting", "play", "--stop", "--json"])
  }

  function poll() {
    if (statusProc.running || statusProc.startPending) return
    statusProc.out = ""
    statusProc.launch([root.bin, "meeting", "play", "--status", "--json"])
  }

  // A `playback` event from the socket: take its state now, then make sure.
  function noteEvent(ev) {
    if (ev.state) root.playState = String(ev.state)
    if (ev.meeting_id) root.meetingId = String(ev.meeting_id)
    if (ev.track) root.track = String(ev.track)
    if (root.playState !== "playing") root.positionMs = -1
    root.poll()
  }

  function apply(s) {
    if (!s || typeof s !== "object") return
    root.playState = s.state === "playing" ? "playing" : "stopped"
    root.meetingId = s.meeting_id ? String(s.meeting_id) : ""
    root.track = s.track ? String(s.track) : "mixed"
    root.positionMs = root.playing && s.position_ms !== null && s.position_ms !== undefined ? Number(s.position_ms)
      : root.playing && s.from_ms !== null && s.from_ms !== undefined ? Number(s.from_ms) : -1
  }

  Timer {
    interval: 500
    repeat: true
    running: root.playing
    onTriggered: root.poll()
  }

  MeetingProcess {
    id: playProc
    property string out: ""
    property string err: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: playProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: playProc.err = String(text || "").trim() }
    onExited: function(code) {
      if (code !== 0) {
        var message = MT.cliProblem(playProc.err, "playing meeting audio") || "voxtype meeting play exited " + code
        root.error = message
        root.failed(message, /without keeping audio/i.test(message))
        return
      }
      var started = null
      try { started = JSON.parse(playProc.out) } catch (e) {}
      if (started) {
        root.playState = "playing"
        root.meetingId = String(started.meeting_id || "")
        root.track = String(started.track || "mixed")
        root.positionMs = Number(started.from_ms) || 0
      }
      root.poll()
    }
    onFailedToStart: {
      root.error = MT.missingCli(root.bin)
      root.failed(root.error, false)
    }
  }

  MeetingProcess {
    id: statusProc
    property string out: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: statusProc.out = String(text || "") }
    onExited: function(code) {
      if (code !== 0) return
      try { root.apply(JSON.parse(statusProc.out)) } catch (e) {}
    }
  }

  MeetingProcess {
    id: stopProc
    property string out: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: stopProc.out = String(text || "") }
    onExited: function(code) {
      try { root.apply(JSON.parse(stopProc.out)) } catch (e) {}
      if (code === 0) {
        root.playState = "stopped"
        root.positionMs = -1
      }
    }
  }
}
