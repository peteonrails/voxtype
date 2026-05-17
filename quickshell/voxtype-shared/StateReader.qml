// Voxtype daemon state file watcher.
//
// Wraps Quickshell.Io.FileView around the daemon's state file at
// $XDG_RUNTIME_DIR/voxtype/state. The file contains exactly one of
// `idle`, `recording`, `streaming`, `transcribing` and is rewritten by
// the daemon on every state machine transition.
//
// Usage:
//
//   import "voxtype-shared" as VT
//   VT.StateReader {
//       id: stateReader
//       onStateChanged: function(newState) {
//           console.log("voxtype is now", newState);
//       }
//   }
//
// The component exposes `state` as a bindable property so consumers
// don't have to listen for the signal:
//
//   border.color: stateReader.state === "recording" ? "red" : "gray"

import QtQuick
import Quickshell
import Quickshell.Io

Item {
    id: root

    /// Filesystem path to the daemon state file. Defaults to
    /// `$XDG_RUNTIME_DIR/voxtype/state` with a `/run/user/$UID`
    /// fallback for environments that don't export XDG_RUNTIME_DIR.
    property string statePath: {
        const xdg = Quickshell.env("XDG_RUNTIME_DIR");
        if (xdg && xdg.length > 0) {
            return xdg + "/voxtype/state";
        }
        const uid = Quickshell.env("UID");
        if (uid && uid.length > 0) {
            return "/run/user/" + uid + "/voxtype/state";
        }
        // Last-resort fallback: assume UID 1000. Better than an empty
        // path that would silently never resolve.
        return "/run/user/1000/voxtype/state";
    }

    /// Current daemon state. One of: idle, recording, streaming,
    /// transcribing. Defaults to "idle" when the file is missing or
    /// unreadable so consumers can always render a sensible default.
    property string state: "idle"

    /// Emitted whenever the daemon writes a new state to disk.
    /// `newState` is the freshly read value with surrounding whitespace
    /// trimmed.
    signal stateChanged(string newState)

    // FileView re-reads on file changes when watchChanges is true.
    // We re-set the `state` property inside onLoaded so the binding-
    // based consumers also update, then fire the signal for imperative
    // consumers (e.g. animation triggers).
    FileView {
        id: stateFile
        path: root.statePath
        watchChanges: true
        printErrors: false

        onLoaded: {
            const next = (text() || "idle").trim();
            if (next !== root.state) {
                root.state = next;
                root.stateChanged(next);
            }
        }

        onLoadFailed: {
            if (root.state !== "idle") {
                root.state = "idle";
                root.stateChanged("idle");
            }
        }

        onFileChanged: reload()
    }
}
