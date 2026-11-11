import QtQuick
import Quickshell
import Quickshell.Wayland
import qs.Commons

// The meeting view's own surface: a card down the right edge, under the bar.
//
// Unlike the Settings panel this is not a modal overlay. It sits on the Top
// layer without a scrim, reserves no space (windows are not re-tiled when a
// meeting starts), and takes the keyboard only when clicked, so the call and
// whatever the user is typing in keep focus while it follows the meeting.
PanelWindow {
  id: window
  anchors { top: true; right: true; bottom: true }
  implicitWidth: Style.space(460)
  color: "transparent"
  exclusionMode: ExclusionMode.Normal
  WlrLayershell.namespace: "voxtype-meetings"
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.keyboardFocus: WlrKeyboardFocus.OnDemand

  readonly property alias view: meetingView

  MeetingView {
    id: meetingView
    anchors.fill: parent
    anchors.margins: Style.gapsOut * 2
  }
}
