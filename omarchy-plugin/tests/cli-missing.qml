import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "plugin" as Plugin

// Driven by test-cli-missing.py. The panel's voxtype is fake-voxtype-migrate.py
// behind a wrapper the driver can move away and back, which is what a stale
// dev.json pointing at a deleted build looks like from the panel's side.
Item {
  id: host
  Plugin.Panel { id: panel; manifest: ({__sourceDir: __PLUGIN__}) }

  // Moves files around mid-test (mv, cp). Not one of the panel's processes.
  Process {
    id: helper
    property bool done: true
    onExited: helper.done = true
  }

  TestCase {
    id: tc
    parent: panel.renderTest
    name: "CliMissing"
    when: false
    property bool started: false
    property string failure: ""

    Timer {
      interval: 100
      running: !tc.started
      repeat: true
      onTriggered: {
        if (!panel.opened) return
        tc.started = true
        try {
          if (__SCENARIO__ === "missing") tc.driveMissing()
          else if (__SCENARIO__ === "nopath") tc.driveNoPath()
          else if (__SCENARIO__ === "vanish") tc.driveVanish()
          else if (__SCENARIO__ === "infofail") tc.driveInfoFail()
          else tc.check(false, "unknown scenario " + __SCENARIO__)
        } catch (e) {
          if (tc.failure === "") tc.failure = String(e && e.message ? e.message : e)
        }
        console.log(tc.failure === "" ? "CLI_MISSING_PASS " + __SCENARIO__
                                      : "CLI_MISSING_FAIL " + __SCENARIO__ + ": " + tc.failure)
        Qt.callLater(Qt.quit)
      }
    }

    function check(ok, message) {
      if (!ok) {
        tc.failure = message
        throw new Error(message)
      }
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
      wait(100)
      mouseClick(item, item.width / 2, item.height / 2)
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

    function count(obj) {
      var n = 0
      for (var k in obj) n++
      return n
    }

    function reopen() {
      panel.close()
      panel.open('{"section":"Engine"}')
    }

    function title() { return find(panel.renderTest, "degradedTitle").text }
    function detail() { return find(panel.renderTest, "degradedDetail").text }

    function waitLoaded() {
      waitFor(function() {
        return !panel.degraded && panel.voxtypeVersion === "1.2.0-rc4" && panel.modelOptions.length > 0
          && panel.statusKnown && panel.upgradeStatus !== null && panel.pluginLink !== null
          && panel.devices.length === 1 && panel.styles.length === 1
          && panel.engineAvailability["parakeet"] === true
      }, "everything loaded from the fake CLI")
    }

    // Nothing the panel shows may outlive the binary that said it.
    function checkCleared() {
      check(panel.keys.length === 0 && panel.sections.length === 0, "schema cleared")
      check(panel.voxtypeVersion === "", "version cleared: " + panel.voxtypeVersion)
      check(panel.engine === "" && panel.modelValue === "", "configured model cleared: " + panel.modelValue)
      check(panel.modelOptions.length === 0 && count(panel.engines) === 0, "model list cleared")
      check(count(panel.engineAvailability) === 0, "engine list cleared")
      check(panel.devices.length === 0 && panel.styles.length === 0, "device and style lists cleared")
      check(!panel.statusKnown && panel.daemonState === "", "daemon status cleared")
      check(panel.upgradeStatus === null, "upgrade offers cleared")
      check(panel.pluginLink === null, "plugin link cleared")
      check(!shown(panel.formTest), "settings form hidden")
      check(shown(find(panel.renderTest, "degradedState")), "error shown")
    }

    // dev.json names a path that does not exist.
    function driveMissing() {
      waitFor(function() { return panel.cliMissing !== "" }, "CLI failure noticed")
      check(panel.degraded, "panel degraded")
      check(panel.cliMissing === __MISSING__, "binary named: " + panel.cliMissing)
      check(title() === "Voxtype Settings could not run " + __MISSING__ + ".", "title: " + title())
      check(detail().indexOf("set in " + __DEVJSON__ + ".") !== -1, "detail names dev.json: " + detail())
      checkCleared()
      click(find(panel.renderTest, "installButton"), "Install / Update")
      waitFor(function() { return !panel.opened }, "dismissed before the installer launches")
      wait(800)
    }

    // No dev.json and no voxtype on PATH.
    function driveNoPath() {
      waitFor(function() { return panel.cliMissing !== "" }, "CLI failure noticed")
      check(panel.cliMissing === "voxtype", "binary named: " + panel.cliMissing)
      check(panel.cliMissingSource === "", "no dev.json source")
      check(title() === "Voxtype Settings could not run voxtype.", "title: " + title())
      check(detail() === "Is voxtype installed? Install it, or make sure voxtype is on PATH.", "detail: " + detail())
      checkCleared()
    }

    // The binary disappears between two summons: the second must not show the
    // first one's answers. Putting it back recovers on the next summon.
    function driveVanish() {
      waitLoaded()
      check(panel.modelValue === "parakeet-tdt-0.6b-v3", "model before: " + panel.modelValue)
      sh(["mv", __FAKE__, __FAKE__ + ".gone"])
      reopen()
      waitFor(function() { return panel.cliMissing !== "" }, "CLI failure noticed on re-summon")
      check(panel.cliMissing === __FAKE__, "binary named: " + panel.cliMissing)
      check(title().indexOf(__FAKE__) !== -1, "title names the binary: " + title())
      check(detail().indexOf(__DEVJSON__) !== -1, "detail names dev.json: " + detail())
      checkCleared()
      // A section jump on the still-broken panel must not resurrect anything.
      panel.ipcTest.section("Engine")
      wait(300)
      checkCleared()

      sh(["mv", __FAKE__ + ".gone", __FAKE__])
      reopen()
      waitLoaded()
      check(panel.cliMissing === "" && !panel.degraded, "recovered after the binary came back")
    }

    // The binary runs, but its list readers fail: say so, drop the stale
    // lists, and keep the rest of the panel working.
    function driveInfoFail() {
      waitLoaded()
      var banner = find(panel.renderTest, "cliErrorBanner")
      check(banner !== null && !shown(banner), "no error banner while everything works")
      sh(["cp", __FAIL_STATE__, __STATE__])
      check(panel.ipcTest.section("Engine") === "ok", "section verb")
      waitFor(function() { return shown(banner) }, "error banner shown")
      waitFor(function() { return panel.devices.length === 0 && panel.styles.length === 0
                           && count(panel.engineAvailability) === 0 }, "failed lists cleared")
      var text = find(banner, "cliErrorText").text
      check(text.indexOf("voxtype info engines failed: engines broke") !== -1, "engines error: " + text)
      check(text.indexOf("voxtype info devices failed: no audio backend") !== -1, "devices error: " + text)
      check(text.indexOf("voxtype info styles failed: styles broke") !== -1, "styles error: " + text)
      check(!panel.degraded && shown(panel.formTest), "the rest of the panel still works")
      check(panel.modelOptions.length > 0, "models unaffected")

      sh(["cp", __GOOD_STATE__, __STATE__])
      panel.ipcTest.section("Engine")
      waitFor(function() { return !shown(banner) && panel.devices.length === 1 }, "banner gone once readers succeed")
    }
  }

  Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Engine"}') })
  Timer { interval: 40000; running: true; onTriggered: { console.log("CLI_MISSING_FAIL timeout"); Qt.quit() } }
}
