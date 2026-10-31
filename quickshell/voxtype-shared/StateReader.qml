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
// The component also exposes `osdSuppressed`, set while the daemon is
// running a recording started with `voxtype record start --no-osd`. OSD
// surfaces should stay hidden while it is true; the daemon state itself is
// still reported truthfully, because Waybar and every other status consumer
// depends on it.
//
// `session` and `outcome` mirror the daemon's recording.json and
// outcome.json (see src/osd/session.rs): the recording in flight (wall
// clock start, max_duration_secs, engine, model), or null when idle; and
// how the last session ended, or null before the first one. Both stay
// null against a daemon too old to write them.
//
// The component exposes `state` as a bindable property so consumers
// don't have to listen for the signal:
//
//   border.color: stateReader.state === "recording" ? "red" : "gray"

import QtQuick
import Quickshell
import Quickshell.Io

// Use QtObject as the root rather than Item so we don't collide with
// Item's built-in `state` property (and its auto-generated `stateChanged`
// signal, which Quickshell rejects as a duplicate when redeclared).
QtObject {
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
    /// QML auto-generates a `stateChanged()` signal for property
    /// changes; consumers read the current value off the property.
    property string state: "idle"

    /// Sibling marker written by the daemon while a `--no-osd` recording is
    /// in flight. True means "do not draw the OSD for this one".
    property string osdSuppressedPath: {
        const base = root.statePath;
        const cut = base.lastIndexOf("/");
        return (cut > 0 ? base.substring(0, cut) : base) + "/osd_suppressed";
    }

    /// True while the in-flight recording asked for the OSD to stay hidden.
    property bool osdSuppressed: false

    readonly property string _runtimeDir: {
        const cut = root.statePath.lastIndexOf("/");
        return cut > 0 ? root.statePath.substring(0, cut) : root.statePath;
    }

    property var session: null
    property var outcome: null

    function _parse(view) {
        try {
            const value = JSON.parse(view.text() || "null");
            return value && typeof value === "object" ? value : null;
        } catch (e) {
            return null;
        }
    }

    // FileView re-reads on file changes when watchChanges is true. We
    // update the `state` property inside onLoaded; QML's property change
    // signal fires automatically for binding-based and imperative
    // consumers.
    property FileView _fileView: FileView {
        path: root.statePath
        watchChanges: true
        printErrors: false

        onLoaded: {
            // The daemon rewrites the file in place (truncate, then
            // write), so a read can land in between and see nothing.
            // Keep the current state rather than flashing to idle; a
            // removed file still resets through onLoadFailed.
            const next = (text() || "").trim();
            if (next.length > 0 && next !== root.state) {
                root.state = next;
            }
        }

        onLoadFailed: {
            if (root.state !== "idle") {
                root.state = "idle";
            }
        }

        onFileChanged: reload()
    }

    // The marker is created and removed rather than rewritten, so both the
    // load and the failure path carry meaning: present means suppress, absent
    // (the common case) means draw normally.
    property FileView _osdSuppressedView: FileView {
        path: root.osdSuppressedPath
        watchChanges: true
        printErrors: false

        onLoaded: {
            if (!root.osdSuppressed) {
                root.osdSuppressed = true;
            }
        }

        onLoadFailed: {
            if (root.osdSuppressed) {
                root.osdSuppressed = false;
            }
        }

        onFileChanged: reload()
    }

    property FileView _sessionView: FileView {
        path: root._runtimeDir + "/recording.json"
        watchChanges: true
        printErrors: false
        onLoaded: root.session = root._parse(this)
        onLoadFailed: root.session = null
        onFileChanged: reload()
    }

    property FileView _outcomeView: FileView {
        path: root._runtimeDir + "/outcome.json"
        watchChanges: true
        printErrors: false
        onLoaded: root.outcome = root._parse(this)
        onLoadFailed: root.outcome = null
        onFileChanged: reload()
    }
}
