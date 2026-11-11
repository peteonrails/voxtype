import QtQuick
import Quickshell.Io

// A Process that also reports when it never got going: Quickshell's Process
// emits neither `started` nor `exited` for a command that cannot start (a
// missing voxtype), so launch() records a pending start and failedToStart()
// fires if `running` drops while it is still pending. The same rule as the
// Settings panel's CliProcess.
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
