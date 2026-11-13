import QtQuick
import Quickshell
import Quickshell.Io
import "../" as Plugin

// Run through test-schema-boundary.py so qs.Commons and qs.Ui resolve to the
// real Omarchy shell. This exercises the actual panel, not a JS-only copy.
Item {
  id: test
  property bool finished: false
  property string failure: ""

  Plugin.Panel { id: panel }

  QtObject {
    id: relay
    signal schemaLoaded(var schema)
    onSchemaLoaded: function(schema) {
      panel.applySchema(schema)
      check(panel.keys.length === 3, "schema keys were discarded at the QML signal boundary")
      check(JSON.stringify(panel.sections) === JSON.stringify(["Engine", "Audio", "Replacements"]),
            "missing sections: " + JSON.stringify(panel.sections))
      check(panel.engineChoices.length === 2, "engine choices were discarded at the QML boundary")
      check(String(panel.engineChoices[1]) === "parakeet", "engine choices changed")
      check(panel.modelKey === "", "unexpected model key")
      check(panel.keys.length > 1 && panel.keys[1].value === "default", "setting values changed")
      // Reload is the same path used after writes and external config changes.
      panel.applySchema(panel.schema)
      check(panel.keys.length === 3, "schema refetch lost keys")
      check(panel.engineChoices.length === 2, "schema refetch lost engine choices")
      panel.applySchema({ keys: [], engine: "whisper", replacements: {} })
      check(panel.sections.length === 1 && panel.sections[0] === "Replacements", "empty schema handling changed")
      panel.applySchema({ keys: null, engine: "whisper" })
      check(panel.keys.length === 0 && panel.sections.length === 0, "null keys handling changed")
      finish()
    }
  }

  function check(ok, message) {
    if (!ok && test.failure === "") test.failure = message
  }

  function finish() {
    test.finished = true
    console.log(test.failure === "" ? "SCHEMA_BOUNDARY_PASS" : "SCHEMA_BOUNDARY_FAIL: " + test.failure)
    Qt.callLater(Qt.quit)
  }

  Component.onCompleted: {
    // JSON.parse is what VoxtypeCli uses, then emits through signal(var).
    Qt.callLater(function() { relay.schemaLoaded(JSON.parse(JSON.stringify({
      engine: "whisper",
      replacements: {},
      keys: [
        { key: "engine", section: "Engine", choices: ["whisper", "parakeet"], value: "whisper" },
        { key: "audio.device", section: "Audio", value: "default" },
        { key: "parakeet.threads", section: "Other engine only", engine: "parakeet", value: 4 }
      ]
    }))) })
  }
  Timer {
    interval: 10000
    running: !test.finished
    onTriggered: { test.failure = "timeout"; test.finish() }
  }
}
