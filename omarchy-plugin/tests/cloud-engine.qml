import QtQuick
import QtTest
import Quickshell
import "plugin" as Plugin

// Driven by test-cloud-engine.py. The fake CLI reports Deepgram compiled, with
// no entry in `info models --json`, as the real one does. Browsing Deepgram
// must offer "Use Deepgram", and the click must leave it the configured engine.
Item {
  id: host
  Plugin.Panel { id: panel; manifest: ({__sourceDir: __PLUGIN__}) }

  readonly property string note: "Deepgram runs in the cloud and has no model to download. It reads its API key from DEEPGRAM_API_KEY in the daemon's environment, or [deepgram] api_key; without one the daemon will not start."
  readonly property string switchFailed: "Could not switch to this engine."
  readonly property string modelFailed: "Could not use this model."

  TestCase {
    id: tc
    parent: panel.renderTest
    name: "CloudEngine"
    when: false
    property bool started: false
    property string failure: ""

    Timer {
      interval: 100
      running: !tc.started
      repeat: true
      onTriggered: {
        if (!panel.opened || panel.engine !== __START__ || panel.activeSection !== "Engine") return
        if (tc.count(panel.engines) === 0) return
        if (__SCENARIO__ === "engines-unread" ? panel.infoErrors["engines"] === undefined
                                              : panel.engineAvailability["deepgram"] !== true) return
        tc.started = true
        try {
          if (__SCENARIO__ === "use") tc.driveUse()
          else if (__SCENARIO__ === "current") tc.driveCurrent()
          else if (__SCENARIO__ === "with-models") tc.driveWithModels()
          else if (__SCENARIO__ === "engines-unread") tc.driveEnginesUnread()
          else if (__SCENARIO__ === "switch-fails") tc.driveSwitchFails()
          else if (__SCENARIO__ === "model-fails") tc.driveModelFails()
          else tc.check(false, "unknown scenario " + __SCENARIO__)
        } catch (e) {
          if (tc.failure === "") tc.failure = String(e && e.message ? e.message : e)
        }
        console.log(tc.failure === "" ? "CLOUD_ENGINE_PASS " + __SCENARIO__
                                      : "CLOUD_ENGINE_FAIL " + __SCENARIO__ + ": " + tc.failure)
        Qt.callLater(Qt.quit)
      }
    }

    function count(map) {
      var n = 0
      for (var k in map) n++
      return n
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

    function shownByName(name) {
      var item = find(panel.renderTest, name)
      return item !== null && shown(item)
    }

    function shownKeys() {
      var list = panel.formTest.visibleKeys
      var out = []
      for (var i = 0; i < list.length; i++) out.push(String(list[i].key))
      return out
    }

    function browse(name) {
      // The engine dropdown only browses another engine's catalog.
      panel.formTest.engineChangeRequested(name)
      waitFor(function() { return panel.catalogEngine === name }, "browsing " + name)
      wait(100)
    }

    // The card says why there is no model list, instead of reporting none.
    function expectNote() {
      waitFor(function() { return shownByName("cataloglessNote") }, "the no-model line for Deepgram")
      var text = find(panel.renderTest, "cataloglessNote").text
      check(text === host.note, "no-model line reads: " + text)
      check(!shownByName("noModelsText"), "\"This engine reports no downloadable models.\" still shown")
      var header = find(panel.renderTest, "catalogHeader").text
      check(header === "DEEPGRAM", "card header reads: " + header)
    }

    function expectHeader(wanted) {
      var header = find(panel.renderTest, "catalogHeader").text
      check(header === wanted, "card header reads " + header + ", expected " + wanted)
    }

    // Browse Deepgram and click its Use, which must be offered.
    function clickUseDeepgram() {
      browse("deepgram")
      var use = null
      waitFor(function() {
        use = find(panel.renderTest, "useEngine:deepgram")
        return use !== null && shown(use)
      }, "Use button for Deepgram while Parakeet is configured")
      check(use.enabled, "Use button enabled")
      check(use.text === "Use Deepgram", "Use button reads: " + use.text)
      check(!shownByName("engineInUse:deepgram"), "Deepgram marked configured before Use")
      expectNote()
      mouseClick(use, use.width / 2, use.height / 2)
    }

    function expectDeepgramConfigured() {
      waitFor(function() { return panel.engine === "deepgram" },
              "Deepgram configured after Use (engine " + panel.engine + ")")
      waitFor(function() { return shownByName("engineInUse:deepgram") }, "Deepgram marked configured")
      waitFor(function() { return !shownByName("useEngine:deepgram") }, "Use hidden once Deepgram is configured")
      check(panel.catalogEngine === "deepgram", "still on Deepgram's card")
      check(shownKeys().indexOf("deepgram.model") !== -1, "Deepgram's settings shown: " + shownKeys())
      expectNote()
      // Long enough for a second write, had the click queued one.
      wait(500)
    }

    // The engine picker's summary line for a failed engine write.
    function expectEngineError(prefix) {
      waitFor(function() { return shownByName("engineErrorText") }, "an engine error shown")
      var text = find(panel.renderTest, "engineErrorText").text
      check(text.indexOf(prefix) === 0, "engine error reads: " + text)
    }

    // Use on Deepgram: one `config set engine deepgram`, then it is current.
    function driveUse() {
      check(!shownByName("useEngine:parakeet"), "engine Use offered for Parakeet, which has models")
      clickUseDeepgram()
      expectDeepgramConfigured()
    }

    // Deepgram already configured: marked so, and nothing to click.
    function driveCurrent() {
      waitFor(function() { return shownByName("engineInUse:deepgram") }, "Deepgram marked configured")
      check(!shownByName("useEngine:deepgram"), "Use offered for the configured engine")
      check(shownKeys().indexOf("deepgram.model") !== -1, "Deepgram's settings shown: " + shownKeys())
      expectNote()
    }

    // Control: an engine with a catalog keeps its model list and model Use,
    // and gets no engine Use or no-model line.
    function driveWithModels() {
      waitFor(function() { return shownByName("useModel:parakeet-tdt-0.6b-v3-Q8_0.gguf") },
              "Parakeet's model rows")
      check(!shownByName("useEngine:parakeet"), "engine Use offered for Parakeet")
      check(!shownByName("cataloglessNote"), "no-model line shown for Parakeet")
      expectHeader("MODELS FOR PARAKEET")
      browse("omnilingual")
      waitFor(function() { return shownByName("useModel:omnilingual-300m") }, "Omnilingual's model Use")
      check(!shownByName("useEngine:omnilingual"), "engine Use offered for Omnilingual")
      check(!shownByName("cataloglessNote"), "no-model line shown for Omnilingual")
      check(!shownByName("noModelsText"), "Omnilingual reported without models")
      expectHeader("MODELS FOR OMNILINGUAL")
    }

    // `info engines` failed. The dropdown then offers every engine, and so
    // does the card: `config set engine` is what refuses a missing one.
    function driveEnginesUnread() {
      clickUseDeepgram()
      expectDeepgramConfigured()
    }

    // `config set engine deepgram` fails: the picker says the engine switch
    // failed, and Use stays offered.
    function driveSwitchFails() {
      clickUseDeepgram()
      expectEngineError(host.switchFailed)
      check(panel.engine === "parakeet", "engine changed to " + panel.engine)
      waitFor(function() { return shownByName("useEngine:deepgram") }, "Use still offered after the failure")
    }

    // Control: a failed model pick keeps the model wording.
    function driveModelFails() {
      browse("omnilingual")
      var use = null
      waitFor(function() {
        use = find(panel.renderTest, "useModel:omnilingual-300m")
        return use !== null && shown(use)
      }, "Omnilingual's model Use")
      mouseClick(use, use.width / 2, use.height / 2)
      expectEngineError(host.modelFailed)
      check(panel.engine === "parakeet", "engine changed to " + panel.engine)
    }
  }

  Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Engine"}') })
  Timer { interval: 30000; running: true; onTriggered: { console.log("CLOUD_ENGINE_FAIL timeout"); Qt.quit() } }
}
