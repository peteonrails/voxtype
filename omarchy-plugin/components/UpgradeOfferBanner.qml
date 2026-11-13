import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// Model upgrade offers, at the top of the Engine section.
//
// Everything here is read from `voxtype setup migrate --json` (the same object
// `voxtype status --format json --extended` carries as `model_upgrade`), so the
// panel never decides on its own which model is better. The banner shows each
// offer with its size and an Update button, a "Keep current model" choice, the
// 2.0 removal notice once the user has kept their model, and the offer to
// delete old model files after a switch.
//
// The banner stays up while offers exist, declined or not: a declined offer can
// still be accepted later, which is the point of keeping it visible.
Item {
  id: root

  // The contract object, or null when nothing applies or the CLI has no
  // `setup migrate`.
  property var status: null

  // The offer whose `--accept` is running, if any. Progress fields are owned by
  // VoxtypeCli and shared with the model rows' Download flow.
  property string busyOfferId: ""
  property real downloadPct: -1
  property string downloadFile: ""
  property bool downloadCancelling: false
  // Any download at all, including a plain model Download from the list below.
  // One at a time, as everywhere else in the panel.
  property bool downloadBusy: false

  // offer id → message from the accept that failed on it.
  property var offerErrors: ({})

  property bool declining: false
  property bool cleaningUp: false
  // Set the moment Keep current model succeeds, so the notice appears without
  // waiting for the re-read.
  property bool declinedHere: false
  property string actionError: ""

  signal acceptRequested(string offerId)
  signal cancelRequested()
  signal declineRequested()
  signal cleanupRequested()

  readonly property color dim: Qt.darker(ShellTheme.Color.foreground, 1.5)

  // Plain copies, recommended first. Walks `length` rather than trusting
  // Array.isArray: the list crosses a `var` property boundary on its way here,
  // and a wrapped sequence would otherwise read as no offers at all.
  readonly property var offers: {
    var raw = root.status ? root.status.offers : null
    if (!raw || raw.length === undefined) return []
    var recommended = []
    var rest = []
    for (var i = 0; i < raw.length; i++) {
      var entry = raw[i]
      if (!entry || entry.id === undefined || entry.id === null) continue
      var copy = {
        id: String(entry.id),
        kind: String(entry.kind || ""),
        model: String(entry.model || ""),
        title: String(entry.title || entry.model || entry.id),
        detail: String(entry.detail || ""),
        bytes: Number(entry.download_bytes),
        recommended: entry.recommended === true,
        declined: entry.declined === true
      }
      if (copy.recommended) recommended.push(copy)
      else rest.push(copy)
    }
    return recommended.concat(rest)
  }

  readonly property var cleanup: {
    var c = root.status ? root.status.cleanup : null
    if (!c || typeof c !== "object") return null
    return { model: String(c.model || ""), bytes: Number(c.bytes) }
  }

  readonly property var removalNotice: {
    var n = root.status ? root.status.removal_notice : null
    if (!n || typeof n !== "object") return null
    return { message: String(n.message || ""), pinCommand: String(n.pin_command || "") }
  }

  readonly property bool anyDeclined: {
    for (var i = 0; i < root.offers.length; i++) if (root.offers[i].declined) return true
    return false
  }
  readonly property bool allDeclined: {
    if (root.offers.length === 0) return false
    for (var i = 0; i < root.offers.length; i++) if (!root.offers[i].declined) return false
    return true
  }

  readonly property bool hasOffers: offers.length > 0
  readonly property bool active: hasOffers || cleanup !== null
  readonly property bool showKeep: hasOffers && !allDeclined && !declinedHere
  readonly property bool showNotice: hasOffers && removalNotice !== null
    && removalNotice.message !== "" && (declinedHere || anyDeclined)
  readonly property bool accepting: busyOfferId !== ""
  // Nothing else may start while an accept, a decline or a cleanup is running,
  // or while a plain model download holds the one download slot.
  readonly property bool locked: downloadBusy || declining || cleaningUp

  function formatBytes(bytes) {
    var n = Number(bytes)
    if (!isFinite(n) || n <= 0) return ""
    if (n >= 1e9) return (n / 1e9).toFixed(1) + " GB"
    if (n >= 1e6) return Math.round(n / 1e6) + " MB"
    return Math.max(1, Math.round(n / 1e3)) + " kB"
  }

  visible: active
  implicitHeight: active ? card.implicitHeight : 0
  height: implicitHeight

  BorderSurface {
    id: card
    anchors.left: parent.left
    anchors.right: parent.right
    implicitHeight: body.implicitHeight + contentTopInset + contentBottomInset
    height: implicitHeight
    color: Util.alpha(ShellTheme.Color.accent, 0.10)
    radius: Style.cornerRadius
    borderSpec: Border.flat(Util.alpha(ShellTheme.Color.accent, 0.45), Math.max(1, Style.normalBorderWidth))
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
        visible: root.hasOffers
        text: "MODEL UPDATE AVAILABLE"
      }

      Repeater {
        model: root.offers

        Column {
          id: offer
          required property var modelData
          required property int index

          readonly property string offerId: String(modelData.id)
          readonly property bool running: root.busyOfferId === offer.offerId
          readonly property string errorText: String(root.offerErrors[offer.offerId] || "")
          readonly property string sizeText: root.formatBytes(modelData.bytes)

          objectName: "upgradeOffer:" + offer.offerId
          width: parent.width
          spacing: Style.spacing.xxs

          Item {
            width: parent.width
            implicitHeight: Math.max(offerText.implicitHeight, actions.implicitHeight)
            height: implicitHeight

            Column {
              id: offerText
              anchors.left: parent.left
              anchors.right: actions.left
              anchors.rightMargin: Style.spacing.controlGap
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.spacing.xxs

              Text {
                objectName: "upgradeOfferTitle"
                width: parent.width
                text: offer.modelData.title
                color: ShellTheme.Color.foreground
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                font.bold: true
                wrapMode: Text.WordWrap
              }

              Text {
                visible: text !== ""
                width: parent.width
                text: offer.modelData.detail
                color: root.dim
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                wrapMode: Text.WordWrap
              }

              Text {
                visible: text !== ""
                width: parent.width
                text: offer.sizeText === "" ? offer.modelData.model
                  : offer.modelData.model + "  ·  " + offer.sizeText + " download"
                color: root.dim
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
                elide: Text.ElideMiddle
              }
            }

            Row {
              id: actions
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.spacing.controlGap

              Button {
                objectName: "upgradeAccept:" + offer.offerId
                anchors.verticalCenter: parent.verticalCenter
                visible: !offer.running
                enabled: !root.locked
                opacity: root.locked ? 0.45 : 1.0
                // A failed accept left everything as it was, so the same button
                // tries again.
                text: offer.errorText !== "" ? "Retry" : "Update"
                iconText: "󰇚"
                tooltipText: "Download " + offer.modelData.model
                  + (offer.sizeText !== "" ? " (" + offer.sizeText + ")" : "")
                  + " and switch to it"
                bordered: true
                onClicked: root.acceptRequested(offer.offerId)
              }

              DownloadProgress {
                anchors.verticalCenter: parent.verticalCenter
                visible: offer.running
                pct: root.downloadPct
                file: root.downloadFile
                cancelling: root.downloadCancelling
                onCancelRequested: root.cancelRequested()
              }
            }
          }

          Text {
            objectName: "upgradeOfferError"
            visible: offer.errorText !== ""
            width: parent.width
            text: offer.errorText
            color: ShellTheme.Color.urgent
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            wrapMode: Text.WordWrap
          }
        }
      }

      Row {
        visible: root.showKeep
        width: parent.width
        spacing: Style.spacing.sm

        Button {
          id: keepButton
          objectName: "upgradeKeep"
          enabled: !root.locked
          opacity: root.locked ? 0.45 : 1.0
          text: root.declining ? "Keeping current model…" : "Keep current model"
          tooltipText: "Stay on the current model; the offer stays available here"
          bordered: true
          onClicked: root.declineRequested()
        }

        // Only an ONNX model carries a removal notice; keeping a GGUF model
        // has no 2.0 consequence, so no hint then.
        Text {
          objectName: "upgradeKeepHint"
          visible: root.removalNotice !== null
          width: parent.width - keepButton.width - parent.spacing
          anchors.verticalCenter: keepButton.verticalCenter
          text: "Keeping this model means you can't upgrade to voxtype 2.0."
          color: root.dim
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }
      }

      // Selectable rather than copied: the panel never writes the clipboard on
      // the user's behalf, so the command is there to select and copy by hand.
      Column {
        objectName: "upgradeRemovalNotice"
        visible: root.showNotice
        width: parent.width
        spacing: Style.spacing.xxs

        TextEdit {
          objectName: "upgradeRemovalMessage"
          width: parent.width
          readOnly: true
          selectByMouse: true
          text: root.removalNotice ? root.removalNotice.message : ""
          color: ShellTheme.Color.foreground
          selectionColor: Util.alpha(ShellTheme.Color.accent, 0.45)
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: TextEdit.Wrap
        }

        TextEdit {
          objectName: "upgradePinCommand"
          visible: text !== ""
          width: parent.width
          readOnly: true
          selectByMouse: true
          text: root.removalNotice ? root.removalNotice.pinCommand : ""
          color: ShellTheme.Color.accent
          selectionColor: Util.alpha(ShellTheme.Color.accent, 0.45)
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: TextEdit.Wrap
          leftPadding: Style.spacing.controlPaddingX
        }
      }

      Item {
        objectName: "upgradeCleanup"
        visible: root.cleanup !== null
        width: parent.width
        implicitHeight: visible ? Math.max(cleanupText.implicitHeight, cleanupButton.implicitHeight) : 0
        height: implicitHeight

        Text {
          id: cleanupText
          objectName: "upgradeCleanupText"
          anchors.left: parent.left
          anchors.right: cleanupButton.left
          anchors.rightMargin: Style.spacing.controlGap
          anchors.verticalCenter: parent.verticalCenter
          text: {
            if (!root.cleanup) return ""
            var size = root.formatBytes(root.cleanup.bytes)
            return "Old model files for " + root.cleanup.model + " are still on disk"
              + (size !== "" ? " (" + size + ")." : ".")
          }
          color: ShellTheme.Color.foreground
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.WordWrap
        }

        Button {
          id: cleanupButton
          objectName: "upgradeCleanupButton"
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          enabled: !root.locked
          opacity: root.locked ? 0.45 : 1.0
          text: root.cleaningUp ? "Removing…" : "Remove old model files"
          iconText: "󰆴"
          tooltipText: root.cleanup ? "Delete the files of " + root.cleanup.model : ""
          bordered: true
          onClicked: root.cleanupRequested()
        }
      }

      Text {
        objectName: "upgradeActionError"
        visible: root.actionError !== ""
        width: parent.width
        text: root.actionError
        color: ShellTheme.Color.urgent
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        wrapMode: Text.WordWrap
      }
    }
  }
}
