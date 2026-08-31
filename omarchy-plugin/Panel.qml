import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Commons
import qs.Ui
import "components"

// Voxtype settings, as an Omarchy panel plugin.
//
//   omarchy-shell shell toggle io.voxtype.settings '{}'
//   omarchy-shell shell toggle io.voxtype.settings '{"section":"Hotkey"}'
//
// or through this plugin's own IPC target, which is what a keybind wants
// because it needs no JSON payload:
//
//   omarchy-shell io.voxtype.settings toggle
//   omarchy-shell io.voxtype.settings section Hotkey
//
// The panel owns no settings model of its own: `voxtype config schema --json`
// is the whole source of truth, re-read on every summon and after every write,
// and each control writes straight back through `voxtype config set`. That
// keeps the CLI, the TUI, and this panel from disagreeing about defaults.
//
// The shell injects `shell` and `manifest`, calls open(payloadJson) on summon
// and close() on hide, and expects dismissal to route back through
// shell.hide(id) so its open/closed bookkeeping stays right.
Item {
  id: root

  property var shell: null
  property var manifest: null
  property string omarchyPath: ""

  property bool opened: false

  // ---------------------------------------------------------------- state

  property var schema: null
  property var keys: []
  property var sections: []
  property string activeSection: ""
  property string searchText: ""
  property string engine: ""
  property string voxtypeVersion: ""
  property string daemonVersionLabel: ""
  property bool daemonVersionDiffers: false
  property string configPath: ""
  property var replacements: ({})
  property var engineChoices: []
  property var engines: ({})
  property var engineAvailability: ({})
  property var devices: []

  property string schemaError: ""
  property var errors: ({})
  property string engineError: ""
  property string replacementsError: ""

  property string daemonState: ""
  property bool daemonRunning: false
  property bool statusKnown: false

  property string unitState: ""
  property string mainPid: ""
  property string vramLabel: ""
  property string vramText: ""

  property string accelState: ""
  property string accelBackend: ""
  property string accelEvidence: ""

  property bool restartNeeded: false
  property string restartError: ""

  // model name → message from the download that failed on it.
  property var downloadErrors: ({})

  readonly property string replacementsSectionName: "Replacements"
  readonly property bool degraded: schemaError !== ""

  readonly property var modelOptions: {
    var info = root.engines[root.engine]
    // Length walk, not Array.isArray: markModelInstalled rebuilds this list, and
    // an array crossing a `var` boundary does not reliably answer that question.
    if (!info || !info.models || info.models.length === undefined) return []
    var out = []
    for (var i = 0; i < info.models.length; i++) {
      var entry = info.models[i]
      if (!entry) continue
      var name = String(entry.name)
      out.push({ value: name, label: entry.installed === true ? name : name + "  (not downloaded)" })
    }
    return out
  }

  readonly property var deviceOptions: {
    var out = []
    for (var i = 0; i < root.devices.length; i++) {
      var entry = root.devices[i]
      if (!entry) continue
      var name = String(entry.name)
      out.push({ value: name, label: entry.default === true ? name + "  (default)" : name })
    }
    return out
  }

  // Which key holds the model for the engine in effect, and what it says.
  // Resolved here rather than in the form because the runtime facts line needs
  // the same answer, and two copies of this loop would be two chances to
  // disagree about what is loaded.
  readonly property string modelKey: {
    for (var i = 0; i < root.keys.length; i++) {
      var spec = root.keys[i]
      if (!spec || String(spec.type) !== "dynamic_enum") continue
      if (String(spec.source || "") !== "models") continue
      if (spec.engine !== undefined && spec.engine !== null && spec.engine !== ""
          && String(spec.engine) !== root.engine) continue
      return String(spec.key)
    }
    return ""
  }

  readonly property string modelValue: {
    for (var i = 0; i < root.keys.length; i++) {
      var spec = root.keys[i]
      if (spec && String(spec.key) === root.modelKey)
        return spec.value === undefined || spec.value === null ? "" : String(spec.value)
    }
    return ""
  }

  // ---------------------------------------------------------------- lifecycle

  function open(payloadJson) {
    var payload = {}
    try { payload = JSON.parse(payloadJson || "{}") || {} } catch (e) {}

    root.opened = true
    // Re-read on every summon: the config file may have been edited by the
    // TUI, another shell, or an upgrade since the last time we looked.
    root.searchText = ""
    root.errors = ({})
    root.engineError = ""
    root.replacementsError = ""
    root.restartError = ""
    root.schemaError = ""
    if (payload.section !== undefined) root.activeSection = String(payload.section)

    cli.fetchSchema()
    cli.fetchStatus()
    cli.fetchUnit()
    cli.fetchAccel()
    cli.fetchModels()
    cli.fetchEngines()
    cli.fetchDevices()

    // The layer surface is mapped after this returns, so focus has to be taken
    // once the window exists or Escape and typing land nowhere. The card's key
    // catcher is the fallback holder; the search field takes it from there when
    // there is a form to search.
    Qt.callLater(function() {
      if (!root.opened) return
      settingsCard.keyCatcher.forceActiveFocus()
      if (!root.degraded) sidebar.focusSearch()
    })
  }

  // The sidebar is hidden in the degraded state, and a hidden item cannot hold
  // focus, so Escape needs a home again.
  function refocusCard() {
    Qt.callLater(function() {
      if (root.opened) settingsCard.keyCatcher.forceActiveFocus()
    })
  }

  function close() {
    root.opened = false
  }

  function dismiss() {
    if (root.shell && typeof root.shell.hide === "function")
      root.shell.hide(root.pluginId)
    else close()
  }

  // Anything that opens its own window has to be launched after this panel is
  // out of the way. The panel is a layer-shell surface on the overlay layer
  // holding exclusive keyboard focus, so a terminal (or an editor) spawned while
  // it is up appears *behind* it and never takes focus — the user clicks the
  // button and, as far as they can tell, nothing happens. Dismiss first, spawn
  // on the next event-loop pass so the surface is actually gone.
  function dismissThen(action) {
    root.dismiss()
    Qt.callLater(action)
  }

  // A finished download is a fact the panel already knows, so it does not wait
  // for the catalog to confirm it. The re-fetch still follows and remains the
  // authority — this only closes the window where a successful download would
  // otherwise still be labelled "not downloaded".
  function markModelInstalled(name) {
    var wanted = String(name)
    var next = ({})
    for (var engineName in root.engines) {
      var info = root.engines[engineName]
      var models = info ? info.models : null
      if (!models || models.length === undefined) {
        next[engineName] = info
        continue
      }
      var copied = []
      var changed = false
      for (var i = 0; i < models.length; i++) {
        var entry = models[i]
        if (entry && String(entry.name) === wanted && entry.installed !== true) {
          var updated = ({})
          for (var f in entry) updated[f] = entry[f]
          updated.installed = true
          copied.push(updated)
          changed = true
        } else {
          copied.push(entry)
        }
      }
      if (!changed) {
        next[engineName] = info
        continue
      }
      var copiedInfo = ({})
      for (var k in info) copiedInfo[k] = info[k]
      copiedInfo.models = copied
      next[engineName] = copiedInfo
    }
    root.engines = next
  }

  function noteDownloadError(model, message) {
    var next = ({})
    for (var k in root.downloadErrors) if (k !== model) next[k] = root.downloadErrors[k]
    if (message !== "") next[model] = message
    root.downloadErrors = next
  }

  // ---------------------------------------------------------------- ipc
  //
  // A second way in, for keybinds and scripts: `omarchy-shell
  // io.voxtype.settings toggle` instead of `omarchy-shell shell toggle
  // io.voxtype.settings '{}'`. Both land in the same place — every verb here
  // routes through the shell's summon/hide rather than flipping `opened`
  // directly, so the shell's own record of which panels are open stays right
  // and the existing summon path is untouched.
  //
  // The handler only exists while this item does, which is why the manifest
  // sets `keepLoaded`: without it the panel is instantiated on summon and the
  // target would be missing exactly when a keybind needs it.

  readonly property string pluginId: (root.manifest && root.manifest.id) || "io.voxtype.settings"

  function requestOpen(payloadJson) {
    if (!root.shell || typeof root.shell.summon !== "function") return "no-shell"
    return root.shell.summon(root.pluginId, payloadJson) ? "ok" : "unknown"
  }

  IpcHandler {
    target: "io.voxtype.settings"

    function open(): string {
      // Already up: leave the panel as the user left it rather than resetting
      // its search and section.
      if (root.opened) return "ok"
      return root.requestOpen("{}")
    }

    function close(): string {
      if (!root.shell || typeof root.shell.hide !== "function") return "no-shell"
      return root.shell.hide(root.pluginId) ? "ok" : "unknown"
    }

    function toggle(): string {
      if (!root.shell || typeof root.shell.toggle !== "function") return "no-shell"
      root.shell.toggle(root.pluginId, "{}")
      return "ok"
    }

    // Opens on a named section, or jumps to it if the panel is already up. An
    // unknown name lands on the first section, same as a stale payload does.
    function section(name: string): string {
      var wanted = String(name || "")
      if (wanted === "") return "usage: section <name>"
      if (root.opened) {
        if (root.sections.indexOf(wanted) === -1) return "unknown section: " + wanted
        root.activeSection = wanted
        root.searchText = ""
        sidebar.searchField.text = ""
        form.clearHighlight()
        return "ok"
      }
      return root.requestOpen(JSON.stringify({ section: wanted }))
    }

    // Restarts the daemon, not the panel — the same button the restart banner
    // offers, reachable without opening anything. There is deliberately no
    // `status` verb: `voxtype status --format json` already answers that, and a
    // closed panel has not asked the daemon anything yet.
    function restart(): string {
      cli.restartDaemon()
      return "ok"
    }

    // Starts the same download the model row's button starts, tracked by the
    // same progress and reported by the same header chip. Worth having for its
    // own sake — it is the only way to drive a download without a pointer — and
    // it makes the download path scriptable alongside the other verbs.
    function download(model: string): string {
      var name = String(model || "")
      if (name === "") return "usage: download <model>"
      if (cli.downloading) return "busy: already downloading " + cli.downloadModel
      root.noteDownloadError(name, "")
      cli.startDownload(name)
      return "ok"
    }

    function cancelDownload(): string {
      if (!cli.downloading) return "idle"
      cli.cancelDownload()
      return "ok"
    }
  }

  // ---------------------------------------------------------------- schema

  function applySchema(next) {
    root.schemaError = ""
    root.schema = next
    root.keys = Array.isArray(next.keys) ? next.keys : []
    root.engine = next.engine === undefined || next.engine === null ? "" : String(next.engine)
    root.voxtypeVersion = next.voxtype_version ? String(next.voxtype_version) : ""
    root.daemonVersionLabel = next.daemon_version_label ? String(next.daemon_version_label) : ""
    root.daemonVersionDiffers = next.daemon_version_differs === true
    root.configPath = next.config_path ? String(next.config_path) : ""
    root.replacements = (next.replacements && typeof next.replacements === "object") ? next.replacements : ({})
    root.sections = computeSections(root.keys, next.replacements !== undefined)
    root.engineChoices = computeEngineChoices(root.keys)

    if (root.sections.indexOf(root.activeSection) === -1)
      root.activeSection = root.sections.length > 0 ? root.sections[0] : ""

    // Delegates are rebuilt from scratch on a new schema, so any focus they
    // were holding is gone with them.
    form.activeEditors = 0
  }

  // Sidebar order is schema order: the first time a section name appears in
  // keys[] is where it sits.
  function computeSections(list, hasReplacements) {
    var out = []
    var seen = ({})
    for (var i = 0; i < list.length; i++) {
      var spec = list[i]
      if (!spec) continue
      // A key belonging to another engine must not conjure a section that has
      // nothing in it.
      if (spec.engine !== undefined && spec.engine !== null && spec.engine !== ""
          && String(spec.engine) !== root.engine) continue
      var section = String(spec.section || "")
      if (section === "" || seen[section]) continue
      seen[section] = true
      out.push(section)
    }
    if (hasReplacements) out.push(root.replacementsSectionName)
    return out
  }

  function computeEngineChoices(list) {
    for (var i = 0; i < list.length; i++) {
      var spec = list[i]
      if (spec && String(spec.key) === "engine" && Array.isArray(spec.choices)) return spec.choices
    }
    return []
  }

  // ---------------------------------------------------------------- writes

  function recordError(key, message) {
    if (key === "engine") {
      root.engineError = message
      return
    }
    if (key.indexOf("text.replacements.") === 0) {
      root.replacementsError = message
      return
    }
    var next = ({})
    for (var k in root.errors) next[k] = root.errors[k]
    next[key] = message
    root.errors = next
  }

  function clearError(key) {
    if (key === "engine") root.engineError = ""
    if (key.indexOf("text.replacements.") === 0) root.replacementsError = ""
    if (root.errors[key] === undefined) return
    var next = ({})
    for (var k in root.errors) if (k !== key) next[k] = root.errors[k]
    root.errors = next
  }

  VoxtypeCli {
    id: cli
    manifest: root.manifest

    onSchemaLoaded: function(next) { root.applySchema(next) }
    onSchemaFailed: function(message) {
      root.schemaError = message
      root.keys = []
      root.sections = []
      root.refocusCard()
    }
    onModelsLoaded: function(engines) { root.engines = engines }
    onDevicesLoaded: function(devices) { root.devices = devices }
    onEnginesLoaded: function(list) {
      var next = ({})
      for (var i = 0; i < list.length; i++) {
        var entry = list[i]
        if (entry && entry.name !== undefined) next[String(entry.name)] = entry.compiled === true
      }
      root.engineAvailability = next
    }
    onStatusLoaded: function(state, running) {
      root.daemonState = state
      root.daemonRunning = running
      root.statusKnown = true
    }
    onUnitLoaded: function(activeState, pid) {
      root.unitState = activeState
      root.mainPid = pid
      // The GPU query needs the pid to attribute memory to, so it follows the
      // unit rather than running beside it.
      cli.fetchVram(pid)
    }
    onVramLoaded: function(label, text) {
      root.vramLabel = label
      root.vramText = text
    }
    onAccelLoaded: function(state, backend, evidence) {
      root.accelState = state
      root.accelBackend = backend
      root.accelEvidence = evidence
    }
    onWriteSucceeded: function(key) {
      root.clearError(key)
      // The daemon only reads config at startup, so a successful write is
      // exactly when the restart prompt becomes true.
      root.restartNeeded = true
      root.restartError = ""
      cli.fetchSchema()
      if (key === "engine") cli.fetchModels()
    }
    onWriteFailed: function(key, message) {
      root.recordError(key, message)
      // The control is showing a value the config never took; the schema is
      // the only thing that knows what it actually holds now.
      cli.fetchSchema()
    }
    onDownloadFinished: function(model, ok, message) {
      // A cancellation reports ok=false with no message: the user asked for it,
      // so there is nothing to tell them.
      root.noteDownloadError(model, ok ? "" : message)
      if (ok) root.markModelInstalled(model)
      // The catalog is what says whether the file is on disk now, so re-read it
      // either way — a cancelled or failed download may still have left a
      // partial state worth reflecting.
      cli.fetchModels()
    }
    onRestarted: function(ok, message) {
      if (ok) {
        root.restartNeeded = false
        root.restartError = ""
        // A restart is the one moment acceleration can change: the daemon has
        // just re-read the config and re-picked a backend. The reading is stale
        // until the new process has got far enough to say so, hence the delay.
        accelAfterRestart.restart()
      } else {
        root.restartError = message
      }
    }
  }

  Timer {
    id: accelAfterRestart
    interval: 2500
    repeat: false
    onTriggered: cli.fetchAccel()
  }

  Timer {
    id: poll
    property int ticks: 0

    interval: 5000
    running: root.opened && !root.degraded
    repeat: true
    onTriggered: {
      cli.fetchStatus()
      cli.fetchUnit()
      poll.ticks++
      // The catalog changes far less often than the daemon's state, so it is
      // re-read on every other tick rather than every one. This is the layer
      // that heals a stale "not downloaded" no matter which earlier step failed
      // to notice, so it must not depend on a download having been observed.
      if (poll.ticks % 2 === 0) cli.fetchModels()
    }
  }

  // ------------------------------------------------------- external edits
  //
  // The shell does not call open() again on a summon that finds the panel
  // already open, so a config change made behind the panel's back used to leave
  // it rendering a schema that no longer existed: switch engines with `voxtype
  // config set engine parakeet` while the panel is up and it kept offering the
  // whisper model list, marked "in use".
  //
  // Watching the file itself covers every writer — the CLI, the TUI, the Edit
  // config button, another shell — and does not care which one it was. Our own
  // writes trip it too, which costs one redundant read the write path was
  // already doing.
  FileView {
    id: configWatcher
    path: root.opened && root.configPath !== "" ? root.configPath : ""
    watchChanges: true
    printErrors: false
    // `voxtype config set` writes atomically, so the path is replaced rather
    // than edited; reload() is what re-arms the watch on the new file.
    onFileChanged: {
      reload()
      reloadDebounce.restart()
    }
  }

  // A single `config set` can produce more than one change notification, and a
  // TUI save writes the whole file; coalesce them into one refetch.
  Timer {
    id: reloadDebounce
    interval: 400
    repeat: false
    onTriggered: {
      if (!root.opened) return
      cli.fetchSchema()
      cli.fetchModels()
      // The banner's claim is that the file on disk no longer matches what the
      // daemon loaded at startup, which an edit from the CLI or the TUI makes
      // just as true as one made here. Without this the panel would quietly
      // refresh to show a setting the running daemon is not using.
      root.restartNeeded = true
    }
  }

  // Two cursors on screen at once would be a lie about where the keys are
  // going, so the row cursor stands down whenever the search box takes over.
  Connections {
    target: sidebar
    function onSearchActiveChanged() {
      if (sidebar.searchActive) form.clearHighlight()
    }
  }

  // ---------------------------------------------------------------- window

  PanelWindow {
    id: window
    visible: root.opened
    anchors { top: true; bottom: true; left: true; right: true }
    color: "transparent"
    exclusionMode: ExclusionMode.Ignore
    WlrLayershell.namespace: "voxtype-settings"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive

    SettingsCard {
      id: settingsCard
      anchors.fill: parent
      cardWidth: Style.space(900)
      cardHeight: Style.space(640)
      keyBlocked: sidebar.searchActive || form.editorActive
      onDismissed: root.dismiss()

      // j/k and the arrows drive one cursor down the form; Enter and Space act
      // on the row under it. The card only sees these once the search field has
      // let go of the keyboard (`keyBlocked` above), which Down out of the
      // search box is what does.
      onMoveRequested: function(dx, dy) {
        if (root.degraded) return
        if (dy !== 0) form.moveHighlight(dy)
      }
      onActivateRequested: {
        if (!root.degraded) form.activateHighlight()
      }

      ColumnLayout {
        anchors.fill: parent
        spacing: Style.spacing.panelGap

        HeaderBar {
          Layout.fillWidth: true
          // Above the rows below it so the badge's evidence panel, which hangs
          // below the header, paints over the facts line instead of under it.
          z: 2
          voxtypeVersion: root.voxtypeVersion
          daemonVersionLabel: root.daemonVersionLabel
          daemonVersionDiffers: root.daemonVersionDiffers
          configPath: root.configPath
          restarting: cli.restarting
          accelState: root.accelState
          accelBackend: root.accelBackend
          accelEvidence: root.accelEvidence
          onRestartRequested: cli.restartDaemon()
          onTuiRequested: root.dismissThen(function() { cli.openTui() })
          onEditConfigRequested: root.dismissThen(function() {
            cli.openConfigEditor(root.configPath)
          })
          onCloseRequested: root.dismiss()
        }

        RuntimeFacts {
          Layout.fillWidth: true
          visible: !root.degraded
          engine: root.engine
          model: root.modelValue
          daemonState: root.daemonState
          statusKnown: root.statusKnown
          unitState: root.unitState
          mainPid: root.mainPid
          vramLabel: root.vramLabel
          vramText: root.vramText
        }

        // A PanelSeparator binds its own width to its parent, which fights a
        // layout for control of it; inside a ColumnLayout the rule is a plain
        // rectangle instead.
        Rectangle {
          Layout.fillWidth: true
          implicitHeight: 1
          color: Util.alpha(Color.foreground, 0.12)
        }

        RestartBanner {
          Layout.fillWidth: true
          active: root.restartNeeded || root.restartError !== ""
          restarting: cli.restarting
          message: root.restartError !== ""
            ? root.restartError
            : "Changes take effect after restart."
          onRestartRequested: cli.restartDaemon()
        }

        // ------------------------------------------------ degraded state
        //
        // Everything here depends on `config schema --json`, which older
        // voxtype builds do not have. Say so, and offer the two things that
        // fix it.
        Item {
          Layout.fillWidth: true
          Layout.fillHeight: true
          visible: root.degraded

          BorderSurface {
            anchors.centerIn: parent
            width: Math.min(parent.width, Style.space(460))
            implicitHeight: degradedBody.implicitHeight + contentTopInset + contentBottomInset
            height: implicitHeight
            color: Util.alpha(Color.foreground, 0.04)
            radius: Style.cornerRadius
            borderSpec: Border.flat(Util.alpha(Color.urgent, 0.45), Math.max(1, Style.normalBorderWidth))
            padding: Style.spacing.panelPadding

            Column {
              id: degradedBody
              anchors.left: parent.left
              anchors.right: parent.right
              anchors.top: parent.top
              anchors.margins: Style.spacing.panelPadding
              spacing: Style.spacing.xxl

              Text {
                width: parent.width
                text: "This panel requires voxtype 0.8 or newer."
                color: Color.foreground
                font.family: Style.font.family
                font.pixelSize: Style.font.title
                wrapMode: Text.WordWrap
              }

              Text {
                width: parent.width
                text: "It reads every setting from `voxtype config schema --json`, which earlier builds do not provide."
                color: Qt.darker(Color.foreground, 1.5)
                font.family: Style.font.family
                font.pixelSize: Style.font.bodySmall
                wrapMode: Text.WordWrap
              }

              Text {
                width: parent.width
                visible: root.schemaError !== ""
                text: root.schemaError
                color: Color.urgent
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                wrapMode: Text.WordWrap
              }

              Row {
                spacing: Style.spacing.controlGap

                Button {
                  text: "Install / Update"
                  iconText: "󰇚"
                  bordered: true
                  onClicked: root.dismissThen(function() { cli.runInstaller() })
                }

                Button {
                  text: "Open TUI"
                  iconText: "󰆍"
                  bordered: true
                  onClicked: root.dismissThen(function() { cli.openTui() })
                }
              }
            }
          }
        }

        // ------------------------------------------------ the settings body

        RowLayout {
          Layout.fillWidth: true
          Layout.fillHeight: true
          visible: !root.degraded
          spacing: Style.spacing.panelGap

          SectionSidebar {
            id: sidebar
            Layout.fillHeight: true
            Layout.preferredWidth: Style.space(180)
            sections: root.sections
            activeSection: root.activeSection
            searching: root.searchText.trim() !== ""
            onSectionSelected: function(section) {
              root.activeSection = section
              root.searchText = ""
              sidebar.searchField.text = ""
            }
            onSearchChanged: function(text) { root.searchText = text }
            onDismissRequested: root.dismiss()

            // Down out of the search box is how the keyboard gets from typing
            // to traversing: the field lets go, the card's key catcher takes
            // over, and the cursor lands on the first row.
            onTraverseRequested: {
              root.refocusCard()
              form.moveHighlight(1)
            }
          }

          Rectangle {
            Layout.fillHeight: true
            width: 1
            color: Util.alpha(Color.foreground, 0.12)
          }

          SchemaForm {
            id: form
            Layout.fillWidth: true
            Layout.fillHeight: true
            keys: root.keys
            engine: root.engine
            activeSection: root.activeSection
            searchText: root.searchText
            engines: root.engines
            engineChoices: root.engineChoices
            engineAvailability: root.engineAvailability
            modelOptions: root.modelOptions
            deviceOptions: root.deviceOptions
            modelKey: root.modelKey
            modelValue: root.modelValue
            downloadingModel: cli.downloadModel
            downloadPct: cli.downloadPct
            downloadCancelling: cli.downloadCancelling
            downloadErrors: root.downloadErrors
            replacements: root.replacements
            errors: root.errors
            engineError: root.engineError
            replacementsError: root.replacementsError
            replacementsSectionName: root.replacementsSectionName
            overlayHost: settingsCard.card

            onFocusReturned: root.refocusCard()

            onSetRequested: function(key, value) { cli.setKey(key, value) }
            onUnsetRequested: function(key) { cli.unsetKey(key) }
            onEngineChangeRequested: function(name) { cli.setKey("engine", name) }
            onDownloadRequested: function(name) {
              root.noteDownloadError(name, "")
              cli.startDownload(name)
            }
            onCancelDownloadRequested: cli.cancelDownload()
            onReplacementSetRequested: function(from, to) { cli.setReplacement(from, to) }
            onReplacementUnsetRequested: function(from) { cli.unsetReplacement(from) }
          }
        }
      }
    }
  }
}
