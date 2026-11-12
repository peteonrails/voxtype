import QtQuick
import QtQuick.Layouts

// Share ▾: what would leave (the preview), whether the transcript goes with
// it, and where to: the clipboard, a file, an email draft, or the configured
// share command. Each destination the CLI cannot reach is greyed, with why.
// Nothing is sent until one is clicked.
Rectangle {
  id: root

  property var theme
  property var share: null
  signal closeRequested()
  signal openFolderRequested(string path)

  radius: Math.min(root.theme.radius, 8)
  color: Qt.rgba(root.theme.bg.r, root.theme.bg.g, root.theme.bg.b, 1)
  border.width: 1
  border.color: root.theme.border

  readonly property var preview: root.share ? root.share.preview : null
  readonly property var sectionNames: ({
    overview: "overview", key_points: "key points", decisions: "decisions", action_items: "action items",
    chapters: "chapters", notes: "your notes", highlights: "highlights", transcript: "the transcript"
  })

  function includes() {
    if (!root.preview || !root.preview.sections) return ""
    var out = []
    for (var i = 0; i < root.preview.sections.length; i++) {
      var s = String(root.preview.sections[i])
      out.push(root.sectionNames[s] || s.replace(/_/g, " "))
    }
    return out.length > 0 ? "Includes " + out.join(", ") + "." : "Nothing to share yet: no summary, notes or highlights."
  }

  // Why a destination is greyed: the CLI's reason, or what it needs.
  function whyNot(to) {
    var said = root.share ? root.share.why(to) : ""
    if (said !== "") return said
    if (to === "clipboard") return "Copying needs wl-copy (wl-clipboard)."
    if (to === "email") return "Email needs xdg-open to open a draft."
    if (to === "file") return "Saving is not available."
    return ""
  }

  readonly property string unavailableText: {
    if (!root.preview) return ""
    var out = []
    var all = ["clipboard", "file", "email"]
    for (var i = 0; i < all.length; i++)
      if (!root.share.available(all[i])) out.push(root.whyNot(all[i]))
    return out.join(" ")
  }

  readonly property string resultText: {
    if (!root.share || !root.share.result) return ""
    var r = root.share.result
    var to = String(r.to || root.share.sentTo)
    if (to === "clipboard") return "Copied to the clipboard."
    if (to === "file") return "Saved to " + String(r.path || "") + "."
    if (to === "email") return "Email draft opened." + (r.path ? " Attach the transcript: " + String(r.path) : "")
    if (to === "command") return "Sent." + (r.command_status ? " " + String(r.command_status).trim() : "")
    return "Shared."
  }

  function dirOf(path) {
    var p = String(path || "")
    var at = p.lastIndexOf("/")
    return at > 0 ? p.slice(0, at) : p
  }

  MouseArea { anchors.fill: parent; onClicked: {} }
  Keys.onEscapePressed: root.closeRequested()

  ColumnLayout {
    anchors.fill: parent
    anchors.margins: root.theme.padX
    spacing: root.theme.md

    RowLayout {
      Layout.fillWidth: true
      spacing: root.theme.gap
      Text {
        objectName: "shareTitle"
        Layout.fillWidth: true
        text: root.preview && root.preview.subject ? "Share “" + String(root.preview.subject) + "”" : "Share"
        elide: Text.ElideRight
        color: root.theme.fg
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.body
        font.bold: true
      }
      MeetingButton {
        objectName: "shareClose"
        theme: root.theme
        text: "✕"
        onClicked: root.closeRequested()
      }
    }

    MeetingButton {
      objectName: "shareTranscript"
      theme: root.theme
      bordered: true
      active: root.share !== null && root.share.transcript
      text: (root.share && root.share.transcript ? "☑ " : "☐ ") + "Include the transcript"
        + (root.preview && root.share.transcript && root.preview.transcript_lines !== undefined
           ? " (" + root.preview.transcript_lines + " lines)" : "")
      onClicked: root.share.setTranscript(!root.share.transcript)
    }

    Text {
      objectName: "shareIncludes"
      Layout.fillWidth: true
      visible: root.preview !== null
      text: root.includes()
      color: root.theme.dim
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.bodySmall
      wrapMode: Text.WordWrap
    }

    // Exactly what would leave.
    Rectangle {
      Layout.fillWidth: true
      Layout.fillHeight: true
      Layout.minimumHeight: 80
      radius: Math.min(root.theme.radius, 6)
      color: root.theme.alpha(root.theme.fg, 0.05)
      border.width: 1
      border.color: root.theme.alpha(root.theme.fg, 0.12)

      Flickable {
        id: previewFlick
        anchors.fill: parent
        anchors.margins: root.theme.md
        clip: true
        contentWidth: width
        contentHeight: previewText.implicitHeight
        boundsBehavior: Flickable.StopAtBounds
        Text {
          id: previewText
          objectName: "sharePreview"
          width: previewFlick.width
          textFormat: Text.PlainText
          text: root.share && root.share.previewError !== "" ? root.share.previewError
            : root.preview ? String(root.preview.body || "")
            : "Preparing the preview…"
          color: root.share && root.share.previewError !== "" ? root.theme.urgent : root.theme.fg
          font.family: root.theme.fontFamily
          font.pixelSize: root.theme.caption
          wrapMode: Text.WrapAnywhere
        }
      }
    }

    Text {
      objectName: "shareSize"
      Layout.fillWidth: true
      visible: root.preview !== null && root.preview.chars !== undefined
      text: root.preview ? Number(root.preview.chars).toLocaleString(Qt.locale("en_US"), "f", 0) + " characters" : ""
      color: root.theme.dim
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.caption
    }

    Flow {
      Layout.fillWidth: true
      spacing: root.theme.gap
      MeetingButton {
        objectName: "shareCopy"
        theme: root.theme
        bordered: true
        text: root.share && root.share.sending && root.share.sentTo === "clipboard" ? "Copying…" : "Copy"
        enabled: root.share !== null && root.share.available("clipboard") && !root.share.sending
        onClicked: root.share.send("clipboard")
      }
      MeetingButton {
        objectName: "shareSave"
        theme: root.theme
        bordered: true
        text: "Save…"
        enabled: root.share !== null && root.share.available("file") && !root.share.sending
        onClicked: root.share.send("file")
      }
      MeetingButton {
        objectName: "shareEmail"
        theme: root.theme
        bordered: true
        text: "Email…"
        enabled: root.share !== null && root.share.available("email") && !root.share.sending
        onClicked: root.share.send("email")
      }
      MeetingButton {
        objectName: "shareCommand"
        theme: root.theme
        bordered: true
        visible: root.share !== null && root.share.available("command")
        text: root.preview && root.preview.command ? "Send with " + String(root.preview.command).split(" ")[0] : "Send"
        enabled: root.share !== null && !root.share.sending
        onClicked: root.share.send("command")
      }
    }

    Text {
      objectName: "shareUnavailable"
      Layout.fillWidth: true
      visible: text !== ""
      text: root.unavailableText
      color: root.theme.dim
      font.family: root.theme.fontFamily
      font.pixelSize: root.theme.caption
      wrapMode: Text.WordWrap
    }

    RowLayout {
      Layout.fillWidth: true
      visible: shareStatus.text !== ""
      spacing: root.theme.gap
      Text {
        id: shareStatus
        objectName: "shareStatus"
        Layout.fillWidth: true
        text: root.share && root.share.sendError !== "" ? root.share.sendError : root.resultText
        color: root.share && root.share.sendError !== "" ? root.theme.urgent : root.theme.fg
        font.family: root.theme.fontFamily
        font.pixelSize: root.theme.bodySmall
        // Words where they fit, a long path wherever it must break.
        wrapMode: Text.Wrap
      }
      MeetingButton {
        objectName: "shareOpenFolder"
        theme: root.theme
        visible: root.share !== null && root.share.result !== null && root.share.result.path !== undefined && root.share.result.path !== null
        text: "Open folder"
        onClicked: root.openFolderRequested(root.dirOf(root.share.result.path))
      }
    }
  }
}
