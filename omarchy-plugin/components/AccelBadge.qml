import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// Whether transcription is actually running on the GPU, from `voxtype info accel
// --json`.
//
// The state that matters is `cpu-fallback`: the user configured acceleration,
// believes they have it, and does not. That one is deliberately loud. `gpu` is
// tinted as confirmation, `cpu-only` is quiet because it is not a problem — a
// CPU build running on the CPU is working as intended.
//
// `unknown` and `not-running` render nothing at all. This badge answers "is it
// accelerated", and a badge that appears when the answer is not known would be
// answering a question nobody asked.
Item {
  id: root

  // Not `state`: Item already has one, for QML States.
  property string accelState: ""
  property string backend: ""

  // The lines `info accel` offers as its reasoning, newline-joined, shown on
  // hover. A string rather than an array: see VoxtypeCli.accelLoaded.
  property string evidence: ""

  readonly property bool isGpu: accelState === "gpu"
  readonly property bool isFallback: accelState === "cpu-fallback"
  readonly property bool isCpuOnly: accelState === "cpu-only"
  readonly property bool known: isGpu || isFallback || isCpuOnly

  // A chip for the two states worth drawing attention to; plain muted text for
  // the ordinary one.
  readonly property bool chipped: isGpu || isFallback

  readonly property color tint: isFallback ? ShellTheme.Color.urgent : ShellTheme.Color.accent

  // The contract's backend names are lowercase; these are how they are written.
  // Anything unrecognised is shown as reported rather than dropped.
  function backendLabel(name) {
    var raw = String(name || "")
    if (raw === "") return ""
    if (raw === "vulkan") return "Vulkan"
    if (raw === "cuda") return "CUDA"
    if (raw === "migraphx") return "MIGraphX"
    if (raw === "metal") return "Metal"
    return raw
  }

  readonly property string label: {
    if (root.isGpu) {
      var backendText = root.backendLabel(root.backend)
      return backendText === "" ? "GPU" : "GPU · " + backendText
    }
    if (root.isFallback) return "CPU fallback"
    if (root.isCpuOnly) return "CPU"
    return ""
  }

  readonly property string evidenceText: String(root.evidence || "").trim()

  visible: known

  // Sized from the label, so a caller is free to override the height — a header
  // that sets Layout.preferredHeight gets a chip that matches the buttons beside
  // it instead of one that shrinks to its own text. The chip fills whatever
  // height it ends up with; the label stays centred in it either way.
  //
  // These read the label rather than the chip on purpose: the chip fills the
  // root, so deriving the root's size from the chip would be circular.
  implicitWidth: visible
    ? (chipped ? text.implicitWidth + Style.spacing.controlPaddingX * 2 : text.implicitWidth)
    : 0
  implicitHeight: visible ? text.implicitHeight + Style.spacing.xs * 2 : 0

  BorderSurface {
    id: chip
    anchors.fill: parent
    visible: root.chipped
    color: Util.alpha(root.tint, 0.14)
    // Same token the header buttons use, so the row reads as one line of chips.
    radius: Style.cornerRadius
    borderSpec: Border.flat(Util.alpha(root.tint, 0.5), 1)
  }

  Text {
    id: text
    anchors.centerIn: parent
    text: root.label
    color: root.isFallback
      ? ShellTheme.Color.urgent
      : (root.isGpu ? ShellTheme.Color.foreground : Qt.darker(ShellTheme.Color.foreground, 1.5))
    font.family: Style.font.family
    font.pixelSize: Style.font.caption
    font.bold: root.isFallback
  }

  MouseArea {
    id: hover
    anchors.fill: parent
    hoverEnabled: root.evidenceText !== ""
    acceptedButtons: Qt.NoButton
  }

  // Drawn as a plain item rather than a ToolTip on purpose: a Qt Quick Controls
  // ToolTip is a Popup and needs a window overlay to attach to, which is not
  // something a layer-shell surface provides. This panel already routes dropdown
  // popups through an explicit host item for the same reason.
  //
  // Two things this has to fight. The background is forced fully opaque because
  // the theme's tooltip colour may carry alpha, and a translucent panel over a
  // line of text leaves both unreadable. And the whole header needs a `z` above
  // its siblings (set where HeaderBar is used) — a later sibling in the column
  // paints over an earlier one's children no matter what `z` they carry inside.
  Rectangle {
    id: evidencePanel
    visible: hover.containsMouse && root.evidenceText !== ""
    z: 100
    anchors.top: parent.bottom
    anchors.topMargin: Style.spacing.xs
    anchors.left: parent.left
    width: evidenceLabel.implicitWidth + Style.spacing.controlPaddingX * 2
    height: evidenceLabel.implicitHeight + Style.spacing.controlPaddingY * 2
    color: Qt.rgba(ShellTheme.Color.tooltip.background.r, ShellTheme.Color.tooltip.background.g,
                   ShellTheme.Color.tooltip.background.b, 1.0)
    border.color: ShellTheme.Color.tooltip.border
    border.width: Math.max(1, Style.normalBorderWidth)
    radius: Style.cornerRadius

    Text {
      id: evidenceLabel
      anchors.centerIn: parent
      text: root.evidenceText
      color: ShellTheme.Color.tooltip.text
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
    }
  }
}
