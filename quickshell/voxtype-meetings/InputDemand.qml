import QtQuick

// A text input's claim on the keyboard: while the pointer is on it or it has
// focus, the meeting window asks the compositor for keyboard focus (on
// demand); otherwise it asks for none, so clicks and drags elsewhere on the
// card never take focus from the call. The theme holds the count because
// every component already has it.
QtObject {
  property var theme: null
  property bool active: false
  property bool counted: false

  function sync() {
    if (!theme) return
    if (active && !counted) {
      theme.inputDemand++
      counted = true
    } else if (!active && counted) {
      theme.inputDemand = Math.max(0, theme.inputDemand - 1)
      counted = false
    }
  }

  onActiveChanged: sync()
  onThemeChanged: sync()
  Component.onDestruction: {
    if (counted && theme) theme.inputDemand = Math.max(0, theme.inputDemand - 1)
  }
}
