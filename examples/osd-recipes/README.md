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

Minimal styles, which show only the state, the live voice level, and the
clock:

- `capsule.toml`: a small dark pill with one flowing voice line.
- `shell-line.toml`: the same flowing strands in an Omarchy Shell panel:
  live Hyprland rounding, the theme's popup border (including gradients),
  monospace text, and shell-scaled padding. No glow or voice-driven dimming.
- `shell-dots.toml`: the same shell frame with five voice-assistant dots
  that stretch with speech and bounce while transcribing.
- `aura.toml`: a pill with an animated gradient edge (`border = "aura"`) and
  voice-assistant dots.
- `halo.toml`: a round badge with an aura edge and a voice line across it.

Showcases, which layer more instruments:

- `showcase-bars.toml`: wide, readable level bars over a visible host background.
- `showcase-orbit.toml`: compact shadow/pulse/ring style for a more animated meter.
- `showcase-prism-scope.toml`: square scope-style panel: a ring around the
  state icon, history bars, and a meter rail.
- `showcase-signal-stack.toml`: a scrolling waveform plus meter over a visible
  host background.

Shell Line and Shell Dots are also available as manifest-only style
packages under `examples/osd-packages/shell-line/` and
`examples/osd-packages/shell-dots/`, for selection through `[osd] style`
without applying a config recipe.

Their `shell` layout reads popup border/text and font/spacing tokens
from the active theme's `shell.toml`, with `~/.config/omarchy/shell.toml`
overrides. Theme changes apply live. Without Omarchy tokens it uses the
active palette and shell defaults; without Hyprland it uses square corners.
Apply it from a checkout with:

```bash
voxtype setup osd --recipe examples/osd-recipes/shell-line.toml --dry-run
voxtype setup osd --recipe examples/osd-recipes/shell-line.toml
```

Swap `shell-line.toml` for `shell-dots.toml` to use dots instead.

The pill and shell layouts, the `line` and `dots` layers, and the aura border need
a voxtype release newer than 1.1.1. Older daemons reject these recipes.
