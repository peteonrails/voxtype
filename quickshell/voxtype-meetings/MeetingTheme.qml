import QtQuick

// Colours, type sizes and spacing for the meeting view. The view uses nothing
// else, so it runs under any host: the Omarchy plugin binds these to the
// shell's own theme (OmarchyMeetingsHost.qml), the standalone Quickshell shell
// to voxtype-shared's Theme (MeetingControls.qml). The defaults are a usable
// dark palette for a host that sets nothing.
QtObject {
  property color fg: "#dcdfe4"
  property color bg: Qt.rgba(0.10, 0.10, 0.12, 0.97)
  property color border: "#6e89c2"
  property color accent: "#6e89c2"
  property color urgent: "#e06c75"
  property color dim: Qt.darker(fg, 1.5)

  property string fontFamily: ""
  // Nerd Font glyphs; a host without one gets labels only (see MeetingButton).
  property string iconFamily: fontFamily

  property int radius: 8

  property int caption: 10
  property int bodySmall: 11
  property int body: 12
  property int title: 14
  property int heading: 16
  property int iconLarge: 18
  property int display: 28

  property int xxs: 2
  property int xs: 3
  property int sm: 4
  property int md: 6
  property int lg: 8
  property int xxl: 12
  property int gap: 8
  property int padX: 10
  property int controlHeight: 28
  property int padding: 14
  // Space between the card and the screen edge.
  property int margin: 10

  function alpha(c, a) { return Qt.rgba(c.r, c.g, c.b, a) }
}
