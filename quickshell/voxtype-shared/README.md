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
current daemon state.

Public properties:

| Property    | Default                                  | Purpose                              |
|-------------|------------------------------------------|--------------------------------------|
| `statePath` | `$XDG_RUNTIME_DIR/voxtype/state`         | Override for tests / custom setups   |
| `state`     | `"idle"`                                 | Current daemon state                 |

Signals:

- `stateChanged(string newState)` — fired on every transition.

### `AudioBridge`

Wraps the `voxtype-audio-bridge` sidecar binary. The bridge reads the
daemon's audio Unix socket and emits NDJSON to stdout; this component
parses each line into properties + signals.

Public properties:

| Property         | Default                  | Purpose                                                            |
|------------------|--------------------------|--------------------------------------------------------------------|
| `bridgeBinary`   | `voxtype-audio-bridge`   | Path or PATH-resolvable name of the sidecar                        |
| `bridgeArgs`     | `[]`                     | Additional CLI args for the bridge                                 |
| `restartDelayMs` | `1000`                   | Delay between exit and respawn                                     |
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
| `config`            | Parsed runtime style object |
| `color(role, fallback)` | Resolve semantic tokens or literal colors |
| `customQmlUrl()`    | `file://` URL for an explicitly selected trusted QML package entry |

### `RecipeRenderer`

Canvas renderer for no-code OSD visual recipes. It consumes a style
loader, recent audio peak ring, peak-hold state, and daemon state. Layer
types are `shadow`, `background`, `waveform`, `bars`, `pulse`, `ring`,
`meter`, `icon`, and `label`. Colors are semantic roles by default (`accent`,
`background`, `foreground`, `success`, `warning`, `error`) and normally
come from the active Omarchy theme.

## Minimal import

```qml
import "voxtype-shared" as VT

Rectangle {
    color: VT.Theme.bgColor

    VT.StateReader {
        id: state
        onStateChanged: console.log("state:", newState)
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
`Theme` as a singleton plus the two `Item` subclasses), so the import
above resolves when Quickshell is launched from the parent
`quickshell/` directory or when the module is symlinked into the
user's Quickshell config tree.

## Scope

These modules back the three Quickshell widgets that ship with voxtype
since v0.7.5: `OsdSurface`, `EnginePicker`, and `MeetingControls` (see
[../README.md](../README.md)). `shell.qml` instantiates one `StateReader`
and one `AudioBridge` and passes them to the widgets so only a single
sidecar process runs.
