import QtQuick
import Quickshell.Io

// A finished meeting's kept audio as peaks, for the waveform player.
//
//   voxtype meeting waveform <id> --buckets N --json
//     {"duration_ms", "bucket_ms", "mic": [peak dBFS per bucket, ...],
//      "loopback": [...] or null, "mic_track": "mic" | "mixed"}
//   -100 is silence, as in `levels`. "mixed": the meeting kept only both
//   sides together (an import), and `mic` holds that one track.
//   Exit 2: no kept audio; exit 1: a track would not decode.
//
// A voxtype without the command, a meeting without kept audio, or a failed
// decode leaves `available` false and the player hidden; nothing is
// reported, since the line-by-line ▶ still plays.
Item {
  id: root

  property string bin: "voxtype"
  property int buckets: 160

  property string meetingId: ""
  property real durationMs: 0
  // How long each peak covers; 0 from a voxtype that does not say.
  property real bucketMs: 0
  property var mic: []
  property var loopback: null
  property string micTrack: "mic"
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
    root.bucketMs = 0
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
      root.micTrack = data.mic_track ? String(data.mic_track) : "mic"
      root.bucketMs = Number(data.bucket_ms) || 0
      root.durationMs = Number(data.duration_ms) || 0
    }
  }
}
