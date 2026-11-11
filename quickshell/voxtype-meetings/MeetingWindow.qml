import QtQuick
import Quickshell
import Quickshell.Wayland

// The meeting view's own surface: a card down the right edge.
//
// Not a modal overlay. It sits on the Top layer without a scrim, reserves no
// space (windows are not re-tiled when a meeting starts), and takes the
// keyboard only when clicked, so the call and whatever the user is typing in
// keep focus while it follows the meeting. Generic layer-shell, so every host
// uses it as is.
PanelWindow {
  id: window
  anchors { top: true; right: true; bottom: true }
  implicitWidth: 460
  color: "transparent"
  exclusionMode: ExclusionMode.Normal
  WlrLayershell.namespace: "voxtype-meetings"
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.keyboardFocus: WlrKeyboardFocus.OnDemand

  property var theme: MeetingTheme {}
  readonly property alias view: meetingView

  MeetingView {
    id: meetingView
    anchors.fill: parent
    anchors.margins: window.theme ? window.theme.margin : 10
    theme: window.theme
  }
}
