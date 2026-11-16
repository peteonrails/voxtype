import QtQuick
import Quickshell.Io

// A Process that also reports when it never got going.
//
// Quickshell's Process has `started` and `exited`, and a command that cannot
// start (a dev.json naming a binary that is gone, voxtype not installed) emits
// neither: it logs a warning and `running` drops back to false. Every handler
// hung off `exited` then never runs, and whatever the panel showed last stays
// on screen as if it were current.
//
// launch() records that a start is pending; `started` clears it. If `running`
// falls while it is still pending, the start failed and failedToStart() fires
// once. Observed on Quickshell 0.3.2 for both an absolute path that does not
// exist and a bare name that is not on PATH.
Process {
  id: proc

  property bool startPending: false

  signal failedToStart()

  function launch(args) {
    proc.command = args
    proc.startPending = true
    proc.running = true
  }

  onStarted: proc.startPending = false
  onRunningChanged: {
    if (proc.running || !proc.startPending) return
    proc.startPending = false
    proc.failedToStart()
  }
}
