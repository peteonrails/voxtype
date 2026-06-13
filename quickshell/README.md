# Voxtype Quickshell Frontend

A Quickshell/QML frontend for the voxtype daemon, shipped since v0.7.5 as an
alternative to the GTK4 (`voxtype-osd-gtk4`) and native (`voxtype-osd-native`)
OSD frontends. Intended for users running a Quickshell-based desktop shell
(Omarchy 4+, end_4/dots-hyprland, custom shells).

## Contents

| File | Purpose |
|------|---------|
| `shell.qml` | Composition root. Wires the shared state and audio bridges into each widget. |
| `OsdSurface.qml` | Recipe-driven recording HUD: state icon + tint, scrolling waveform, peak meter, and optional trusted custom QML style package. Visible whenever the daemon is not idle. |
| `EnginePicker.qml` | Floating panel listing every transcription engine, with the active one marked. Switches engines via `voxtype config set engine <name>`. Engines not compiled into the running binary are shown dimmed. |
| `MeetingControls.qml` | Meeting status panel (title, elapsed time, chunk count) with Start / Stop / Pause / Resume buttons that shell out to `voxtype meeting <action>`. |
| `voxtype-shared/` | Shared QML module: `Theme` (palette/sizing singleton), `StateReader` (daemon state file watcher), `AudioBridge` (wrapper around the `voxtype-audio-bridge` sidecar), `StyleLoader`, and `RecipeRenderer`. See [voxtype-shared/README.md](voxtype-shared/README.md). |

## Install

The packaged builds (AUR, deb, rpm) install the QML tree system-wide at
`/usr/share/voxtype/quickshell/` and the audio bridge at
`/usr/lib/voxtype/voxtype-audio-bridge`, so no extra step is needed there.
For source builds, or to get a per-user copy you can customize:

```bash
voxtype setup quickshell
```

This copies the QML files to `$XDG_DATA_HOME/voxtype/quickshell/`
(`~/.local/share/voxtype/quickshell/` by default), symlinks
`voxtype-audio-bridge` into `~/.local/bin/` so the waveform can find it on
PATH, and prints compositor binding examples for the popup widgets. See
`voxtype setup quickshell --help` for `--target`, `--source`, `--force`,
`--bridge`, and `--skip-bridge` options.

To make the daemon's OSD supervisor launch this frontend:

```toml
[osd]
frontend = "quickshell"
```

## Run

The `voxtype-osd-quickshell` launcher finds the installed QML tree
(`--qml-path` / `VOXTYPE_OSD_QML_PATH` override, then the per-user copy,
then `/usr/share/voxtype/quickshell/`, then `quickshell/` relative to the
current directory) and execs `qs -d -p <dir>`:

```bash
voxtype-osd-quickshell
```

To run directly from a repo checkout during development:

```bash
qs -p quickshell
```

Press your voxtype hotkey and watch the screen edge: the OSD card appears
with a red tint and live waveform during `recording`, blue during
`streaming`, amber during `transcribing`, and disappears at `idle`.

## Toggling the popup widgets

The engine picker and meeting controls are toggled by touching flag files
under `$XDG_RUNTIME_DIR/voxtype/`, which the QML watches via `FileView`:

```bash
# Hyprland examples (see `voxtype setup quickshell --print-bindings`
# for Sway and River equivalents)
bind = SUPER, E, exec, mkdir -p $XDG_RUNTIME_DIR/voxtype && touch $XDG_RUNTIME_DIR/voxtype/engine-picker.flag
bind = SUPER, M, exec, mkdir -p $XDG_RUNTIME_DIR/voxtype && touch $XDG_RUNTIME_DIR/voxtype/meeting-controls.flag
```

## How it works

- **State**: `StateReader` watches `$XDG_RUNTIME_DIR/voxtype/state` with a
  `FileView { watchChanges: true }`, so the UI follows the daemon's state
  machine without polling.
- **Audio**: Quickshell cannot read Unix domain sockets natively, so the
  `voxtype-audio-bridge` sidecar reads the daemon's audio socket
  (`$XDG_RUNTIME_DIR/voxtype/audio.sock`) and emits one NDJSON line per
  frame on stdout. `AudioBridge` spawns it via Quickshell's `Process`
  element and parses peak / RMS / VAD values for the waveform and meter.
  `shell.qml` creates a single `AudioBridge` and passes it to widgets so
  only one sidecar process runs.
- **Style**: `StyleLoader` reads launcher-resolved runtime JSON from
  `VOXTYPE_OSD_STYLE_FILE`, including Omarchy palette tokens, recipe layers,
  and optional trusted custom QML package entry.
- **Recipes**: `RecipeRenderer` draws no-code visual recipes from
  `[osd.visual.layers]`. The default recipe matches the compact waveform and
  peak-meter card, and users can customize the surface without editing
  packaged QML.
- **Surfaces**: each widget is a `PanelWindow` on the `WlrLayer.Overlay`
  layer with no keyboard focus, matching the GTK4 frontend's surface
  semantics.
- **No new daemon IPC**: the picker and meeting controls read existing
  files (`config.toml`, `meeting_state`) and shell out to existing CLI
  commands (`voxtype config set`, `voxtype meeting <action>`,
  `voxtype info variants --json`).

## Documentation

- [User manual: `voxtype setup quickshell`](../docs/USER_MANUAL.md#voxtype-setup-quickshell)
- [Configuration: OSD frontend and style options](../docs/CONFIGURATION.md#osd-frontend)

## Customization

```toml
[osd]
frontend = "quickshell"
style = "default"
palette = "omarchy"
layout = "compact"

[osd.frame]
background = "none"
border = "none"
glow = true
halo = true

[[osd.visual.layers]]
type = "bars"
source = "peak"
color = "accent"
order = 10
gain = 1.2
mirror = true
```

Layer types are `shadow`, `background`, `waveform`, `bars`, `pulse`, `ring`,
`meter`, `icon`, and `label`.

`palette = "omarchy"` maps the active Omarchy theme's `colors.toml` into
semantic roles such as `accent`, `background`, `foreground`, `success`,
`warning`, and `error`.

`layout` controls the host frame, not just the animation area. `compact`,
`wide`, and `minimal` are strip layouts; `tile` is a square card; `orb` is a
circular frame for ring-focused recipes.

`[osd.frame]` controls the host card around the rendered recipe. Use
`background = "none"` and `border = "none"` for a frameless animation, or use
semantic roles/literal colors to recolor them.

Shareable style packages are directories with `voxtype-osd.toml`, optional
`assets/`, optional `[colors]` token overrides, and optionally a QML entry file.
Custom QML is trusted code and only loads when selected through `[osd] style` or
`plugin_path`.

## Not covered

- Custom QML is not sandboxed. Treat third-party packages like shell scripts.
- GTK4/native frontends ignore Quickshell recipe/package fields.

## License

Same as the rest of voxtype (MIT).
