import QtQuick
import QtTest
import Quickshell
import "plugin" as Plugin

// Driven by test-engine-switch-back.py. The config starts on the engine named
// by __START__ with Parakeet Ultra on disk. Browsing to Parakeet must offer
// Use on Ultra, and the click must leave Parakeet Ultra configured.
Item {
  id: host
  Plugin.Panel { id: panel; manifest: ({__sourceDir: __PLUGIN__}) }

  readonly property string target: __TARGET__
  readonly property string ultra: __MODEL__

  TestCase {
    id: tc
    parent: panel.renderTest
    name: "EngineSwitchBack"
    when: false
    property bool started: false
    property string failure: ""

    Timer {
      interval: 100
      running: !tc.started
      repeat: true
      onTriggered: {
        if (!panel.opened || panel.engine !== __START__ || panel.activeSection !== "Engine") return
        if (!panel.engines[host.target]) return
        tc.started = true
        try {
          tc.drive()
        } catch (e) {
          if (tc.failure === "") tc.failure = String(e && e.message ? e.message : e)
        }
        console.log(tc.failure === "" ? "ENGINE_SWITCH_BACK_PASS " + __SCENARIO__
                                      : "ENGINE_SWITCH_BACK_FAIL " + __SCENARIO__ + ": " + tc.failure)
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
      for (var p = item; p; p = p.parent) if (!p.visible) return false
      return true
    }

    function drive() {
      // The engine dropdown only browses another engine's catalog.
      panel.formTest.engineChangeRequested(host.target)
      waitFor(function() { return panel.catalogEngine === host.target }, "browsing " + host.target)
      var use = null
      waitFor(function() {
        use = find(panel.renderTest, "useModel:" + host.ultra)
        return use !== null && shown(use)
      }, "Use button on " + host.ultra + " while " + __START__ + " is configured")
      wait(100)
      check(shown(use) && use.enabled, "Use button settled")
      mouseClick(use, use.width / 2, use.height / 2)
      waitFor(function() { return panel.engine === host.target && panel.modelValue === host.ultra },
              host.ultra + " configured after Use (engine " + panel.engine + ", model " + panel.modelValue + ")")
      waitFor(function() { var b = find(panel.renderTest, "useModel:" + host.ultra); return b === null || !shown(b) },
              "Use hidden once Ultra is the configured model")
    }
  }

  Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Engine"}') })
  Timer { interval: 30000; running: true; onTriggered: { console.log("ENGINE_SWITCH_BACK_FAIL timeout"); Qt.quit() } }
}
