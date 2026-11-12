import QtQuick
import Quickshell.Io
import "MeetingText.js" as MT

// Searching every meeting (`voxtype meeting search`), for the box on the
// recent-meetings list.
//
//   voxtype meeting search <query> --limit N --json
//     {"query", "speaker", "since", "results": [{"meeting_id", "title",
//      "date", "kind": "segment"|"title"|"notes"|"summary", "segment_id",
//      "start_ms", "speaker", "snippet", "snippet_plain",
//      "highlights": [[start, end), ...] in code points, "rank"}]}
//
// Best matches come first. The query goes as one argument: the command
// reads it the same as separate words, and a "quoted phrase" or a word*
// prefix stays intact. A query with no words fails; that is not run.
Item {
  id: root

  property string bin: "voxtype"
  property int limit: 30

  // What was asked (trimmed), and what the results answer: a reply to an
  // older query arriving late is dropped.
  property string query: ""
  property string answered: ""
  property var results: []
  property string error: ""
  readonly property bool searching: searchProc.running || searchProc.startPending || root.queued !== ""
  property string queued: ""

  function search(text) {
    var q = String(text || "").trim()
    root.query = q
    root.error = ""
    if (q === "") {
      root.results = []
      root.answered = ""
      root.queued = ""
      return
    }
    if (searchProc.running || searchProc.startPending) {
      // One at a time; the newest waits and runs when this one ends.
      root.queued = q
      return
    }
    root.run(q)
  }

  function run(q) {
    root.queued = ""
    searchProc.asked = q
    searchProc.out = ""
    searchProc.err = ""
    searchProc.launch([root.bin, "meeting", "search", q, "--limit", String(root.limit), "--json"])
  }

  function clear() { root.search("") }

  MeetingProcess {
    id: searchProc
    property string asked: ""
    property string out: ""
    property string err: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: searchProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: searchProc.err = String(text || "").trim() }
    onExited: function(code) {
      var asked = searchProc.asked
      if (root.queued !== "") {
        Qt.callLater(function() { root.run(root.queued) })
        return
      }
      if (asked !== root.query) return
      root.answered = asked
      var data = null
      try { data = JSON.parse(searchProc.out) } catch (e) {}
      if (code !== 0 || !data || !data.results) {
        root.results = []
        root.error = MT.cliProblem(searchProc.err, "searching meetings") || "voxtype meeting search exited " + code
        return
      }
      root.results = data.results
    }
    onFailedToStart: {
      root.queued = ""
      root.results = []
      root.error = MT.missingCli(root.bin)
    }
  }
}
