import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import qs.Ui

// One row of the form: label + description on the left, the control for this
// key's type on the right, and a dot marking keys the config file overrides
// (with the action that removes the override again).
//
// Every control writes through immediately — there is no Apply button — so
// each type declares when "immediately" is: switches and dropdowns on change,
// text and number fields when editing finishes, sliders on a 300ms debounce.
//
// The control that matches `spec.type` is the visible one; the rest are
// instantiated but hidden. They are cheap, and picking between them this way
// keeps the row's geometry static instead of relayouting per type.
Item {
  id: root

  // One entry from `voxtype config schema --json`.
  property var spec: null

  // Options for a dynamic_enum, as [{ value, label }] — the panel fills these
  // from `voxtype info models --json` / `info devices --json`.
  property var dynamicOptions: []

  // A plain enum's choices, resolved by the form rather than read out of `spec`:
  // an array inside a spec that has passed through a Repeater's modelData no
  // longer satisfies Array.isArray, which used to leave every enum row with only
  // its current value to pick from.
  property var choices: []

  property string errorText: ""
  property int labelColumnWidth: Style.space(280)

  // The form's keyboard cursor is on this row: ring the control so it is clear
  // what Enter will hit.
  property bool highlighted: false

  signal setRequested(string key, var value)
  signal unsetRequested(string key)
  signal editorFocusChanged(bool active)

  // Escape pressed while one of this row's fields held the keyboard. The form
  // takes focus back rather than letting it bubble up to the card, where it
  // would close the whole panel mid-edit.
  signal editorEscaped()

  readonly property string specKey: spec ? String(spec.key) : ""
  readonly property string specType: spec ? String(spec.type) : "string"
  readonly property string specLabel: specKey === "text.lard_disfluency"
    ? "LARD disfluency cleanup"
    : ((spec && spec.label) ? String(spec.label) : specKey)
  readonly property string specDescription: (spec && spec.description) ? String(spec.description) : ""
  readonly property bool compiled: !spec || spec.compiled !== false
  readonly property bool modified: spec !== null && spec.file_value !== null && spec.file_value !== undefined
  readonly property bool bounded: spec !== null && spec.min !== undefined && spec.min !== null
    && spec.max !== undefined && spec.max !== null
  readonly property real specMin: bounded ? Number(spec.min) : 0
  readonly property real specMax: bounded ? Number(spec.max) : 1

  // An `open` enum lists useful presets but accepts any string (an evdev key
  // name, a sound theme that may be a path). The picker therefore carries a
  // "Custom value…" row that swaps it for a text field.
  readonly property bool openEnum: spec !== null && spec.open === true
  readonly property string customSentinel: "__voxtype_custom__"
  property bool customEditing: false

  // Written value shown optimistically until the next schema fetch replaces
  // `spec`. Without it a switch snaps back for as long as the write takes.
  property var pendingValue: undefined
  onSpecChanged: {
    root.pendingValue = undefined
    root.customEditing = false
  }

  readonly property var currentValue: pendingValue !== undefined
    ? pendingValue
    : (spec ? spec.value : undefined)

  readonly property string currentText: currentValue === undefined || currentValue === null
    ? "" : String(currentValue)

  readonly property string controlKind: {
    if (specType === "bool") return "bool"
    if (specType === "enum" || specType === "dynamic_enum") {
      if (root.customEditing) return "text"
      return optionList.length > 8 ? "searchable" : "dropdown"
    }
    if (specType === "int") return "int"
    if (specType === "float") return bounded ? "slider" : "text"
    return "text"
  }

  // dynamic_enum options come from the CLI, plain enums from the schema. The
  // value in effect is always offered even when the source list omits it, so
  // a model that was uninstalled behind our back still displays.
  readonly property var optionList: {
    var out = []
    var seen = ({})
    var source = specType === "dynamic_enum" ? root.dynamicOptions : root.choices
    if (source === undefined || source === null) source = []
    for (var i = 0; i < source.length; i++) {
      var entry = source[i]
      var value = (entry && typeof entry === "object") ? String(entry.value) : String(entry)
      var label = (entry && typeof entry === "object" && entry.label) ? String(entry.label) : value
      if (seen[value]) continue
      seen[value] = true
      out.push({ value: value, label: label })
    }
    if (root.currentText !== "" && !seen[root.currentText])
      out.unshift({ value: root.currentText, label: root.currentText })
    if (root.openEnum) out.push({ value: root.customSentinel, label: "Custom value…" })
    return out
  }

  // Picking a row makes Dropdown write its own `value`, which drops the
  // binding to the schema. Put it back after every selection.
  function rebindPicker(picker) {
    picker.value = Qt.binding(function() { return root.currentText })
  }

  function handlePick(picker, next) {
    rebindPicker(picker)
    if (next === root.customSentinel) {
      root.customEditing = true
      Qt.callLater(function() { textField.forceActiveFocus() })
      return
    }
    root.apply(next)
  }

  readonly property int controlAreaHeight: Math.max(Style.space(34), Style.spacing.controlHeight + Style.spacing.md)
  readonly property color dim: Qt.darker(ShellTheme.Color.foreground, 1.5)

  function apply(value) {
    root.pendingValue = value
    root.setRequested(root.specKey, value)
  }

  // What Enter means on this row, per control type: flip a switch, drop a
  // picker open, or hand the keyboard to a field. A slider has nothing to
  // activate — it is driven by dragging, and the panel's h/l are the section
  // cursor's, not this row's.
  function activate() {
    if (!root.compiled) return
    if (root.controlKind === "bool") {
      root.apply(!(root.currentValue === true))
      return
    }
    if (root.controlKind === "dropdown") {
      dropdown.open()
      return
    }
    if (root.controlKind === "searchable") {
      searchable.open()
      return
    }
    if (root.controlKind === "int") {
      numberField.field.forceActiveFocus()
      return
    }
    if (root.controlKind === "text") {
      textField.forceActiveFocus()
      textField.selectAll()
    }
  }

  enabled: compiled
  opacity: compiled ? 1.0 : 0.45
  implicitHeight: Math.max(labels.implicitHeight, controlArea.height)

  Column {
    id: labels
    anchors.left: parent.left
    anchors.verticalCenter: parent.verticalCenter
    width: root.labelColumnWidth
    spacing: Style.spacing.xxs

    Row {
      spacing: Style.spacing.md

      Text {
        text: root.specLabel
        color: ShellTheme.Color.foreground
        font.family: Style.font.family
        font.pixelSize: Style.font.body
      }

      Text {
        visible: !root.compiled
        anchors.verticalCenter: parent.verticalCenter
        text: "not in this build"
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
      }
    }

    Text {
      visible: root.specDescription !== ""
      width: parent.width
      text: root.specDescription
      color: root.dim
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
      text: root.specKey
      color: Qt.darker(ShellTheme.Color.foreground, 2.0)
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      elide: Text.ElideRight
      width: parent.width
    }
  }

  // Override marker + the action that clears the override.
  Row {
    id: overrideMarker
    anchors.right: controlArea.left
    anchors.rightMargin: Style.spacing.controlGap
    anchors.verticalCenter: parent.verticalCenter
    spacing: Style.spacing.sm

    Rectangle {
      anchors.verticalCenter: parent.verticalCenter
      visible: root.modified
      width: Style.space(6)
      height: width
      radius: width / 2
      color: ShellTheme.Color.accent
    }

    PanelActionButton {
      anchors.verticalCenter: parent.verticalCenter
      visible: root.modified
      iconText: "󰜉"
      tooltipText: "Reset to default"
      onClicked: {
        root.pendingValue = undefined
        root.unsetRequested(root.specKey)
      }
    }
  }

  Item {
    id: controlArea
    anchors.right: parent.right
    anchors.verticalCenter: parent.verticalCenter
    width: Style.spacing.dropdownWidth
    height: root.controlAreaHeight

    ToggleSwitch {
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      visible: root.controlKind === "bool"
      checked: root.currentValue === true
      hasCursor: root.highlighted
      onToggled: root.apply(!checked)
    }

    Dropdown {
      id: dropdown
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      visible: root.controlKind === "dropdown"
      showLabel: false
      options: root.optionList
      value: root.currentText
      hasCursor: root.highlighted
      onChanged: function(next) { root.handlePick(dropdown, next) }
      onPopupOpenChanged: root.editorFocusChanged(popupOpen)
    }

    SearchableDropdown {
      id: searchable
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      visible: root.controlKind === "searchable"
      showLabel: false
      options: root.optionList
      value: root.currentText
      hasCursor: root.highlighted
      onChanged: function(next) { root.handlePick(searchable, next) }
      onPopupOpenChanged: root.editorFocusChanged(popupOpen)
    }

    NumberField {
      id: numberField
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      visible: root.controlKind === "int"
      // An unbounded int still needs a range for the spin box; this one is
      // wide enough for every duration/threshold voxtype exposes.
      from: root.bounded ? Math.round(root.specMin) : 0
      to: root.bounded ? Math.round(root.specMax) : 1000000
      value: {
        var n = Number(root.currentValue)
        return isFinite(n) ? Math.round(n) : 0
      }
      hasCursor: root.highlighted
      onModified: function(next) { root.apply(next) }

      // The spin box holds the focus, so an unhandled Escape arrives here on
      // its way up the chain. Take it before the card does.
      Keys.onEscapePressed: function(event) {
        if (!numberField.field.activeFocus) return
        root.editorEscaped()
        event.accepted = true
      }

      Connections {
        target: numberField.field
        function onActiveFocusChanged() { root.editorFocusChanged(numberField.field.activeFocus) }
      }
    }

    Row {
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      visible: root.controlKind === "slider"
      spacing: Style.spacing.controlGap

      PanelSlider {
        id: slider
        anchors.verticalCenter: parent.verticalCenter
        width: parent.width - valueLabel.width - parent.spacing
        bar: root.sliderBar
        minimum: root.specMin
        maximum: root.specMax
        step: Math.max(0.01, (root.specMax - root.specMin) / 20)
        value: {
          var n = Number(root.currentValue)
          return isFinite(n) ? n : root.specMin
        }
        onMoved: function(next) { sliderDebounce.restart() }
        onReleased: function(next) {
          sliderDebounce.stop()
          root.apply(Math.round(next * 1000) / 1000)
        }
      }

      Text {
        id: valueLabel
        anchors.verticalCenter: parent.verticalCenter
        width: Style.space(44)
        horizontalAlignment: Text.AlignRight
        text: slider.liveValue.toFixed(2)
        color: root.dim
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
      }
    }

    TextField {
      id: textField
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      visible: root.controlKind === "text"
      text: root.currentText
      placeholderText: root.openEnum ? "Custom value" : ""
      hasCursor: root.highlighted
      onEditingFinished: {
        if (text === root.currentText) return
        root.apply(text)
      }
      onActiveFocusChanged: root.editorFocusChanged(activeFocus)

      // Escape backs out of "Custom value…" without writing, and otherwise
      // hands the keyboard back to the form's row cursor. Either way it stops
      // here rather than bubbling up to the card, which would close the panel.
      Keys.onEscapePressed: function(event) {
        if (root.customEditing) {
          root.customEditing = false
          event.accepted = true
          return
        }
        if (!textField.activeFocus) return
        root.editorEscaped()
        event.accepted = true
      }
    }
  }

  // A slider drag emits a value per mouse move; only the settled value is
  // worth a subprocess.
  Timer {
    id: sliderDebounce
    interval: 300
    repeat: false
    onTriggered: root.apply(Math.round(slider.liveValue * 1000) / 1000)
  }

  // PanelSlider paints from a bar's palette. This panel is not a bar, so it
  // hands over the theme colors directly — the same shim the shell's own dev
  // gallery uses.
  readonly property var sliderBar: QtObject {
    readonly property color foreground: ShellTheme.Color.foreground
    readonly property color background: ShellTheme.Color.popups.background
    readonly property color urgent: ShellTheme.Color.urgent
    readonly property string fontFamily: Style.font.family
    readonly property string position: "top"
    readonly property bool vertical: false
  }
}
