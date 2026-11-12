import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "plugin" as Plugin
import "standalone" as Standalone
import "standalone/voxtype-meetings" as VM

// Driven by test-meeting-view.py. The meeting view runs under one of its two
// hosts (__HOST__: "omarchy", the Settings plugin's Panel.qml, or
// "standalone", quickshell/MeetingControls.qml) against a fake meeting.sock
// (fake-meeting-sock.py) and a fake CLI (fake-voxtype-meeting.py); this file
// feeds the socket through its control file and checks what the view shows.
Item {
  id: host

  readonly property bool omarchy: __HOST__ === "omarchy"
  readonly property var hostItem: hostLoader.item
  // Times the view went from closed to open, whichever host.
  property int opens: 0
  property bool wasOpen: false

  Loader {
    id: hostLoader
    sourceComponent: host.omarchy ? omarchyHost : standaloneHost
  }

  Component {
    id: omarchyHost
    Plugin.Panel { manifest: ({__sourceDir: __PLUGIN__}); shell: fakeShell }
  }

  Component {
    id: standaloneHost
    Standalone.MeetingControls { voxtypeBinary: __FAKE_BIN__ }
  }

  Timer {
    interval: 20
    running: true
    repeat: true
    onTriggered: {
      var open = host.viewOpen()
      if (open && !host.wasOpen) host.opens++
      host.wasOpen = open
    }
  }

  function ctrl() { return host.omarchy ? host.hostItem.meetingsTest : host.hostItem.meetings }
  function viewOpen() {
    if (!host.hostItem) return false
    return host.omarchy ? host.hostItem.opened && host.hostItem.view === "meetings" : host.hostItem.open
  }

  // Stands in for the Omarchy shell: records summons and opens the panel the
  // way the shell's summon() does.
  QtObject {
    id: fakeShell
    property var summons: []
    function summon(id, payload) {
      fakeShell.summons = fakeShell.summons.concat([String(payload)])
      host.hostItem.open(payload)
      return true
    }
    function hide(id) { host.hostItem.close(); return true }
    function toggle(id, payload) { if (host.hostItem.opened) fakeShell.hide(id); else fakeShell.summon(id, payload) }
  }

  Process {
    id: helper
    property bool done: true
    onExited: helper.done = true
  }

  TestCase {
    id: tc
    name: "MeetingView"
    when: false
    property bool started: false
    property string failure: ""
    property int shotCount: 0

    Timer {
      interval: 100
      running: !tc.started
      repeat: true
      onTriggered: {
        tc.started = true
        try {
          var s = __SCENARIO__
          if (s === "live") tc.driveLive()
          else if (s === "autoopen") tc.driveAutoOpen(true)
          else if (s === "autoopen-oldcli") tc.driveAutoOpen(true)
          else if (s === "autoopen-off") tc.driveAutoOpen(false)
          else if (s === "autoopen-gtk4") tc.driveAutoOpen(false)
          else if (s === "autoopen-settings") tc.driveAutoOpenSettings()
          else if (s === "reconnect") tc.driveReconnect()
          else if (s === "idle") tc.driveIdle()
          else if (s === "nodaemon") tc.driveNoDaemon()
          else if (s === "scroll") tc.driveScroll()
          else if (s === "reassign") tc.driveReassign()
          else if (s === "reassign-fails") tc.driveReassignFails()
          else if (s === "new-speaker") tc.driveNewSpeaker()
          else if (s === "restore") tc.driveRestore()
          else if (s === "external-update") tc.driveExternalUpdate()
          else if (s === "ended-edit") tc.driveEndedEdit()
          else if (s === "notes") tc.driveNotes()
          else if (s === "annotate") tc.driveAnnotate()
          else if (s === "ask") tc.driveAsk()
          else if (s === "ask-chips") tc.driveAskChips()
          else if (s === "ask-notify") tc.driveAskNotify()
          else if (s === "consent") tc.driveConsent()
          else if (s === "consent-choose") tc.driveConsentChoose()
          else if (s === "local-only") tc.driveLocalOnly()
          else if (s === "summary") tc.driveSummary()
          else if (s === "regenerate") tc.driveRegenerate()
          else if (s === "scroll-tabs") tc.driveScrollTabs()
          else if (s === "view-request") tc.driveViewRequest()
          else if (s === "list") tc.driveList()
          else if (s === "list-old") tc.driveListOld()
          else if (s === "past-view") tc.drivePastView()
          else if (s === "past-summary") tc.drivePastSummary()
          else if (s === "past-summarize") tc.drivePastSummarize()
          else if (s === "past-reassign") tc.drivePastReassign()
          else if (s === "start-while-browsing") tc.driveStartWhileBrowsing()
          else if (s === "ask-error") tc.driveAskError()
          else if (s === "cli-missing") tc.driveCliMissing()
          else if (s === "daemon-old") tc.driveDaemonOld()
          else if (s === "agent-missing") tc.driveAgentMissing()
          else if (s === "old-voxtype") tc.driveOldVoxtype()
          else if (s === "copy") tc.driveCopy()
          else if (s === "minimize") tc.driveMinimize()
          else if (s === "minimize-autoopen") tc.driveMinimizeAutoOpen()
          else if (s === "resize") tc.driveResize()
          else if (s === "move") tc.driveMove()
          else if (s === "opacity") tc.driveOpacity()
          else if (s === "view-state") tc.driveViewState()
          else if (s === "play") tc.drivePlay()
          else if (s === "play-no-audio") tc.drivePlayNoAudio()
          else if (s === "play-errors") tc.drivePlayErrors()
          else if (s === "line-edit") tc.driveLineEdit()
          else if (s === "line-delete") tc.driveLineDelete(false)
          else if (s === "line-delete-flag") tc.driveLineDelete(true)
          else if (s === "export") tc.driveExport()
          else if (s === "export-past") tc.driveExportPast()
          else if (s === "summary-error") tc.driveSummaryError()
          else if (s === "soundcheck") tc.driveSoundCheck()
          else if (s === "levels") tc.driveLevels()
          else if (s === "recovery") tc.driveRecovery()
          else if (s === "undo-edit") tc.driveUndoEdit()
          else if (s === "undo-reassign") tc.driveUndoReassign()
          else if (s === "seek") tc.driveSeek()
          else if (s === "waveform") tc.driveWaveform()
          else if (s === "waveform-missing") tc.driveWaveformMissing()
          else if (s === "recovery-old") tc.driveRecoveryOld()
          else if (s === "search") tc.driveSearch()
          else if (s === "search-old") tc.driveSearchOld()
          else if (s === "soundcheck-no-loopback") tc.driveSoundCheckNoLoopback()
          else if (s === "soundcheck-error") tc.driveSoundCheckError()
          else if (s === "summary-local-error") tc.driveSummaryLocalError()
          else if (s === "summary-error-cli") tc.driveSummaryErrorCli()
          else if (s === "summary-stale") tc.driveSummaryStale()
          else tc.check(false, "unknown scenario " + s)
        } catch (e) {
          if (tc.failure === "") tc.failure = String(e && e.message ? e.message : e)
        }
        console.log(tc.failure === "" ? "MEETING_VIEW_PASS " + __SCENARIO__
                                      : "MEETING_VIEW_FAIL " + __SCENARIO__ + ": " + tc.failure)
        Qt.callLater(Qt.quit)
      }
    }

    // ------------------------------------------------------------ helpers

    function check(ok, message) {
      if (!ok) {
        tc.failure = message
        throw new Error(message)
      }
    }

    function waitFor(predicate, message, timeout) {
      var until = Date.now() + (timeout || 5000)
      while (Date.now() < until) {
        if (predicate()) return
        wait(40)
      }
      check(false, "timed out: " + message)
    }

    function find(item, name) {
      if (!item) return null
      if (item.objectName === name) return item
      var kids = item.children || []
      for (var i = 0; i < kids.length; i++) {
        var hit = find(kids[i], name)
        if (hit) return hit
      }
      if (item.contentItem && item.contentItem !== item && kids.indexOf(item.contentItem) === -1)
        return find(item.contentItem, name)
      return null
    }

    function shown(item) {
      if (!item) return false
      for (var p = item; p; p = p.parent) if (!p.visible) return false
      return true
    }

    function click(item, what) {
      check(item !== null, what + " missing")
      wait(80)
      check(shown(item), what + " not visible")
      check(item.enabled, what + " disabled")
      mouseClick(item, item.width / 2, item.height / 2)
    }

    function sh(args) {
      helper.done = false
      helper.command = args
      helper.running = true
      waitFor(function() { return helper.done }, "helper " + args.join(" "))
    }

    // One command to the fake socket (see fake-meeting-sock.py).
    function ctl(cmd) {
      sh(["sh", "-c", 'printf "%s\\n" "$1" >> "$2"', "_", JSON.stringify(cmd), __CTL__])
    }

    function seg(id, startS, text, source, speaker, chunk) {
      var s = { id: id, start_ms: Math.round(startS * 1000), end_ms: Math.round(startS * 1000) + 1500,
                text: text, source: source, chunk_id: chunk }
      if (speaker) s.speaker_id = speaker
      return s
    }

    function feed() { return host.ctrl().feed }
    function view() { var w = host.ctrl().window; return w ? w.view : null }

    // Open the view the way a user would on this host: the IPC verb in
    // Omarchy, the keybinding's flag file in the standalone shell.
    function openHost() {
      if (host.omarchy) {
        check(host.hostItem.ipcTest.meetings() === "ok", "meetings verb")
        return
      }
      sh(["sh", "-c", 'mkdir -p "$1" && touch "$1/meeting-controls.flag"', "_", __RUNTIME__])
    }
    function txt(name) { var it = find(view(), name); return it ? String(it.text) : "<missing " + name + ">" }

    function chips() {
      var out = []
      var c = feed().chips
      for (var i = 0; i < c.length; i++) out.push(c[i].name)
      return out
    }

    // The chip delegates actually on screen, by name.
    function renderedChips() {
      var flow = find(view(), "speakerChips")
      var out = []
      for (var i = 0; i < flow.children.length; i++) {
        var chip = flow.children[i]
        if (String(chip.objectName).indexOf("speakerChip_") === 0) out.push(String(chip.name))
      }
      return out
    }

    // [time, speaker, text] per line, from the delegates on screen.
    function lines() {
      var lv = find(view(), "transcriptList")
      // Every row instantiated, so a row scrolled out of view is still read
      // (the tabs leave the transcript less room than the old layout had).
      if (lv.cacheBuffer < 100000) {
        lv.cacheBuffer = 100000
        wait(60)
      }
      // Rows removed or inserted this turn are laid out before reading.
      lv.forceLayout()
      var out = []
      for (var i = 0; i < lv.count; i++) {
        var item = lv.itemAtIndex(i)
        if (!item) continue
        out.push([String(find(item, "lineTime").text), String(find(item, "lineSpeaker").text),
                  String(find(item, "lineText").text)])
      }
      return out
    }

    function expectLines(wanted, what) {
      var got = JSON.stringify(lines())
      check(got === JSON.stringify(wanted), what + ": " + got)
    }

    function openView() {
      openHost()
      waitFor(function() { return view() !== null && host.viewOpen() }, "view opened")
      // Key events go to the TestCase's window: the meeting window's.
      tc.parent = view()
    }

    function rename(index, name) {
      var chip = find(view(), "speakerChip_" + index)
      click(chip, "chip " + index)
      var field = find(chip, "speakerRenameField")
      waitFor(function() { return field.activeFocus }, "rename field focused")
      check(field.text === String(chip.name), "field starts with the name: " + field.text)
      field.text = name
      keyClick(Qt.Key_Return)
    }

    function shot(name, wholeWindow) {
      if (__EVIDENCE__ === "") return
      var done = false
      wait(150)
      // The window's content includes the busy backdrop behind the card.
      var target = wholeWindow ? host.ctrl().window.shotItem : view()
      target.grabToImage(function(result) {
        result.saveToFile(__EVIDENCE__ + "/" + (++tc.shotCount) + "-" + name + ".png")
        done = true
      })
      waitFor(function() { return done }, "screenshot " + name)
    }

    // ------------------------------------------------------------ scenarios

    function driveLive() {
      openView()
      waitFor(function() { return feed().title === "Standup" && lines().length === 4 }, "hello rendered")
      check(txt("meetingTitle") === "Standup", "title: " + txt("meetingTitle"))
      check(txt("meetingState") === "Recording", "state: " + txt("meetingState"))
      check(/^1:0\d$/.test(txt("meetingElapsed")), "elapsed about 65 s: " + txt("meetingElapsed"))
      check(shown(find(view(), "listeningIndicator")), "listening indicator")
      check(txt("listeningText").indexOf("Listening") === 0, "listening: " + txt("listeningText"))
      check(!shown(find(view(), "idleState")), "no idle state while recording")
      expectLines([["0:04", "You", "Morning. Can everyone hear me?"],
                   ["0:09", "Remote", "Yes, loud and clear."],
                   ["0:21", "You", "Great, let's start with the release."],
                   ["0:35", "Remote", "The build went green overnight."]], "hello backlog")
      check(JSON.stringify(renderedChips()) === '["You","Remote"]', "chips: " + renderedChips())
      shot("recording")

      // A chunk's microphone lines arrive before its loopback lines, which
      // started earlier: the view inserts by start time.
      ctl({ cmd: "segments", chunk_id: 2, segments: [seg(5, 50, "I'll cut the tag after lunch.", "microphone", "You", 2)] })
      ctl({ cmd: "segments", chunk_id: 2, segments: [seg(6, 42, "Any blockers from QA?", "loopback", "Remote", 2)] })
      waitFor(function() { return lines().length === 6 }, "two new lines")
      expectLines([["0:04", "You", "Morning. Can everyone hear me?"],
                   ["0:09", "Remote", "Yes, loud and clear."],
                   ["0:21", "You", "Great, let's start with the release."],
                   ["0:35", "Remote", "The build went green overnight."],
                   ["0:42", "Remote", "Any blockers from QA?"],
                   ["0:50", "You", "I'll cut the tag after lunch."]], "inserted by start time")
      check(txt("listeningText").indexOf("last update") !== -1, "last update shown: " + txt("listeningText"))

      // Hybrid diarization hears a second remote voice.
      ctl({ cmd: "rename", from: "Remote", to: "Remote 1" })
      ctl({ cmd: "segments", chunk_id: 3, segments: [seg(7, 61, "Nothing from QA, ship it.", "loopback", "Remote 2", 3)] })
      waitFor(function() { return JSON.stringify(renderedChips()) === '["You","Remote 1","Remote 2"]' },
              "renamed and new speaker: " + renderedChips())
      waitFor(function() { return lines().length === 7 }, "the new voice's line rendered")
      check(lines()[1][1] === "Remote 1" && lines()[4][1] === "Remote 1", "old lines follow the rename: " + JSON.stringify(lines()))
      check(lines()[6][1] === "Remote 2", "new voice")
      shot("three-speakers")

      rename(1, "Alice")
      waitFor(function() { return JSON.stringify(renderedChips()) === '["You","Alice","Remote 2"]' }, "Alice: " + renderedChips())
      check(lines()[1][1] === "Alice" && lines()[3][1] === "Alice", "lines show Alice")

      // Two speakers named alike are one chip.
      rename(2, "Alice")
      waitFor(function() { return JSON.stringify(renderedChips()) === '["You","Alice"]' }, "merged: " + renderedChips())
      check(feed().chips[1].keys.length === 2, "the merged chip holds both keys")
      check(lines()[6][1] === "Alice", "merged speaker's lines")
      shot("merged")

      // Renaming the merged chip labels every key behind it.
      rename(1, "Team")
      waitFor(function() { return JSON.stringify(renderedChips()) === '["You","Team"]' }, "Team: " + renderedChips())

      // A segment arriving after a label carries it, and changes nothing.
      ctl({ cmd: "segments", chunk_id: 4, segments: [seg(8, 70, "Thanks all.", "loopback", "Remote 2", 4)] })
      waitFor(function() { return lines().length === 8 }, "labelled segment")
      check(lines()[7][1] === "Team", "label from the daemon: " + lines()[7][1])

      click(find(view(), "pauseButton"), "Pause")
      waitFor(function() { return txt("meetingState") === "Paused" }, "paused: " + txt("meetingState"))
      check(find(view(), "pauseButton").text === "Resume", "button offers Resume")
      check(txt("listeningText").indexOf("Paused") === 0, "paused line: " + txt("listeningText"))
      shot("paused")
      click(find(view(), "pauseButton"), "Resume")
      waitFor(function() { return txt("meetingState") === "Recording" }, "resumed: " + txt("meetingState"))

      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Finalizing" }, "finalizing: " + txt("meetingState"), 3000)
      check(!find(view(), "pauseButton").enabled && !find(view(), "stopButton").enabled, "no controls while finalizing")
      check(lines().length === 8, "transcript kept while finalizing")
      shot("finalizing")
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended after stop: " + txt("meetingState"))
      check(!shown(find(view(), "idleState")), "the ended meeting stays on screen")
      check(lines().length === 8 && txt("meetingTitle") === "Standup", "transcript and title kept")
      check(host.viewOpen(), "view stays open after the meeting")
      shot("ended")
    }

    function driveAutoOpen(expectOpen) {
      waitFor(function() { return feed().connected && feed().status === "idle" }, "feed connected while closed")
      check(!host.viewOpen(), "closed to begin with")
      // As `voxtype meeting start`, the detection prompt's Record button or
      // the tray would: the daemon starts a meeting and says so.
      ctl({ cmd: "start", title: "Design review" })
      waitFor(function() { return feed().meetingId !== "" && feed().title === "Design review" }, "hello for the new meeting")
      if (!expectOpen) {
        wait(1500)
        check(!host.viewOpen() && host.opens === 0, "show_view_on_start = false keeps the view closed")
        return
      }
      waitFor(function() { return host.viewOpen() && view() !== null }, "view opened itself")
      if (host.omarchy)
        check(JSON.stringify(fakeShell.summons) === JSON.stringify(['{"view":"meetings"}']), "summoned through the shell: " + JSON.stringify(fakeShell.summons))
      tc.parent = view()
      waitFor(function() { return txt("meetingTitle") === "Design review" }, "title: " + txt("meetingTitle"))
      check(txt("meetingState") === "Recording", "state: " + txt("meetingState"))
      shot("opened-on-start")
    }

    function driveAutoOpenSettings() {
      waitFor(function() { return feed().connected && feed().status === "idle" }, "feed connected")
      host.hostItem.open("{}")
      check(host.hostItem.opened && host.hostItem.view === "settings", "Settings open")
      ctl({ cmd: "start", title: "Design review" })
      waitFor(function() { return feed().meetingId !== "" }, "meeting seen")
      wait(1200)
      check(host.hostItem.view === "settings", "Settings not taken over")
      check(fakeShell.summons.length === 0, "no summon: " + JSON.stringify(fakeShell.summons))
    }

    function driveReconnect() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")

      // The daemon drops a slow subscriber: a fresh hello, no duplicates.
      ctl({ cmd: "drop" })
      wait(300)
      waitFor(function() { return feed().connected && lines().length === 4 }, "back after a drop: " + lines().length, 6000)

      // The socket goes away entirely.
      ctl({ cmd: "down" })
      waitFor(function() { return !feed().connected }, "disconnect noticed")
      check(shown(find(view(), "reconnectNotice")), "reconnecting notice")
      check(lines().length === 4, "transcript kept while disconnected")
      check(!find(view(), "pauseButton").enabled && !find(view(), "stopButton").enabled, "no controls while disconnected")
      check(!shown(find(view(), "listeningIndicator")), "no listening claim while disconnected")
      shot("reconnecting")
      // Lines the daemon produced meanwhile arrive in the next hello.
      ctl({ cmd: "segments", chunk_id: 2, segments: [seg(5, 44, "Did we lose you?", "loopback", "Remote", 2)] })
      ctl({ cmd: "up" })
      waitFor(function() { return feed().connected && lines().length === 5 }, "reconnected with the backlog", 12000)
      check(!shown(find(view(), "reconnectNotice")), "notice cleared")
      check(lines()[4][2] === "Did we lose you?", "new line from the hello")

      // The daemon restarts: the meeting is gone.
      ctl({ cmd: "down" })
      waitFor(function() { return !feed().connected }, "second disconnect")
      ctl({ cmd: "status", status: "idle" })
      ctl({ cmd: "up" })
      // The daemon restarts: the meeting is gone, but its lines stay to fix.
      waitFor(function() { return feed().connected && txt("meetingState") === "Ended" }, "Ended after a restart", 12000)
      check(lines().length === 5, "transcript kept after a restart")
      check(!shown(find(view(), "reconnectNotice")), "no reconnect notice once connected")
    }

    function driveIdle() {
      openView()
      waitFor(function() { return feed().connected && shown(find(view(), "idleState")) }, "idle state")
      check(txt("idleTitle") === "No meeting in progress", "idle title: " + txt("idleTitle"))
      check(!shown(find(view(), "lastMeeting")), "no last meeting yet")
      check(find(view(), "startButton").enabled, "Start enabled")
      shot("idle")

      find(view(), "startTitleField").text = "Retro"
      click(find(view(), "startButton"), "Start")
      waitFor(function() { return shown(find(view(), "actionError")) }, "start failure shown")
      check(view().actionError.indexOf("Meeting mode is disabled") !== -1, "stderr shown: " + view().actionError)
      check(find(view(), "startTitleField").text === "Retro", "title kept after a failed start")
      shot("start-failed")

      sh(["sh", "-c", 'printf "{}" > "$1"', "_", __STATE__])
      click(find(view(), "startButton"), "Start again")
      waitFor(function() { return feed().status === "recording" && txt("meetingTitle") === "Retro" }, "recording Retro: " + txt("meetingTitle"))
      check(!shown(find(view(), "actionError")), "error cleared")
      check(host.opens === 1, "opened once, by the user")
      shot("started")
    }

    function driveNoDaemon() {
      openView()
      wait(600)
      check(!feed().connected, "nothing to connect to")
      check(txt("idleTitle") === "Voxtype is not running", "idle title: " + txt("idleTitle"))
      check(!find(view(), "startButton").enabled, "Start disabled without a daemon")
      shot("no-daemon")
      ctl({ cmd: "up" })
      waitFor(function() { return feed().connected }, "connected once the daemon is up", 12000)
      waitFor(function() { return txt("idleTitle") === "No meeting in progress" }, "idle title: " + txt("idleTitle"))
      check(find(view(), "startButton").enabled, "Start enabled")
    }

    // ------------------------------------------------------------ reassign

    function lineItem(i) { return find(view(), "transcriptList").itemAtIndex(i) }
    function popup() { return find(view(), "reassignPopup") }

    function openChooser(i) {
      click(find(lineItem(i), "lineSpeakerButton"), "speaker on line " + i)
      waitFor(function() { return popup().visible }, "chooser open for line " + i)
    }

    // [name, current] for each chip the chooser offers.
    function chooserChips() {
      var flow = find(popup(), "reassignChips")
      var out = []
      for (var i = 0; i < flow.children.length; i++) {
        var c = flow.children[i]
        if (String(c.objectName).indexOf("reassignChip_") === 0) out.push([String(c.name), c.current])
      }
      return out
    }

    function pick(name) {
      var flow = find(popup(), "reassignChips")
      for (var i = 0; i < flow.children.length; i++) {
        var c = flow.children[i]
        if (String(c.objectName).indexOf("reassignChip_") === 0 && String(c.name) === name) {
          click(c, "chooser chip " + name)
          return
        }
      }
      check(false, "no chooser chip " + name + ": " + JSON.stringify(chooserChips()))
    }

    function moved(i) {
      var m = find(lineItem(i), "lineMoved")
      return m && m.visible ? String(m.text) : ""
    }

    function idleCli() { return host.ctrl().cli.busy === "" && host.ctrl().cli.queue.length === 0 }

    function typeNewSpeaker(name) {
      var field = find(popup(), "newSpeakerField")
      click(field, "New speaker field")
      waitFor(function() { return field.activeFocus }, "New speaker field focused")
      field.text = name
      keyClick(Qt.Key_Return)
    }

    function driveReassign() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      openChooser(1)
      check(txt("reassignLineText") === "“Yes, loud and clear.”", "chooser names the line: " + txt("reassignLineText"))
      check(JSON.stringify(chooserChips()) === '[["You",false],["Remote",true]]', "chooser chips: " + JSON.stringify(chooserChips()))
      check(!shown(find(popup(), "restoreButton")), "no Restore for a line never moved")
      shot("chooser")
      // Escape closes the chooser, not the view.
      keyClick(Qt.Key_Escape)
      waitFor(function() { return !popup().visible }, "Escape closed the chooser")
      check(host.viewOpen() && view() !== null, "view still open")

      openChooser(1)
      pick("You")
      check(!popup().visible, "chooser closes on a pick")
      check(lines()[1][1] === "You", "moved at once: " + lines()[1][1])
      waitFor(idleCli, "reassign finished")
      wait(300)
      check(lines()[1][1] === "You", "still moved after the CLI and the socket's echo: " + lines()[1][1])
      check(moved(1) === "moved from Remote", "marked moved: " + moved(1))
      check(JSON.stringify(renderedChips()) === '["You","Remote"]', "chips: " + renderedChips())
      check(!shown(find(view(), "actionError")), "no error")
      shot("moved")
    }

    function driveReassignFails() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      openChooser(1)
      pick("You")
      check(lines()[1][1] === "You", "moved at once: " + lines()[1][1])
      waitFor(function() { return shown(find(view(), "actionError")) }, "refusal shown")
      check(view().actionError.indexOf("cannot be reassigned") !== -1, "stderr in the banner: " + view().actionError)
      check(lines()[1][1] === "Remote", "rolled back: " + lines()[1][1])
      check(moved(1) === "", "not marked moved after a rollback: " + moved(1))
      shot("reassign-failed")
    }

    function driveNewSpeaker() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      openChooser(3)
      typeNewSpeaker("Dana")
      waitFor(function() { return !popup().visible }, "chooser closed")
      check(lines()[3][1] === "Dana", "moved to Dana at once: " + lines()[3][1])
      waitFor(idleCli, "reassign finished")
      wait(300)
      check(lines()[3][1] === "Dana", "Dana after the CLI: " + lines()[3][1])
      check(JSON.stringify(renderedChips()) === '["You","Remote","Dana"]', "a chip for Dana: " + renderedChips())
      shot("new-speaker")

      // A typed name that is an existing speaker's goes to that speaker.
      openChooser(1)
      typeNewSpeaker("you")
      waitFor(idleCli, "second reassign finished")
      wait(300)
      check(lines()[1][1] === "You", "matched You: " + lines()[1][1])
      // Remote has no lines left, so its chip goes.
      check(JSON.stringify(renderedChips()) === '["You","Dana"]', "no new chip for you: " + renderedChips())
    }

    function driveRestore() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      openChooser(1)
      pick("You")
      waitFor(idleCli, "reassign finished")
      wait(300)
      openChooser(1)
      var restore = find(popup(), "restoreButton")
      check(shown(restore), "Restore original offered")
      check(String(restore.text) === "Restore original (Remote)", "names the original: " + restore.text)
      shot("restore-offered")
      click(restore, "Restore original")
      check(lines()[1][1] === "Remote", "restored at once: " + lines()[1][1])
      waitFor(idleCli, "reset finished")
      wait(300)
      check(lines()[1][1] === "Remote" && moved(1) === "", "restored: " + lines()[1][1] + " " + moved(1))
      openChooser(1)
      check(!shown(find(popup(), "restoreButton")), "no Restore after restoring")
      keyClick(Qt.Key_Escape)
    }

    function driveExternalUpdate() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      // The daemon's form: tagged `event`, the override in original_speaker.
      var s4 = seg(4, 35, "The build went green overnight.", "loopback", "You", 1)
      s4.original_speaker = "Remote"
      ctl({ cmd: "update", key: "event", segment: s4 })
      waitFor(function() { return lines()[3][1] === "You" }, "segment_updated applied: " + JSON.stringify(lines()))
      check(moved(3) === "moved from Remote", "marked moved: " + moved(3))
      openChooser(3)
      check(String(find(popup(), "restoreButton").text) === "Restore original (Remote)", "Restore offered from the segment")
      shot("external-update")
      keyClick(Qt.Key_Escape)

      // A segment without original_speaker has no override, whatever it
      // was before: no mark, no Restore.
      ctl({ cmd: "update", key: "event", segment: seg(3, 21, "Great, let's start with the release.", "microphone", "Remote", 0) })
      waitFor(function() { return lines()[2][1] === "Remote" }, "plain update applied")
      check(moved(2) === "", "no mark without original_speaker: " + moved(2))
      openChooser(2)
      check(!shown(find(popup(), "restoreButton")), "no Restore without original_speaker")
      keyClick(Qt.Key_Escape)

      // The mark comes back with the hello after a reconnect.
      ctl({ cmd: "drop" })
      wait(300)
      waitFor(function() { return feed().connected && lines().length === 4 }, "reconnected", 6000)
      check(lines()[3][1] === "You" && moved(3) === "moved from Remote", "mark survives a reconnect: " + moved(3))

      // A reset from elsewhere arrives without the field.
      ctl({ cmd: "update", key: "event", segment: seg(4, 35, "The build went green overnight.", "loopback", "Remote", 1) })
      waitFor(function() { return lines()[3][1] === "Remote" && moved(3) === "" }, "reset from elsewhere")

      // An early draft of the contract tagged it `type`; still understood.
      var s1 = seg(1, 4, "Morning. Can everyone hear me?", "microphone", "Remote", 0)
      s1.original_speaker = "You"
      ctl({ cmd: "update", key: "type", segment: s1 })
      waitFor(function() { return lines()[0][1] === "Remote" && moved(0) === "moved from You" }, "type-tagged update")
      check(lines().length === 4, "no line added or lost")
    }

    function driveEndedEdit() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended: " + txt("meetingState"))
      check(lines().length === 4 && txt("meetingTitle") === "Standup", "transcript and title kept")
      check(/^at \d\d:\d\d$/.test(txt("meetingEndedAt")), "end time: " + txt("meetingEndedAt"))
      check(/^1:0\d$/.test(txt("meetingElapsed")), "length: " + txt("meetingElapsed"))
      check(!shown(find(view(), "pauseButton")) && !shown(find(view(), "stopButton")), "no Pause or Stop")
      check(shown(find(view(), "endedStartButton")) && find(view(), "endedStartButton").enabled, "Start meeting offered")
      check(!shown(find(view(), "idleState")), "not the idle screen")
      // One how-to, in the Transcript tab; none in the header.
      check(!shown(find(view(), "listeningIndicator")), "no second hint under the header")
      check(shown(find(view(), "speakerHint")), "the speaker how-to")
      shot("ended")
      click(find(view(), "speakerHintDismiss"), "dismiss the how-to")
      waitFor(function() { return !shown(find(view(), "speakerHint")) }, "how-to dismissed")
      check(view().speakerHintDismissed === true, "remembered by the controller for the session")

      // Fixing a line after the call. No socket echo: only --json answers.
      openChooser(1)
      pick("You")
      waitFor(idleCli, "reassign finished")
      check(lines()[1][1] === "You" && moved(1) === "moved from Remote", "moved after the end: " + JSON.stringify(lines()[1]))
      rename(1, "Sam")
      waitFor(function() { return lines()[3][1] === "Sam" }, "rename after the end: " + JSON.stringify(lines()))
      check(JSON.stringify(renderedChips()) === '["You","Sam"]', "chips: " + renderedChips())
      shot("ended-edited")

      // A new meeting replaces it.
      find(view(), "endedTitleField").text = "Second"
      click(find(view(), "endedStartButton"), "Start meeting")
      waitFor(function() { return feed().status === "recording" && txt("meetingTitle") === "Second" }, "new meeting: " + txt("meetingTitle"))
      check(lines().length === 0 && renderedChips().length === 0, "old transcript gone")

      // Closing the view drops an ended meeting.
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Ended" }, "second meeting ended")
      var close = find(view(), "closeButton")
      tc.parent = host
      click(close, "Close")
      waitFor(function() { return !host.viewOpen() && view() === null }, "view closed")
      check(!feed().ended && feed().segments.count === 0, "ended meeting dropped on close")
      openView()
      waitFor(function() { return shown(find(view(), "idleState")) }, "idle after reopening")
    }

    // ------------------------------------------------------------ assistant

    function tab(name) {
      click(find(view(), "tab_" + name), "tab " + name)
      waitFor(function() { return view().currentTab === name }, "on the " + name + " tab")
    }

    function assistant() { return host.ctrl().assistant }
    function thread() { return assistant().thread }
    function answered(n) {
      return thread().count >= n * 2 && !thread().get(n * 2 - 1).pending
    }
    function chipLabel() {
      var pane = find(view(), view().currentTab === "summary" ? "summaryPane" : "askPane")
      return String(find(pane, "agentChip").text)
    }

    // A request as `voxtype meeting view` (or a notification's Open) writes it.
    function request(req) {
      if (req.requested_at === undefined) req.requested_at = Date.now()
      sh(["sh", "-c", 'mkdir -p "$1" && printf "%s" "$2" > "$1/meeting-view.request.tmp" && mv "$1/meeting-view.request.tmp" "$1/meeting-view.request"',
          "_", __RUNTIME__, JSON.stringify(req)])
    }

    function typeAsk(q) {
      var field = find(find(view(), "askPane"), "askField")
      click(field, "Ask field")
      field.text = q
      keyClick(Qt.Key_Return)
    }

    function driveNotes() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      tab("notes")
      var area = find(view(), "notesArea")
      click(area, "notes pad")
      area.text = "Ask about the release date"
      check(txt("notesStatus") === "Edited", "edited: " + txt("notesStatus"))
      waitFor(function() { return txt("notesStatus") === "Saved" }, "autosaved: " + txt("notesStatus"))
      shot("notes")

      // Changed elsewhere: notes_updated reloads the pad.
      sh(["sh", "-c", 'printf "From the CLI" > "$1/notes-$2.md"', "_", __DATA__, __MEETING_ID__])
      ctl({ cmd: "notes_updated", meeting_id: __MEETING_ID__ })
      waitFor(function() { return area.text === "From the CLI" }, "reloaded: " + area.text)

      // Unsaved typing is never overwritten by an update from elsewhere.
      area.text = "Mine"
      ctl({ cmd: "notes_updated", meeting_id: __MEETING_ID__, notes: "Theirs" })
      wait(300)
      check(area.text === "Mine", "kept the typing: " + area.text)
      waitFor(function() { return txt("notesStatus") === "Saved" }, "saved Mine")
    }

    function driveAnnotate() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      var item = lineItem(2)
      mouseMove(item, 30, 4)
      var note = find(item, "lineNoteButton")
      waitFor(function() { return note.visible }, "Add note on hover")
      check(String(note.text) === "Add note", "Add note: " + note.text)
      mouseClick(note, note.width / 2, note.height / 2)
      var editor = find(view(), "noteEditor")
      waitFor(function() { return editor.visible }, "note editor open")
      var field = find(editor, "noteField")
      waitFor(function() { return field.activeFocus }, "note field focused")
      field.text = "check this number"
      keyClick(Qt.Key_Return)
      check(!editor.visible, "editor closes on save")
      check(find(lineItem(2), "lineMarker").visible, "margin marker at once")
      check(String(find(lineItem(2), "lineAnnotation").text) === "Note: check this number", "note shown")
      waitFor(idleCli, "annotate finished")
      wait(300)
      check(find(lineItem(2), "lineMarker").visible, "marker after the daemon's segment_updated")
      shot("annotated")

      // A highlight from elsewhere marks the margin without a note.
      var s2 = seg(2, 9, "Yes, loud and clear.", "loopback", "Remote", 0)
      s2.annotation = { text: "", highlight: true }
      ctl({ cmd: "update", key: "event", segment: s2 })
      waitFor(function() { return find(lineItem(1), "lineMarker").visible }, "highlight marker")
      check(!find(lineItem(1), "lineAnnotation").visible, "no note text for a highlight")

      // A bare highlight from the editor: --highlight with no text.
      mouseMove(lineItem(0), 30, 4)
      note = find(lineItem(0), "lineNoteButton")
      waitFor(function() { return note.visible }, "Add note on line 0")
      mouseClick(note, note.width / 2, note.height / 2)
      waitFor(function() { return editor.visible }, "editor for a highlight")
      click(find(editor, "noteHighlightButton"), "Highlight")
      click(find(editor, "noteSaveButton"), "Save")
      check(find(lineItem(0), "lineMarker").visible && !find(lineItem(0), "lineAnnotation").visible, "highlighted, no note")
      waitFor(idleCli, "highlight finished")
      wait(300)
      check(find(lineItem(0), "lineMarker").visible, "highlight kept after the daemon")
      check(String(find(lineItem(0), "lineNoteButton").text) === "Edit note", "a highlighted line offers Edit")

      mouseMove(lineItem(2), 30, 4)
      note = find(lineItem(2), "lineNoteButton")
      waitFor(function() { return note.visible && String(note.text) === "Edit note" }, "Edit note")
      mouseClick(note, note.width / 2, note.height / 2)
      waitFor(function() { return editor.visible }, "editor again")
      check(find(editor, "noteField").text === "check this number", "editor holds the note")
      click(find(editor, "noteRemoveButton"), "Remove")
      check(!find(lineItem(2), "lineMarker").visible, "marker gone at once")
      waitFor(idleCli, "remove finished")
      wait(300)
      check(!find(lineItem(2), "lineMarker").visible, "marker gone after the daemon")
    }

    function driveAsk() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      tab("ask")
      waitFor(function() { return chipLabel() === "Claude  ▾" }, "agent chip: " + chipLabel())
      typeAsk("What did we decide?")
      waitFor(function() { return thread().count === 2 }, "question and answer rows")
      check(thread().get(0).text === "What did we decide?", "question shown")
      // It streams: text arrives while the answer is still pending.
      waitFor(function() { return thread().get(1).pending && thread().get(1).text !== "" }, "a partial answer", 3000)
      var partial = thread().get(1).text
      waitFor(function() { return answered(1) }, "answer finished", 8000)
      var full = thread().get(1).text
      check(partial.length < full.length, "streamed in pieces: " + partial.length + " < " + full.length)
      check(full.indexOf("[00:35] The build went green") === 0, "answer: " + full)
      var row = find(view(), "askThread").itemAtIndex(1)
      check(String(find(row, "answerBy").text) === "Answered by Claude", "attribution: " + find(row, "answerBy").text)
      shot("ask")

      // The answer opens with its [0:35] citation: clicking it shows that line.
      var answerText = find(row, "answerText")
      check(String(answerText.text).indexOf('<a href="#t=35000"') === 0, "the answer's citation linked: " + answerText.text)
      check(String(answerText.text).indexOf('<a href="#t=50000"') !== -1, "both citations: " + answerText.text)
      mouseClick(answerText, 6, 8)
      waitFor(function() { return view().currentTab === "transcript" }, "citation opened the transcript")
      waitFor(function() { return find(view(), "transcriptPane").flashSegId === 4 }, "the cited line flashes: " + find(view(), "transcriptPane").flashSegId)
      shot("citation")
    }

    function driveAskChips() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      tab("ask")
      var pane = find(view(), "askPane")
      check(String(find(pane, "askChip_lastSpeaker").text) === "What did Remote just say?", "last speaker chip: " + find(pane, "askChip_lastSpeaker").text)
      click(find(pane, "askChip_last5"), "Last 5 minutes")
      waitFor(function() { return answered(1) }, "first chip answered", 8000)
      click(find(pane, "askChip_lastSpeaker"), "last speaker")
      waitFor(function() { return answered(2) }, "second chip answered", 8000)
      click(find(pane, "askChip_actions"), "action items")
      waitFor(function() { return answered(3) }, "third chip answered", 8000)
      check(thread().get(2).text === "What did Remote just say?", "question text: " + thread().get(2).text)
      shot("ask-chips")
    }

    function driveAskNotify() {
      waitFor(function() { return feed().connected && feed().meetingId !== "" }, "feed has the meeting")
      check(!host.viewOpen(), "closed")
      // A quick question with the view closed: answered as a notification.
      request({ ask: "last-5m", open: false })
      waitFor(function() { return thread().count === 2 && answered(1) }, "answered while closed", 8000)
      check(!host.viewOpen(), "still closed")
      var askId = thread().get(1).askId
      check(askId === "ask-1", "the answer's id from the stream: " + askId)
      // The notification's Open action: open_command `voxtype meeting view`
      // with VOXTYPE_MEETING_ID and VOXTYPE_ASK_ID set writes this.
      request({ meeting_id: __MEETING_ID__, ask_id: askId, tab: "ask", open: true })
      waitFor(function() { return host.viewOpen() && view() !== null }, "Open raised the view")
      tc.parent = view()
      waitFor(function() { return view().currentTab === "ask" }, "on the Ask tab: " + view().currentTab)
      check(find(view(), "askThread").count === 2, "the answer is in the thread")
      waitFor(function() { return find(view(), "askPane").flashRow === 1 }, "that answer is shown")
      shot("opened-from-notification")
    }

    function driveConsent() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      tab("ask")
      waitFor(function() { return assistant().status !== null }, "assistant status read")
      typeAsk("What did we decide?")
      var dialog = find(view(), "consentDialog")
      waitFor(function() { return dialog.visible }, "consent asked")
      check(txt("consentTitle") === "Send meeting transcripts to Claude?", "title: " + txt("consentTitle"))
      check(thread().count === 0, "nothing asked yet")
      shot("consent")
      click(find(dialog, "consentNotNow"), "Not now")
      check(!dialog.visible && thread().count === 0, "Not now asks nothing")

      typeAsk("What did we decide?")
      waitFor(function() { return dialog.visible }, "asked again")
      click(find(dialog, "consentAllow_claude"), "Send to Claude")
      waitFor(function() { return answered(1) }, "answered after consent", 8000)
      waitFor(function() { return assistant().blocker === "" }, "consent recorded")
      typeAsk("And the date?")
      waitFor(function() { return answered(2) }, "second question needs no dialog", 8000)
      check(!dialog.visible, "no dialog the second time")
    }

    function driveConsentChoose() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      tab("ask")
      waitFor(function() { return assistant().status !== null }, "assistant status read")
      typeAsk("What did we decide?")
      var dialog = find(view(), "consentDialog")
      waitFor(function() { return dialog.visible }, "consent asked")
      check(txt("consentTitle") === "Send meeting transcripts to Claude or Codex?", "title: " + txt("consentTitle"))
      check(find(dialog, "consentAllow_claude") !== null && find(dialog, "consentAllow_codex") !== null, "one button per agent")
      shot("consent-choose")
      click(find(dialog, "consentLocal"), "Use a local model instead")
      waitFor(function() { return answered(1) }, "answered locally", 8000)
      waitFor(function() { return chipLabel() === "Local (llama3.2)  ▾" }, "chip says local: " + chipLabel())
      var row = find(view(), "askThread").itemAtIndex(1)
      check(String(find(row, "answerBy").text) === "Answered by Local (llama3.2)", "attribution: " + find(row, "answerBy").text)
    }

    function driveLocalOnly() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      tab("ask")
      waitFor(function() { return chipLabel() === "Claude  ▾" }, "agent chip: " + chipLabel())
      var menu = find(view(), "agentMenu")
      click(find(find(view(), "askPane"), "agentChip"), "agent chip")
      waitFor(function() { return menu.visible }, "menu open")
      check(find(menu, "agentOption_claude") !== null && find(menu, "agentOption_codex") !== null, "agents listed")
      check(shown(find(menu, "openSettings")) === host.omarchy, "Change in Settings only where the host can open them")
      check(String(find(menu, "agentNote_codex").text).indexOf("bwrap jail") !== -1, "codex's jail explained: " + find(menu, "agentNote_codex").text)
      shot("agent-menu")
      click(find(menu, "localOnlyToggle"), "Local model only")
      check(!menu.visible, "menu closes")
      waitFor(function() { return chipLabel() === "Local (llama3.2)  ▾" }, "local: " + chipLabel())
      click(find(find(view(), "askPane"), "agentChip"), "agent chip")
      waitFor(function() { return menu.visible }, "menu again")
      check(String(find(menu, "localOnlyToggle").text) === "✓ Local model only (llama3.2)", "checked: " + find(menu, "localOnlyToggle").text)
      keyClick(Qt.Key_Escape)
      check(!menu.visible && host.viewOpen(), "Escape closes the menu, not the view")
      click(find(find(view(), "askPane"), "agentChip"), "agent chip")
      waitFor(function() { return menu.visible }, "menu a third time")
      click(find(menu, "localOnlyToggle"), "Local model only off")
      waitFor(function() { return chipLabel() === "Claude  ▾" }, "back to Claude: " + chipLabel())
      click(find(find(view(), "askPane"), "agentChip"), "agent chip")
      waitFor(function() { return menu.visible }, "menu for Codex")
      click(find(menu, "agentOption_codex"), "Codex")
      waitFor(function() { return chipLabel() === "Codex  ▾" }, "Codex: " + chipLabel())
      if (host.omarchy) {
        click(find(find(view(), "askPane"), "agentChip"), "agent chip")
        waitFor(function() { return menu.visible }, "menu for Settings")
        var settings = find(menu, "openSettings")
        tc.parent = host
        click(settings, "Change in Settings")
        waitFor(function() { return host.hostItem.opened && host.hostItem.view === "settings" }, "Settings opened")
      }
    }

    function driveSummary() {
      ctl({ cmd: "autosummary", summary: {
        summary: "A short standup about the release.", decisions: ["Ship today."],
        action_items: [{ description: "Cut the tag", assignee: "Pete", completed: false }], key_points: [],
        chapters: [{ start_ms: 4000, title: "Greetings" }, { start_ms: 35000, title: "Release status" }] } })
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      check(find(view(), "tab_summary") === null || !find(view(), "tab_summary").visible, "no Summary tab while recording")
      click(find(view(), "stopButton"), "Stop")
      // The daemon is idle before it summarizes: the meeting shows Ended
      // first, then the summarizing event (naming it) takes over the tab.
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended", 5000)
      waitFor(function() { return view().currentTab === "summary" && shown(find(view(), "summarizing")) }, "summarizing", 5000)
      check(txt("meetingState") === "Summarizing", "state: " + txt("meetingState"))
      check(chipLabel() === "Claude  ▾", "agent chip in the Summary header: " + chipLabel())
      shot("summarizing")
      waitFor(function() { return txt("summaryText").indexOf("## Overview") === 0 }, "summary ready: " + txt("summaryText"), 8000)
      var md = txt("summaryText")
      check(md.indexOf("**Pete**: Cut the tag") !== -1, "action item with owner: " + md)
      check(/<a href="#t=35000"[^>]*>0:35<\/a> Release status/.test(md), "chapter linked: " + md)
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended after summary_ready")
      shot("summary")
      // A chapter jumps into the transcript.
      find(view(), "summaryText").linkActivated("#t=35000")
      waitFor(function() { return view().currentTab === "transcript" }, "chapter opened the transcript")
      waitFor(function() { return find(view(), "transcriptPane").flashSegId === 4 }, "the chapter's line flashes")
    }

    function driveRegenerate() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended")
      tab("summary")
      check(shown(find(view(), "summaryEmpty")), "no summary yet")
      var button = find(view(), "regenerateButton")
      check(String(button.text) === "Summarize", "button: " + button.text)
      click(button, "Summarize")
      waitFor(function() { return shown(find(view(), "summarizing")) }, "summarizing")
      check(!button.enabled, "no second run while one is going")
      waitFor(function() { return txt("summaryText").indexOf("Release status") !== -1 }, "summary", 8000)
      check(String(button.text) === "Regenerate", "Regenerate now: " + button.text)
      sh(["sh", "-c", 'printf "%s" "$2" > "$1"', "_", __STATE__,
          JSON.stringify({ summary_json: { summary: "Regenerated after fixing speakers [0:21].", key_points: [],
                                           decisions: [], action_items: [], generated_at: "2026-11-11T17:00:00Z" } })])
      click(button, "Regenerate")
      waitFor(function() { return txt("summaryText").indexOf("Regenerated after fixing speakers") !== -1 }, "regenerated", 8000)
      shot("regenerated")
    }

    function driveScrollTabs() {
      openView()
      var lv = null
      waitFor(function() { lv = find(view(), "transcriptList"); return lv && lv.count === 40 }, "40 lines")
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended")
      tab("summary")
      click(find(view(), "regenerateButton"), "Summarize")
      var flick = find(view(), "summaryFlick")
      waitFor(function() { return txt("summaryText").indexOf("Section 30") !== -1 && flick.contentHeight > flick.height * 2 }, "a long summary", 8000)
      flick.contentY = 300
      tab("transcript")
      lv.positionViewAtIndex(12, ListView.Beginning)
      lv.follow = false
      wait(100)
      var y1 = lv.contentY
      check(y1 > 0 && !lv.atYEnd, "transcript scrolled to the middle: " + y1)
      tab("summary")
      check(Math.abs(flick.contentY - 300) < 1, "summary kept its place: " + flick.contentY)
      tab("transcript")
      check(Math.abs(lv.contentY - y1) < 1, "transcript kept its place: " + lv.contentY + " vs " + y1)
      tab("summary")
      check(Math.abs(flick.contentY - 300) < 1, "summary still in place: " + flick.contentY)
      shot("summary-scrolled")
    }

    function driveViewRequest() {
      waitFor(function() { return feed().connected && feed().meetingId !== "" }, "feed has the meeting")
      // `voxtype meeting view <id> --tab notes`
      request({ meeting_id: __MEETING_ID__, tab: "notes", open: true })
      waitFor(function() { return host.viewOpen() && view() !== null }, "opened by request")
      tc.parent = view()
      waitFor(function() { return view().currentTab === "notes" }, "on Notes: " + view().currentTab)
      if (host.omarchy) {
        // The plugin's own payload form picks the tab too.
        fakeShell.summon("io.voxtype.settings", JSON.stringify({ view: "meetings", tab: "ask" }))
        waitFor(function() { return view().currentTab === "ask" }, "payload tab: " + view().currentTab)
      }
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended")
      var close = find(view(), "closeButton")
      tc.parent = host
      click(close, "Close")
      waitFor(function() { return !host.viewOpen() && feed().meetingId === "" }, "closed, ended meeting dropped")

      // A request that is too old (a host that started later) is ignored.
      request({ meeting_id: __MEETING_ID__, open: true, requested_at: Date.now() - 120000 })
      wait(800)
      check(!host.viewOpen(), "stale request ignored")

      // A meeting that is not on screen opens from its saved transcript.
      request({ meeting_id: __MEETING_ID__, tab: "transcript", open: true })
      waitFor(function() { return host.viewOpen() && view() !== null }, "opened on a saved meeting")
      tc.parent = view()
      waitFor(function() { return feed().ended && lines().length === 4 }, "saved transcript loaded: " + (view() ? lines().length : -1))
      check(txt("meetingState") === "Ended" && txt("meetingTitle") === "Standup", "shown as Ended: " + txt("meetingState"))
      shot("saved-meeting")
    }

    // ------------------------------------------------------------ past meetings

    function pastList() { return find(view(), "meetingList") }
    function row(i) { return pastList().itemAtIndex(i) }
    function rowText(i, name) { var r = row(i); return r ? String(find(r, name).text) : "<no row " + i + ">" }

    function openList() {
      openView()
      waitFor(function() { return shown(find(view(), "idleState")) && pastList() && pastList().count > 0 }, "the list")
    }

    function openPast(i, title) {
      click(row(i), "row " + i)
      waitFor(function() { return feed().ended && txt("meetingTitle") === title }, "opened " + title + ": " + txt("meetingTitle"))
    }

    function driveList() {
      openList()
      check(shown(find(view(), "startButton")) && shown(find(view(), "startTitleField")), "Start meeting stays at the top")
      check(txt("idleTitle") === "No meeting in progress", "idle title: " + txt("idleTitle"))
      check(pastList().count === 50, "first page of 50: " + pastList().count)
      check(rowText(0, "meetingRowTitle") === "Design review" && rowText(1, "meetingRowTitle") === "1:1 with Sam", "newest first")
      waitFor(function() { return rowText(0, "meetingRowDetails").indexOf("3 speakers") !== -1 }, "speakers read from the folder: " + rowText(0, "meetingRowDetails"))
      var d = rowText(0, "meetingRowDetails")
      check(d.indexOf("2026, ") !== -1 && d.indexOf("45 min") !== -1, "date and length: " + d)
      check(rowText(1, "meetingRowDetails").indexOf("25 min · 2 speakers") !== -1, "second row: " + rowText(1, "meetingRowDetails"))
      check(find(row(0), "meetingRowSummary").visible, "Summary badge on the summarized meeting")
      check(!find(row(1), "meetingRowSummary").visible, "no badge without a summary")
      check(rowText(2, "meetingRowTitle") === "Untitled meeting", "untitled: " + rowText(2, "meetingRowTitle"))
      shot("list")
      pastList().positionViewAtEnd()
      wait(200)
      var more = find(view(), "meetingListMore")
      waitFor(function() { return more && shown(more) }, "Show older meetings")
      click(more, "Show older meetings")
      waitFor(function() { return pastList().count === 60 }, "all 60 after the second page: " + pastList().count)
      waitFor(function() { return !shown(find(view(), "meetingListMore")) }, "no more to show")
    }

    // An older voxtype: no has_summary or speaker_count, so the rows read
    // their own folders.
    function driveListOld() {
      openList()
      check(pastList().count === 2, "two meetings: " + pastList().count)
      waitFor(function() { return rowText(0, "meetingRowDetails").indexOf("3 speakers") !== -1 }, "speakers from transcript.json: " + rowText(0, "meetingRowDetails"))
      check(rowText(1, "meetingRowDetails").indexOf("25 min · 2 speakers") !== -1, "second row: " + rowText(1, "meetingRowDetails"))
      waitFor(function() { return find(row(0), "meetingRowSummary").visible }, "badge from metadata.json or summary.md")
      check(!find(row(1), "meetingRowSummary").visible, "no badge without a summary")
    }

    function drivePastView() {
      openList()
      openPast(0, "Design review")
      check(txt("meetingState") === "Ended" && txt("meetingElapsed") === "45:00", "Ended, 45 min: " + txt("meetingState") + " " + txt("meetingElapsed"))
      check(txt("meetingEndedAt").indexOf("Tue 10 Nov 2026, ") === 0, "its date: " + txt("meetingEndedAt"))
      waitFor(function() { return lines().length === 4 }, "transcript loaded")
      check(JSON.stringify(renderedChips()) === '["You","Remote 1","Remote 2"]', "chips: " + renderedChips())
      check(shown(find(view(), "backButton")), "Back offered")
      check(!shown(find(view(), "pauseButton")), "no Pause on a past meeting")
      shot("past-meeting")
      click(find(view(), "backButton"), "Back")
      waitFor(function() { return shown(find(view(), "idleState")) && pastList().count === 2 }, "back to the list")
      check(!feed().ended && feed().meetingId === "", "nothing held after Back")
      openPast(1, "1:1 with Sam")
      waitFor(function() { return lines().length === 3 }, "the other meeting's transcript")
    }

    function drivePastSummary() {
      openList()
      openPast(0, "Design review")
      check(view().currentTab === "transcript", "opens on the transcript")
      check(shown(find(view(), "tab_summary")), "Summary tab for a past meeting")
      tab("summary")
      waitFor(function() { return txt("summaryText").indexOf("## Overview") === 0 }, "summary from show --json: " + txt("summaryText"))
      check(txt("summaryText").indexOf("**Dana**: Draft the empty state") !== -1, "action item: " + txt("summaryText"))
      check(String(find(view(), "regenerateButton").text) === "Regenerate", "Regenerate offered")
      shot("past-summary")
      find(view(), "summaryText").linkActivated("#t=61000")
      waitFor(function() { return view().currentTab === "transcript" && find(view(), "transcriptPane").flashSegId === 2 }, "chapter jumps to its line")
      tab("summary")
      check(txt("summaryText").indexOf("## Overview") === 0, "summary still there")
    }

    function drivePastSummarize() {
      openList()
      openPast(1, "1:1 with Sam")
      tab("summary")
      check(shown(find(view(), "summaryEmpty")), "no summary yet")
      var button = find(view(), "regenerateButton")
      check(String(button.text) === "Summarize", "Summarize: " + button.text)
      click(button, "Summarize")
      waitFor(function() { return shown(find(view(), "summarizing")) }, "summarizing")
      waitFor(function() { return txt("summaryText").indexOf("Release status") !== -1 }, "rendered the output", 8000)
      shot("past-summarized")
    }

    function drivePastReassign() {
      openList()
      openPast(1, "1:1 with Sam")
      waitFor(function() { return lines().length === 3 }, "transcript loaded")
      openChooser(1)
      pick("You")
      check(lines()[1][1] === "You", "moved at once")
      waitFor(idleCli, "reassign finished")
      check(lines()[1][1] === "You" && moved(1) === "moved from Remote", "kept with the CLI's segment: " + JSON.stringify(lines()[1]) + " " + moved(1))
      // It is stored: Back, then open it again.
      click(find(view(), "backButton"), "Back")
      waitFor(function() { return shown(find(view(), "idleState")) && pastList().count > 0 }, "list")
      openPast(1, "1:1 with Sam")
      waitFor(function() { return lines().length === 3 && lines()[1][1] === "You" && moved(1) === "moved from Remote" }, "still moved after reopening")
    }

    function driveStartWhileBrowsing() {
      openList()
      openPast(0, "Design review")
      ctl({ cmd: "start", title: "Live one" })
      waitFor(function() { return feed().status === "recording" && txt("meetingTitle") === "Live one" }, "the live meeting took over: " + txt("meetingTitle"))
      check(!feed().ended && lines().length === 0, "the past meeting is gone")
      check(!shown(find(view(), "backButton")) && shown(find(view(), "pauseButton")), "live controls, no Back")
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended")
      click(find(view(), "backButton"), "Back")
      waitFor(function() { return shown(find(view(), "idleState")) && pastList().count === 3 && rowText(0, "meetingRowTitle") === "Live one" }, "the ended meeting tops the list: " + (pastList() ? rowText(0, "meetingRowTitle") : ""))
      shot("list-after-meeting")
    }

    // ------------------------------------------------------------ failure paths

    function driveCliMissing() {
      openView()
      waitFor(function() { return feed().connected && shown(find(view(), "idleState")) }, "idle")
      var missing = __FAKE_BIN__
      waitFor(function() { return shown(find(view(), "meetingListError")) }, "the list says why it is empty")
      check(txt("meetingListError") === "Could not run " + missing + ". Is voxtype installed and on PATH?", "list: " + txt("meetingListError"))
      click(find(view(), "startButton"), "Start")
      waitFor(function() { return shown(find(view(), "actionError")) }, "Start says why")
      check(view().actionError === "Could not run " + missing + ". Is voxtype installed and on PATH?", "start: " + view().actionError)
      shot("cli-missing")
    }

    function driveDaemonOld() {
      openView()
      waitFor(function() { return txt("idleTitle") === "Voxtype is running without its meeting socket" }, "old daemon named: " + txt("idleTitle"), 8000)
      check(txt("idleDetail").indexOf("systemctl --user restart voxtype") !== -1, "says what to do: " + txt("idleDetail"))
      check(!find(view(), "startButton").enabled, "no Start without the socket")
      shot("daemon-old")
    }

    function driveAgentMissing() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      tab("ask")
      var problem = find(find(view(), "askPane"), "assistantProblem")
      waitFor(function() { return shown(problem) }, "the Ask tab says why it cannot answer")
      check(String(problem.text).indexOf("No agent or local model was found.") === 0, "problem: " + problem.text)
      check(chipLabel() === "No agent  ▾", "chip: " + chipLabel())
      var menu = find(view(), "agentMenu")
      click(find(find(view(), "askPane"), "agentChip"), "agent chip")
      waitFor(function() { return menu.visible }, "menu")
      check(!shown(find(menu, "agentOption_claude")), "an agent that was not found is not offered")
      check(shown(find(menu, "agentOption_codex")) && !find(menu, "agentOption_codex").enabled, "an unsupported agent is shown, disabled")
      check(String(find(menu, "agentNote_codex").text).indexOf("bwrap is not installed") === 0, "its note: " + find(menu, "agentNote_codex").text)
      shot("agent-missing")
      keyClick(Qt.Key_Escape)
      // Asking anyway gets the CLI's own error, not a blank answer.
      typeAsk("What did we decide?")
      waitFor(function() { return answered(1) }, "the ask ended", 8000)
      check(thread().get(1).error.indexOf("No agent or local model is available") === 0, "error: " + thread().get(1).error)
    }

    function driveOldVoxtype() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      tab("ask")
      var problem = find(find(view(), "askPane"), "assistantProblem")
      waitFor(function() { return shown(problem) }, "the Ask tab says to update")
      check(String(problem.text).indexOf("This voxtype is too old for the meeting assistant") === 0, "ask: " + problem.text)
      tab("notes")
      waitFor(function() { return shown(find(view(), "notesError")) }, "the Notes tab says to update")
      check(txt("notesError").indexOf("This voxtype is too old for meeting notes") === 0, "notes: " + txt("notesError"))
      tab("transcript")
      openChooser(1)
      pick("You")
      waitFor(function() { return shown(find(view(), "actionError")) }, "reassign says to update")
      check(view().actionError.indexOf("This voxtype is too old for moving lines") === 0, "reassign: " + view().actionError)
      check(lines()[1][1] === "Remote", "rolled back")
      shot("old-voxtype")
    }

    function driveCopy() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      tab("ask")
      typeAsk("What did we decide?")
      waitFor(function() { return answered(1) }, "answered", 8000)
      var row = find(view(), "askThread").itemAtIndex(1)
      var copy = find(row, "answerCopy")
      check(shown(copy) && String(copy.text) === "Copy", "Copy on the answer")
      mouseClick(copy, copy.width / 2, copy.height / 2)
      waitFor(function() { return String(copy.text) === "Copied" }, "Copied")
      shot("copied")

      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended")
      tab("summary")
      click(find(view(), "regenerateButton"), "Summarize")
      waitFor(function() { return txt("summaryText").indexOf("## Overview") === 0 }, "summary", 8000)
      var scopy = find(view(), "summaryCopy")
      click(scopy, "Copy summary")
      waitFor(function() { return String(scopy.text) === "Copied" }, "summary Copied")

      // Without wl-copy, say what is missing.
      host.ctrl().copyCommand = "/nonexistent/wl-copy"
      waitFor(function() { return String(scopy.text) === "Copy" }, "label back")
      click(scopy, "Copy without wl-copy")
      waitFor(function() { return shown(find(view(), "actionError")) }, "the reason")
      check(view().actionError === "Copying needs /nonexistent/wl-copy (the wl-clipboard package), which is not installed.", "message: " + view().actionError)
    }

    // ------------------------------------------------------------ after the call

    function lineAction(i, name) {
      var item = lineItem(i)
      mouseMove(item, 30, 4)
      var act = find(item, name)
      waitFor(function() { return act && shown(act) }, name + " on hover of line " + i)
      // A Row places a newly shown action on the next polish.
      wait(100)
      mouseClick(act, act.width / 2, act.height / 2)
    }

    function driveLineEdit() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      lineAction(1, "lineEditButton")
      var field = find(lineItem(1), "lineEditField")
      waitFor(function() { return field.visible && field.activeFocus }, "inline editor focused")
      check(field.text === "Yes, loud and clear.", "editor holds the text: " + field.text)
      check(!find(lineItem(1), "lineActions").visible, "no actions while editing")
      shot("editing")
      field.text = "Yes, very loud and clear."
      keyClick(Qt.Key_Return)
      check(!field.visible && lines()[1][2] === "Yes, very loud and clear.", "saved at once: " + JSON.stringify(lines()[1]))
      waitFor(idleCli, "edit finished")
      wait(300)
      check(lines()[1][2] === "Yes, very loud and clear.", "kept after the daemon: " + lines()[1][2])
      check(shown(find(lineItem(1), "lineEdited")), "marked edited (original_text)")
      shot("edited")
      // Restore original text: `edit --reset`.
      lineAction(1, "lineRestoreTextButton")
      check(lines()[1][2] === "Yes, loud and clear." && !shown(find(lineItem(1), "lineEdited")), "restored at once")
      waitFor(idleCli, "reset finished")
      wait(300)
      check(lines()[1][2] === "Yes, loud and clear." && !shown(find(lineItem(1), "lineEdited")), "restored after the daemon")

      // Escape leaves it as it was.
      lineAction(2, "lineEditButton")
      field = find(lineItem(2), "lineEditField")
      waitFor(function() { return field.activeFocus }, "second editor focused")
      field.text = "Something else"
      keyClick(Qt.Key_Escape)
      check(!field.visible && lines()[2][2] === "Great, let's start with the release.", "Escape cancels: " + lines()[2][2])
      check(host.viewOpen(), "Escape in the editor does not close the view")

      // A refused edit goes back.
      sh(["sh", "-c", 'printf "%s" "$2" > "$1"', "_", __STATE__, JSON.stringify({ fail: { edit: "Error: segment 4 is locked." } })])
      lineAction(3, "lineEditButton")
      field = find(lineItem(3), "lineEditField")
      waitFor(function() { return field.activeFocus }, "third editor focused")
      field.text = "It went red again."
      keyClick(Qt.Key_Return)
      waitFor(function() { return shown(find(view(), "actionError")) }, "refusal shown")
      check(view().actionError === "segment 4 is locked.", "message: " + view().actionError)
      check(lines()[3][2] === "The build went green overnight.", "rolled back: " + lines()[3][2])
    }

    function driveLineDelete(flag) {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      lineAction(1, "lineDeleteButton")
      check(lines().length === 3 && lines()[1][2] === "Great, let's start with the release.", "gone at once: " + JSON.stringify(lines()))
      var bar = find(view(), "undoBar")
      check(shown(bar), "Undo offered")
      if (!flag) shot("undo")
      click(find(bar, "undoButton"), "Undo")
      check(lines().length === 4 && lines()[1][2] === "Yes, loud and clear.", "back in its place")
      check(!shown(bar), "Undo bar gone")
      wait(300)
      check(idleCli(), "nothing ran for an undone delete")

      lineAction(1, "lineDeleteButton")
      check(lines().length === 3, "deleted again")
      if (flag) {
        // Closing the view accepts it at once.
        var close = find(view(), "closeButton")
        tc.parent = host
        click(close, "Close")
        waitFor(function() { return !host.viewOpen() }, "closed")
        waitFor(function() { return idleCli() && host.ctrl().cli.busy === "" }, "delete ran on close")
        wait(400)
        openView()
        waitFor(function() { return lines().length === 3 }, "still gone after reopening (hello without it)")
      } else {
        waitFor(function() { return !shown(bar) }, "the window closed", 8000)
        waitFor(idleCli, "delete ran")
        wait(400)
        check(lines().length === 3, "stays gone when the daemon confirms")
      }
      // A line deleted elsewhere goes too, whichever shape the daemon uses.
      ctl({ cmd: "delete", segment_id: 4, shape: flag ? "flag" : "event" })
      waitFor(function() { return lines().length === 2 }, "deleted elsewhere")
      check(lines()[1][2] === "Great, let's start with the release.", "the right line went: " + JSON.stringify(lines()))
      // `delete-line --restore` elsewhere: the whole segment again, inserted
      // by start time.
      ctl({ cmd: "update", key: "event", segment: seg(2, 9, "Yes, loud and clear.", "loopback", "Remote", 0) })
      waitFor(function() { return lines().length === 3 && lines()[1][2] === "Yes, loud and clear." }, "restored in its place: " + JSON.stringify(lines()))
    }

    function driveExport() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      var menu = find(view(), "moreMenu")
      click(find(view(), "moreButton"), "More")
      waitFor(function() { return menu.visible }, "More menu")
      waitFor(function() { return find(menu, "openMeetingFolder").enabled }, "the meeting's folder is known")
      shot("more-menu")
      click(find(menu, "exportOption_markdown"), "Markdown")
      waitFor(function() { return shown(find(view(), "exportNotice")) }, "export notice", 8000)
      check(txt("exportNoticeText").indexOf("Exported to " + __DOCUMENTS__ + "/Voxtype/") === 0, "notice: " + txt("exportNoticeText"))
      shot("exported")
      click(find(view(), "exportOpenFolder"), "Open folder")
      wait(500)
      click(find(view(), "moreButton"), "More again")
      waitFor(function() { return menu.visible }, "More menu again")
      click(find(menu, "openMeetingFolder"), "Open meeting folder")
      wait(500)
    }

    function driveExportPast() {
      openList()
      openPast(0, "Design review")
      var menu = find(view(), "moreMenu")
      click(find(view(), "moreButton"), "More")
      waitFor(function() { return menu.visible && find(menu, "openMeetingFolder").enabled }, "More menu with the folder")
      var formats = ["markdown", "text", "json", "srt", "vtt"]
      for (var f = 0; f < formats.length; f++)
        check(shown(find(menu, "exportOption_" + formats[f])), formats[f] + " offered")
      click(find(menu, "exportOption_srt"), "SRT")
      waitFor(function() { return shown(find(view(), "exportNotice")) }, "export notice", 8000)
      check(txt("exportNoticeText") === "Exported to " + __DOCUMENTS__ + "/Voxtype/2026-11-10 Design review.srt", "notice: " + txt("exportNoticeText"))
      click(find(view(), "moreButton"), "More again")
      waitFor(function() { return menu.visible }, "menu")
      click(find(menu, "openMeetingFolder"), "Open meeting folder")
      wait(500)
    }

    // ------------------------------------------------------------ window

    function placement() { return host.ctrl().placement }

    // A known screen, so the math does not depend on the offscreen one.
    function screen1600() {
      placement().screenWidth = 1600
      placement().screenHeight = 1000
      placement().screenName = "DP-1"
    }

    function driveMinimize() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      click(find(view(), "minimizeButton"), "Minimize")
      var pill = find(view(), "pill")
      waitFor(function() { return pill.visible && placement().minimized }, "the pill")
      var win = host.ctrl().window
      check(win.implicitWidth === 300 && win.implicitHeight === 40, "pill-sized window: " + win.implicitWidth + "x" + win.implicitHeight)
      check(host.viewOpen(), "the view is still open (the pill is the view)")
      check(txt("pillState") === "Recording" && /^1:0\d$/.test(txt("pillElapsed")), "state and time: " + txt("pillState") + " " + txt("pillElapsed"))
      check(txt("pillLine") === "Remote: The build went green overnight.", "last line: " + txt("pillLine"))
      shot("pill", true)
      click(find(view(), "pillDrag"), "the pill")
      waitFor(function() { return !placement().minimized && shown(find(view(), "transcriptList")) }, "expanded by a click")

      // `voxtype meeting view --minimize`, then `--expand`.
      request({ window: "minimize" })
      waitFor(function() { return placement().minimized }, "minimized by request")
      request({ window: "expand" })
      waitFor(function() { return !placement().minimized }, "expanded by request")
      check(host.opens === 1, "one window throughout: " + host.opens)
    }

    function driveMinimizeAutoOpen() {
      waitFor(function() { return feed().connected && feed().status === "idle" }, "feed connected")
      openView()
      click(find(view(), "minimizeButton"), "Minimize")
      waitFor(function() { return placement().minimized }, "minimized")
      ctl({ cmd: "start", title: "Design review" })
      waitFor(function() { return !placement().minimized && feed().title === "Design review" }, "a meeting start expands the pill")
      check(host.opens === 1, "no second window: " + host.opens)
      waitFor(function() { return txt("meetingTitle") === "Design review" }, "the new meeting")
    }

    function driveResize() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      screen1600()
      var p = placement()
      check(p.cardWidth === 460 && p.left === 1600 - 460 - p.gap && p.handleOnLeft, "starts on the right edge: " + p.left)
      var handle = find(view(), "resizeHandle")
      // Drag the inner (left) edge 120 px out: wider, right side kept.
      mousePress(handle, 4, 100)
      mouseMove(handle, 4 - 120, 100)
      mouseRelease(handle, 4 - 120, 100)
      check(p.cardWidth === 580 && p.left + p.cardWidth === 1600 - p.gap, "wider to the left: " + p.cardWidth + " at " + p.left)
      shot("wide", true)
      // Clamped: never more than half the screen, never under 280 px.
      p.resizeBy(-5000)
      check(p.cardWidth === 800, "max is half the screen: " + p.cardWidth)
      p.resizeBy(5000)
      check(p.cardWidth === 280, "min 280: " + p.cardWidth)
      p.endResize()
      shot("narrow", true)
      // Compact height: 40 % of the screen.
      click(find(view(), "heightToggle"), "compact")
      check(p.compact && !p.fullHeight && p.windowHeight === 400, "compact 400: " + p.windowHeight)
      click(find(view(), "heightToggle"), "full")
      check(!p.compact && p.fullHeight, "full height again")
      // Reset position in the menu.
      click(find(view(), "moreButton"), "More")
      waitFor(function() { return find(view(), "moreMenu").visible }, "menu")
      click(find(view(), "resetPosition"), "Reset position")
      check(p.x === -1 && !p.compact && p.cardWidth === 460 && p.left === 1600 - 460 - p.gap, "back to the default: " + p.left)
    }

    function driveMove() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      screen1600()
      var p = placement()
      click(find(view(), "heightToggle"), "compact, so it can move vertically")
      var drag = find(view(), "headerDrag")
      var x0 = p.left
      var y0 = p.top
      mousePress(drag, 60, 10)
      mouseMove(drag, 60 - 500, 10 + 200)
      check(p.left === x0 - 500 && p.top === y0 + 200, "moved by the drag: " + p.left + "," + p.top)
      mouseRelease(drag, 60 - 500, 10 + 200)
      check(p.left === x0 - 500 && p.top === y0 + 200, "nowhere near an edge, so no snap: " + p.left + "," + p.top)
      // Within 24 px of an edge snaps onto it (a gap in).
      p.moveBy(-(p.left - 25), 0)
      p.endMove()
      check(p.left === p.gap, "snapped to the left edge: " + p.left)
      p.moveBy(1600 - p.cardWidth - p.left - 30, 0)
      p.endMove()
      check(p.left === 1600 - p.cardWidth - p.gap, "snapped to the right edge: " + p.left)
      p.moveBy(0, -(p.top - 20))
      p.endMove()
      check(p.top === p.gap, "snapped to the top: " + p.top)
      // Clamped to the screen however far it is dragged.
      p.moveBy(-9999, 9999)
      check(p.left === 0 && p.top === 1000 - p.windowHeight, "clamped: " + p.left + "," + p.top)
      p.moveBy(300, -300)
      p.endMove()
      var want = { x: p.left, y: p.top, width: p.cardWidth, compact: p.compact }
      // The card still takes clicks after a drag.
      click(find(view(), "moreButton"), "More after a drag")
      waitFor(function() { return find(view(), "moreMenu").visible }, "menu after a drag")
      keyClick(Qt.Key_Escape)
      check(!find(view(), "moreMenu").visible && host.viewOpen(), "Escape closes the menu")
      // The drag needs no keyboard: the window never asked for it.
      check(host.ctrl().window.keyboardPolicy === "none", "no keyboard for a drag: " + host.ctrl().window.keyboardPolicy)

      // Remembered: a new instance (a restarted shell) reads it back.
      wait(800)
      var fresh = Qt.createQmlObject('import "standalone/voxtype-meetings"; MeetingPlacement {}', host)
      fresh.host = p.host
      fresh.screenWidth = 1600
      fresh.screenHeight = 1000
      fresh.screenName = "DP-1"
      waitFor(function() { return fresh.loaded }, "state file read")
      check(fresh.left === want.x && fresh.top === want.y && fresh.cardWidth === want.width && fresh.compact === want.compact,
            "round trip: " + JSON.stringify({ x: fresh.left, y: fresh.top, w: fresh.cardWidth, c: fresh.compact }) + " vs " + JSON.stringify(want))
      // Per output: another output starts from the default.
      fresh.screenName = "HDMI-A-1"
      wait(100)
      check(fresh.x === want.x || fresh.saved["standalone:HDMI-A-1"] === undefined, "")
      check(fresh.saved[p.host + ":HDMI-A-1"] === undefined, "nothing saved for another output")

    }

    function driveOpacity() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      waitFor(function() { return view().restingOpacity === 0.5 }, "view_opacity from config: " + view().restingOpacity)
      // The pointer away from the card: translucent.
      mouseMove(view(), -40, -40)
      waitFor(function() { return !view().engaged && view().cardAlpha === 0.5 }, "translucent at rest: " + view().cardAlpha, 4000)
      wait(500)
      shot("translucent", true)
      // Over the card: opaque at once.
      // On the meeting's state line: card, but nothing typeable (speaker
      // names and chips ask for the keyboard, since they rename).
      var spot = find(view(), "meetingState").mapToItem(view(), 4, 4)
      mouseMove(view(), spot.x, spot.y)
      waitFor(function() { return view().engaged && view().cardAlpha === 1.0 }, "opaque on hover")
      check(host.ctrl().window.keyboardPolicy === "none", "hovering the card asks for no keyboard")
      // Away again: still opaque for a moment, then back.
      mouseMove(view(), -40, -40)
      wait(400)
      check(view().cardAlpha === 1.0, "lingers opaque")
      waitFor(function() { return view().cardAlpha === 0.5 }, "eases back after the linger", 4000)
      // Typing: opaque, and now the window asks for the keyboard.
      tab("ask")
      var field = find(find(view(), "askPane"), "askField")
      click(field, "Ask field")
      mouseMove(view(), -40, -40)
      wait(2000)
      check(view().cardAlpha === 1.0 && host.ctrl().window.keyboardPolicy === "on-demand", "opaque while typing, keyboard on demand: "
            + view().cardAlpha + " " + host.ctrl().window.keyboardPolicy)
      // Typing ends (here: focus moves elsewhere in the card).
      view().forceActiveFocus()
      waitFor(function() { return host.ctrl().window.keyboardPolicy === "none" }, "keyboard given back when typing ends")
    }

    FileView { id: stateFileView; path: __RUNTIME__ + "/meeting-view.state"; blockLoading: true; printErrors: false }
    function viewState() {
      stateFileView.reload()
      try { return JSON.parse(stateFileView.text()) } catch (e) { return null }
    }

    function driveViewState() {
      host.ctrl().stateHeartbeatMs = 300
      waitFor(function() { var s = viewState(); return s && s.visible === false }, "written as not visible at start")
      openView()
      waitFor(function() { var s = viewState(); return s && s.visible === true && s.minimized === false }, "visible on show")
      var s = viewState()
      check(s.host === __HOST__ && s.pid > 1, "host and pid: " + JSON.stringify(s))
      var first = s.updated_at
      waitFor(function() { var t = viewState(); return t && t.updated_at > first }, "heartbeat while visible", 4000)
      click(find(view(), "minimizeButton"), "Minimize")
      waitFor(function() { var t = viewState(); return t && t.visible === true && t.minimized === true }, "minimized counts as visible")
      click(find(view(), "pillDrag"), "expand")
      waitFor(function() { var t = viewState(); return t && t.minimized === false }, "expanded")
      var close = find(view(), "closeButton")
      tc.parent = host
      click(close, "Close")
      waitFor(function() { var t = viewState(); return t && t.visible === false }, "not visible after close")
      var closed = viewState().updated_at
      wait(1500)
      check(viewState().updated_at === closed, "no heartbeat while closed")
    }

    // ------------------------------------------------------------ playback

    function playing(i) { var m = find(lineItem(i), "linePlaying"); return m && m.visible }

    function drivePlay() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      lineAction(1, "linePlayButton")
      waitFor(function() { return shown(find(view(), "transportStop")) }, "the transport")
      check(txt("transportTrack") === "▶ Playing both sides", "track: " + txt("transportTrack"))
      waitFor(function() { return playing(1) }, "the played line is lit")
      shot("playing")
      // At ten times speed the position reaches 0:21 within about 1.2 s.
      waitFor(function() { return playing(2) && !playing(1) }, "the light follows position_ms", 5000)
      click(find(view(), "transportStop"), "Stop")
      waitFor(function() { return !shown(find(view(), "transportStop")) }, "transport gone")
      check(!playing(1) && !playing(2) && !playing(3), "nothing lit")

      // Stopped elsewhere: the playback event says so.
      lineAction(1, "linePlayButton")
      waitFor(function() { return shown(find(view(), "transportStop")) }, "playing again")
      sh(["sh", "-c", '"$1" meeting play --stop --json >/dev/null', "_", __FAKE_BIN__])
      waitFor(function() { return !shown(find(view(), "transportStop")) }, "stopped from elsewhere")
    }

    function drivePlayNoAudio() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      waitFor(function() { return host.ctrl().assistant.audioRetained === false }, "audio_retained read")
      var hint = find(view(), "audioHint")
      check(shown(hint), "the hint, once")
      check(txt("audioHint") !== "", "")
      var item = lineItem(1)
      mouseMove(item, 30, 4)
      waitFor(function() { return shown(find(item, "lineEditButton")) }, "hover actions")
      check(!shown(find(item, "linePlayButton")), "no ▶ without audio")
      check(!find(item, "lineSeek").enabled, "the time is not a link without audio")
      check(shown(find(view(), "audioHintSettings")) === host.omarchy, "Settings link where the host has Settings")
      shot("audio-not-kept")
      click(find(view(), "audioHintDismiss"), "dismiss")
      check(!shown(find(view(), "audioHint")), "dismissed")
    }

    function drivePlayErrors() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      lineAction(1, "linePlayButton")
      waitFor(function() { return shown(find(view(), "actionError")) }, "the refusal")
      check(view().actionError === "This meeting was recorded without keeping audio. Turn on meeting.retain_audio for future meetings.",
            "verbatim: " + view().actionError)
      waitFor(function() { return shown(find(view(), "audioHint")) }, "now known: the hint")
      var item = lineItem(1)
      mouseMove(item, 30, 4)
      waitFor(function() { return shown(find(item, "lineEditButton")) }, "hover actions")
      check(!shown(find(item, "linePlayButton")), "and no more ▶")
      // Missing tools: said as the CLI says it, and ▶ stays.
      sh(["sh", "-c", 'printf "%s" "$2" > "$1"', "_", __STATE__, JSON.stringify({ play_error: "Error: Playing meeting audio needs ffmpeg, which is not installed." })])
      // (Back to not knowing, as for a meeting whose manifest says it kept audio.)
      host.ctrl().assistant.audioRetained = null
      // The hint going away moves the lines up; let it settle before hovering.
      wait(300)
      lineAction(1, "linePlayButton")
      waitFor(function() { return view().actionError === "Playing meeting audio needs ffmpeg, which is not installed." }, "tools: " + view().actionError)
      check(host.ctrl().assistant.audioRetained === null, "a tools problem does not mean no audio")
    }

    function driveAskError() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      tab("ask")
      typeAsk("What did we decide?")
      waitFor(function() { return answered(1) }, "the ask ended", 8000)
      check(thread().get(1).error === "Claude failed: the agent exited with status 1.", "error message: " + thread().get(1).error)
      var row = find(view(), "askThread").itemAtIndex(1)
      check(shown(find(row, "answerError")) && !shown(find(row, "answerBy")), "error shown, no attribution")
      shot("ask-error")
    }

    // The error card, alone at the top of the Summary pane: above any
    // summary text, Summarizing gone, the actions as wanted.
    function checkErrorCard(message, useAgent) {
      // Let the layout place the card it has just shown.
      wait(150)
      var card = find(view(), "summaryErrorCard")
      check(shown(card), "error card shown")
      check(!shown(find(view(), "summarizingRow")), "Summarizing cleared")
      check(txt("summaryErrorTitle") === "Couldn't write the summary", "title: " + txt("summaryErrorTitle"))
      check(txt("summaryError") === message, "message: " + txt("summaryError"))
      var pane = find(view(), "summaryPane")
      var flick = find(pane, "summaryFlick")
      var head = find(pane, "regenerateButton").mapToItem(pane, 0, 0).y + find(pane, "regenerateButton").height
      var cardY = card.mapToItem(pane, 0, 0).y
      check(cardY >= head && cardY < head + 40, "card right under the header: " + cardY + " (header ends " + head + ")")
      if (shown(flick)) check(cardY < flick.mapToItem(pane, 0, 0).y, "card above the summary text")
      check(shown(find(view(), "summaryRetry")), "Retry offered")
      check(shown(find(view(), "summaryUseAgent")) === (useAgent !== ""), "Use agent offered: " + shown(find(view(), "summaryUseAgent")))
      if (useAgent !== "") check(txt("summaryUseAgent") === "Use " + useAgent + " instead", "use: " + txt("summaryUseAgent"))
      check(shown(find(view(), "summarySettings")) === view().canOpenSettings, "Settings offered where the host has it")
      check(txt("meetingState") === "Ended", "Ended: " + txt("meetingState"))
    }

    function driveSummaryError() {
      ctl({ cmd: "autosummary", error: { code: "agent_failed", message: "Summary failed: Claude exited with status 1." } })
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return shown(find(view(), "summarizing")) }, "summarizing", 6000)
      check(txt("summarizing") === "Summarizing with Claude…", "summarizing: " + txt("summarizing"))
      wait(100)
      var spane = find(view(), "summaryPane")
      var shead = find(spane, "regenerateButton").mapToItem(spane, 0, 0).y + find(spane, "regenerateButton").height
      var rowY = find(view(), "summarizingRow").mapToItem(spane, 0, 0).y
      check(rowY >= shead && rowY < shead + 40, "summarizing right under the header: " + rowY + " (header ends " + shead + ")")
      shot("summarizing-with")
      waitFor(function() { return shown(find(view(), "summaryErrorCard")) }, "the failure", 6000)
      // An agent failed: nothing to offer instead of it.
      checkErrorCard("Summary failed: Claude exited with status 1.", "")
      shot("summary-error")
      click(find(view(), "summaryRetry"), "Retry")
      waitFor(function() { return txt("summaryText").indexOf("## Overview") === 0 }, "retried summary", 8000)
      check(!shown(find(view(), "summaryErrorCard")) && !shown(find(view(), "summarizingRow")), "card and spinner gone")
    }

    function driveSummaryLocalError() {
      ctl({ cmd: "autosummary", error: { code: "local_failed",
        message: "Local model (llama3.2 at http://localhost:11434) failed: LLM request failed: http://localhost:11434/api/generate: status code 500" } })
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      waitFor(function() { return assistant().status !== null }, "status read")
      tab("ask")
      check(chipLabel() === "Claude  ▾", "Ask chip names what answers: " + chipLabel())
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return view().currentTab === "summary" && shown(find(view(), "summarizing")) }, "summarizing", 6000)
      check(chipLabel() === "Local (llama3.2)  ▾", "Summary chip names what summarizes: " + chipLabel())
      check(txt("summarizing") === "Summarizing with Local (llama3.2)…", "summarizing: " + txt("summarizing"))
      waitFor(function() { return shown(find(view(), "summaryErrorCard")) }, "the failure", 6000)
      checkErrorCard("Local model (llama3.2 at http://localhost:11434) failed: LLM request failed: http://localhost:11434/api/generate: status code 500", "Claude")
      shot("summary-local-error")
      click(find(view(), "summaryUseAgent"), "Use Claude instead")
      waitFor(function() { return shown(find(view(), "summarizing")) }, "summarizing again")
      check(txt("summarizing") === "Summarizing with Claude…", "summarizing with: " + txt("summarizing"))
      check(!shown(find(view(), "summaryErrorCard")), "card gone while it runs")
      waitFor(function() { return txt("summaryText").indexOf("## Overview") === 0 }, "summary by Claude", 8000)
      check(!shown(find(view(), "summaryErrorCard")) && !shown(find(view(), "summarizingRow")), "card and spinner gone")
      tab("ask")
      check(chipLabel() === "Claude  ▾", "Ask chip unchanged: " + chipLabel())
    }

    function driveSummaryErrorCli() {
      var message = "Local model (llama3.2 at http://localhost:11434) failed: LLM request failed: http://localhost:11434/api/generate: status code 500"
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended", 5000)
      tab("summary")
      waitFor(function() { return assistant().status !== null }, "status read")
      check(chipLabel() === "Local (llama3.2)  ▾", "local_only chip: " + chipLabel())
      click(find(view(), "regenerateButton"), "Summarize")
      waitFor(function() { return shown(find(view(), "summarizing")) }, "summarizing")
      check(txt("summarizing") === "Summarizing with Local (llama3.2)…", "summarizing: " + txt("summarizing"))
      waitFor(function() { return shown(find(view(), "summaryErrorCard")) }, "the CLI failure", 6000)
      // local_only: the user said nothing goes to an agent.
      checkErrorCard(message, "")
      shot("summary-error-cli")
      click(find(view(), "summaryRetry"), "Retry")
      waitFor(function() { return shown(find(view(), "summarizingRow")) }, "retrying")
      check(!shown(find(view(), "summaryErrorCard")), "card gone while retrying")
      waitFor(function() { return shown(find(view(), "summaryErrorCard")) }, "failed again", 6000)
      checkErrorCard(message, "")
    }

    function driveSummaryStale() {
      ctl({ cmd: "autosummary", anonymous: true, error: { code: "summary_failed", message: "Summary failed: no backend answered." } })
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return shown(find(view(), "summarizing")) }, "summarizing", 6000)
      // No summary_backend in this status: the chip falls back to backend_name.
      check(chipLabel() === "Claude  ▾", "fallback chip: " + chipLabel())
      check(txt("summarizing") === "Summarizing with Claude…", "summarizing: " + txt("summarizing"))
      waitFor(function() { return shown(find(view(), "summaryErrorCard")) }, "the unnamed error ends it", 6000)
      checkErrorCard("Summary failed: no backend answered.", "")
      check(feed().lastError === "", "not repeated in the banner: " + feed().lastError)
      shot("summary-stale")
    }

    // ------------------------------------------------------------ sound check

    function meterFill(name) { return find(find(view(), name), "meterFill").width }

    function openSoundCheck() {
      openView()
      waitFor(function() { return shown(find(view(), "idleState")) }, "idle state")
      check(shown(find(view(), "meetingListPane")), "the list first")
      click(find(view(), "soundCheckButton"), "Sound check")
      waitFor(function() { return shown(find(view(), "soundCheckPane")) }, "sound check pane")
      check(!shown(find(view(), "meetingListPane")), "the pane replaces the list")
    }

    function editLine(i, text) {
      lineAction(i, "lineEditButton")
      var field = find(lineItem(i), "lineEditField")
      waitFor(function() { return field.visible && field.activeFocus }, "editor on line " + i)
      field.text = text
      keyClick(Qt.Key_Return)
      waitFor(idleCli, "edit finished")
    }

    function driveUndoEdit() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      editLine(1, "Yes, very loud and clear.")
      waitFor(function() { return shown(find(view(), "undoBar")) }, "the undo bar")
      check(txt("undoText") === "Line edited.", "says: " + txt("undoText"))
      shot("undo-edit")
      click(find(view(), "undoButton"), "Undo")
      check(lines()[1][2] === "Yes, loud and clear.", "back at once: " + lines()[1][2])
      check(!shown(find(view(), "undoBar")), "bar gone after Undo")
      waitFor(idleCli, "reset finished")
      wait(300)
      check(lines()[1][2] === "Yes, loud and clear." && !shown(find(lineItem(1), "lineEdited")), "the transcribed text, not marked edited")
      // A second correction undoes to the first one, not to the original.
      editLine(3, "It went amber.")
      editLine(3, "It went red.")
      click(find(view(), "undoButton"), "Undo")
      waitFor(idleCli, "undo finished")
      wait(300)
      check(lines()[3][2] === "It went amber." && shown(find(lineItem(3), "lineEdited")), "the earlier correction: " + lines()[3][2])
      // Nothing to undo once it ran; the window passes without one.
      check(!shown(find(view(), "undoBar")), "no bar after the undo")
    }

    function driveUndoReassign() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      openChooser(1)
      pick("You")
      waitFor(idleCli, "move finished")
      waitFor(function() { return shown(find(view(), "undoBar")) }, "the undo bar")
      check(txt("undoText") === "Line moved to You.", "says: " + txt("undoText"))
      click(find(view(), "undoButton"), "Undo")
      waitFor(idleCli, "undo finished")
      wait(300)
      check(lines()[1][1] === "Remote" && moved(1) === "", "back on Remote, not marked moved: " + lines()[1][1] + " " + moved(1))
      // A refused move goes back by itself and leaves nothing to undo.
      sh(["sh", "-c", 'printf "%s" "$2" > "$1"', "_", __STATE__, JSON.stringify({ fail: { reassign: "Error: segment 4 is locked." } })])
      openChooser(3)
      pick("You")
      waitFor(function() { return shown(find(view(), "actionError")) }, "refusal shown")
      waitFor(function() { return lines()[3][1] === "Remote" }, "put back")
      check(!shown(find(view(), "undoBar")), "nothing to undo")
    }

    function driveWaveform() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      check(!shown(find(view(), "waveform")), "no waveform while recording")
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended", 5000)
      waitFor(function() { return shown(find(view(), "waveform")) }, "the waveform", 6000)
      check(view().waveform.mic.length === 160 && view().waveform.loopback !== null, "both tracks, 160 buckets")
      var strip = find(view(), "waveformArea")
      // Laid out first: a strip still at width 0 puts every point at 0:00.
      waitFor(function() { return strip.width > 100 }, "the strip laid out")
      wait(100)
      // Twenty seconds in, a little past so rounding cannot land on 0:19.
      var x = (20.4 * 1000) / view().waveform.durationMs * strip.width
      // Enter, then move: a single synthetic move onto a just-shown item
      // does not always register as hover.
      mouseMove(strip, Math.max(1, x - 6), strip.height / 2)
      wait(80)
      mouseMove(strip, x, strip.height / 2)
      waitFor(function() { return shown(find(view(), "waveformHoverTime")) && txt("waveformHoverTime") === "0:20" }, "hover time: " + txt("waveformHoverTime"))
      mouseClick(strip, x, strip.height / 2)
      waitFor(function() { return shown(find(view(), "transportStop")) && shown(find(view(), "waveformPlayhead")) }, "playing from there")
      var x0 = find(view(), "waveformPlayhead").x
      waitFor(function() { return find(view(), "waveformPlayhead").x > x0 }, "the playhead moves", 4000)
      mouseMove(view(), -40, -40)
      shot("waveform")
      click(find(view(), "transportStop"), "Stop audio")
      waitFor(function() { return !shown(find(view(), "waveformPlayhead")) }, "playhead gone")
    }

    function driveWaveformMissing() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      click(find(view(), "stopButton"), "Stop")
      waitFor(function() { return txt("meetingState") === "Ended" }, "Ended", 5000)
      wait(1500)
      check(!shown(find(view(), "waveform")), "no strip")
      check(!shown(find(view(), "actionError")), "nothing reported")
    }

    function driveSeek() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      var stamp = find(lineItem(2), "lineSeek")
      check(stamp.enabled, "the time is a link")
      click(stamp, "line 3's time")
      waitFor(function() { return shown(find(view(), "transportStop")) && playing(2) }, "plays from line 3")
      shot("seek")
      click(find(lineItem(0), "lineSeek"), "line 1's time")
      waitFor(function() { return playing(0) && !playing(2) }, "moved to line 1", 5000)
      click(find(view(), "transportStop"), "Stop")
      waitFor(function() { return !shown(find(view(), "transportStop")) }, "stopped")
    }

    function driveRecovery() {
      openView()
      waitFor(function() { return shown(find(view(), "recoveryCard_1")) }, "two unfinished recordings")
      var first = find(view(), "recoveryCard_0")
      check(String(find(first, "recoveryText").text).indexOf("Planning, started ") === 0, "names it: " + find(first, "recoveryText").text)
      shot("recovery")
      click(find(first, "recoveryFinish"), "Finish it")
      waitFor(function() { return !shown(find(view(), "recoveryCard_1")) && shown(find(view(), "recoveryCard_0")) }, "one left", 6000)
      var second = find(view(), "recoveryCard_0")
      check(String(find(second, "recoveryText").text).indexOf("Vendor call") === 0, "the other one: " + find(second, "recoveryText").text)
      click(find(second, "recoveryDiscard"), "Discard")
      waitFor(function() { return shown(find(second, "recoveryDiscardConfirm")) }, "asks first")
      check(String(find(second, "recoveryText").text).indexOf("Discarding deletes") !== -1, "says what goes")
      click(find(second, "recoveryKeep"), "Keep it")
      waitFor(function() { return shown(find(second, "recoveryFinish")) }, "backed out")
      click(find(second, "recoveryDiscard"), "Discard again")
      click(find(second, "recoveryDiscardConfirm"), "Discard for good")
      waitFor(function() { return !shown(find(view(), "recoveryCard_0")) }, "no unfinished recordings left", 6000)
      check(!shown(find(view(), "recoveryError")), "no error")
    }

    function driveRecoveryOld() {
      openView()
      waitFor(function() { return shown(find(view(), "recoveryCard_0")) }, "the card")
      click(find(find(view(), "recoveryCard_0"), "recoveryFinish"), "Finish it")
      waitFor(function() { return shown(find(view(), "recoveryError")) }, "the explanation", 6000)
      check(txt("recoveryError").indexOf("finishes it the next time it starts") !== -1, "says when: " + txt("recoveryError"))
      check(shown(find(view(), "recoveryCard_0")), "still listed")
      shot("recovery-old")
    }

    function searchFor(text) {
      var field = find(view(), "meetingSearchField")
      click(field, "search box")
      field.text = text
    }

    function driveSearch() {
      openView()
      waitFor(function() { return shown(find(view(), "meetingRow_0")) }, "the list")
      searchFor("empty state")
      waitFor(function() { return txt("searchStatus") === "Searching…" && shown(find(view(), "searchStatus")) }, "Searching…: " + txt("searchStatus"))
      check(!shown(find(view(), "meetingList")), "results replace the list")
      waitFor(function() { return shown(find(view(), "searchResult_0")) }, "a result", 6000)
      var r = find(view(), "searchResult_0")
      check(String(find(r, "searchResultTitle").text) === "Design review", "title: " + find(r, "searchResultTitle").text)
      check(/ · 1:01 · Remote 2$/.test(String(find(r, "searchResultWhere").text)), "where: " + find(r, "searchResultWhere").text)
      var snip = String(find(r, "searchResultSnippet").text)
      check(snip.indexOf("<b>") !== -1 && snip.indexOf(">empty</font></b>") !== -1 && snip.indexOf(">state</font></b>") !== -1, "highlighted: " + snip)
      check(find(view(), "searchResults").count === 1, "one line has both words: " + find(view(), "searchResults").count)
      shot("search")
      click(r, "the result")
      waitFor(function() { return txt("meetingTitle") === "Design review" && view().currentTab === "transcript" }, "the meeting opened")
      waitFor(function() { return find(view(), "transcriptPane").flashSegId === 2 }, "on the line: " + find(view(), "transcriptPane").flashSegId)
      click(find(view(), "backButton"), "back")
      waitFor(function() { return shown(find(view(), "searchResult_0")) }, "the search kept on Back")
      check(String(find(view(), "meetingSearchField").text) === "empty state", "query kept")
      // Escape clears it, back to the list.
      var field = find(view(), "meetingSearchField")
      click(field, "search box")
      keyClick(Qt.Key_Escape)
      waitFor(function() { return shown(find(view(), "meetingList")) && !shown(find(view(), "searchResults")) }, "the list again")
      check(host.viewOpen(), "Escape in the box does not close the view")
      searchFor("zebra")
      waitFor(function() { return txt("searchStatus") === "No meetings match \u201czebra\u201d." }, "no match: " + txt("searchStatus"), 6000)
      searchFor("1:1")
      waitFor(function() { return shown(find(view(), "searchResult_0")) }, "title match", 6000)
      check(/ · in the title$/.test(String(find(find(view(), "searchResult_0"), "searchResultWhere").text)), "in the title")
    }

    function driveSearchOld() {
      openView()
      waitFor(function() { return shown(find(view(), "meetingRow_0")) }, "the list")
      searchFor("layout")
      waitFor(function() { return txt("searchStatus").indexOf("This voxtype is too old for searching meetings") === 0 }, "too old: " + txt("searchStatus"), 6000)
      shot("search-old")
    }

    function driveLevels() {
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      check(!shown(find(view(), "liveMeters")), "no meters before any levels")
      for (var i = 0; i < 4; i++) {
        ctl({ cmd: "levels", t_ms: 100 * i, mic: -30, mic_peak: -24, loopback: null, loopback_peak: null })
        wait(100)
      }
      waitFor(function() { return shown(find(view(), "liveMeters")) && meterFill("liveMicMeter") > 0 }, "mic meter")
      check(!shown(find(view(), "liveCallMeter")), "no call meter while loopback is null")
      // A sound check's levels and another meeting's are not this meeting's.
      var mic = feed().micDb
      ctl({ cmd: "levels", soundcheck: true, mic: -5, mic_peak: -2, loopback: -5, loopback_peak: -2 })
      ctl({ cmd: "levels", meeting_id: "00000000-0000-4000-8000-000000000000", mic: -5, mic_peak: -2, loopback: -5, loopback_peak: -2 })
      wait(300)
      check(feed().micDb === mic && !feed().loopbackSeen, "ignored: " + feed().micDb + " " + feed().loopbackSeen)
      for (var j = 0; j < 6; j++) {
        ctl({ cmd: "levels", t_ms: 500 + 100 * j, mic: -20, mic_peak: -12, loopback: -18, loopback_peak: -9 })
        wait(100)
      }
      waitFor(function() { return shown(find(view(), "liveCallMeter")) && meterFill("liveCallMeter") > 0 }, "call meter")
      shot("levels")
      // Levels stop (paused, or the daemon went quiet): the bars fall.
      waitFor(function() { return meterFill("liveMicMeter") === 0 && meterFill("liveCallMeter") === 0 }, "meters fall", 2000)
      check(shown(find(view(), "liveMeters")), "meters stay, empty")
    }

    function driveSoundCheck() {
      openSoundCheck()
      waitFor(function() { return txt("soundCheckStatus").indexOf("s left") !== -1 }, "running: " + txt("soundCheckStatus"))
      check(txt("soundCheckMic") === "voxtype_meeting_aec_source", "mic: " + txt("soundCheckMic"))
      check(txt("soundCheckEcho").indexOf("WebRTC") !== -1, "echo: " + txt("soundCheckEcho"))
      waitFor(function() { return meterFill("soundCheckMicMeter") > 0 && meterFill("soundCheckLoopbackMeter") > 0 }, "both meters move")
      check(!shown(find(view(), "soundCheckNoLoopback")), "call audio captured")
      check(shown(find(view(), "soundCheckStop")), "Stop offered")
      wait(300)
      shot("soundcheck")
      click(find(view(), "soundCheckStop"), "Stop")
      waitFor(function() { return txt("soundCheckStatus") === "Stopped." }, "stopped: " + txt("soundCheckStatus"))
      waitFor(function() { return meterFill("soundCheckMicMeter") === 0 && meterFill("soundCheckLoopbackMeter") === 0 }, "meters fall", 2000)
      check(String(find(view(), "soundCheckStart").text) === "Check again", "Check again offered")
      click(find(view(), "soundCheckStart"), "Check again")
      waitFor(function() { return txt("soundCheckStatus").indexOf("s left") !== -1 }, "running again")
      waitFor(function() { return txt("soundCheckStatus").indexOf("Done.") === 0 }, "ran to its end: " + txt("soundCheckStatus"), 8000)
      shot("soundcheck-done")
      click(find(view(), "soundCheckClose"), "back to the meetings")
      waitFor(function() { return shown(find(view(), "meetingListPane")) && !shown(find(view(), "soundCheckPane")) }, "list again")
    }

    function driveSoundCheckNoLoopback() {
      openSoundCheck()
      waitFor(function() { return txt("soundCheckStatus").indexOf("s left") !== -1 }, "running")
      waitFor(function() { return meterFill("soundCheckMicMeter") > 0 }, "mic meter moves")
      check(shown(find(view(), "soundCheckNoLoopback")) && !shown(find(view(), "soundCheckLoopbackMeter")), "no call audio said so")
      check(txt("soundCheckEcho").indexOf("No echo cancellation") === 0, "echo: " + txt("soundCheckEcho"))
      shot("soundcheck-no-loopback")
      // Leaving the panel while it runs stops the check.
      click(find(view(), "soundCheckClose"), "back while running")
      waitFor(function() { return shown(find(view(), "meetingListPane")) }, "list again")
      waitFor(function() { return view().soundcheck.checkState === "stopped" && view().soundcheck.reason === "stopped" }, "stopped by leaving: " + view().soundcheck.reason, 4000)
    }

    function driveSoundCheckError() {
      openSoundCheck()
      waitFor(function() { return view().soundcheck.checkState === "stopped" }, "failed")
      check(txt("soundCheckStatus") === "Audio connection failed: no such device 'usb-mic'", "message: " + txt("soundCheckStatus"))
      check(shown(find(view(), "soundCheckStart")) && !shown(find(view(), "soundCheckStop")), "can try again")
      check(meterFill("soundCheckMicMeter") === 0, "no level shown")
      shot("soundcheck-error")
    }

    function driveScroll() {
      openView()
      var lv = null
      waitFor(function() { lv = find(view(), "transcriptList"); return lv && lv.count === 40 }, "40 lines")
      waitFor(function() { return lv.atYEnd }, "starts at the latest line")
      ctl({ cmd: "segments", chunk_id: 4, segments: [seg(41, 125, "Line forty-one.", "microphone", "You", 4)] })
      waitFor(function() { return lv.count === 41 }, "41 lines")
      waitFor(function() { return lv.atYEnd }, "followed the new line")

      for (var i = 0; i < 4; i++) {
        mouseWheel(lv, lv.width / 2, lv.height / 2, 0, 120)
        wait(60)
      }
      waitFor(function() { return !lv.moving && !lv.flicking }, "wheel settled")
      wait(200)
      check(!lv.atYEnd, "scrolled up")
      check(!lv.follow, "stopped following")
      check(shown(find(view(), "jumpToLatest")), "Jump to latest offered")
      var y = lv.contentY
      ctl({ cmd: "segments", chunk_id: 4, segments: [seg(42, 128, "Line forty-two.", "loopback", "Remote", 4)] })
      waitFor(function() { return lv.count === 42 }, "42 lines")
      wait(300)
      check(Math.abs(lv.contentY - y) < 1, "did not jump while scrolled up: " + y + " -> " + lv.contentY)
      shot("scrolled-up")

      click(find(view(), "jumpToLatest"), "Jump to latest")
      waitFor(function() { return lv.atYEnd && lv.follow }, "back at the latest line")
      check(!shown(find(view(), "jumpToLatest")), "Jump to latest gone")
      ctl({ cmd: "segments", chunk_id: 4, segments: [seg(43, 131, "Line forty-three.", "microphone", "You", 4)] })
      waitFor(function() { return lv.count === 43 }, "43 lines")
      waitFor(function() { return lv.atYEnd }, "following again")
    }
  }

  Timer { interval: 80000; running: true; onTriggered: { console.log("MEETING_VIEW_FAIL timeout"); Qt.quit() } }
}
