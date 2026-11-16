import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import "meetings"

// The Omarchy shell's theme mapped onto MeetingTheme, for views the plugin
// hosts in their own window. OmarchyMeetingsHost.qml carries the same mapping
// for the meeting view.
MeetingTheme {
  fg: ShellTheme.Color.foreground
  bg: ShellTheme.Color.popups.background
  border: ShellTheme.Color.popups.border
  accent: ShellTheme.Color.accent
  urgent: ShellTheme.Color.urgent
  fontFamily: Style.font.family
  radius: Style.cornerRadius
  caption: Style.font.caption
  bodySmall: Style.font.bodySmall
  body: Style.font.body
  title: Style.font.title
  heading: Style.font.heading
  iconLarge: Style.font.iconLarge
  display: Style.font.displayLarge
  xxs: Style.spacing.xxs
  xs: Style.spacing.xs
  sm: Style.spacing.sm
  md: Style.spacing.md
  lg: Style.spacing.lg
  xxl: Style.spacing.xxl
  gap: Style.spacing.controlGap
  padX: Style.spacing.controlPaddingX
  controlHeight: Style.spacing.controlHeight
  padding: Style.spacing.panelPadding
  margin: Style.gapsOut * 2
}
