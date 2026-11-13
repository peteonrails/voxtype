import QtQuick
import Quickshell
import Quickshell.Io
import "MeetingText.js" as MT

// Notes, Ask and Summary for the meeting the feed holds. Non-visual and owned
// by Meetings.qml, so an answer still streams in, and the notes still save,
// after the view closes.
//
// The CLI it drives, as docs/INTEGRATIONS.md documents it (1.3.0):
//
//   voxtype meeting assistant status --json
//       {agent, agent_name, source, consent, choice_required, local_only,
//        local: {model, url, name}, backend: agent|local|none,
//        backend_name (the chip label), blocked (why backend is none, e.g.
//        "consent_required"), candidates: [{id, name, found, supported,
//        tool_less, sandbox, consent, note?}], ...}
//   voxtype meeting assistant allow <agent>
//   voxtype config set meeting.assistant.local_only true|false
//   voxtype config set meeting.assistant.agent <id>
//   voxtype meeting ask <id> "<question>" [--last 5m] [--notify] --json
//       NDJSON: {"event":"start", agent_name, ...}, {"event":"delta","text"}...,
//       then {"event":"answer", id, answer, agent_name, citations:[{ts, ms}]}
//       or {"event":"error", code, message} with exit 1
//   voxtype meeting summarize <id> [--regenerate] --format json
//       the MeetingSummary; without --regenerate a saved one is printed
//   voxtype meeting show <id> --json     metadata.summary, and `asks` (the
//                                        saved answer records)
//   voxtype meeting notes <id> --json    {"meeting_id", "notes", "updated_at"}
//   voxtype meeting notes <id> --set-file <path>
//   socket: {"event":"notes_updated", meeting_id, updated_at} (no text);
//           {"event":"status","status":"summarizing", meeting_id} and
//           {"event":"summary_ready", meeting_id, agent_name, path, summary},
//           or {"event":"error", meeting_id, code, message}, after the meeting
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
  // The chip: what would answer now ("Claude", "Local (llama3.2)"), or, while
  // nothing can, the agent waiting for consent.
  readonly property string agentLabel: {
    if (status === null) return ""
    if (status.backend_name) return String(status.backend_name)
    if (status.agent_name) return String(status.agent_name)
    return "No agent"
  }
  readonly property var candidates: status && status.candidates && status.candidates.length !== undefined
    ? status.candidates : []

  // Summaries can go elsewhere than questions (a legacy [meeting.summary]
  // backend such as Ollama): `summary_backend`/`summary_backend_name`, and the
  // Ask chip's backend where a status predates them.
  readonly property string summaryLabel: {
    if (status !== null && status.summary_backend_name) return String(status.summary_backend_name)
    return root.agentLabel
  }
  readonly property bool summaryIsLocal: {
    if (status === null) return false
    var b = status.summary_backend !== undefined && status.summary_backend !== null ? String(status.summary_backend)
      : String(status.backend || "")
    return b === "local" || b === "ollama" || b === "remote" || b.indexOf("local") === 0
  }
  // An agent the user has allowed, found and usable: what "Use … instead"
  // runs a failed local summary with. null when there is none, and when
  // local_only says nothing goes to an agent.
  readonly property var allowedAgent: {
    if (status !== null && status.local_only === true) return null
    for (var i = 0; i < root.candidates.length; i++) {
      var c = root.candidates[i]
      if (c.found === true && c.supported === true && c.consent === true) return c
    }
    return null
  }
  // The agents worth offering: found and supported.
  readonly property var usableCandidates: {
    var out = []
    for (var i = 0; i < root.candidates.length; i++)
      if (root.candidates[i].found === true && root.candidates[i].supported === true) out.push(root.candidates[i])
    return out
  }

  // What a question or summary needs before it can go out: "" (nothing the
  // view can help with), "consent" (the agent is not allowed yet) or "choose"
  // (several agents could answer and none is allowed). Other `blocked` codes
  // come back from the command itself as an error.
  readonly property string blocker: {
    if (status === null || status.blocked !== "consent_required") return ""
    return status.choice_required === true ? "choose" : "consent"
  }

  // Why nothing can answer, for the Ask and Summary tabs; "" when something
  // can, or when consent is all that is missing (the dialog handles that).
  readonly property string problem: {
    if (root.status === null) return root.statusError
    var s = root.status
    if (s.backend !== "none" || s.blocked === "consent_required") return ""
    if (s.problem) return String(s.problem)
    if (s.blocked === "no_backend")
      return "No agent or local model was found. Questions and summaries need Claude Code or Codex, "
        + "or Ollama for a local model. Install one, or name one in [meeting.assistant] agent."
    if (s.blocked === "agent_unavailable")
      return (s.agent_name || "The configured agent") + " was not found. Install it, or pick another agent."
    return "The meeting assistant cannot answer right now (" + s.blocked + ")."
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
  // Rows: {role: "q"|"a", text, agent, pending, error, askId, citations}, with
  // `citations` the answer's [{ts, ms}] as JSON (a list does not survive a
  // ListModel role).
  readonly property alias thread: threadModel
  ListModel { id: threadModel }

  // An ask to scroll to and flash once the thread has it (a notification's
  // Open).
  property string focusAskId: ""
  signal askFocused(int row)
  // The last answer focused, and when: a view that opens just after still
  // shows it.
  property int focusedRow: -1
  property real focusedAt: 0
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
    threadModel.append({ role: "q", text: job.question, agent: "", pending: false, error: "", askId: "", citations: "[]" })
    threadModel.append({ role: "a", text: "", agent: root.agentLabel, pending: true, error: "", askId: "", citations: "[]" })
    var args = [root.bin, "meeting", "ask", root.feed.meetingId, job.question]
    if (job.last !== "") args.push("--last", job.last)
    if (job.notify) args.push("--notify")
    args.push("--json")
    askQueue.push({ args: args, job: job })
    root.pumpAsk()
  }

  // The meeting's saved asks (`show --json`), as the thread. Not while a
  // question is still being answered here: its row would be lost.
  function applyAsks(meetingId, asks) {
    if (!root.feed || meetingId !== root.feed.meetingId) return
    root.syncThread()
    for (var i = 0; i < threadModel.count; i++)
      if (threadModel.get(i).pending) return
    threadModel.clear()
    var list = asks && asks.length !== undefined ? asks : []
    for (var j = 0; j < list.length; j++) {
      var a = list[j]
      threadModel.append({ role: "q", text: String(a.question || ""), agent: "", pending: false, error: "", askId: String(a.id || ""), citations: "[]" })
      threadModel.append({ role: "a", text: String(a.answer || ""), agent: String(a.agent_name || a.agent || ""),
                           pending: false, error: "", askId: String(a.id || ""),
                           citations: JSON.stringify(a.citations || []) })
    }
    root.focusPendingAsk()
  }

  function focusPendingAsk() {
    if (root.focusAskId === "") return
    for (var i = 0; i < threadModel.count; i++) {
      var r = threadModel.get(i)
      if (r.role === "a" && r.askId === root.focusAskId) {
        root.focusAskId = ""
        root.focusedRow = i
        root.focusedAt = Date.now()
        root.askFocused(i)
        return
      }
    }
  }

  function showAsk(askId) {
    root.focusAskId = String(askId || "")
    root.loadAsks()
  }

  function loadAsks() {
    if (!root.feed || root.feed.meetingId === "") return
    asksProc.load(root.feed.meetingId)
  }

  property var askQueue: []
  function pumpAsk() {
    if (askProc.running || askProc.startPending || root.askQueue.length === 0) return
    var next = root.askQueue.shift()
    askProc.err = ""
    askProc.job = next.job
    askProc.answerRow = root.pendingAnswerRow()
    askProc.launch(next.args)
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
  // The backend a summary in progress (or the failed one) went to, and the
  // failure's code ("local_failed", …) when there is one.
  property string summaryWith: ""
  property string summaryErrorCode: ""
  // A failure came from a local backend: offering an allowed agent instead.
  property bool summaryFailedLocal: false

  function regenerate() {
    if (!root.feed || root.feed.meetingId === "") return
    if (root.blocker !== "") {
      root.pending = { kind: "summary" }
      root.consentNeeded()
      return
    }
    root.runSummary()
  }

  // `meeting summarize` prints a saved summary as it is; --regenerate writes
  // a new one, which is what the button means once one exists.
  // `agent`: run with that agent for this summary only (`--agent`), the
  // way out of a local model that failed.
  function runSummary(agent) {
    if (summaryProc.running) return
    var regenerate = root.summaryMarkdown !== "" && root.summaryMeeting === root.feed.meetingId
    root.summaryMeeting = root.feed.meetingId
    root.beginSummarizing(agent ? root.nameOf(agent) : root.summaryLabel)
    summaryProc.local = !agent && root.summaryIsLocal
    summaryProc.out = ""
    summaryProc.err = ""
    var args = [root.bin, "meeting", "summarize", root.feed.meetingId, "--format", "json"]
    if (regenerate) args.push("--regenerate")
    if (agent) args.push("--agent", String(agent))
    summaryProc.launch(args)
  }

  function nameOf(agentId) {
    for (var i = 0; i < root.candidates.length; i++)
      if (root.candidates[i].id === agentId) return String(root.candidates[i].name || agentId)
    return String(agentId)
  }

  function beginSummarizing(withName) {
    root.summaryState = "summarizing"
    root.summaryWith = withName
    root.summaryError = ""
    root.summaryErrorCode = ""
    root.summaryFailedLocal = false
  }

  // Every way a summary can fail ends here, so "Summarizing…" never outlives
  // the attempt.
  function failSummary(message, code, local) {
    root.summaryState = "error"
    root.summaryError = message !== "" ? message : "The summary failed" + (code ? " (" + code + ")" : "") + "."
    root.summaryErrorCode = code || ""
    root.summaryFailedLocal = local === true || code === "local_failed"
  }

  function useAgentInstead() {
    if (root.allowedAgent) root.runSummary(String(root.allowedAgent.id))
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
    if (showProc.running) {
      showProc.again = meetingId
      return
    }
    showProc.meeting = meetingId
    showProc.out = ""
    showProc.launch([root.bin, "meeting", "show", meetingId, "--json"])
  }

  // Read the summary on screen again: a speaker rename moved its action
  // items' owners. Nothing is regenerated.
  function refreshSummary(meetingId) {
    if (meetingId === "" || meetingId !== root.summaryMeeting || root.summaryState !== "ready") return
    root.loadSummary(meetingId)
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

  // The meeting's folder (metadata.storage_path from `show --json`), for
  // Open folder; "" until read.
  property string meetingFolder: ""
  // Whether the meeting kept its audio (metadata.audio_retained): null until
  // known, which leaves ▶ offered.
  property var audioRetained: null

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
    function onSummaryUpdated(meetingId) { root.refreshSummary(meetingId) }
    function onSummaryReady(meetingId, markdown, summary) {
      if (meetingId !== root.feed.meetingId) return
      root.summaryMeeting = meetingId
      root.summaryError = ""
      root.summaryErrorCode = ""
      root.summaryFailedLocal = false
      var md = markdown !== "" ? markdown : root.summaryToMarkdown(summary)
      if (md !== "") {
        root.summaryMarkdown = md
        root.summaryState = "ready"
      } else {
        root.loadSummary(meetingId)
      }
    }
    // After the meeting, so these name it: a new one may be running by then.
    function onSummarizing(meetingId) {
      if (meetingId !== root.feed.meetingId) return
      root.summaryMeeting = meetingId
      root.beginSummarizing(root.summaryLabel)
      summaryProc.local = root.summaryIsLocal
    }
    // An error naming this meeting, or one naming none (an older daemon)
    // while a summary is being written, is the summary's failure: it goes on
    // the Summary tab, not in the banner.
    function onMeetingError(meetingId, code, message) {
      if (root.summaryState !== "summarizing") return
      if (meetingId !== "" && meetingId !== root.feed.meetingId) return
      root.failSummary(message, code, root.summaryIsLocal)
      if (meetingId === "") root.feed.lastError = ""
    }
  }

  // The feed moved to another meeting (a start, a stored meeting, or none):
  // notes and summary belong to the old one.
  function followMeeting() {
    var id = root.feed ? root.feed.meetingId : ""
    if (id === root.notesMeeting) return
    if (root.notesDirty) root.saveNotes()
    root.notesMeeting = id
    root.meetingFolder = ""
    root.audioRetained = null
    root.notes = ""
    root.notesDirty = false
    root.notesError = ""
    if (id !== root.summaryMeeting) {
      root.summaryState = ""
      root.summaryMarkdown = ""
      root.summaryError = ""
    }
    if (id !== "") {
      root.loadNotes()
      root.loadAsks()
    }
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
    property string err: ""
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: statusProc.err = String(text || "").trim() }
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
          root.statusError = "voxtype meeting assistant status did not return JSON."
        }
      } else {
        root.status = null
        root.statusError = MT.cliProblem(statusProc.err, "the meeting assistant (Ask and summaries)")
          || "voxtype meeting assistant status exited " + code + "."
      }
      root.runAfterStatus()
    }
    onFailedToStart: {
      root.status = null
      root.statusError = MT.missingCli(root.bin)
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
      root.assistantError(MT.missingCli(root.bin))
    }
  }

  MeetingProcess {
    id: askProc
    property int answerRow: -1
    property var job: null
    property string err: ""
    property string errorCode: ""

    function row() { return askProc.answerRow >= 0 && askProc.answerRow < threadModel.count ? askProc.answerRow : -1 }

    function handle(ev) {
      var r = askProc.row()
      if (r < 0) return
      var kind = String(ev.event || "")
      if (kind === "start") {
        if (ev.agent_name) threadModel.setProperty(r, "agent", String(ev.agent_name))
      } else if (kind === "delta") {
        threadModel.setProperty(r, "text", threadModel.get(r).text + String(ev.text || ""))
      } else if (kind === "answer") {
        threadModel.setProperty(r, "text", String(ev.answer || threadModel.get(r).text).trim())
        threadModel.setProperty(r, "askId", String(ev.id || ""))
        if (r > 0) threadModel.setProperty(r - 1, "askId", String(ev.id || ""))
        threadModel.setProperty(r, "citations", JSON.stringify(ev.citations || []))
        if (ev.agent_name) threadModel.setProperty(r, "agent", String(ev.agent_name))
      } else if (kind === "error") {
        askProc.errorCode = String(ev.code || "")
        threadModel.setProperty(r, "error", String(ev.message || ev.code || "The question failed."))
      }
    }

    stdout: SplitParser {
      onRead: function(line) {
        var text = String(line).trim()
        if (text === "") return
        try { askProc.handle(JSON.parse(text)) } catch (e) {}
      }
    }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: askProc.err = String(text || "").trim() }
    onExited: function(code) {
      var r = askProc.row()
      var job = askProc.job
      var consent = askProc.errorCode === "consent_required"
      if (r >= 0) {
        threadModel.setProperty(r, "pending", false)
        if (code !== 0 && threadModel.get(r).error === "")
          threadModel.setProperty(r, "error", MT.cliProblem(askProc.err, "the meeting assistant (Ask)") || "voxtype meeting ask exited " + code)
        // Finished without an answer or an error: say so rather than leave a
        // blank reply under the question.
        else if (code === 0 && !consent && threadModel.get(r).text === "" && threadModel.get(r).error === "")
          threadModel.setProperty(r, "error", "The assistant finished without an answer.")
      }
      askProc.answerRow = -1
      askProc.errorCode = ""
      // Consent went away (revoked elsewhere): ask, then try once more.
      if (consent && job) {
        if (r >= 1) threadModel.remove(r - 1, 2)
        root.pending = job
        root.fetchStatus()
        root.consentNeeded()
      }
      Qt.callLater(root.pumpAsk)
    }
    onFailedToStart: {
      var r = askProc.row()
      if (r >= 0) {
        threadModel.setProperty(r, "pending", false)
        threadModel.setProperty(r, "error", MT.missingCli(root.bin))
      }
      askProc.answerRow = -1
      Qt.callLater(root.pumpAsk)
    }
  }

  MeetingProcess {
    id: asksProc
    property string meeting: ""
    property string out: ""
    property bool again: false
    function load(id) {
      if (asksProc.running || asksProc.startPending) {
        asksProc.again = true
        return
      }
      asksProc.meeting = id
      asksProc.out = ""
      asksProc.launch([root.bin, "meeting", "show", id, "--json"])
    }
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: asksProc.out = String(text || "") }
    onExited: function(code) {
      if (asksProc.again) {
        asksProc.again = false
        Qt.callLater(root.loadAsks)
        return
      }
      if (code !== 0) return
      var data = null
      try { data = JSON.parse(asksProc.out) } catch (e) {}
      if (!data) return
      if (asksProc.meeting === root.feed.meetingId && data.metadata) {
        if (data.metadata.storage_path) root.meetingFolder = String(data.metadata.storage_path)
        if (data.metadata.audio_retained !== undefined) root.audioRetained = data.metadata.audio_retained === true
      }
      root.applyAsks(asksProc.meeting, data.asks)
    }
  }

  MeetingProcess {
    id: summaryProc
    property string out: ""
    property string err: ""
    // This run went to a local backend.
    property bool local: false
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: summaryProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: summaryProc.err = String(text || "").trim() }
    onExited: function(code) {
      if (code !== 0) {
        // Consent revoked since the status was read: ask, then try again.
        if (summaryProc.err.indexOf("consent") !== -1) {
          root.summaryState = root.summaryMarkdown !== "" ? "ready" : ""
          root.pending = { kind: "summary" }
          root.fetchStatus()
          root.consentNeeded()
          return
        }
        var code2 = /local_failed|local model|ollama|11434/i.test(summaryProc.err) ? "local_failed" : ""
        root.failSummary(MT.cliProblem(summaryProc.err, "summaries") || "voxtype meeting summarize exited " + code, code2,
                         summaryProc.local)
        return
      }
      var parsed = null
      try { parsed = JSON.parse(summaryProc.out) } catch (e) {}
      var md = root.summaryToMarkdown(parsed)
      if (md === "") {
        root.failSummary("voxtype meeting summarize printed no summary.", "", false)
        return
      }
      root.summaryMarkdown = md
      root.summaryState = "ready"
    }
    onFailedToStart: root.failSummary(MT.missingCli(root.bin), "", false)
  }

  MeetingProcess {
    id: showProc
    property string out: ""
    property string meeting: ""
    // A meeting to read again once this read ends: the summary changed
    // while it ran.
    property string again: ""
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: showProc.out = String(text || "") }
    onExited: function(code) {
      if (showProc.again !== "") {
        var next = showProc.again
        showProc.again = ""
        Qt.callLater(function() { root.loadSummary(next) })
        return
      }
      var data = null
      try { data = JSON.parse(showProc.out) } catch (e) {}
      var md = data && data.metadata ? root.summaryToMarkdown(data.metadata.summary) : ""
      if (code !== 0 || md === "") {
        root.failSummary("The summary is ready but could not be read.", "", false)
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
    property string err: ""
    property bool again: false
    stdout: StdioCollector { waitForEnd: true; onStreamFinished: notesReadProc.out = String(text || "") }
    stderr: StdioCollector { waitForEnd: true; onStreamFinished: notesReadProc.err = String(text || "").trim() }
    onFailedToStart: root.notesError = MT.missingCli(root.bin)
    onExited: function(code) {
      if (notesReadProc.again) {
        notesReadProc.again = false
        Qt.callLater(root.loadNotes)
        return
      }
      if (notesReadProc.meeting !== root.notesMeeting) return
      if (code !== 0) {
        root.notesError = MT.cliProblem(notesReadProc.err, "meeting notes") || "Could not read the notes."
        return
      }
      if (root.notesError.indexOf("too old") !== -1 || root.notesError.indexOf("Could not") === 0) root.notesError = ""
      if (root.notesDirty) return
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
        root.notesError = MT.cliProblem(notesSaveProc.err, "meeting notes") || "voxtype meeting notes exited " + code
        return
      }
      root.notesError = ""
      // Typing during the save leaves the newer text dirty for the next one.
      if (root.notes === notesSaveProc.saved) root.notesDirty = false
      else notesDebounce.restart()
    }
    onFailedToStart: root.notesError = MT.missingCli(root.bin)
  }
}
