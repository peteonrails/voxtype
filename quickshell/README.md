# Voxtype Quickshell OSD (proof of concept)

A Quickshell-based on-screen indicator for the voxtype daemon, intended as a
drop-in alternative to the GTK4 OSD (`voxtype-osd-gtk4`) for users running a
Quickshell-based desktop shell (Omarchy 4+, end_4/dots-hyprland, custom shells).

## Status

POC, not yet a shipped feature. Reads voxtype's state file and shows a small
overlay card with state-specific icon + color. The waveform / peak-meter visual
that the GTK4 OSD renders is not yet ported (the GTK4 binary draws audio frames
streamed over a Unix socket; the QML port of that visualizer is the v0.7.3+
scope).

## Run standalone

```bash
qs -p quickshell/voxtype-osd/shell.qml
```

While running, watch the corner of your screen as you press your voxtype
hotkey: the card appears with a red border + microphone icon during
`recording`, a blue border + radio-tower icon during `streaming`, an amber
border + clock icon during `transcribing`, and disappears when the daemon
returns to `idle`.

## How it works

- `FileView { path: "$XDG_RUNTIME_DIR/voxtype/state"; watchChanges: true }`
  reads the daemon's state file. Quickshell's `FileView` element re-reads on
  every change, so the OSD follows the daemon's state machine without
  polling.
- `PanelWindow` with `WlrLayershell` overlay configures the surface as a
  click-through overlay-layer window with no keyboard focus, matching the
  GTK4 version's surface semantics.
- `visible: shell.state !== "idle"` hides the card whenever the daemon goes
  back to idle. No timers or animation; the toggle is purely reactive to
  state file changes.

## Drop into an Omarchy shell config

If you're running the omarchy-shell, copy this directory under the shell's
plugin tree and import it from the shell entry:

```bash
mkdir -p ~/.local/share/omarchy-shell/default/quickshell/omarchy-shell/plugins/voxtype-osd
cp quickshell/voxtype-osd/shell.qml ~/.local/share/omarchy-shell/default/quickshell/omarchy-shell/plugins/voxtype-osd/VoxtypeOsd.qml
```

Then add `VoxtypeOsd { }` to the plugin registry in `shell.qml`.

## What this POC does not (yet) cover

- The audio waveform + peak-meter visualizer rendered by `voxtype-osd-gtk4`.
  That requires reading the daemon's audio Unix socket
  (`$XDG_RUNTIME_DIR/voxtype/audio.sock`) and decoding the `AudioFrame` IPC
  format from `src/osd/ipc.rs`. Quickshell does not expose Unix domain socket
  reads natively; the integration path would either:
  1. Add a tiny side process that reads the socket and emits decoded frame
     values as text lines on stdout, which Quickshell's `Process` element
     consumes via `stdout`.
  2. Add a custom Quickshell module (Qt6 C++) that exposes a `Socket` type to
     QML.
- A `voxtype setup quickshell` installer (mirrors `voxtype setup compositor`)
  that drops the QML files into the right location and updates the user's
  Quickshell config to import them.
- TUI / config integration so `osd.frontend = "quickshell"` chooses this
  visualizer when the daemon's OSD supervisor decides to spawn one.

These are v0.7.3+ scope, not v0.7.2.

## License

Same as the rest of voxtype (MIT).
