import QtQuick
import QtTest
import Quickshell
import "plugin" as Plugin

// Driven by test-calendar-link.py. The calendar link row on the Meeting
// section: Set, a refused link, Clear, the status line, the keyboard cursor
// and search. Buttons get real pointer clicks and the field real key events;
// the Python side checks that the link only ever reached the CLI on stdin.
Item {
  id: host
  Plugin.Panel { id: panel; manifest: ({__sourceDir: __PLUGIN__}) }

  TestCase {
    id: tc
    parent: panel.renderTest
    name: "CalendarLink"
    when: false
    property bool started: false
    property string failure: ""
    // Status answers, and schemas read: old-cli has no status to wait for.
    property int statusLoads: 0
    property int schemaLoads: 0
    property string evidence: __EVIDENCE__
    property int shots: 0

    Connections {
      target: panel.cliTest
      function onSecretStatusLoaded(key, status) { tc.statusLoads++ }
      function onSchemaLoaded(schema) { tc.schemaLoads++ }
    }

    readonly property string scenario: __SCENARIO__
    readonly property string secret: __SECRET__
    // The link's host, as the CLI shows it.
    readonly property string linkHost: __HOST__

    Timer {
      interval: 100
      running: !tc.started
      repeat: true
      onTriggered: {
        if (!panel.opened || panel.keys.length === 0) return
        if (tc.scenario === "old-cli" ? tc.schemaLoads === 0 : tc.statusLoads === 0) return
        if (panel.activeSection !== "Meeting") return
        tc.started = true
        try {
          var s = tc.scenario
          if (s === "set") tc.driveSet()
          else if (s === "fail") tc.driveFail()
          else if (s === "saved") tc.driveSaved()
          else if (s === "keys") tc.driveKeys()
          else if (s === "search") tc.driveSearch()
          else if (s === "old-cli") tc.driveOldCli()
          else if (s === "real") tc.driveReal()
          else tc.check(false, "unknown scenario " + s)
        } catch (e) {
          if (tc.failure === "") tc.failure = String(e && e.message ? e.message : e)
        }
        console.log(tc.failure === "" ? "CALENDAR_LINK_PASS " + tc.scenario
                                      : "CALENDAR_LINK_FAIL " + tc.scenario + ": " + tc.failure)
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
        wait(50)
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
      return null
    }

    function shown(item) {
      if (!item) return false
      for (var p = item; p; p = p.parent) if (!p.visible) return false
      return true
    }

    function click(item, what) {
      check(item !== null, what + " missing")
      check(shown(item), what + " not visible")
      check(item.enabled, what + " disabled")
      wait(100)
      check(item.enabled && shown(item), what + " changed while settling")
      mouseClick(item, item.width / 2, item.height / 2)
    }

    function form() { return panel.formTest }
    function el(name) { return find(form(), name) }
    function txt(name) { var e = el(name); return e && shown(e) ? String(e.text) : "" }
    function field() { return el("secretField") }

    // Every visible Text in the form: the link must never be one of them.
    function visibleTexts() {
      var out = []
      function walk(item) {
        if (!item || !item.visible) return
        if (item.text !== undefined && item.echoMode === undefined) out.push(String(item.text))
        var kids = item.children || []
        for (var i = 0; i < kids.length; i++) walk(kids[i])
      }
      walk(form())
      return out
    }

    function noSecretShown(when) {
      var texts = visibleTexts()
      for (var i = 0; i < texts.length; i++)
        check(texts[i].indexOf("private-") === -1, "the link is on screen " + when + ": " + texts[i])
    }

    function shot(label) {
      if (tc.evidence === "") return
      tc.shots++
      var target = panel.renderTest
      var path = tc.evidence + "/" + tc.shots + "-" + label + ".png"
      waitFor(function() {
        return target.grabToImage(function(result) { result.saveToFile(path) })
      }, "grab " + label)
      wait(200)
    }

    // Types into the field with real key events, the way a paste lands.
    function typeLink(text) {
      var f = field()
      click(f, "the link field")
      waitFor(function() { return f.activeFocus }, "the field focused")
      f.selectAll()
      keyClick(Qt.Key_Delete)
      for (var i = 0; i < text.length; i++) keyClick(text.charAt(i))
      check(String(f.text) === text, "typed: " + f.text.length + " characters")
      check(f.echoMode === TextInput.Password, "the field is masked")
    }

    // Scrolls the secret row into view, as a user would: the real schema's
    // Meeting section is long enough to push it below the fold, where a
    // click lands nowhere.
    function reveal() {
      form().scrollTo(rowIndexOf("meeting.calendar.ics_link") + form().leadingRows)
      wait(200)
    }

    function rowIndexOf(key) {
      var keys = form().visibleKeys
      for (var i = 0; i < keys.length; i++) if (String(keys[i].key) === key) return i
      return -1
    }

    // ------------------------------------------------------------ scenarios

    function driveSet() {
      waitFor(function() { return shown(el("secretLabel")) }, "the row on the Meeting section")
      var at = rowIndexOf("meeting.calendar.ics_link")
      check(at === rowIndexOf("meeting.calendar.command") - 1 && at > 0,
            "right above the calendar keys: " + at + " vs " + rowIndexOf("meeting.calendar.command"))
      check(txt("secretStatus").indexOf("No link set. Meetings that start without a title") === 0,
            "status: " + txt("secretStatus"))
      check(!shown(el("secretClear")), "no Clear without a link")
      check(!el("secretSet").enabled, "Set waits for a link")
      // The help, folded until asked for.
      check(!shown(el("secretHelp")), "help folded")
      click(el("secretHelpToggle"), "Where to find it")
      waitFor(function() { return txt("secretHelp").indexOf("Secret address in iCal format") !== -1 },
              "help shown: " + txt("secretHelp"))
      shot("empty")

      // A link the CLI refuses stays in the field to be corrected.
      typeLink("calendar.example.com/feed")
      click(el("secretSet"), "Set")
      waitFor(function() { return txt("secretError") !== "" }, "refused")
      check(txt("secretError") === "That is not a calendar link: it must start with https://, http:// or webcal://",
            "the CLI's reason, without its prefix: " + txt("secretError"))
      check(String(field().text) === "calendar.example.com/feed", "kept to correct")

      // The real one. Enter in the field sets it too.
      typeLink(tc.secret)
      noSecretShown("while typing")
      keyClick(Qt.Key_Return)
      waitFor(function() { return txt("secretNotice") !== "" }, "saved", 6000)
      check(txt("secretNotice") === "Linked to calendar.google.com: 2 events in the next 48 hours.",
            "notice: " + txt("secretNotice"))
      check(String(field().text) === "", "the field is emptied once saved")
      check(txt("secretError") === "", "the error is gone")
      waitFor(function() { return txt("secretStatus").indexOf("Linked to calendar.google.com. Fetched at ") === 0 },
              "status from the CLI: " + txt("secretStatus"))
      check(txt("secretStatus").indexOf("Next: Design review, ") !== -1, "the next event: " + txt("secretStatus"))
      check(String(el("secretSet").text) === "Replace", "Set becomes Replace: " + el("secretSet").text)
      check(String(field().placeholderText).indexOf("Saved (calendar.google.com)") === 0,
            "placeholder: " + field().placeholderText)
      noSecretShown("after saving")
      shot("saved")

      click(el("secretClear"), "Clear")
      waitFor(function() { return txt("secretNotice") === "Removed." }, "cleared: " + txt("secretNotice"))
      waitFor(function() { return txt("secretStatus").indexOf("No link set.") === 0 }, "status after clear")
      check(!shown(el("secretClear")), "Clear goes with the link")
    }

    // Against a real voxtype, whose calendar store starts empty and whose
    // feed is a local server with one event an hour from now.
    function driveReal() {
      waitFor(function() { return shown(el("secretLabel")) }, "the row on the Meeting section")
      reveal()
      check(txt("secretStatus").indexOf("No link set. Meetings that start without a title") === 0,
            "status from the real CLI: " + txt("secretStatus"))
      click(el("secretHelpToggle"), "Where to find it")
      waitFor(function() { return txt("secretHelp").indexOf("Google Calendar") !== -1 }, "the real help text")
      typeLink("calendar.example.com/feed")
      click(el("secretSet"), "Set")
      waitFor(function() { return txt("secretError") !== "" }, "refused", 8000)
      check(txt("secretError") === "That is not a calendar link: it must start with https://, http:// or webcal://",
            "the real CLI's reason: " + txt("secretError"))
      typeLink(tc.secret)
      keyClick(Qt.Key_Return)
      waitFor(function() { return txt("secretNotice") !== "" || (txt("secretError") !== "" && String(field().text) === tc.secret && el("secretSet").enabled) }, "saved", 15000)
      check(txt("secretNotice") === "Linked to " + tc.linkHost + ": 1 event in the next 48 hours.",
            "notice: " + txt("secretNotice") + " / error: " + txt("secretError"))
      check(String(field().text) === "", "the field is emptied once saved")
      waitFor(function() { return txt("secretStatus").indexOf("Linked to " + tc.linkHost + ". Fetched at ") === 0 },
              "the real status: " + txt("secretStatus"))
      check(txt("secretStatus").indexOf("Next: Design review, ") !== -1, "the next event: " + txt("secretStatus"))
      noSecretShown("after saving")
      shot("real-saved")
      click(el("secretClear"), "Clear")
      waitFor(function() { return txt("secretStatus").indexOf("No link set.") === 0 }, "cleared: " + txt("secretStatus"), 8000)
    }

    function driveFail() {
      waitFor(function() { return shown(el("secretLabel")) }, "the row")
      check(txt("secretStatus") === "No link set. The Next Meeting widget on the bar names untitled meetings.",
            "the plugin names meetings: " + txt("secretStatus"))
      typeLink(tc.secret)
      click(el("secretSet"), "Set")
      waitFor(function() { return txt("secretError") !== "" }, "refused", 6000)
      check(txt("secretError").indexOf("The calendar server (calendar.google.com) answered 404") === 0,
            "the CLI's line: " + txt("secretError"))
      check(String(field().text) === tc.secret, "kept to correct")
      check(txt("secretNotice") === "", "no notice")
      check(!shown(el("secretClear")), "still no link")
      noSecretShown("after a refusal")
      shot("refused")
    }

    function driveSaved() {
      waitFor(function() { return shown(el("secretLabel")) }, "the row")
      check(txt("secretStatus").indexOf("Linked to outlook.office365.com. Fetched at ") === 0,
            "status: " + txt("secretStatus"))
      check(txt("secretStatus").indexOf("Nothing in the next 48 hours.") !== -1, "no events: " + txt("secretStatus"))
      check(txt("secretError") === "Last fetch failed: The calendar server (outlook.office365.com) did not answer.",
            "last error: " + txt("secretError"))
      check(String(el("secretSet").text) === "Replace", "Replace")
      check(shown(el("secretClear")), "Clear")
      shot("last-error")
      click(el("secretClear"), "Clear")
      waitFor(function() { return txt("secretStatus").indexOf("No link set.") === 0 }, "cleared")
      check(txt("secretError") === "", "the old error goes with the link: " + txt("secretError"))
    }

    function driveKeys() {
      waitFor(function() { return shown(el("secretLabel")) }, "the row")
      var at = rowIndexOf("meeting.calendar.ics_link") + form().leadingRows
      form().clearHighlight()
      for (var i = 0; i <= at; i++) form().moveHighlight(1)
      check(form().highlightIndex === at, "the cursor reaches the row: " + form().highlightIndex)
      var control = form().rowItem(at).control
      check(control !== null && control.savedCount !== undefined, "the cursor's control is the calendar row")
      form().activateHighlight()
      waitFor(function() { return field().activeFocus }, "Enter puts the cursor in the field")
      check(form().editorActive, "the card's key catcher stands aside")
      keyClick(Qt.Key_Escape)
      waitFor(function() { return !field().activeFocus }, "Escape leaves the field")
      check(!form().editorActive, "and gives the keys back")
      check(panel.opened, "Escape in the field does not close the panel")
      form().moveHighlight(1)
      check(String(form().visibleKeys[form().highlightIndex - form().leadingRows].key) === "meeting.calendar.command",
            "the next row is the calendar command")
    }

    function driveSearch() {
      waitFor(function() { return shown(el("secretLabel")) }, "the row")
      panel.searchText = "ical"
      waitFor(function() { return form().searching && shown(el("secretLabel")) }, "found by searching for ical")
      panel.searchText = "opacity"
      waitFor(function() { return !shown(el("secretLabel")) }, "not for other searches")
      panel.searchText = ""
    }

    function driveOldCli() {
      waitFor(function() { return rowIndexOf("meeting.calendar.command") !== -1 }, "the calendar keys")
      wait(300)
      check(rowIndexOf("meeting.calendar.ics_link") === -1 && el("secretLabel") === null,
            "a CLI without `meeting calendar` shows no link row")
    }

    Component.onCompleted: Qt.callLater(function() { panel.open(JSON.stringify({ section: "Meeting" })) })
  }
}
