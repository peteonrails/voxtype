import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "plugin" as Plugin

// Driven by test-meeting-view.py. The plugin runs against a fake meeting.sock
// (fake-meeting-sock.py) and a fake CLI (fake-voxtype-meeting.py); this file
// feeds the socket through its control file and checks what the view shows.
Item {
  id: host

  // Stands in for the Omarchy shell: records summons and opens the panel the
  // way the shell's summon() does.
  QtObject {
    id: fakeShell
    property var summons: []
    function summon(id, payload) {
      fakeShell.summons = fakeShell.summons.concat([String(payload)])
      panel.open(payload)
      return true
    }
    function hide(id) { panel.close(); return true }
    function toggle(id, payload) { if (panel.opened) fakeShell.hide(id); else fakeShell.summon(id, payload) }
  }

  Plugin.Panel { id: panel; manifest: ({__sourceDir: __PLUGIN__}); shell: fakeShell }

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

    function feed() { return panel.meetingsTest.feed }
    function view() { var w = panel.meetingsTest.window; return w ? w.view : null }
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
      check(panel.ipcTest.meetings() === "ok", "meetings verb")
      waitFor(function() { return view() !== null && panel.view === "meetings" }, "view opened")
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
      waitFor(function() { return shown(find(view(), "idleState")) }, "idle after stop")
      check(txt("idleTitle") === "No meeting in progress", "idle title: " + txt("idleTitle"))
      check(txt("lastMeeting").indexOf("“Standup” ended at") === 0 && txt("lastMeeting").indexOf("8 lines") !== -1,
            "last meeting: " + txt("lastMeeting"))
      check(panel.opened && panel.view === "meetings", "view stays open after the meeting")
      shot("idle-after")
    }

    function driveAutoOpen(expectOpen) {
      waitFor(function() { return feed().connected && feed().status === "idle" }, "feed connected while closed")
      check(!panel.opened, "closed to begin with")
      // As `voxtype meeting start`, the detection prompt's Record button or
      // the tray would: the daemon starts a meeting and says so.
      ctl({ cmd: "start", title: "Design review" })
      waitFor(function() { return feed().meetingId !== "" && feed().title === "Design review" }, "hello for the new meeting")
      if (!expectOpen) {
        wait(1500)
        check(!panel.opened, "show_view_on_start = false keeps the view closed")
        check(fakeShell.summons.length === 0, "no summon: " + JSON.stringify(fakeShell.summons))
        return
      }
      waitFor(function() { return panel.opened && panel.view === "meetings" && view() !== null }, "view opened itself")
      check(JSON.stringify(fakeShell.summons) === JSON.stringify(['{"view":"meetings"}']), "summoned through the shell: " + JSON.stringify(fakeShell.summons))
      tc.parent = view()
      waitFor(function() { return txt("meetingTitle") === "Design review" }, "title: " + txt("meetingTitle"))
      check(txt("meetingState") === "Recording", "state: " + txt("meetingState"))
      shot("opened-on-start")
    }

    function driveAutoOpenSettings() {
      waitFor(function() { return feed().connected && feed().status === "idle" }, "feed connected")
      panel.open("{}")
      check(panel.opened && panel.view === "settings", "Settings open")
      ctl({ cmd: "start", title: "Design review" })
      waitFor(function() { return feed().meetingId !== "" }, "meeting seen")
      wait(1200)
      check(panel.view === "settings", "Settings not taken over")
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
      waitFor(function() { return feed().connected && shown(find(view(), "idleState")) }, "idle after a restart", 12000)
      check(txt("idleTitle") === "No meeting in progress", "idle title: " + txt("idleTitle"))
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
      check(fakeShell.summons.length === 1, "only the meetings verb summoned: " + JSON.stringify(fakeShell.summons))
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
