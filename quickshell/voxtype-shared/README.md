# voxtype-shared

Shared Quickshell QML modules used by every voxtype QML frontend: the
OSD card, the engine picker menu, and (eventually) the meeting controls
canvas. Extracted so the frontends don't each ship their own copy of
theme constants, state file watcher, and audio bridge wrapper.

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
}
```

The directory is structured as a QML module (`qmldir` registers
`Theme` as a singleton plus the two `Item` subclasses), so the import
above resolves when Quickshell is launched from the parent
`quickshell/` directory or when the module is symlinked into the
user's Quickshell config tree.

## Scope

These modules are the foundation for v0.7.3 Wave 2 work: the waveform
OSD, the engine picker menu, and the meeting controls panel. None of
those frontends ship yet; this commit only lands the shared modules
plus a refactor of the existing OSD proof-of-concept to consume them.
