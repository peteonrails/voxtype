# Voxtype Quickshell Frontend

A Quickshell/QML frontend for the voxtype daemon, shipped since v0.7.5 as an
alternative to the GTK4 (`voxtype-osd-gtk4`) and native (`voxtype-osd-native`)
OSD frontends. Intended for users running a Quickshell-based desktop shell
(Omarchy 4+, end_4/dots-hyprland, custom shells).

## Contents

| File | Purpose |
|------|---------|
| `shell.qml` | Composition root. Wires the shared state and audio bridges into each widget. |
| `OsdSurface.qml` | Recipe-driven recording HUD: state icon + tint, scrolling waveform, peak meter, elapsed time and limit countdown, a brief engine and model chip, a result card when the session ends, and optional trusted custom QML style package. Visible whenever the daemon is not idle (plus a moment afterwards for the result), on the focused output under Hyprland and Sway/i3, and on every output elsewhere. |
| `EnginePicker.qml` | Floating panel listing every transcription engine, with the active one marked. Switches engines via `voxtype config set engine <name>`. Engines not compiled into the running binary are shown dimmed. |
| `MeetingControls.qml` | Meeting status panel (title, elapsed time, chunk count) with Start / Stop / Pause / Resume buttons that shell out to `voxtype meeting <action>`. |
| `voxtype-shared/` | Shared QML module: `Theme` (palette/sizing singleton), `StateReader` (daemon state file watcher), `AudioBridge` (wrapper around the `voxtype-audio-bridge` sidecar), `StyleLoader`, `RecipeRenderer`, and `StateIcon` (vector state and result icons). See [voxtype-shared/README.md](voxtype-shared/README.md). |

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

Press your voxtype hotkey and watch the screen edge: the OSD card animates
in with the recording tint and a live waveform during `recording`, the
streaming tint during `streaming`, and the transcribing tint with a slow
travelling ripple during `transcribing`. The exact colors come from the
palette (the Omarchy palette maps them from the active theme). When the
session ends, the card shows the result for a moment (a check and the
number of characters, "No speech detected", "Transcription failed", and so
on) and then animates out.

While recording, the card shows the elapsed time. In the last stretch before
`audio.max_duration_secs` stops the recording (the final 15 seconds, or the
final half for limits under 30 seconds) it becomes a countdown, amber and
then red for the last 5 seconds, with a bar along the card's bottom edge
draining toward the limit. The engine and model serving the recording appear
in a small chip for the first couple of seconds.

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
  machine without polling. It also reads `recording.json` (the session in
  flight: start time, `max_duration_secs`, engine, model) and `outcome.json`
  (how the last session ended) from the same directory. The daemon writes
  both atomically; see [Configuration: OSD runtime files](../docs/CONFIGURATION.md#osd-runtime-files).
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
  packaged QML. Each layer is a scene-graph item (Rectangles, Text, and
  QtQuick.Shapes paths), so the GPU does the drawing. One animation tick of
  about 60 Hz reads the newest audio values and drives every layer and
  decoration, so a recording redraws at most 60 times a second even on a
  high-refresh display.
- **Surfaces**: the HUD is a `PanelWindow` on the `WlrLayer.Overlay` layer
  with no keyboard focus, matching the GTK4 frontend's surface semantics.
  It follows the focused output when the compositor reports one (Hyprland,
  Sway/i3) and shows on every output otherwise. The engine picker and
  meeting controls sit on `WlrLayer.Top`, take keyboard focus while open,
  and open on the focused output.
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

Layer types are `shadow`, `background`, `waveform`, `line`, `bars`, `dots`,
`pulse`, `ring`, `meter`, `icon`, and `label`.

`waveform` and `bars` place peak levels on a decibel scale: -52 dBFS (room
noise) is empty and -6 dBFS is full height, and each column shows the
loudest sample in its slice of history. A layer's `gain` still multiplies
the result. The `icon` layer draws vector icons, so it does not need a Nerd
Font installed.

`palette = "omarchy"` maps the active Omarchy theme's `colors.toml` into
semantic roles such as `accent`, `background`, `foreground`, `success`,
`warning`, and `error`. Omarchy theme switches apply live: the launcher
rewrites the resolved style when the theme changes and `StyleLoader` reloads
it without a restart. (The GTK4 frontend picks a new theme up on restart.)

`layout` controls the host frame, not just the animation area. `compact`,
`wide`, `minimal`, `pill`, and `shell` are strip layouts; `tile` is a square
card; `orb` is a circular frame for ring-focused recipes. `shell` follows
Omarchy's live Hyprland rounding, popup border (including gradient stops),
monospace font, and spacing scale, with a steady 97% theme-background fill.
It reads the theme's `shell.toml` and optional user shell overrides without
importing Omarchy's QML modules. The `shell-line` recipe is Capsule's voice
line in this frame; `shell-dots` uses Aura's voice-assistant dots without its
animated edge. Both are also available as manifest-only style packages. In `tile` and `orb` the elapsed
time, the result message, and the model chip sit just outside the card.

`[osd.frame]` controls the host card around the rendered recipe. Use
`background = "none"` and `border = "none"` for a frameless animation, or use
semantic roles/literal colors to recolor them.

Shareable style packages are directories with `voxtype-osd.toml`, optional
`assets/`, optional `[colors]` token overrides, and optionally a QML entry file.
Custom QML is trusted code and only loads when selected through `[osd] style` or
`plugin_path`.

The custom item is anchored to fill the whole OSD window and positions
itself. It may declare any of these properties, and the host sets each one
it declares:

| Property | Value |
|----------|-------|
| `daemonState` | The daemon state string (`idle`, `recording`, `streaming`, `transcribing`). |
| `audio` | The shared `AudioBridge` (`frameReceived(peak, rms, vad, tsMs)` signal, plus `peak`, `rms`, `vad`, `running`). |
| `theme` | The `StyleLoader` object: `theme.color(role, fallback)` and `theme.config`. |
| `recipe` | `config.visual` from the resolved style. |
| `assetRoot` | The package's `assets/` directory, or `""`. |
| `session` | Parsed `recording.json`, or `null` when idle. |
| `outcome` | Parsed `outcome.json`, or `null` before the first session ends. |

The host re-syncs these when the daemon state, session, or outcome changes,
and when the style reloads (for example on an Omarchy theme switch).

## Testing shell chrome

```bash
cargo test --lib osd::
bash tests/quickshell-shell-style.sh
```

The QML checks run in a headless `qs` instance with isolated runtime/cache
files under `target/`, not the user's running OSD. Quickshell's types need
`qs`; the plain Qt `qmltestrunner` is not a substitute.

## Not covered

- Custom QML is not sandboxed. Treat third-party packages like shell scripts.
- GTK4/native frontends ignore Quickshell recipe/package fields.

## License

Same as the rest of voxtype (MIT).
