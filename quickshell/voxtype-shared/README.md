# voxtype-shared

Shared Quickshell QML modules used by every voxtype QML frontend: the
OSD card, the engine picker menu, and meeting controls canvas. Extracted
so the frontends don't each ship their own copy of theme constants,
state file watcher, audio bridge wrapper, or recipe renderer.

## Modules

### `Theme` (singleton)

Color palette and sizing defaults mirrored from `src/osd/visual.rs`
(`Palette::fallback`) and `src/osd/config.rs` (`OsdConfig::default`).

Public properties:

| Property            | Default                                | Purpose                                  |
|---------------------|----------------------------------------|------------------------------------------|
| `bgColor`           | rgba(0.10, 0.10, 0.12, 0.85)           | Translucent card background              |
| `accentColor`       | rgba(0.40, 0.78, 1.00, 1.0)            | Waveform fill, primary action            |
| `idleColor`         | `#abb2bf`                              | Idle-state indicator                     |
| `recordingColor`    | `#e06c75`                              | Recording-state indicator                |
| `streamingColor`    | `#61afef`                              | Streaming-state indicator                |
| `transcribingColor` | `#e5c07b`                              | Transcribing-state indicator             |
| `textColor`         | `#dcdfe4`                              | Foreground text                          |
| `waveformColor`     | bound to `accentColor`                 | Waveform body                            |
| `waveformPeakColor` | `#FCFBF8`                              | Waveform held-peak tick                  |
| `cornerRadius`      | `12`                                   | Card radius (swayosd parity)             |
| `padding`           | `14`                                   | Inner padding for OSD cards              |
| `marginPx`          | `24`                                   | Distance from screen edge                |
| `defaultWidthPx`    | `400`                                  | Mirrors `OsdConfig::width_px`            |
| `defaultHeightPx`   | `48`                                   | Mirrors `OsdConfig::height_px`           |
| `defaultOpacity`    | `0.95`                                 | Mirrors `OsdConfig::opacity`             |
| `waveformWindowSecs`| `3.0`                                  | Mirrors `OsdConfig::waveform_window_secs`|

### `StateReader`

Watches `$XDG_RUNTIME_DIR/voxtype/state` for changes and exposes the
current daemon state, plus the sibling runtime files the daemon writes for
OSD frontends (`osd_suppressed`, `recording.json`, `outcome.json`).

Public properties:

| Property        | Default                                  | Purpose                              |
|-----------------|------------------------------------------|--------------------------------------|
| `statePath`     | `$XDG_RUNTIME_DIR/voxtype/state`         | Override for tests / custom setups. The other files are read from the same directory. |
| `state`         | `"idle"`                                 | Current daemon state                 |
| `osdSuppressed` | `false`                                  | True while a `record start --no-osd` session runs |
| `session`       | `null`                                   | Parsed `recording.json`: `mode`, `started_at_ms`, `max_duration_secs`, `engine`, `model`. `null` when idle. |
| `outcome`       | `null`                                   | Parsed `outcome.json`: `outcome`, `chars`, `limit_reached`, `at_ms`. `null` before the first session ends. |

`session` and `outcome` stay `null` against a daemon too old to write those
files. An empty read of the state file (the daemon rewrites it in place)
keeps the previous state instead of reporting `idle`.

Signals:

- `stateChanged()` is QML's automatic change signal for `state`; read the
  new value from the property.

### `AudioBridge`

Wraps the `voxtype-audio-bridge` sidecar binary. The bridge reads the
daemon's audio Unix socket and emits NDJSON to stdout; this component
parses each line into properties + signals.

Public properties:

| Property         | Default                  | Purpose                                                            |
|------------------|--------------------------|--------------------------------------------------------------------|
| `bridgeBinary`   | `voxtype-audio-bridge`   | Path or PATH-resolvable name of the sidecar                        |
| `bridgeArgs`     | `[]`                     | Additional CLI args for the bridge                                 |
| `restartDelayMs` | `1000`                   | Delay before the first respawn after an exit; doubles on each consecutive exit |
| `maxRestartDelayMs` | `30000`               | Ceiling for the respawn delay; it resets once the bridge connects  |
| `running`        | `false`                  | True once frames are arriving                                      |
| `peak`           | `0.0`                    | Latest peak amplitude (0.0..=1.0)                                  |
| `rms`            | `0.0`                    | Latest RMS amplitude (0.0..=1.0)                                   |
| `vad`            | `false`                  | Latest voice-activity-detection result                             |
| `tsMs`           | `0`                      | Latest frame timestamp in ms (monotonic, from the daemon)          |

Signals:

- `frameReceived(real peak, real rms, bool vad, var tsMs)` — emitted on every audio frame.
- `connected()` — emitted on `{"status":"connected"}`.
- `disconnected()` — emitted on `{"status":"disconnected"}` or when the bridge process exits.

### `StyleLoader`

Reads the runtime style JSON path from `VOXTYPE_OSD_STYLE_FILE`. The
`voxtype-osd-quickshell` launcher writes this file after resolving
`[osd]` config, package manifests, and Omarchy palette tokens.

