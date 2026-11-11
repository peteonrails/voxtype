import QtQuick
import Quickshell
import Quickshell.Io

// Notes, Ask and Summary for the meeting the feed holds. Non-visual and owned
// by Meetings.qml, so an answer still streams in, and the notes still save,
// after the view closes.
//
// The CLI it drives (the assistant contract, 1.3.0):
//
//   voxtype meeting assistant status --json
//       {"local_only": bool, "local_model": "llama3.2"|null,
//        "agent": "claude"|null, "agent_label": "Claude"|null,
//        "consented": bool,                      (for `agent`)
//        "candidates": [{"id", "label", "consented"}]}
//   voxtype meeting assistant allow <agent>
//   voxtype config set meeting.assistant.local_only true|false
//   voxtype config set meeting.assistant.agent <id>
//   voxtype meeting ask <id> "<question>" [--last 5m] [--notify]
//       the answer streams on stdout; failure exits non-zero with stderr
//   voxtype meeting summarize <id>                   summary markdown on stdout
//   voxtype meeting show <id> --json                 metadata.summary for a
//                                                    summary_ready without text
//   voxtype meeting notes <id> --json                {"meeting_id", "notes", "updated_at"}
//   voxtype meeting notes <id> --set-file <path>
//   socket: {"event": "notes_updated", "meeting_id", "updated_at"} (no text)
Item {
  id: root

  property string bin: "voxtype"
  property var feed: null
  // The view is up. A question asked while it is not gets --notify, so the
  // answer arrives as a notification whose Open action raises the view.
  property bool viewShown: false

  // ------------------------------------------------------------ status

  // The `assistant status --json` object, or null until read.
  property var status: null
  property string statusError: ""

  readonly property bool localOnly: status !== null && status.local_only === true
  readonly property string agentLabel: {
    if (status === null) return ""
    if (status.local_only === true)
      return "Local" + (status.local_model ? " (" + status.local_model + ")" : "")
    if (status.agent_label) return String(status.agent_label)
    if (status.agent) return String(status.agent)
    return "No agent"
  }
  readonly property var candidates: status && status.candidates && status.candidates.length !== undefined
    ? status.candidates : []

  // What a question or summary needs before it can go out: "" (nothing),
  // "consent" (an agent nobody has allowed), "choose" (several agents found,
  // none configured or allowed).
  readonly property string blocker: {
    if (status === null || status.local_only === true) return ""
    if (status.agent && status.consented === true) return ""
    if (status.agent) return "consent"
    for (var i = 0; i < root.candidates.length; i++) if (root.candidates[i].consented === true) return ""
    return root.candidates.length > 0 ? "choose" : ""
  }

  // The work waiting on the consent dialog: {kind: "ask"|"summary", ...}.
  property var pending: null
  signal consentNeeded()

  // Work to run once a fresh status is in (after a consent or settings change,
  // so its answer is attributed to the agent that actually gives it).
  property var afterStatus: null

  function fetchStatus() {
    if (statusProc.running || statusProc.startPending) {
      statusProc.again = true
      return
    }
    statusProc.out = ""
    statusProc.launch([root.bin, "meeting", "assistant", "status", "--json"])
  }

  // ------------------------------------------------------------ ask

  // The Q&A thread for the meeting the feed holds: role "q" or "a".
  readonly property alias thread: threadModel
  ListModel { id: threadModel }
  property string threadMeeting: ""
  readonly property bool asking: askProc.running || askProc.startPending

  function ask(question, last) {
    var q = String(question || "").trim()
    if (q === "" || !root.feed || root.feed.meetingId === "") return
    var job = { kind: "ask", question: q, last: last || "", notify: !root.viewShown }
    if (root.blocker !== "") {
      root.pending = job
      root.consentNeeded()
      return
    }
    root.runAsk(job)
  }

  // The quick chips, by name, for the view and for `voxtype meeting view --ask`.
  function askChip(kind) {
    if (kind === "last-5m") return root.ask("Summarize the last 5 minutes.", "5m")
    if (kind === "actions") return root.ask("What are the action items so far, and who owns each?", "")
    if (kind === "last-speaker") {
      var who = root.feed ? root.feed.lastRemoteSpeaker() : ""
      return root.ask(who !== "" ? "What did " + who + " just say?" : "What was just said?", "2m")
    }
  }

  function runAsk(job) {
    root.syncThread()
    threadModel.append({ role: "q", text: job.question, agent: "", pending: false, error: "" })
    threadModel.append({ role: "a", text: "", agent: root.agentLabel, pending: true, error: "" })
    var args = [root.bin, "meeting", "ask", root.feed.meetingId, job.question]
    if (job.last !== "") args.push("--last", job.last)
    if (job.notify) args.push("--notify")
    askQueue.push(args)
    root.pumpAsk()
  }

  property var askQueue: []
  function pumpAsk() {
    if (askProc.running || askProc.startPending || root.askQueue.length === 0) return
    var args = root.askQueue.shift()
    askProc.err = ""
    askProc.answerRow = root.pendingAnswerRow()
    askProc.launch(args)
  }

  function pendingAnswerRow() {
    for (var i = 0; i < threadModel.count; i++)
      if (threadModel.get(i).role === "a" && threadModel.get(i).pending) return i
    return -1
  }

  // A new meeting starts a new thread.
  function syncThread() {
    if (!root.feed || root.feed.meetingId === root.threadMeeting) return
    threadModel.clear()
    root.threadMeeting = root.feed.meetingId
  }

  // ------------------------------------------------------------ consent

  function allowAndContinue(agent) {
    var job = root.pending
    root.pending = null
    configProc.queue([[root.bin, "meeting", "assistant", "allow", String(agent)]], job)
  }

  function useLocalAndContinue() {
    var job = root.pending
    root.pending = null
    configProc.queue([[root.bin, "config", "set", "meeting.assistant.local_only", "true"]], job)
  }

  function declinePending() { root.pending = null }

  function setLocalOnly(on) {
    configProc.queue([[root.bin, "config", "set", "meeting.assistant.local_only", on ? "true" : "false"]], null)
  }

  function chooseAgent(id) {
    configProc.queue([[root.bin, "config", "set", "meeting.assistant.agent", String(id)],
                      [root.bin, "config", "set", "meeting.assistant.local_only", "false"]], null)
  }

  signal assistantError(string message)

  // ------------------------------------------------------------ summary

  // "" (none yet), "summarizing", "ready" or "error".
  property string summaryState: ""
  property string summaryMarkdown: ""
  property string summaryError: ""
  property string summaryMeeting: ""

  function regenerate() {
    if (!root.feed || root.feed.meetingId === "") return
    if (root.blocker !== "") {
      root.pending = { kind: "summary" }
      root.consentNeeded()
      return
    }
    root.runSummary()
  }

  function runSummary() {
    if (summaryProc.running) return
    root.summaryMeeting = root.feed.meetingId
    root.summaryState = "summarizing"
    root.summaryError = ""
    summaryProc.out = ""
    summaryProc.err = ""
    summaryProc.launch([root.bin, "meeting", "summarize", root.feed.meetingId])
  }

  // A summary that came with the meeting (`meeting show --json`).
  function setStoredSummary(meetingId, summary) {
    var md = root.summaryToMarkdown(summary)
    if (md === "" || meetingId !== root.feed.meetingId) return
    root.summaryMeeting = meetingId
    root.summaryMarkdown = md
    root.summaryState = "ready"
  }

  // The stored summary, for a summary_ready that does not carry its text.
  function loadSummary(meetingId) {
    if (showProc.running) return
    showProc.out = ""
    showProc.launch([root.bin, "meeting", "show", meetingId, "--json"])
  }

  // MeetingSummary (src/meeting/data.rs) as markdown, with chapters if present.
  function summaryToMarkdown(s) {
    if (!s) return ""
    var out = []
    if (s.summary) out.push("## Overview\n\n" + s.summary)
    var list = function(title, items, fmt) {
      if (!items || items.length === undefined || items.length === 0) return
      var lines = []
      for (var i = 0; i < items.length; i++) lines.push("- " + fmt(items[i]))
      out.push("## " + title + "\n\n" + lines.join("\n"))
    }
    list("Decisions", s.decisions, function(d) { return String(d) })
    list("Action items", s.action_items, function(a) {
      return (a.assignee ? "**" + a.assignee + "**: " : "") + a.description + (a.due_date ? " (due " + a.due_date + ")" : "")
    })
    list("Key points", s.key_points, function(k) { return String(k) })
    list("Chapters", s.chapters, function(c) {
      var ms = Number(c.start_ms) || 0
      var t = Math.floor(ms / 1000)
      return "[" + Math.floor(t / 60) + ":" + (t % 60 < 10 ? "0" : "") + (t % 60) + "] " + (c.title || "")
    })
    return out.join("\n\n")
  }

  // ------------------------------------------------------------ notes

  property string notes: ""
  property string notesMeeting: ""
  // Typed but not yet saved; a reload never overwrites it.
  property bool notesDirty: false
  property string notesError: ""
  readonly property bool notesSaving: notesSaveProc.running

  function loadNotes() {
    if (!root.feed || root.feed.meetingId === "") return
    if (notesReadProc.running) {
      notesReadProc.again = true
      return
    }
    notesReadProc.meeting = root.feed.meetingId
    notesReadProc.out = ""
    notesReadProc.launch([root.bin, "meeting", "notes", root.feed.meetingId, "--json"])
  }

  function editNotes(text) {
    if (String(text) === root.notes) return
    root.notes = String(text)
    root.notesDirty = true
    notesDebounce.restart()
  }

  readonly property string notesPath: {
    var dir = Quickshell.env("XDG_RUNTIME_DIR")
    return (dir ? String(dir) : "/tmp") + "/voxtype/meeting-notes-" + root.notesMeeting + ".md"
  }

  function saveNotes() {
    if (!root.notesDirty || root.notesMeeting === "") return
    if (notesSaveProc.running) {
      notesDebounce.restart()
      return
    }
    notesSaveProc.saved = root.notes
    notesFile.setText(root.notes)
  }

  Timer {
    id: notesDebounce
    interval: 800
    repeat: false
    onTriggered: root.saveNotes()
  }

  FileView {
    id: notesFile
    path: root.notesMeeting !== "" ? root.notesPath : ""
    printErrors: false
    atomicWrites: true
    onSaved: {
      notesSaveProc.err = ""
      notesSaveProc.launch([root.bin, "meeting", "notes", root.notesMeeting, "--set-file", root.notesPath])
    }
    onSaveFailed: function(error) { root.notesError = "Could not write the notes file: " + error }
  }

  // ------------------------------------------------------------ wiring

  Connections {
    target: root.feed
    function onMeetingIdChanged() { root.followMeeting() }
    function onNotesUpdated(meetingId, notes) {
      if (meetingId !== root.notesMeeting || root.notesDirty) return
      if (notes !== null && notes !== undefined) root.notes = String(notes)
      else root.loadNotes()
    }
    function onSummaryReady(meetingId, markdown, summary) {
      if (meetingId !== root.feed.meetingId) return
      root.summaryMeeting = meetingId
      var md = markdown !== "" ? markdown : root.summaryToMarkdown(summary)
      if (md !== "") {
        root.summaryMarkdown = md
        root.summaryState = "ready"
      } else {
        root.loadSummary(meetingId)
      }
    }
    function onTransitioned(from, to) {
      if (to === "summarizing") {
        root.summaryMeeting = root.feed.meetingId
        root.summaryState = "summarizing"
      }
    }
  }

  // The feed moved to another meeting (a start, a stored meeting, or none):
  // notes and summary belong to the old one.
  function followMeeting() {
    var id = root.feed ? root.feed.meetingId : ""
    if (id === root.notesMeeting) return
    if (root.notesDirty) root.saveNotes()
    root.notesMeeting = id
    root.notes = ""
    root.notesDirty = false
    root.notesError = ""
    if (id !== root.summaryMeeting) {
      root.summaryState = ""
      root.summaryMarkdown = ""
      root.summaryError = ""
    }
    if (id !== "") root.loadNotes()
  }

  function runAfterStatus() {
    var job = root.afterStatus
    root.afterStatus = null
    if (job && job.kind === "ask") root.runAsk(job)
    else if (job && job.kind === "summary") root.runSummary()
  }

  MeetingProcess {
    id: statusProc
    property string out: ""
    property bool again: false
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: statusProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: root.statusError = String(text || "").trim() }
    onExited: function(code) {
      if (statusProc.again) {
        statusProc.again = false
        Qt.callLater(root.fetchStatus)
        return
      }
      if (code === 0) {
        try {
          var parsed = JSON.parse(statusProc.out)
          if (parsed && typeof parsed === "object") {
            root.status = parsed
            root.statusError = ""
          }
        } catch (e) {
          root.statusError = "voxtype meeting assistant status did not return JSON"
        }
      }
      root.runAfterStatus()
    }
    onFailedToStart: {
      root.statusError = "Could not run " + root.bin
      root.runAfterStatus()
    }
  }

  // Consent and agent settings, run in order, then the work they unblocked.
  MeetingProcess {
    id: configProc
    property var steps: []
    property var job: null
    property string err: ""

    function queue(list, then) {
      configProc.steps = list.slice()
      configProc.job = then
      configProc.next()
    }
    function next() {
      if (configProc.steps.length === 0) {
        root.afterStatus = configProc.job
        configProc.job = null
        root.fetchStatus()
        return
      }
      configProc.err = ""
      configProc.launch(configProc.steps.shift())
    }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: configProc.err = String(text || "").trim() }
    onExited: function(code) {
      if (code !== 0) {
        configProc.steps = []
        configProc.job = null
        root.fetchStatus()
        root.assistantError(configProc.err || "voxtype exited " + code)
        return
      }
      Qt.callLater(configProc.next)
    }
    onFailedToStart: {
      configProc.steps = []
      configProc.job = null
      root.assistantError("Could not run " + root.bin)
    }
  }

  MeetingProcess {
    id: askProc
    property int answerRow: -1
    property string err: ""
    stdout: SplitParser {
      // Every chunk as it arrives: the answer streams.
      splitMarker: ""
      onRead: function(chunk) {
        if (askProc.answerRow < 0 || askProc.answerRow >= threadModel.count) return
        threadModel.setProperty(askProc.answerRow, "text", threadModel.get(askProc.answerRow).text + chunk)
      }
    }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: askProc.err = String(text || "").trim() }
    onExited: function(code) {
      var row = askProc.answerRow
      if (row >= 0 && row < threadModel.count) {
        threadModel.setProperty(row, "pending", false)
        if (code !== 0) threadModel.setProperty(row, "error", askProc.err || "voxtype meeting ask exited " + code)
        else threadModel.setProperty(row, "text", threadModel.get(row).text.trim())
      }
      askProc.answerRow = -1
      Qt.callLater(root.pumpAsk)
    }
    onFailedToStart: {
      var row = askProc.answerRow
      if (row >= 0 && row < threadModel.count) {
        threadModel.setProperty(row, "pending", false)
        threadModel.setProperty(row, "error", "Could not run " + root.bin)
      }
      askProc.answerRow = -1
      Qt.callLater(root.pumpAsk)
    }
  }

  MeetingProcess {
    id: summaryProc
    property string out: ""
    property string err: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: summaryProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: summaryProc.err = String(text || "").trim() }
    onExited: function(code) {
      if (code !== 0) {
        root.summaryState = "error"
        root.summaryError = summaryProc.err || "voxtype meeting summarize exited " + code
        return
      }
      root.summaryMarkdown = summaryProc.out.trim()
      root.summaryState = "ready"
    }
    onFailedToStart: {
      root.summaryState = "error"
      root.summaryError = "Could not run " + root.bin
    }
  }

  MeetingProcess {
    id: showProc
    property string out: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: showProc.out = String(text || "") }
    onExited: function(code) {
      var data = null
      try { data = JSON.parse(showProc.out) } catch (e) {}
      var md = data && data.metadata ? root.summaryToMarkdown(data.metadata.summary) : ""
      if (code !== 0 || md === "") {
        root.summaryState = "error"
        root.summaryError = "The summary is ready but could not be read."
        return
      }
      root.summaryMarkdown = md
      root.summaryState = "ready"
    }
  }

  MeetingProcess {
    id: notesReadProc
    property string meeting: ""
    property string out: ""
    property bool again: false
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: notesReadProc.out = String(text || "") }
    onExited: function(code) {
      if (notesReadProc.again) {
        notesReadProc.again = false
        Qt.callLater(root.loadNotes)
        return
      }
      if (code !== 0 || notesReadProc.meeting !== root.notesMeeting || root.notesDirty) return
      var parsed = null
      try { parsed = JSON.parse(notesReadProc.out) } catch (e) {}
      root.notes = parsed && parsed.notes !== undefined && parsed.notes !== null ? String(parsed.notes) : ""
    }
  }

  MeetingProcess {
    id: notesSaveProc
    property string saved: ""
    property string err: ""
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: notesSaveProc.err = String(text || "").trim() }
    onExited: function(code) {
      if (code !== 0) {
        root.notesError = notesSaveProc.err || "voxtype meeting notes exited " + code
        return
      }
      root.notesError = ""
      // Typing during the save leaves the newer text dirty for the next one.
      if (root.notes === notesSaveProc.saved) root.notesDirty = false
      else notesDebounce.restart()
    }
    onFailedToStart: root.notesError = "Could not run " + root.bin
  }
}
