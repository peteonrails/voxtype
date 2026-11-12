import QtQuick

// Where answers and summaries come from: "Claude", "Local (llama3.2)". Click
// for the agent menu.
MeetingButton {
  id: chip
  objectName: "agentChip"
  property var assistant: null
  // What to name: the Ask chip names what answers questions, the Summary
  // chip what writes summaries (they can differ).
  property string label: chip.assistant ? chip.assistant.agentLabel : ""
  signal menuRequested(var anchor)

  text: chip.label !== "" ? chip.label + "  ▾" : "Agent  ▾"
  bordered: true
  onClicked: chip.menuRequested(chip)
}
