import QtQuick
import qs.Commons
import qs.Commons as ShellTheme
import "meetings"

// The meeting view (quickshell/voxtype-meetings, shared with the standalone
// Quickshell shell) hosted in the Omarchy plugin: the shell's theme mapped
// onto MeetingTheme, and opening routed through Panel.qml's shell.summon.
Item {
  id: root

  property string bin: "voxtype"
  property bool shown: false
  property bool panelOpen: false

  signal openRequested()
  signal closeRequested()
  signal settingsRequested()

  readonly property alias feed: meetings.feed
  readonly property alias cli: meetings.cli
  readonly property alias assistant: meetings.assistant
  readonly property alias window: meetings.window

  function select(payload) { meetings.select(payload) }

  MeetingTheme {
    id: omarchyTheme
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

  Meetings {
    id: meetings
    bin: root.bin
    theme: omarchyTheme
    shown: root.shown
    panelOpen: root.panelOpen
    canOpenSettings: true
    onOpenRequested: root.openRequested()
    onCloseRequested: root.closeRequested()
    onSettingsRequested: root.settingsRequested()
  }
}
