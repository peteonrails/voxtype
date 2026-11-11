import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "plugin" as Plugin
import "standalone" as Standalone

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

    function shot(name) {
      if (__EVIDENCE__ === "") return
      var done = false
      wait(150)
      view().grabToImage(function(result) {
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
      check(txt("meetingEndedAt").indexOf("ended at ") === 0, "end time: " + txt("meetingEndedAt"))
      check(/^1:0\d$/.test(txt("meetingElapsed")), "length: " + txt("meetingElapsed"))
      check(!shown(find(view(), "pauseButton")) && !shown(find(view(), "stopButton")), "no Pause or Stop")
      check(shown(find(view(), "endedStartButton")) && find(view(), "endedStartButton").enabled, "Start meeting offered")
      check(!shown(find(view(), "idleState")), "not the idle screen")
      shot("ended")

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
      check(full.indexOf("[0:35] The build went green") === 0, "answer: " + full)
      var row = find(view(), "askThread").itemAtIndex(1)
      check(String(find(row, "answerBy").text) === "Answered by Claude", "attribution: " + find(row, "answerBy").text)
      shot("ask")

      // The answer opens with its [0:35] citation: clicking it shows that line.
      var answerText = find(row, "answerText")
      check(String(answerText.text).indexOf('<a href="#t=35000"') === 0, "citation linked: " + answerText.text)
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
      // The notification's Open action: `voxtype meeting view <id> --tab ask`.
      request({ meeting_id: __MEETING_ID__, tab: "ask", open: true })
      waitFor(function() { return host.viewOpen() && view() !== null }, "Open raised the view")
      tc.parent = view()
      waitFor(function() { return view().currentTab === "ask" }, "on the Ask tab: " + view().currentTab)
      check(find(view(), "askThread").count === 2, "the answer is in the thread")
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
      shot("agent-menu")
      click(find(menu, "localOnlyToggle"), "Local model only")
      check(!menu.visible, "menu closes")
      waitFor(function() { return chipLabel() === "Local (llama3.2)  ▾" }, "local: " + chipLabel())
      click(find(find(view(), "askPane"), "agentChip"), "agent chip")
      waitFor(function() { return menu.visible }, "menu again")
      check(String(find(menu, "localOnlyToggle").text) === "✓ Local model only", "checked: " + find(menu, "localOnlyToggle").text)
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
      ctl({ cmd: "autosummary", markdown: null, summary: {
        summary: "A short standup about the release.", decisions: ["Ship today."],
        action_items: [{ description: "Cut the tag", assignee: "Pete", completed: false }], key_points: [],
        chapters: [{ start_ms: 4000, title: "Greetings" }, { start_ms: 35000, title: "Release status" }] } })
      openView()
      waitFor(function() { return lines().length === 4 }, "hello rendered")
      check(find(view(), "tab_summary") === null || !find(view(), "tab_summary").visible, "no Summary tab while recording")
      click(find(view(), "stopButton"), "Stop")
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
          JSON.stringify({ summary_md: "## Overview\n\nRegenerated after fixing speakers [0:21]." })])
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