Public properties and helpers:

| Property / function | Purpose |
|---------------------|---------|
| `config`            | Parsed runtime style object; `config.shell` holds resolved shell chrome only for the `shell` layout |
| `shellCornerRadius` | Effective Hyprland rounding for the shell layout; zero without Hyprland |
| `color(role, fallback)` | Resolve semantic tokens or literal colors |
| `customQmlUrl()`    | `file://` URL for an explicitly selected trusted QML package entry |

### `ShellMetrics` and `ShellBorder`

`ShellMetrics` queries Hyprland rounding on activation and config reload,
without polling or starting processes for other layouts. `ShellBorder`
draws a static rounded edge with the resolved popup gradient, alpha, and
uniform thickness. Neither imports Omarchy's shell modules. TOML tokens and
aliases are resolved in Rust (`src/osd/shell_style.rs`).

### `RecipeRenderer`

Renderer for no-code OSD visual recipes. Each layer is its own scene-graph
item (Rectangles, Text, and QtQuick.Shapes paths with static gradients)
rather than strokes on a software Canvas, so the GPU does the drawing. One
animation tick (about 60 Hz, running only while `active` and not idle)
reads the newest audio values, smooths them, and updates every layer.

Layer types are `shadow`, `background`, `waveform`, `line`, `bars`, `dots`,
`pulse`, `ring`, `meter`, `icon`, and `label`. Colors are semantic roles by default (`accent`,
`background`, `foreground`, `success`, `warning`, `error`) and normally
come from the active Omarchy theme. `waveform` and `bars` map peaks onto a
-52..-6 dBFS scale, taking the loudest sample in each column. While
`daemonState` is `transcribing` they show a slow travelling ripple instead
of the last audio, and the meter drains.

Public properties and functions:

| Property / function | Purpose |
|---------------------|---------|
| `style`             | A `StyleLoader`; layers come from `style.config.visual.layers` |
| `active`            | False while the host window is hidden; stops the tick |
| `daemonState`       | Daemon state; outside `recording`/`streaming` audio input is ignored |
| `meter`             | Optional live object `{ peak, rms, vad, peakDbfs, heldDbfs, ring }`, read once per tick. A host receiving 100 Hz frames can mutate it in place so frames trigger no bindings. When `null`, the `peak`, `rms`, `vad`, `currentPeakDbfs`, `heldDbfs`, and `ring` properties are used instead. |
| `waveformColumns`   | Capacity of the peak history ring (default 300, 3 s at 100 Hz) |
| `meterFloorDbfs`    | Meter floor (default -60) |
| `waveformGain`      | Unused since levels moved to a decibel scale; kept so hosts that set it still load |
| `voiceEnergy`       | Output: smoothed voice energy (0..1) for host glows and halos |
| `haloEnergy`        | Output: a hotter mix of the same signal, used by the orb halo |
| `voiceActivity`     | Output: voice activity eased toward 0 or 1 over about 120 ms |
| `phase`             | Output: animation time in seconds, for decorations that move in step with the layers |
| `reset()`           | Drop all smoothed state, e.g. when a new recording starts |
| `repaint()`         | Advance one tick now |

### `StateIcon`

Vector state icon drawn with QtQuick.Shapes, so it renders without a Nerd
Font installed. Shows a microphone for `recording`, audio lines for
`streaming`, and a spinner for `transcribing`. Setting `result` shows how a
session ended instead.

| Property      | Default     | Purpose |
|---------------|-------------|---------|
| `daemonState` | `recording` | Which state icon to draw |
| `result`      | `""`        | Overrides the state icon: `done` (check), `nospeech` (crossed-out microphone), or `error` (crossed circle) |
| `color`       | `#dcdfe4`   | Stroke color |
| `phase`       | `0.0`       | Seconds of animation time; turns the spinner. The host advances it on its own tick, so the icon never runs a free animation. |
| `strokeWidth` | `2.0`       | Stroke width on the icon's 24x24 grid |

## Minimal import

```qml
import "voxtype-shared" as VT

Rectangle {
    color: VT.Theme.bgColor

    VT.StateReader {
        id: stateReader
        onStateChanged: console.log("state:", stateReader.state)
    }

    VT.AudioBridge {
        onFrameReceived: function(peak, rms, vad, tsMs) {
            // push into a ring buffer for the waveform renderer
        }
    }

    VT.StyleLoader {
        id: style
    }
}
```

The directory is structured as a QML module (`qmldir` registers
`Theme` as a singleton plus the other components), so the import
above resolves when Quickshell is launched from the parent
`quickshell/` directory or when the module is symlinked into the
user's Quickshell config tree.

## Scope

These modules back the three Quickshell widgets that ship with voxtype
since v0.7.5: `OsdSurface`, `EnginePicker`, and `MeetingControls` (see
[../README.md](../README.md)). `shell.qml` instantiates one `StateReader`
and one `AudioBridge` and passes them to the widgets so only a single
sidecar process runs.
