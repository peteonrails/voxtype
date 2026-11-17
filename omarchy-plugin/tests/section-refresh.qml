import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "plugin" as Plugin

// Driven by test-section-refresh.py. Opens the panel, changes the fake CLI's
// state behind its back, then sends the `section` IPC verb to the open panel:
// everything open() reads must be read again, and the new answers shown.
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
    name: "SectionRefresh"
    when: false
    property bool started: false
    property string failure: ""
    property var loads: ({})

    Connections {
      target: panel.cliTest
      function onSchemaLoaded(s) { tc.bump("schema") }
      function onModelsLoaded(e) { tc.bump("models") }
      function onEnginesLoaded(l) { tc.bump("engines") }
      function onDevicesLoaded(d) { tc.bump("devices") }
      function onStylesLoaded(s) { tc.bump("styles") }
      function onStatusLoaded(s, r) { tc.bump("status") }
      function onAccelLoaded(s, b, e) { tc.bump("accel") }
      function onUpgradeLoaded(s) { tc.bump("upgrade") }
      function onPluginLinkLoaded(s) { tc.bump("link") }
    }

    function bump(what) {
      var next = ({})
      for (var k in tc.loads) next[k] = tc.loads[k]
      next[what] = (next[what] || 0) + 1
      tc.loads = next
    }

    readonly property var readers: ["schema", "models", "engines", "devices", "styles", "status", "accel", "upgrade", "link"]

    function allAtLeast(n) {
      for (var i = 0; i < tc.readers.length; i++)
        if ((tc.loads[tc.readers[i]] || 0) < n) return false
      return true
    }

    Timer {
      interval: 100
      running: !tc.started
      repeat: true
      onTriggered: {
        if (!panel.opened) return
        tc.started = true
        try {
          if (__SCENARIO__ === "open") tc.driveOpen()
          else if (__SCENARIO__ === "closed") tc.driveClosed()
          else tc.check(false, "unknown scenario " + __SCENARIO__)
        } catch (e) {
          if (tc.failure === "") tc.failure = String(e && e.message ? e.message : e)
        }
        console.log(tc.failure === "" ? "SECTION_REFRESH_PASS " + __SCENARIO__
                                      : "SECTION_REFRESH_FAIL " + __SCENARIO__ + ": " + tc.failure)
        // The Python side counts invocations from here on; mark the boundary.
        console.log("SECTION_REFRESH_LOADS " + JSON.stringify(tc.loads))
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

    function driveOpen() {
      waitFor(function() { return tc.allAtLeast(1) }, "first read of everything: " + JSON.stringify(tc.loads))
      check(panel.modelValue === "parakeet-tdt-0.6b-v3", "model before: " + panel.modelValue)
      check(panel.upgradeStatus !== null && panel.upgradeStatus.offers.length === 0, "no offers before")
      check(panel.activeSection === "Engine", "opened on Engine")

      // Behind the panel's back: the configured model changes and an offer
      // appears, as after an upgrade with the panel left open.
      sh(["cp", __CHANGED_STATE__, __STATE__])
      check(panel.ipcTest.section("Audio") === "ok", "section verb on an open panel")
      check(panel.activeSection === "Audio", "jumped to Audio")
      waitFor(function() { return tc.allAtLeast(2) }, "second read of everything: " + JSON.stringify(tc.loads))
      waitFor(function() { return panel.modelValue === "parakeet-ultra-Q8_0.gguf" }, "new model shown: " + panel.modelValue)
      check(panel.upgradeStatus.offers.length === 1, "new offer shown")
      check(panel.pluginLink !== null && panel.pluginLink.state === "copy", "new link state read")

      // An unknown section still refreshes, and says it is unknown.
      check(panel.ipcTest.section("Nonsense") === "unknown section: Nonsense", "unknown section answer")
      waitFor(function() { return tc.allAtLeast(3) }, "unknown section still re-reads: " + JSON.stringify(tc.loads))
      check(panel.activeSection === "Audio", "unknown section leaves the panel where it was")
    }

    // A closed panel asks the shell to open it rather than refreshing in place.
    function driveClosed() {
      waitFor(function() { return tc.allAtLeast(1) }, "first read of everything")
      panel.close()
      var before = tc.loads["schema"]
      check(panel.ipcTest.section("Audio") === "no-shell", "closed panel routes through the shell")
      wait(300)
      check(tc.loads["schema"] === before, "closed panel does not read anything itself")
    }
  }

  Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Engine"}') })
  Timer { interval: 40000; running: true; onTriggered: { console.log("SECTION_REFRESH_FAIL timeout"); Qt.quit() } }
}
