import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// One switch standing in for two schema keys.
//
// Keeping VRAM free while voxtype is idle takes both `on_demand_loading` (do
// not hold the model in memory between recordings) and `gpu_isolation`
// (transcribe in a child process that exits, so the driver actually hands the
// memory back). Either one alone leaves VRAM allocated, which is the whole
// thing a user who also games wants back — so they are presented as one
// decision here, with both underlying rows still shown below for anyone who
// wants them separately.
//
// The row owns no state: it is ON exactly when both keys read true, and its
// writes go through the same `config set` path as every other control.
Item {
  id: root

  // The two `voxtype config schema --json` entries this row stands for.
  property var loadingSpec: null
  property var isolationSpec: null

  property bool highlighted: false

  // Whatever the last failed `config set` said about either key.
  property string errorText: ""

  signal setRequested(string key, var value)

  readonly property string loadingKey: loadingSpec ? String(loadingSpec.key) : ""
  readonly property string isolationKey: isolationSpec ? String(isolationSpec.key) : ""

  readonly property bool bothOn: loadingSpec !== null && isolationSpec !== null
    && loadingSpec.value === true && isolationSpec.value === true

  // Held from the click until the schema agrees, so the switch does not snap
  // back in the window where the first write has landed and the second has
  // not. A failed write drops it immediately: a switch showing a value the
  // config never took is worse than a switch that flickers.
  property var pending: undefined

  function reconcile() {
    if (root.pending !== undefined && root.bothOn === root.pending) root.pending = undefined
  }

  onLoadingSpecChanged: root.reconcile()
  onIsolationSpecChanged: root.reconcile()
  onErrorTextChanged: if (root.errorText !== "") root.pending = undefined

  readonly property bool checked: root.pending !== undefined ? root.pending === true : root.bothOn

  function toggle() {
    if (root.loadingKey === "" || root.isolationKey === "") return
    var next = !root.checked
    root.pending = next
    root.setRequested(root.loadingKey, next)
    root.setRequested(root.isolationKey, next)
  }

  implicitHeight: surface.implicitHeight

  CursorSurface {
    id: surface
    anchors.left: parent.left
    anchors.right: parent.right
    hasCursor: root.highlighted
    bordered: true
    padding: Style.spacing.panelPadding
    implicitHeight: body.implicitHeight + contentTopInset + contentBottomInset

    Item {
      id: body
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.leftMargin: surface.contentLeftInset
      anchors.rightMargin: surface.contentRightInset
      anchors.topMargin: surface.contentTopInset
      implicitHeight: Math.max(labels.implicitHeight, toggle.height)

      Column {
        id: labels
        anchors.left: parent.left
        anchors.right: toggle.left
        anchors.rightMargin: Style.spacing.controlGap
        anchors.verticalCenter: parent.verticalCenter
        spacing: Style.spacing.xxs

        Text {
          text: "Release GPU memory when idle"
          color: ShellTheme.Color.foreground
          font.family: Style.font.family
          font.pixelSize: Style.font.body
        }

        Text {
          width: parent.width
          text: "Frees VRAM between dictations; the model reloads on the next one, so the first words take a moment longer."
          color: Qt.darker(ShellTheme.Color.foreground, 1.5)
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }

        Text {
          visible: root.errorText !== ""
          width: parent.width
          text: root.errorText
          color: ShellTheme.Color.urgent
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }

        Text {
          width: parent.width
          text: root.loadingKey + "  +  " + root.isolationKey
          color: Qt.darker(ShellTheme.Color.foreground, 2.0)
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          elide: Text.ElideRight
        }
      }

      ToggleSwitch {
        id: toggle
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        checked: root.checked
        hasCursor: root.highlighted
        onToggled: root.toggle()
      }
    }
  }
}
