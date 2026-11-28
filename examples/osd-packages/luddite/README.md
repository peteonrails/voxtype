# Luddite

A compact dictation popup in Omarchy Shell's popup chrome. It is a custom QML
package (`LudditeOsd.qml`) that selects the `shell` layout, so it follows the
active theme's popup border and gradient, border alpha and width, live
Hyprland rounding, monospace typography and the shell's spacing scale and font
size. It is ported from the Luddite Voice OSD Omarchy plugin.

One row shows a microphone icon, a phase label over a level meter, and the
elapsed time:

- **Dictating**: the meter shows the input peak on a -60 to 0 dBFS scale. The
  clock counts from the recording's start in `recording.json`. Streaming
  sessions show the same view.
- **Input is very quiet**: replaces the label, in the theme's error color, once
  a recording is at least 2 seconds old and no frame has reached -65 dBFS (or
  no audio has arrived) for a second.
- **Microphone muted**: the icon is crossed out, the icon and label use the
  theme's error color, and the meter is empty while the system default input is
  muted. The mute state comes from Quickshell's PipeWire service. On a
  Quickshell build without it, the package still loads and only the mute
  indicator is missing.
- **Processing…**: while transcribing, an hourglass shows and a short bar
  travels along the meter track, with its own elapsed time.

The popup hides as soon as the daemon returns to idle. Placement follows
`position`, `margin_px` and `top_margin` the same way as the built-in card. The
OSD window takes no pointer or keyboard input, so the plugin's cancel button is
not part of this package. Cancel from a key binding with `voxtype record cancel`
instead. Icons are drawn with QtQuick.Shapes, so no Nerd Font is needed.

## Use

Select it by name on a packaged install:

```bash
voxtype config set osd.frontend quickshell
voxtype config set osd.style luddite
systemctl --user restart voxtype
```

From a source checkout, point `plugin_path` at this directory instead:

```bash
voxtype config set osd.plugin_path ~/src/voxtype/examples/osd-packages/luddite
```

To customize it, copy the directory to `~/.config/voxtype/osd/luddite/`, which
shadows the shipped copy.

## Manifest

```toml
name = "luddite"
palette = "omarchy"
layout = "shell"
qml_entry = "LudditeOsd.qml"
```

`palette = "omarchy"` resolves `foreground`, `accent`, `background` and `error`
from the active Omarchy theme. `layout = "shell"` makes the launcher resolve the
shell chrome into `theme.config.shell`, which the QML reads. The host passes
`daemonState`, `audio`, `theme` and `session`. The rest are declared but unused.

## Files

- `LudditeOsd.qml`: the popup.
- `LudditeIcon.qml`: microphone, muted microphone and hourglass icons.
- `LudditeEdge.qml`: a copy of `voxtype-shared/ShellBorder.qml`, because a
  package loads from its own directory and cannot import `voxtype-shared`.
