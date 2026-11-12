import QtQuick
import Quickshell.Io

// A finished meeting's kept audio as peaks, for the waveform player.
//
//   voxtype meeting waveform <id> --buckets N --json
//     {"meeting_id", "duration_ms", "bucket_ms",
//      "mic": [peak dBFS per bucket, ...], "loopback": [...] or null}
//   -100 is silence, as in `levels`.
//
// PROPOSED: no voxtype has this command yet (a backend need raised for
// 1.3.1). A voxtype without it, or a meeting without kept audio, leaves
// `available` false and the player hidden; nothing is reported, since the
// line-by-line ▶ still plays.
Item {
  id: root

  property string bin: "voxtype"
  property int buckets: 160

  property string meetingId: ""
  property real durationMs: 0
  property var mic: []
  property var loopback: null
  readonly property bool available: root.durationMs > 0 && root.mic.length > 0

  function load(id) {
    if (id === root.meetingId && (root.available || waveProc.running)) return
    root.clear()
    root.meetingId = id
    if (id === "") return
    waveProc.asked = id
    waveProc.out = ""
    waveProc.launch([root.bin, "meeting", "waveform", id, "--buckets", String(root.buckets), "--json"])
  }

  function clear() {
    root.meetingId = ""
    root.durationMs = 0
    root.mic = []
    root.loopback = null
  }

  MeetingProcess {
    id: waveProc
    property string asked: ""
    property string out: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: waveProc.out = String(text || "") }
    onExited: function(code) {
      if (code !== 0 || waveProc.asked !== root.meetingId) return
      var data = null
      try { data = JSON.parse(waveProc.out) } catch (e) {}
      if (!data || !data.mic || data.mic.length === undefined) return
      root.mic = data.mic
      root.loopback = data.loopback && data.loopback.length !== undefined ? data.loopback : null
      root.durationMs = Number(data.duration_ms) || 0
    }
  }
}
