import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// The model catalog for the engine being browsed: which models this engine
// knows about, which are on disk, and which one is loaded. It needs more than a
// plain form row because a model can be known but not downloaded yet, so it
// gets its own card at the top of the form. An engine with no catalog at all (a
// cloud engine such as Deepgram) has no model to pick, so the card offers the
// engine itself instead.
//
// The engine switch itself lives in EnginePicker, pinned above the scroller,
// because it has to stay reachable while this list is being scrolled.
Item {
  id: root

  property string engine: ""
  // The engine the config has in effect; `engine` is the one being browsed.
  property string activeEngine: ""

  // The `engines` map out of `voxtype info models --json`: per-engine model
  // catalogs with an installed flag on each entry.
  property var engines: ({})

  property string modelKey: ""
  property string modelValue: ""
  property string language: ""

  // Download state, owned by VoxtypeCli and passed down. Only one download runs
  // at a time, so every other row's button goes flat while one is in flight
  // rather than queueing work the CLI would serialise anyway.
  property string downloadingModel: ""
  property real downloadPct: -1
  property string downloadFile: ""
  property bool downloadCancelling: false
  // A download this card does not show (a model upgrade's, reported in the
  // banner above) still holds the one slot.
  property bool otherDownloadBusy: false
  readonly property bool downloadBusy: downloadingModel !== "" || otherDownloadBusy

  // model name → message from the download that failed. Cleared when a new one
  // starts on that model.
  property var downloadErrors: ({})

  // Only an https link is offered; anything else in the catalog is ignored
  // rather than handed to the desktop's URL opener.
  function licenseLink(url) {
    var text = url ? String(url).trim() : ""
    return /^https:\/\/\S+$/.test(text) ? text : ""
  }

  // Indirected so a test can observe the click without opening a browser.
  property var openUrl: function(url) { Qt.openUrlExternally(url) }

  signal modelChangeRequested(string name, string language)
  signal downloadRequested(string name)
  signal cancelDownloadRequested()
  // Use on an engine with no model catalog: `config set engine <name>`, since
  // `config select-model` needs a model and refuses such an engine.
  signal engineUseRequested(string name)

  readonly property color dim: Qt.darker(ShellTheme.Color.foreground, 1.5)
  readonly property bool haveModelInfo: {
    for (var k in root.engines) return true
    return false
  }

  // `voxtype info models --json` answered, and lists no catalog for this
  // engine: it has no model to download (today only Deepgram, a cloud engine).
  // An engine listed with an empty catalog is a different case and keeps its
  // "no downloadable models" line.
  readonly property bool catalogless: root.haveModelInfo && root.engine !== ""
    && root.engines[root.engine] === undefined
  readonly property string engineTitle: root.engine === "" ? ""
    : root.engine.charAt(0).toUpperCase() + root.engine.slice(1)
  readonly property string cataloglessNote: root.engine === "deepgram"
    ? "Deepgram runs in the cloud and has no model to download. It reads its API key from DEEPGRAM_API_KEY in the daemon's environment, or [deepgram] api_key; without one the daemon will not start."
    : root.engineTitle + " has no model to download."

  // Walks `length` rather than asking Array.isArray: this list is rebuilt by the
  // panel when a download finishes, and an array that has crossed a `var`
  // property boundary does not reliably answer that question. Getting it wrong
  // here would empty the whole model list.
  function selectionLanguage(languages) {
    if (!languages || languages.length === undefined || languages.length === 0) return ""
    for (var i = 0; i < languages.length; i++)
      if (String(languages[i]) === root.language) return root.language
    return String(languages[0])
  }

  readonly property var modelRows: {
    var info = root.engines[root.engine]
    if (!info || !info.models || info.models.length === undefined) return []
    var out = []
    for (var i = 0; i < info.models.length; i++) {
      var entry = info.models[i]
      if (!entry) continue
      // Legacy ONNX checkpoints of an engine that has GGUF models are listed
      // only while one is configured: moving to GGUF is one way.
      if (entry.legacy === true && String(entry.name) !== root.modelValue) continue
      out.push({
        name: String(entry.name),
        language: root.selectionLanguage(entry.languages),
        installed: entry.installed === true,
        // Optional: older builds omit both, and the row then shows no link.
        license: entry.license ? String(entry.license) : "",
        licenseUrl: root.licenseLink(entry.license_url)
      })
    }
    return out
  }

  implicitHeight: card.implicitHeight

  BorderSurface {
    id: card
    anchors.left: parent.left
    anchors.right: parent.right
    implicitHeight: body.implicitHeight + contentTopInset + contentBottomInset
    height: implicitHeight
    color: Util.alpha(ShellTheme.Color.foreground, 0.04)
    radius: Style.cornerRadius
    borderSpec: Border.flat(Util.alpha(ShellTheme.Color.foreground, 0.10), 1)
    padding: Style.spacing.rowPaddingX

    Column {
      id: body
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.topMargin: card.contentTopInset
      anchors.leftMargin: card.contentLeftInset
      anchors.rightMargin: card.contentRightInset
      spacing: Style.spacing.lg

      PanelSectionHeader {
        objectName: "catalogHeader"
        // An engine with no model list is headed by its name alone.
        text: root.engine === "" ? "MODELS"
          : root.catalogless ? root.engine.toUpperCase()
          : ("MODELS FOR " + root.engine.toUpperCase())
      }

      Text {
        objectName: "noModelsText"
        visible: root.modelRows.length === 0 && !root.catalogless
        width: parent.width
        text: root.haveModelInfo
          ? "This engine reports no downloadable models."
          : "Model list unavailable — voxtype info models --json returned nothing."
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }

      // An engine with no model to download is chosen by name alone, in the
      // same row shape a model gets.
      Column {
        visible: root.catalogless
        width: parent.width
        spacing: Style.spacing.xxs

        CursorSurface {
          id: engineRow

          readonly property bool inUse: root.engine === root.activeEngine

          width: parent.width
          implicitHeight: Math.max(Style.space(30),
                                   engineName.implicitHeight + Style.spacing.controlGap)
          height: implicitHeight
          current: engineRow.inUse
          foreground: ShellTheme.Color.foreground

          Text {
            id: engineName
            anchors.left: parent.left
            anchors.leftMargin: Style.spacing.controlPaddingX
            anchors.verticalCenter: parent.verticalCenter
            text: root.engineTitle
            color: ShellTheme.Color.foreground
            font.family: Style.font.family
            font.pixelSize: Style.font.body
            font.bold: engineRow.inUse
          }

          Row {
            anchors.right: parent.right
            anchors.rightMargin: Style.spacing.controlPaddingX
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.spacing.controlGap

            Text {
              objectName: "engineInUse:" + root.engine
              anchors.verticalCenter: parent.verticalCenter
              visible: engineRow.inUse
              text: "configured"
              color: ShellTheme.Color.accent
              font.family: Style.font.family
              font.pixelSize: Style.font.caption
              font.bold: true
            }

            // The dropdown never browses an engine `info engines` reports
            // missing. Without that list, `config set engine` refuses a
            // missing engine with an error shown above the card.
            Button {
              objectName: "useEngine:" + root.engine
              anchors.verticalCenter: parent.verticalCenter
              visible: !engineRow.inUse
              text: "Use " + root.engineTitle
              tooltipText: "Switch the engine to " + root.engineTitle
              bordered: true
              onClicked: root.engineUseRequested(root.engine)
            }
          }
        }

        Text {
          objectName: "cataloglessNote"
          width: parent.width
          leftPadding: Style.spacing.controlPaddingX
          text: root.cataloglessNote
          color: root.dim
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }
      }

      Column {
        width: parent.width
        spacing: Style.spacing.xxs

        Repeater {
          model: root.modelRows

          Column {
            id: entry
            required property var modelData

            readonly property string modelName: String(modelData.name)
            readonly property bool downloading: root.downloadingModel === entry.modelName
            readonly property string errorText: String(root.downloadErrors[root.engine + ":" + entry.modelName] || "")
            readonly property string selectionLanguage: String(modelData.language || "")
            readonly property bool changesLanguage: selectionLanguage !== "" && selectionLanguage !== root.language

            width: parent.width
            spacing: Style.spacing.xxs

            CursorSurface {
              id: modelRow

              readonly property bool inUse: entry.modelName === root.modelValue

              width: parent.width
              implicitHeight: Math.max(Style.space(30),
                                       nameText.implicitHeight + Style.spacing.controlGap)
              height: implicitHeight
              current: modelRow.inUse
              foreground: ShellTheme.Color.foreground

              Text {
                id: nameText
                anchors.left: parent.left
                anchors.leftMargin: Style.spacing.controlPaddingX
                anchors.verticalCenter: parent.verticalCenter
                text: entry.modelName
                color: ShellTheme.Color.foreground
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                font.bold: modelRow.inUse
              }

              Row {
                anchors.right: parent.right
                anchors.rightMargin: Style.spacing.controlPaddingX
                anchors.verticalCenter: parent.verticalCenter
                spacing: Style.spacing.controlGap

                Text {
                  anchors.verticalCenter: parent.verticalCenter
                  visible: !entry.downloading
                  text: entry.modelData.installed ? "installed" : "not downloaded"
                  color: root.dim
                  font.family: Style.font.family
                  font.pixelSize: Style.font.caption
                }

                Text {
                  anchors.verticalCenter: parent.verticalCenter
                  visible: modelRow.inUse && !entry.downloading
                  text: "configured"
                  color: ShellTheme.Color.accent
                  font.family: Style.font.family
                  font.pixelSize: Style.font.caption
                  font.bold: true
                }

                Button {
                  objectName: "useModel:" + entry.modelName
                  anchors.verticalCenter: parent.verticalCenter
                  visible: entry.modelData.installed && !modelRow.inUse
                    && root.modelKey !== "" && !entry.downloading
                  text: "Use"
                  // Names in this list can differ by one suffix, and the button
                  // sits a long way to the right of the name it belongs to, so
                  // it says which model it means.
                  tooltipText: "Use " + entry.modelName + (entry.changesLanguage ? " with language " + entry.selectionLanguage : "")
                  bordered: true
                  onClicked: root.modelChangeRequested(entry.modelName, entry.changesLanguage ? entry.selectionLanguage : "")
                }

                Button {
                  anchors.verticalCenter: parent.verticalCenter
                  visible: !entry.modelData.installed && !entry.downloading
                  // One download at a time, so the rest go flat rather than
                  // queueing work behind the one that is running.
                  enabled: !root.downloadBusy
                  opacity: root.downloadBusy ? 0.45 : 1.0
                  text: "Download"
                  iconText: "󰇚"
                  tooltipText: "Download " + entry.modelName
                  bordered: true
                  onClicked: root.downloadRequested(entry.modelName)
                }

                DownloadProgress {
                  anchors.verticalCenter: parent.verticalCenter
                  visible: entry.downloading
                  pct: root.downloadPct
                  file: root.downloadFile
                  cancelling: root.downloadCancelling
                  onCancelRequested: root.cancelDownloadRequested()
                }
              }
            }

            // Informational only: no acceptance step, nothing gated on it.
            Text {
              objectName: "modelLicense:" + entry.modelName
              visible: String(entry.modelData.licenseUrl || "") !== ""
              leftPadding: Style.spacing.controlPaddingX
              text: "License: " + (String(entry.modelData.license || "") !== ""
                                   ? entry.modelData.license : "terms")
              color: licenseMouse.containsMouse ? ShellTheme.Color.accent : root.dim
              font.family: Style.font.family
              font.pixelSize: Style.font.caption
              font.underline: true

              MouseArea {
                id: licenseMouse
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: root.openUrl(String(entry.modelData.licenseUrl))
              }
            }

            Text {
              visible: entry.changesLanguage
              width: parent.width
              text: "Use selects language " + entry.selectionLanguage + "; the saved hint is not supported by this checkpoint."
              color: root.dim
              font.family: Style.font.family
              font.pixelSize: Style.font.caption
              wrapMode: Text.WordWrap
            }

            Text {
              visible: entry.errorText !== ""
              width: parent.width
              leftPadding: Style.spacing.controlPaddingX
              text: entry.errorText
              color: ShellTheme.Color.urgent
              font.family: Style.font.family
              font.pixelSize: Style.font.caption
              wrapMode: Text.WordWrap
            }
          }
        }
      }
    }
  }
}
