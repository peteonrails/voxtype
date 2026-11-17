import QtQuick
import Quickshell
import Quickshell.Wayland

// The meeting view's own surface: a card the user can move, resize, make
// compact or minimize to a pill (MeetingPlacement holds where and how big).
//
// Not a modal overlay. It sits on the Top layer without a scrim and reserves
// no space (windows are not re-tiled when a meeting starts). It takes the
// keyboard only while a text field is under the pointer or being typed in
// (the theme counts them), so clicking a button, dragging the card or
// resizing it never moves focus away from the call.
PanelWindow {
  id: window
  anchors { top: true; left: true; bottom: window.placement.fullHeight }
  margins { top: window.placement.top; left: window.placement.left; bottom: window.placement.fullHeight ? window.placement.gap : 0 }
  implicitWidth: window.placement.windowWidth
  implicitHeight: window.placement.windowHeight
  color: "transparent"
  exclusionMode: ExclusionMode.Normal
  WlrLayershell.namespace: "voxtype-meetings"
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.keyboardFocus: window.theme.inputDemand > 0 ? WlrKeyboardFocus.OnDemand : WlrKeyboardFocus.None

  required property var theme
  required property var placement
  readonly property alias view: meetingView

  MeetingView {
    id: meetingView
    anchors.fill: parent
    theme: window.theme
    placement: window.placement
  }
}
