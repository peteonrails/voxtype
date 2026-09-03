import QtQuick
import Quickshell
import Quickshell.Io

// Every voxtype subprocess the panel needs, in one place. The panel itself
// only ever calls functions here and reacts to signals, so the process
// plumbing (queueing, exit-code mapping, JSON parsing) stays out of the UI.
//
// Binary resolution: an optional `dev.json` next to the plugin's manifest
// may name the binary to drive, so a work-in-progress build can be demoed
// without installing it:
//
//   { "voxtypeBin": "/home/you/voxtype/target/release/voxtype" }
//
// The file is read synchronously (blockLoading) at load time, which is
// before the shell calls the panel's open(), so the very first schema fetch
// already uses the right binary. Without the file we run "voxtype" off PATH.
Item {
  id: root

  // Injected down from Panel.qml, which receives it from the shell.
  property var manifest: null

  readonly property string manifestDir: (manifest && manifest.__sourceDir) ? String(manifest.__sourceDir) : ""
  property string bin: "voxtype"

  // Command used for anything that wants a terminal the user can watch.
  readonly property string terminalLauncher: "omarchy-launch-floating-terminal-with-presentation"

  property bool schemaLoading: false
  property bool restarting: false
  readonly property bool writing: writeProc.running

  signal schemaLoaded(var schema)
  signal schemaFailed(string message)
  signal modelsLoaded(var engines)
  signal enginesLoaded(var list)
  signal devicesLoaded(var devices)
  signal stylesLoaded(var styles)
  signal statusLoaded(string state, bool running)
  signal unitLoaded(string activeState, string mainPid)
  // state is "unknown" whenever the answer could not be obtained, which is also
  // what a build without `info accel` produces.
  //
  // The evidence arrives already joined into one string rather than as an array.
  // An array has to survive four `var` hops to reach the badge, and an array that
  // has crossed a QML property boundary stops answering to Array.isArray and can
  // lose `length` with it; a string crosses unchanged.
  signal accelLoaded(string state, string backend, string evidence)
  // `text` empty means "we could not find out" — the caller hides the reading
  // rather than printing a number nobody measured.
  signal vramLoaded(string label, string text)
  signal writeSucceeded(string key)
  signal writeFailed(string key, string message)
  signal restarted(bool ok, string message)

  // ------------------------------------------------------------- helpers

  function quote(value) {
    return "'" + String(value).replace(/'/g, "'\\''") + "'"
  }

  // `config set` takes the literal the TOML wants. Booleans are the only
  // type whose JS stringification would be wrong for the CLI, everything
  // else round-trips.
  function formatValue(value) {
    if (typeof value === "boolean") return value ? "true" : "false"
    return String(value)
  }

  function launchInTerminal(command) {
    Quickshell.execDetached([root.terminalLauncher, command])
  }

  // ------------------------------------------------------------- schema

  function fetchSchema() {
    if (schemaProc.running) {
      // A summon can land while the previous fetch is still in flight.
      // Queue one re-run rather than dropping the request.
      schemaProc.refetch = true
      return
    }
    root.schemaLoading = true
    schemaProc.refetch = false
    schemaProc.command = [root.bin, "config", "schema", "--json"]
    schemaProc.running = true
  }

  Process {
    id: schemaProc
    property bool refetch: false
    property string out: ""
    property string err: ""

    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: schemaProc.out = String(text || "")
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: schemaProc.err = String(text || "").trim()
    }
    onExited: function(exitCode) {
      if (schemaProc.refetch) {
        schemaProc.refetch = false
        Qt.callLater(root.fetchSchema)
        return
      }
      root.schemaLoading = false
      if (exitCode !== 0) {
        root.schemaFailed(schemaProc.err || ("voxtype config schema exited " + exitCode))
        return
      }
      var parsed = null
      try {
        parsed = JSON.parse(schemaProc.out)
      } catch (e) {
        root.schemaFailed("voxtype config schema did not return JSON")
        return
      }
      if (!parsed || typeof parsed !== "object" || !Array.isArray(parsed.keys)) {
        root.schemaFailed("voxtype config schema returned an unexpected shape")
        return
      }
      root.schemaLoaded(parsed)
    }
  }

  // ------------------------------------------------------------- writes
  //
  // Writes are serialized through one Process. A Process ignores a command
  // change while it is running, so overlapping edits (a toggle flipped
  // while a debounced slider write is in flight) have to queue rather than
  // race. Pending entries for the same key collapse onto the newest value:
  // the user's last intent is the one worth writing.

  property var writeQueue: []

  function enqueueWrite(op) {
    var next = []
    for (var i = 0; i < writeQueue.length; i++) {
      // Index 0 is the entry currently running; never rewrite that one.
      if (i > 0 && writeQueue[i].key === op.key) continue
      next.push(writeQueue[i])
    }
    next.push(op)
    writeQueue = next
    pumpWrites()
  }

  function pumpWrites() {
    if (writeProc.running || writeQueue.length === 0) return
    var op = writeQueue[0]
    writeProc.activeKey = op.key
    writeProc.err = ""
    writeProc.command = op.args
    writeProc.running = true
  }

  function setKey(key, value) {
    enqueueWrite({ key: String(key), args: [root.bin, "config", "set", String(key), formatValue(value)] })
  }

  function unsetKey(key) {
    enqueueWrite({ key: String(key), args: [root.bin, "config", "unset", String(key)] })
  }

  function setReplacement(from, to) {
    setKey("text.replacements." + from, to)
  }

  function unsetReplacement(from) {
    unsetKey("text.replacements." + from)
  }

  Process {
    id: writeProc
    property string activeKey: ""
    property string err: ""

    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: writeProc.err = String(text || "").trim()
    }
    onExited: function(exitCode) {
      var key = writeProc.activeKey
      var message = writeProc.err
      root.writeQueue = root.writeQueue.slice(1)
      if (exitCode === 0) root.writeSucceeded(key)
      else if (exitCode === 2) root.writeFailed(key, message || "voxtype rejected that value")
      else root.writeFailed(key, message || "voxtype could not write the config file")
      root.pumpWrites()
    }
  }

  // ------------------------------------------------------------- info

  // The catalog is the only thing that knows whether a model is on disk, so a
  // dropped read leaves the UI asserting "not downloaded" about a file that is
  // sitting right there. Dropping the request when the process happened to be
  // busy is exactly how that happened after a download: one fetch, no retry,
  // and the wrong answer stuck until the panel was reopened.
  //
  // So a request that arrives mid-flight is queued rather than discarded, and a
  // read that fails or returns something unparseable is retried once.
  property bool pendingModels: false
  property int modelsRetries: 0

  function fetchModels() {
    if (modelsProc.running) {
      root.pendingModels = true
      return
    }
    root.pendingModels = false
    modelsRetry.stop()
    modelsProc.out = ""
    modelsProc.command = [root.bin, "info", "models", "--json"]
    modelsProc.running = true
  }

  Timer {
    id: modelsRetry
    interval: 1000
    repeat: false
    onTriggered: root.fetchModels()
  }

  Process {
    id: modelsProc
    property string out: ""
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: modelsProc.out = String(text || "")
    }
    onExited: function(exitCode) {
      var delivered = false
      if (exitCode === 0) {
        try {
          var parsed = JSON.parse(modelsProc.out)
          if (parsed && typeof parsed === "object" && parsed.engines) {
            root.modelsLoaded(parsed.engines)
            delivered = true
          }
        } catch (e) {
          // A build without `info models --json` leaves the list empty; the
          // schema's own value still renders.
        }
      }

      if (delivered) root.modelsRetries = 0

      // A request that arrived while this one was running takes precedence over
      // any retry: it is the newer question.
      if (root.pendingModels) {
        root.pendingModels = false
        Qt.callLater(root.fetchModels)
        return
      }

      // One retry per failure, so a transient failure heals without turning a
      // permanently broken command into a spin.
      if (!delivered && root.modelsRetries < 1) {
        root.modelsRetries++
        modelsRetry.restart()
      }
    }
  }

  // `info engines --json` is the authority on which engines this binary can
  // actually run: [{ name, compiled, active }, ...].
  function fetchEngines() {
    if (enginesProc.running) return
    enginesProc.command = [root.bin, "info", "engines", "--json"]
    enginesProc.running = true
  }

  Process {
    id: enginesProc
    property string out: ""
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: enginesProc.out = String(text || "")
    }
    onExited: function(exitCode) {
      if (exitCode !== 0) return
      try {
        var parsed = JSON.parse(enginesProc.out)
        if (Array.isArray(parsed)) root.enginesLoaded(parsed)
      } catch (e) {
        // Without this list the panel dims nothing, and an uncompiled engine
        // is caught by `config set` exiting 2 instead.
      }
    }
  }

  function fetchDevices() {
    if (devicesProc.running) return
    devicesProc.command = [root.bin, "info", "devices", "--json"]
    devicesProc.running = true
  }

  Process {
    id: devicesProc
    property string out: ""
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: devicesProc.out = String(text || "")
    }
    onExited: function(exitCode) {
      if (exitCode !== 0) return
      try {
        var parsed = JSON.parse(devicesProc.out)
        if (Array.isArray(parsed)) root.devicesLoaded(parsed)
      } catch (e) {
        // Same as models: an empty list is a survivable answer.
      }
    }
  }

  function fetchStyles() {
    if (stylesProc.running) return
    stylesProc.command = [root.bin, "info", "styles", "--json"]
    stylesProc.running = true
  }

  Process {
    id: stylesProc
    property string out: ""
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: stylesProc.out = String(text || "")
    }
    onExited: function(exitCode) {
      if (exitCode !== 0) return
      try {
        var parsed = JSON.parse(stylesProc.out)
        if (Array.isArray(parsed)) root.stylesLoaded(parsed)
      } catch (e) {
        // Same as models: an empty list is a survivable answer.
      }
    }
  }

  // ------------------------------------------------------------- status

  function fetchStatus() {
    if (statusProc.running) return
    statusProc.command = [root.bin, "status", "--format", "json"]
    statusProc.running = true
  }

  Process {
    id: statusProc
    property string out: ""
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: statusProc.out = String(text || "")
    }
    onExited: function(exitCode) {
      if (exitCode !== 0) {
        root.statusLoaded("stopped", false)
        return
      }
      var state = ""
      try {
        var parsed = JSON.parse(statusProc.out)
        if (parsed && typeof parsed === "object") state = String(parsed["class"] || "")
      } catch (e) {
        state = ""
      }
      if (state === "") state = "stopped"
      root.statusLoaded(state, state !== "stopped")
    }
  }

  // ------------------------------------------------------------- systemd unit
  //
  // `voxtype status` answers "what is the daemon doing", which is a different
  // question from "is the unit up". A unit in `failed` reports no status at all,
  // and collapsing that into "stopped" hides the one state the user has to act
  // on. `--value` prints the two properties bare, in the order asked for.

  function fetchUnit() {
    if (unitProc.running) return
    unitProc.out = ""
    unitProc.command = ["systemctl", "--user", "show", "voxtype", "-p", "ActiveState", "-p", "MainPID", "--value"]
    unitProc.running = true
  }

  Process {
    id: unitProc
    property string out: ""
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: unitProc.out = String(text || "")
    }
    onExited: function(exitCode) {
      if (exitCode !== 0) {
        root.unitLoaded("", "")
        return
      }
      var lines = unitProc.out.split("\n")
      var state = lines.length > 0 ? lines[0].trim() : ""
      var pid = lines.length > 1 ? lines[1].trim() : ""
      // systemd reports 0 for "no main process", which is not a pid.
      if (pid === "0") pid = ""
      root.unitLoaded(state, pid)
    }
  }

  // ------------------------------------------------------- acceleration
  //
  // Deliberately not on the status poll: this reads the daemon's journal, which
  // is far heavier than asking for its state. It is fetched when the panel opens
  // and again after a restart, which are the two moments the answer can change.
  //
  // Anything that fails — a build with no `info accel` subcommand exits 2 — is
  // reported as "unknown", which the badge renders as nothing.

  function fetchAccel() {
    if (accelProc.running) return
    accelProc.out = ""
    accelProc.command = [root.bin, "info", "accel", "--json"]
    accelProc.running = true
  }

  Process {
    id: accelProc
    property string out: ""
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: accelProc.out = String(text || "")
    }
    onExited: function(exitCode) {
      if (exitCode !== 0) {
        root.accelLoaded("unknown", "", "")
        return
      }
      var parsed = null
      try {
        parsed = JSON.parse(accelProc.out)
      } catch (e) {
        root.accelLoaded("unknown", "", "")
        return
      }
      if (!parsed || typeof parsed !== "object") {
        root.accelLoaded("unknown", "", "")
        return
      }
      var state = parsed.state === undefined || parsed.state === null
        ? "unknown" : String(parsed.state)
      var backend = parsed.backend === undefined || parsed.backend === null
        ? "" : String(parsed.backend)
      // Flattened here, at the only point where the parsed JSON is still a
      // plain JS value.
      var lines = []
      var raw = parsed.evidence
      if (raw && raw.length !== undefined) {
        for (var i = 0; i < raw.length; i++) {
          var line = String(raw[i] || "").trim()
          if (line !== "") lines.push(line)
        }
      }
      root.accelLoaded(state, backend, lines.join("\n"))
    }
  }

  // ------------------------------------------------------------- GPU memory
  //
  // Two vendors, two different questions answerable. nvidia-smi reports memory
  // per compute process, so it can be filtered to the daemon's own pid.
  // rocm-smi's per-process accounting is not dependable, so ROCm reports the
  // card total and is labelled as such.
  //
  // Which tool to use is decided by running it, not by whether it is installed:
  // a machine can carry nvidia-smi with no NVIDIA driver behind it, where the
  // binary exists and exits non-zero. Anything that fails or fails to parse
  // ends as an empty reading, and the reading is then not shown at all.

  property string vramPid: ""

  function fetchVram(mainPid) {
    if (nvidiaProc.running || rocmProc.running) return
    root.vramPid = String(mainPid || "")
    nvidiaProc.out = ""
    nvidiaProc.command = ["nvidia-smi", "--query-compute-apps=pid,used_memory", "--format=csv,noheader"]
    nvidiaProc.running = true
  }

  function tryRocm() {
    if (rocmProc.running) return
    rocmProc.out = ""
    rocmProc.command = ["rocm-smi", "--showmeminfo", "vram", "--json"]
    rocmProc.running = true
  }

  function formatBytes(bytes) {
    var n = Number(bytes)
    if (!isFinite(n) || n < 0) return ""
    var gib = n / (1024 * 1024 * 1024)
    if (gib >= 1) return gib.toFixed(1) + " GiB"
    return Math.round(n / (1024 * 1024)) + " MiB"
  }

  Process {
    id: nvidiaProc
    property string out: ""
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: nvidiaProc.out = String(text || "")
    }
    onExited: function(exitCode) {
      // Missing binary, or present with no driver behind it (exit 9).
      if (exitCode !== 0) {
        root.tryRocm()
        return
      }
      // Without a pid there is nothing to attribute the memory to, and the
      // card total is not what this reading claims to be.
      if (root.vramPid === "") {
        root.vramLoaded("", "")
        return
      }
      // "3094859, 412 MiB" per compute process. The daemon not appearing means
      // it holds no CUDA context, which is a reading, not a failure.
      var lines = nvidiaProc.out.split("\n")
      for (var i = 0; i < lines.length; i++) {
        var parts = lines[i].split(",")
        if (parts.length < 2) continue
        if (parts[0].trim() !== root.vramPid) continue
        root.vramLoaded("VRAM", parts[1].trim())
        return
      }
      root.vramLoaded("VRAM", "none held")
    }
  }

  Process {
    id: rocmProc
    property string out: ""
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: rocmProc.out = String(text || "")
    }
    onExited: function(exitCode) {
      if (exitCode !== 0) {
        root.vramLoaded("", "")
        return
      }
      // rocm-smi prefixes warnings (low-power state, permissions) before the
      // JSON, so start at the first brace rather than parsing the whole stream.
      var raw = rocmProc.out
      var start = raw.indexOf("{")
      if (start === -1) {
        root.vramLoaded("", "")
        return
      }
      var parsed = null
      try {
        parsed = JSON.parse(raw.substring(start))
      } catch (e) {
        root.vramLoaded("", "")
        return
      }
      if (!parsed || typeof parsed !== "object") {
        root.vramLoaded("", "")
        return
      }
      // An APU reports its iGPU alongside the discrete card. The card with the
      // most memory is the one a model would be loaded onto; summing them would
      // report a number that describes no actual device.
      var bestCard = ""
      var bestTotal = -1
      var bestUsed = -1
      for (var card in parsed) {
        var entry = parsed[card]
        if (!entry || typeof entry !== "object") continue
        var total = Number(entry["VRAM Total Memory (B)"])
        var used = Number(entry["VRAM Total Used Memory (B)"])
        if (!isFinite(total) || !isFinite(used)) continue
        if (total <= bestTotal) continue
        bestCard = String(card)
        bestTotal = total
        bestUsed = used
      }
      if (bestCard === "") {
        root.vramLoaded("", "")
        return
      }
      var usedText = root.formatBytes(bestUsed)
      var totalText = root.formatBytes(bestTotal)
      if (usedText === "" || totalText === "") {
        root.vramLoaded("", "")
        return
      }
      root.vramLoaded("VRAM used (" + bestCard + ")", usedText + " / " + totalText)
    }
  }

  // ------------------------------------------------------------- restart

  function restartDaemon() {
    if (restartProc.running) return
    root.restarting = true
    restartProc.err = ""
    restartProc.command = ["systemctl", "--user", "restart", "voxtype"]
    restartProc.running = true
  }

  Process {
    id: restartProc
    property string err: ""
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: restartProc.err = String(text || "").trim()
    }
    onExited: function(exitCode) {
      root.restarting = false
      root.restarted(exitCode === 0, exitCode === 0 ? "" : (restartProc.err || "systemctl --user restart voxtype failed"))
      if (exitCode === 0) Qt.callLater(root.fetchStatus)
    }
  }

  // ------------------------------------------------------- model downloads
  //
  // Downloads run here, not in the row whose button started one. A section
  // switch, a schema refetch, or a dismissal all rebuild those rows, and a
  // download has to outlive every one of them; this component lives as long as
  // the panel item does, which the manifest's keepLoaded makes "as long as the
  // shell". Dismissing the panel mid-download therefore leaves it running.
  //
  // One at a time: two downloads would race on the same model directory, and
  // there is no second slot in the UI worth building for it.

  property string downloadModel: ""
  readonly property bool downloading: downloadModel !== ""

  // Negative until the child reports a percentage. A build whose `setup
  // --download` predates --progress-format json prints human-readable lines
  // that parse to nothing, which is what indeterminate means here: the UI shows
  // motion without a number rather than inventing one.
  property real downloadPct: -1
  readonly property bool downloadIndeterminate: downloadPct < 0

  // Progress is reported per file, not per model, so `pct` runs 0→100 once for
  // each file a model is made of. Naming the file in flight is what stops the
  // bar restarting from looking like it went backwards.
  property string downloadFile: ""

  property string downloadError: ""
  property string downloadStderr: ""
  property bool downloadCancelling: false

  // `--progress-format json` is newer than the flag-less form. Rather than
  // require it, the first attempt asks for it and a build that rejects the flag
  // gets one silent retry without it — whose human-readable output parses to
  // nothing, which is what the indeterminate bar is for. That keeps downloads
  // working on installations older than the flag instead of failing on them.
  property bool downloadJsonMode: true

  // ok=false with an empty message is a cancellation: the user already knows.
  signal downloadFinished(string model, bool ok, string message)

  function startDownload(name) {
    if (root.downloading || downloadProc.running) return
    root.downloadModel = String(name)
    root.downloadPct = -1
    root.downloadFile = ""
    root.downloadError = ""
    root.downloadStderr = ""
    root.downloadCancelling = false
    root.downloadJsonMode = true
    root.launchDownload(String(name))
  }

  function launchDownload(name) {
    var args = [root.bin, "setup", "--download", "--model", String(name)]
    if (root.downloadJsonMode) {
      args.push("--progress-format")
      args.push("json")
    }
    downloadProc.command = args
    downloadProc.running = true
  }

  // Only the specific "this build has never heard of that flag" failure, so a
  // genuine download error is never retried into looking like a success.
  function rejectedProgressFlag(exitCode) {
    return root.downloadJsonMode && exitCode !== 0
      && root.downloadStderr.indexOf("--progress-format") !== -1
  }

  function cancelDownload() {
    if (!downloadProc.running) return
    // Quickshell sends SIGTERM when running goes false, and `running` stays
    // true until the child actually exits — so the cancel is recorded here and
    // read back in onExited, where a non-zero exit would otherwise look like a
    // failure worth reporting.
    root.downloadCancelling = true
    downloadProc.running = false
  }

  function handleDownloadLine(line) {
    var text = String(line || "").trim()
    if (text === "") return
    var parsed = null
    try {
      parsed = JSON.parse(text)
    } catch (e) {
      // Not the NDJSON contract: leave the bar indeterminate.
      return
    }
    if (!parsed || typeof parsed !== "object") return
    var event = String(parsed.event || "")
    if (event === "progress") {
      var pct = Number(parsed.pct)
      if (isFinite(pct)) root.downloadPct = Math.max(0, Math.min(100, pct))
      if (parsed.file !== undefined && parsed.file !== null)
        root.downloadFile = String(parsed.file)
      return
    }
    if (event === "error") {
      root.downloadError = String(parsed.message || "")
      return
    }
    if (event === "done") root.downloadPct = 100
  }

  Process {
    id: downloadProc

    stdout: SplitParser { onRead: function(line) { root.handleDownloadLine(line) } }
    // Kept as the fallback message for a non-zero exit that reported no error
    // event of its own.
    stderr: SplitParser {
      onRead: function(line) {
        var text = String(line || "").trim()
        if (text !== "") root.downloadStderr = text
      }
    }

    onExited: function(exitCode) {
      var model = root.downloadModel
      var cancelled = root.downloadCancelling

      // Retry keeps downloadModel set, so the row stays in its downloading
      // state across the relaunch instead of flickering back to a button.
      if (!cancelled && root.rejectedProgressFlag(exitCode)) {
        root.downloadJsonMode = false
        root.downloadStderr = ""
        root.downloadError = ""
        Qt.callLater(function() { root.launchDownload(model) })
        return
      }

      var message = root.downloadError !== "" ? root.downloadError : root.downloadStderr

      root.downloadModel = ""
      root.downloadPct = -1
      root.downloadFile = ""
      root.downloadError = ""
      root.downloadStderr = ""
      root.downloadCancelling = false

      if (cancelled) {
        root.downloadFinished(model, false, "")
        return
      }
      if (exitCode === 0) {
        root.downloadFinished(model, true, "")
        return
      }
      root.downloadFinished(model, false,
                            message || ("voxtype setup --download exited " + exitCode))
    }
  }

  // --------------------------------------------- things that want a terminal
  //
  // Every one of these has to be launched *after* the panel is dismissed. The
  // panel is a layer-shell surface on the overlay layer holding exclusive
  // keyboard focus, so a terminal spawned while it is up opens behind it and
  // never gets focus: from the user's side, clicking the button did nothing.
  // Panel.qml owns that ordering (see dismissThen).

  function openTui() {
    launchInTerminal(quote(root.bin) + " configure")
  }

  function runInstaller() {
    launchInTerminal("omarchy-voxtype-install")
  }

  // Not a terminal launch: omarchy-launch-config-editor picks the user's editor
  // and gives it its own window. argv, not a shell string, so a config path
  // with a space in it stays one argument.
  function openConfigEditor(path) {
    if (!path || String(path) === "") return
    Quickshell.execDetached(["omarchy-launch-config-editor", String(path)])
  }

  // ------------------------------------------------------------- dev.json

  FileView {
    id: devConfig
    path: root.manifestDir !== "" ? root.manifestDir + "/dev.json" : ""
    blockLoading: true
    printErrors: false
    onLoaded: {
      try {
        var parsed = JSON.parse(text())
        if (parsed && typeof parsed.voxtypeBin === "string" && parsed.voxtypeBin.length > 0)
          root.bin = parsed.voxtypeBin
      } catch (e) {
        root.bin = "voxtype"
      }
    }
    onLoadFailed: root.bin = "voxtype"
  }
}
