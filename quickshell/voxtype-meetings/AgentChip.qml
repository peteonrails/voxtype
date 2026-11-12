import QtQuick

// Where answers and summaries come from: "Claude", "Local (llama3.2)". Click
// for the agent menu.
MeetingButton {
  id: chip
  objectName: "agentChip"
  property var assistant: null
  signal menuRequested(var anchor)

  text: chip.assistant && chip.assistant.agentLabel !== "" ? chip.assistant.agentLabel + "  ▾" : "Agent  ▾"
  bordered: true
  onClicked: chip.menuRequested(chip)
}
