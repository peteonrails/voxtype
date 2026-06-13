# Quickshell OSD Showcase Recipes

These examples are complete config snippets for the Quickshell OSD renderer.
They are meant for visual testing and for users who want a starting point for
their own `[[osd.visual.layers]]` recipes.

The orbit showcase keeps `[osd.frame] background = "none"` and
`border = "none"` so the circular animation is not visually boxed in by the
default host card. The wide bars, signal stack, and prism scope showcases use
a visible background layer plus the standard host background while keeping
the border hidden.

They also set `top_margin = 0.74` so the test OSD sits above system OSD
popups that commonly occupy the lower screen band.

## Run One

From the repo root:

```bash
mkdir -p "$XDG_RUNTIME_DIR/voxtype"
printf recording > "$XDG_RUNTIME_DIR/voxtype/state"

cargo run --bin voxtype-osd-quickshell -- \
  --qml-path quickshell \
  --config examples/osd-recipes/showcase-bars.toml \
  --no-daemonize
```

Swap the config path for any recipe in this directory.

The manual state file makes the OSD visible. To see live movement, run a real
VoxType recording while the OSD is open so `voxtype-audio-bridge` receives
audio frames from the daemon.

Hide the OSD:

```bash
printf idle > "$XDG_RUNTIME_DIR/voxtype/state"
```

## Recipes

- `showcase-bars.toml`: wide, readable level bars over a visible host background.
- `showcase-orbit.toml`: compact shadow/pulse/ring style for a more animated meter.
- `showcase-prism-scope.toml`: square scope-style panel with ring, waveform,
  bars, and a meter rail.
- `showcase-signal-stack.toml`: layered waveform plus meter over a visible host background.
