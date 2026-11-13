import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "plugin" as Plugin

// Driven by test-onnx-model-only.py. The fake schema carries Parakeet's
// "Decoder type" flagged onnx_model_only (or, in "legacy", without the flag, as
// an older voxtype emits it). The form must hide it for a .gguf model only.
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
    name: "OnnxModelOnly"
    when: false
    property bool started: false
    property string failure: ""

    Timer {
      interval: 100
      running: !tc.started
      repeat: true
      onTriggered: {
        if (!panel.opened || panel.engine !== "parakeet" || panel.activeSection !== "Engine") return
        tc.started = true
        try {
          if (__SCENARIO__ === "onnx") tc.expectDecoder(true, "ONNX model")
          else if (__SCENARIO__ === "gguf") tc.expectDecoder(false, "GGUF model")
          else if (__SCENARIO__ === "legacy") tc.expectDecoder(true, "schema without the flag")
          else if (__SCENARIO__ === "switch") tc.driveSwitch()
          else tc.check(false, "unknown scenario " + __SCENARIO__)
        } catch (e) {
          if (tc.failure === "") tc.failure = String(e && e.message ? e.message : e)
        }
        console.log(tc.failure === "" ? "ONNX_MODEL_ONLY_PASS " + __SCENARIO__
                                      : "ONNX_MODEL_ONLY_FAIL " + __SCENARIO__ + ": " + tc.failure)
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

    function shownKeys() {
      var list = panel.formTest.visibleKeys
      var out = []
      for (var i = 0; i < list.length; i++) out.push(String(list[i].key))
      return out
    }

    // Both in the Engine section and through search, which skips the
    // section filter and is the other way a row can appear.
    function expectDecoder(shown, why) {
      check(panel.keys.length > 0, "schema loaded")
      var inSection = shownKeys().indexOf("parakeet.model_type") !== -1
      check(inSection === shown, why + ": Engine section shows Decoder type = " + inSection + " " + shownKeys())
      panel.searchText = "decoder"
      wait(50)
      var inSearch = shownKeys().indexOf("parakeet.model_type") !== -1
      panel.searchText = ""
      check(inSearch === shown, why + ": search shows Decoder type = " + inSearch)
    }

    // ONNX model, then the config moves to a GGUF model behind the panel.
    function driveSwitch() {
      expectDecoder(true, "before the switch")
      sh(["cp", __GGUF_STATE__, __STATE__])
      panel.ipcTest.section("Engine")
      waitFor(function() { return panel.modelValue === "parakeet-ultra-Q8_0.gguf" }, "GGUF model read")
      expectDecoder(false, "after the switch")
    }
  }

  Component.onCompleted: Qt.callLater(function() { panel.open('{"section":"Engine"}') })
  Timer { interval: 30000; running: true; onTriggered: { console.log("ONNX_MODEL_ONLY_FAIL timeout"); Qt.quit() } }
}
