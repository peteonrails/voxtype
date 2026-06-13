# Aegis HUD

`aegis-hud` is a full Quickshell OSD style package showcase for VoxType. It
uses a package manifest plus a trusted custom QML entry to replace the built-in
recipe card with a cinematic circular voice HUD.

The design is inspired by high-tech assistant interfaces: concentric rings,
voice-reactive waveform arcs, telemetry labels, scanlines, rotating ticks, and
state-aware colors. It avoids direct Iron Man or Jarvis branding.

## Run

From the repo root:

```bash
mkdir -p "$XDG_RUNTIME_DIR/voxtype"
printf recording > "$XDG_RUNTIME_DIR/voxtype/state"

cargo run --bin voxtype-osd-quickshell -- \
  --qml-path quickshell \
  --style examples/osd-packages/aegis-hud \
  --no-daemonize
```

The manual state file makes the OSD visible. For live movement, start a real
VoxType recording so `voxtype-audio-bridge` receives frames from the daemon.

Hide the OSD:

```bash
printf idle > "$XDG_RUNTIME_DIR/voxtype/state"
```

## Package Shape

- `voxtype-osd.toml`: package metadata, palette, fallback recipe data, and the
  `qml_entry` declaration.
- `AegisHud.qml`: trusted custom QML loaded by the Quickshell OSD host.

Custom QML runs with the same trust level as local shell scripts. Review third
party packages before enabling them.
