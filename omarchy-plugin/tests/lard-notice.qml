import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "plugin" as Plugin

// Driven by test-lard-notice.py. The fake schema carries text.lard_disfluency
// switched on, with the `notice` the CLI adds while the model is missing (or
// without it, as a working install and an older voxtype both send). The row
// must show the notice exactly while the schema carries one.
Item {
  id: host
  Plugin.Panel { id: panel; manifest: ({__sourceDir: __PLUGIN__}) }

  Process {
    id: helper
    property bool done: true
    onExited: helper.done = true
  }

  TestCase {
    id: tc
    parent: panel.renderTest
    name: "LardNotice"
    when: false
    property bool started: false
    property string failure: ""

    Timer {
      interval: 100
      running: !tc.started
      repeat: true
      onTriggered: {
        if (!panel.opened || panel.keys.length === 0 || panel.activeSection !== "Text Cleanup") return
        tc.started = true
        try {
          if (__SCENARIO__ === "missing") tc.expectNotice(__NOTICE__, "model missing")
          else if (__SCENARIO__ === "ready") tc.expectNotice("", "schema without a notice")
          else if (__SCENARIO__ === "installed") tc.driveInstall()
          else tc.check(false, "unknown scenario " + __SCENARIO__)
        } catch (e) {
          if (tc.failure === "") tc.failure = String(e && e.message ? e.message : e)
        }
        console.log(tc.failure === "" ? "LARD_NOTICE_PASS " + __SCENARIO__
                                      : "LARD_NOTICE_FAIL " + __SCENARIO__ + ": " + tc.failure)
        Qt.callLater(Qt.quit)
      }
    }

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

    function sh(args) {
      helper.done = false
      helper.command = args
      helper.running = true
      waitFor(function() { return helper.done }, "helper " + args.join(" "))
    }

    // The LARD switch's row, or null while the form has not built it.
    function lardRow() {
      var form = panel.formTest
      for (var i = 0; i < form.visibleKeys.length; i++) {
        if (String(form.visibleKeys[i].key) !== "text.lard_disfluency") continue
        var row = form.rowItem(i + form.leadingRows)
        return row && row.control ? row.control : null
      }
      return null
    }

    // The notice label inside a row, by the name SettingDelegate gives it.
    function noticeLabel(item) {
      if (!item) return null
      if (item.objectName === "settingNotice") return item
      var kids = item.children || []
      for (var i = 0; i < kids.length; i++) {
        var found = noticeLabel(kids[i])
        if (found) return found
      }
      return null
    }

    function expectNotice(text, why) {
      waitFor(function() { return tc.lardRow() !== null }, "LARD row built")
      var row = lardRow()
      check(row.noticeText === text, why + ": noticeText = " + JSON.stringify(row.noticeText))
      var label = noticeLabel(row)
      check(label !== null, why + ": row has no notice label")
      check(label.visible === (text !== ""), why + ": notice label visible = " + label.visible)
      if (text !== "") check(label.text === text, why + ": label text = " + JSON.stringify(label.text))
    }

    // Model missing, then installed behind the open panel.
    function driveInstall() {
      expectNotice(__NOTICE__, "before the install")
      sh(["cp", __READY_STATE__, __STATE__])
      panel.ipcTest.section("Text Cleanup")
      waitFor(function() { var row = tc.lardRow(); return row !== null && row.noticeText === "" },
              "notice cleared after the refresh")
      expectNotice("", "after the install")
    }
  }

  Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Text Cleanup"}') })
  Timer { interval: 30000; running: true; onTriggered: { console.log("LARD_NOTICE_FAIL timeout"); Qt.quit() } }
}
