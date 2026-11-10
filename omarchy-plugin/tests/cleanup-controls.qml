import QtQuick
import Quickshell
import qs.Commons as ShellTheme
import "plugin" as Plugin

Item {
  id: test
  Plugin.Panel { id: panel }
  property int phase: 0
  property string failure: ""
  property bool finished: false
  property var writes: []
  function check(ok, message) { if (!ok && failure === "") failure = message }
  function spec(key) {
    for (var i = 0; i < panel.keys.length; ++i)
      if (panel.keys[i].key === key) return panel.keys[i]
    return null
  }
  function activate(key) {
    var form = panel.formUnderTest
    for (var i = 0; i < form.visibleKeys.length; ++i) {
      if (form.visibleKeys[i].key === key) {
        form.highlightIndex = i + form.leadingRows
        form.activateHighlight()
        return
      }
    }
    check(false, "missing row: " + key)
  }
  Connections {
    target: panel.formUnderTest
    function onSetRequested(key, value) { test.writes.push([key, value]) }
  }
  Connections {
    target: panel.cliUnderTest
    function onSchemaFailed(message) { test.failure = message; test.finish() }
    function onWriteFailed(key, message) { test.failure = key + ": " + message; test.finish() }
    function onSchemaLoaded(schema) { Qt.callLater(test.step) }
  }
  function step() {
    var form = panel.formUnderTest
    panel.activeSection = "Text Cleanup"
    if (phase === 0) {
      check(panel.sections.indexOf("Text Cleanup") !== -1 && panel.sections.indexOf("Text") === -1, "canonical Text Cleanup title missing")
      check(panel.canonicalSection("Text") === "Text Cleanup", "legacy Text IPC alias missing")
      var summons = []
      panel.shell = ({summon: function(id, payload) { summons.push(JSON.parse(payload).section); return true }})
      check(panel.ipcUnderTest.section("Text") === "ok", "legacy Text IPC request failed")
      check(panel.ipcUnderTest.section("Text Cleanup") === "ok", "Text Cleanup IPC request failed")
      check(JSON.stringify(summons) === JSON.stringify(["Text Cleanup", "Text Cleanup"]), "IPC payload not canonical")
      if (typeof form.cleanupGroup !== "function") { failure = "missing deterministic and LARD groups"; finish(); return }
      check(form.cleanupGroup("text.filter_filler_words") === "Clean up text", "missing deterministic group")
      check(form.cleanupGroup("text.lard_disfluency") === "LARD disfluency cleanup", "LARD must be separate")
      check(form.lardLicenseUrl === "https://creativecommons.org/licenses/by-sa/4.0/", "wrong model license")
      Qt.callLater(function() {
        var found = false
        for (var i = 0; i < form.visibleKeys.length; ++i) {
          if (form.visibleKeys[i].key !== "text.lard_disfluency") continue
          var row = form.rowItem(i + form.leadingRows)
          check(row.control.dim === Qt.darker(ShellTheme.Color.foreground, 1.5), "QtQuick Color shadows the shell theme; cleanup labels unreadable")
          check(row.control.specLabel === "LARD disfluency cleanup", "unclear LARD switch label")
          for (var j = 0; j < row.children.length; ++j) {
            var child = row.children[j]
            if (child.objectName === "lardModelLicense") {
              found = child.text.indexOf('href="' + form.lardLicenseUrl + '"') !== -1
                && child.text.indexOf("English-only") !== -1
                && child.text.indexOf("--source") !== -1
            }
          }
        }
        check(found, "missing real model license/setup affordance")
      })
      check(spec("text.lard_disfluency").value === false, "LARD enabled by default")
      phase = 1
      Qt.callLater(function() { activate("text.filter_filler_words") })
    } else if (phase === 1) {
      check(spec("text.filter_filler_words").value === true, "deterministic toggle not persisted")
      check(spec("text.lard_disfluency").value === false, "deterministic toggle enabled LARD")
      check(spec("text.collapse_restarts").value === false, "deterministic toggle reset sibling option")
      phase = 2
      Qt.callLater(function() { activate("text.lard_disfluency") })
    } else if (phase === 2) {
      check(spec("text.lard_disfluency").value === true, "LARD toggle not persisted")
      check(spec("text.filter_filler_words").value === true && spec("text.collapse_restarts").value === false,
            "LARD toggle changed deterministic options")
      phase = 3
      Qt.callLater(function() { activate("text.filter_filler_words") })
    } else if (phase === 3) {
      check(spec("text.filter_filler_words").value === false && spec("text.lard_disfluency").value === true,
            "deterministic off changed LARD")
      phase = 4
      Qt.callLater(function() { activate("text.lard_disfluency") })
    } else if (phase === 4) {
      check(spec("text.lard_disfluency").value === false, "LARD off not persisted")
      check(JSON.stringify(writes) === JSON.stringify([
        ["text.filter_filler_words", true], ["text.lard_disfluency", true],
        ["text.filter_filler_words", false], ["text.lard_disfluency", false]]), "unexpected config writes")
      var unavailable = JSON.parse(JSON.stringify(panel.schema))
      for (var k = 0; k < unavailable.keys.length; ++k)
        if (unavailable.keys[k].key === "text.lard_disfluency") unavailable.keys[k].compiled = false
      panel.applySchema(unavailable)
      Qt.callLater(function() {
        activate("text.lard_disfluency")
        check(writes.length === 4, "uncompiled LARD control accepted a write")
        test.checkOlderSchemas()
      })
    }
  }
  function checkOlderSchemas() {
      var form = panel.formUnderTest
      // Older executables never gain controls they do not advertise.
      panel.applySchema({engine: "whisper", keys: [{key: "text.spoken_punctuation", section: "Text", type: "bool", value: false}]})
      Qt.callLater(function() {
        check(form.specFor("text.lard_disfluency") === null, "old schema invents LARD support")
        check(form.visibleKeys.length === 1, "old schema gained unsupported controls")
        panel.applySchema(test.oldSchema)
        panel.activeSection = "Text Cleanup"
        Qt.callLater(function() {
          check(form.specFor("text.lard_disfluency") === null, "daily binary falsely offers LARD")
          for (var i = 0; i < form.visibleKeys.length; ++i)
            check(form.visibleKeys[i].key !== "text.lard_disfluency", "unsupported daily-binary switch rendered")
          test.finish()
        })
      })
  }
  function finish() {
    if (finished) return
    finished = true
    console.log(failure === "" ? "CLEANUP_CONTROLS_PASS" : "CLEANUP_CONTROLS_FAIL: " + failure)
    Qt.callLater(Qt.quit)
  }
  Component.onCompleted: Qt.callLater(function() { panel.cliUnderTest.fetchSchema() })
  Timer { interval: 15000; running: !test.finished; onTriggered: { test.failure = "timeout"; test.finish() } }
}
