import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import qs.Commons
import qs.Ui

// The right-hand pane: one scroller holding whatever the current selection
// asks for. Normally that is the active section's keys; on the engine section
// the engine/model card leads; on the replacements section the map editor
// replaces the form entirely; and while a search is running it is every
// matching key from every section.
//
// Keys carrying an `engine` render only for the engine in effect, which is
// what makes per-engine settings reflow after an engine switch.
Item {
  id: root

  property var keys: []
  property string engine: ""
  property string activeSection: ""
  property string searchText: ""

  property var engines: ({})
  property var engineChoices: []
  property var engineAvailability: ({})
  property var modelOptions: []
  property var deviceOptions: []
  property var styleOptions: []
  property var replacements: ({})

  // key → message from the last failed `config set`.
  property var errors: ({})
  property string engineError: ""
  property string replacementsError: ""

  property Item overlayHost: null
  property string replacementsSectionName: "Replacements"

  signal setRequested(string key, var value)
  signal unsetRequested(string key)
  signal engineChangeRequested(string name)
  signal downloadRequested(string name)
  signal replacementSetRequested(string from, string to)
  signal replacementUnsetRequested(string from)
  signal cancelDownloadRequested()

  // A row's field gave the keyboard back (Escape); the panel puts focus on the
  // card's key catcher so j/k drive the cursor again.
  signal focusReturned()

  // Counts the editors that currently own the keyboard, so the card's key
  // catcher can stand aside while any of them is focused.
  property int activeEditors: 0
  readonly property bool editorActive: activeEditors > 0

  function noteEditorFocus(active) {
    root.activeEditors = Math.max(0, root.activeEditors + (active ? 1 : -1))
  }

  readonly property string query: String(searchText).trim().toLowerCase()
  readonly property bool searching: query !== ""

  function appliesToEngine(spec) {
    if (!spec) return false
    if (spec.engine === undefined || spec.engine === null || spec.engine === "") return true
    return String(spec.engine) === root.engine
  }

  function matchesQuery(spec) {
    var haystack = [
      String(spec.key || ""),
      String(spec.label || ""),
      String(spec.description || ""),
      String(spec.section || "")
    ].join(" ").toLowerCase()
    return haystack.indexOf(root.query) !== -1
  }

  // The engine picker is pinned above this pane and the model picker lives in
  // the card at the top of it, so their form rows would be duplicates. Both are
  // resolved by the panel, which needs them for the runtime facts line too.
  property string modelKey: ""
  property string modelValue: ""

  // Download state, owned by VoxtypeCli and handed straight through to the
  // model card.
  property string downloadingModel: ""
  property real downloadPct: -1
  property bool downloadCancelling: false
  property var downloadErrors: ({})

  readonly property string engineSection: {
    for (var i = 0; i < root.keys.length; i++) {
      var spec = root.keys[i]
      if (spec && String(spec.key) === "engine") return String(spec.section || "Engine")
    }
    for (var j = 0; j < root.keys.length; j++) {
      var candidate = root.keys[j]
      if (candidate && candidate.engine) return String(candidate.section || "Engine")
    }
    return "Engine"
  }

  readonly property bool showReplacements: !searching && activeSection === replacementsSectionName
  readonly property bool showEngineCard: !searching && activeSection === engineSection

  readonly property var visibleKeys: {
    var out = []
    for (var i = 0; i < root.keys.length; i++) {
      var spec = root.keys[i]
      if (!spec) continue
      if (!root.appliesToEngine(spec)) continue
      if (root.searching) {
        if (!root.matchesQuery(spec)) continue
      } else {
        if (String(spec.section || "") !== root.activeSection) continue
        if (root.showEngineCard && (String(spec.key) === "engine" || String(spec.key) === root.modelKey)) continue
      }
      out.push(spec)
    }
    return out
  }

  function optionsForSource(source) {
    if (String(source) === "models") return root.modelOptions
    if (String(source) === "devices") return root.deviceOptions
    if (String(source) === "styles") return root.styleOptions
    return []
  }

  // An enum's choices are resolved here, off `keys`, rather than being read out
  // of the row's own modelData.
  //
  // A spec reaching a Repeater delegate as `modelData` keeps its scalars but its
  // arrays no longer answer to Array.isArray, so reading spec.choices in the
  // delegate silently produced an empty list: every enum row offered only the
  // value it already had, and no enum in the form could be changed except
  // through an open enum's "Custom value…" field. Looking the key up in `keys`
  // reads the object the schema was parsed into, where the array is still an
  // array — and the length walk below does not care either way.
  function choicesFor(key) {
    for (var i = 0; i < root.keys.length; i++) {
      var spec = root.keys[i]
      if (!spec || String(spec.key) !== key) continue
      var raw = spec.choices
      if (raw === undefined || raw === null) return []
      var out = []
      for (var j = 0; j < raw.length; j++) out.push(String(raw[j]))
      return out
    }
    return []
  }

  // ------------------------------------------------------- the GPU preset row
  //
  // Two schema keys presented as one decision. Both carry `engine: "whisper"`,
  // so the row appears exactly when its keys do — on the section that holds
  // them, for the engine that has them.

  function specFor(key) {
    for (var i = 0; i < root.keys.length; i++) {
      var spec = root.keys[i]
      if (!spec || String(spec.key) !== key) continue
      return root.appliesToEngine(spec) ? spec : null
    }
    return null
  }

  readonly property var gpuLoadingSpec: specFor("whisper.on_demand_loading")
  readonly property var gpuIsolationSpec: specFor("whisper.gpu_isolation")
  readonly property bool gpuPresetAvailable: gpuLoadingSpec !== null && gpuIsolationSpec !== null
  readonly property string gpuPresetSection: gpuPresetAvailable
    ? String(gpuIsolationSpec.section || "") : ""
  readonly property bool showGpuPreset: !searching && gpuPresetAvailable
    && gpuPresetSection !== "" && activeSection === gpuPresetSection

  // ------------------------------------------------------- the keyboard cursor
  //
  // One index over the rows this pane is showing: the preset row first when it
  // is present, then one per visible key. -1 is "no cursor", which is where it
  // sits until the user presses Down or j.

  property int highlightIndex: -1

  readonly property int leadingRows: showGpuPreset ? 1 : 0
  readonly property int rowCount: leadingRows + visibleKeys.length

  onActiveSectionChanged: root.highlightIndex = -1
  onSearchTextChanged: root.highlightIndex = -1

  // A schema refetch can shorten the list under the cursor.
  onRowCountChanged: {
    if (root.highlightIndex >= root.rowCount) root.highlightIndex = root.rowCount - 1
  }

  function clearHighlight() {
    root.highlightIndex = -1
  }

  function moveHighlight(delta) {
    if (root.rowCount === 0) return
    var next = root.highlightIndex < 0
      ? (delta > 0 ? 0 : root.rowCount - 1)
      : root.highlightIndex + delta
    root.highlightIndex = Math.max(0, Math.min(root.rowCount - 1, next))
    root.scrollTo(root.highlightIndex)
  }

  function activateHighlight() {
    if (root.highlightIndex < 0) return
    if (root.showGpuPreset && root.highlightIndex === 0) {
      gpuPreset.toggle()
      return
    }
    var row = rows.itemAt(root.highlightIndex - root.leadingRows)
    if (row && row.control && typeof row.control.activate === "function") row.control.activate()
  }

  function rowItem(index) {
    if (root.showGpuPreset && index === 0) return gpuPreset
    return rows.itemAt(index - root.leadingRows)
  }

  // Rows are direct children of `column`, which is the flickable's content, so
  // a row's own y is already the content offset to scroll to.
  function scrollTo(index) {
    var item = root.rowItem(index)
    var flick = scroller.contentItem
    if (!item || !flick) return
    var pad = Style.spacing.panelGap
    var top = item.y
    var bottom = top + item.height
    if (top - pad < flick.contentY) {
      flick.contentY = Math.max(0, top - pad)
      return
    }
    if (bottom + pad > flick.contentY + flick.height) {
      flick.contentY = Math.max(0, Math.min(flick.contentHeight - flick.height,
                                           bottom + pad - flick.height))
    }
  }

  ColumnLayout {
    anchors.fill: parent
    spacing: Style.spacing.panelGap

    // Outside the scroller on purpose: this is the one control in the Engine
    // section that has to stay on screen while the model list is scrolled.
    EnginePicker {
      Layout.fillWidth: true
      visible: root.showEngineCard
      engine: root.engine
      engineChoices: root.engineChoices
      engineAvailability: root.engineAvailability
      errorText: root.engineError
      onEngineChangeRequested: function(name) { root.engineChangeRequested(name) }
    }

    ScrollView {
      id: scroller
      Layout.fillWidth: true
      Layout.fillHeight: true
      clip: true
      ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

      Column {
        id: column
        width: scroller.availableWidth
        spacing: Style.spacing.panelGap

        EngineModelCard {
          width: parent.width
          visible: root.showEngineCard
          engine: root.engine
          engines: root.engines
          modelKey: root.modelKey
          modelValue: root.modelValue
          downloadingModel: root.downloadingModel
          downloadPct: root.downloadPct
          downloadCancelling: root.downloadCancelling
          downloadErrors: root.downloadErrors
          onModelChangeRequested: function(name) {
            if (root.modelKey !== "") root.setRequested(root.modelKey, name)
          }
          onDownloadRequested: function(name) { root.downloadRequested(name) }
          onCancelDownloadRequested: root.cancelDownloadRequested()
        }

        GpuIdlePresetRow {
          id: gpuPreset
          width: parent.width
          visible: root.showGpuPreset
          loadingSpec: root.gpuLoadingSpec
          isolationSpec: root.gpuIsolationSpec
          highlighted: root.highlightIndex === 0 && root.showGpuPreset
          errorText: root.gpuPresetAvailable
            ? (root.errors[String(root.gpuLoadingSpec.key)]
               || root.errors[String(root.gpuIsolationSpec.key)] || "")
            : ""
          onSetRequested: function(key, value) { root.setRequested(key, value) }
        }

        ReplacementsEditor {
          width: parent.width
          visible: root.showReplacements
          replacements: root.replacements
          errorText: root.replacementsError
          overlayHost: root.overlayHost
          onSetRequested: function(from, to) { root.replacementSetRequested(from, to) }
          onUnsetRequested: function(from) { root.replacementUnsetRequested(from) }
          onEditorFocusChanged: function(active) { root.noteEditorFocus(active) }
        }

        Text {
          width: parent.width
          visible: !root.showReplacements && root.visibleKeys.length === 0
          text: root.searching
            ? ("Nothing matches \"" + root.searchText.trim() + "\".")
            : "This section has no settings for the current engine."
          color: Qt.darker(Color.foreground, 1.5)
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WordWrap
        }

        Repeater {
          id: rows
          model: root.visibleKeys

          Column {
            id: entry
            required property var modelData
            required property int index

            readonly property alias control: setting
            readonly property bool highlighted: root.highlightIndex === root.leadingRows + entry.index

            width: parent.width
            spacing: Style.spacing.md

            // While searching, rows come from every section, so each one has to
            // say where it lives.
            PanelSectionHeader {
              visible: root.searching
              text: String(entry.modelData.section || "").toUpperCase()
            }

            // The cursor paints the whole row, not just its control, because
            // the label is what tells you which setting you are about to
            // change.
            CursorSurface {
              width: parent.width
              height: setting.implicitHeight + Style.spacing.md * 2
              hasCursor: entry.highlighted

              SettingDelegate {
                id: setting
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.leftMargin: Style.spacing.md
                anchors.rightMargin: Style.spacing.md
                spec: entry.modelData
                choices: root.choicesFor(String(entry.modelData.key))
                dynamicOptions: root.optionsForSource(entry.modelData.source)
                errorText: root.errors[String(entry.modelData.key)] || ""
                highlighted: entry.highlighted
                onSetRequested: function(key, value) { root.setRequested(key, value) }
                onUnsetRequested: function(key) { root.unsetRequested(key) }
                onEditorFocusChanged: function(active) { root.noteEditorFocus(active) }
                onEditorEscaped: root.focusReturned()
              }
            }

            PanelSeparator {}
          }
        }
      }
    }
  }
}
