# Shell Line

Capsule's three flowing voice strands in an Omarchy Shell panel. This is a
manifest-only style: it uses the built-in renderer, not a custom QML entry.
It follows live Hyprland rounding, the theme's popup border and gradient,
monospace typography, and shell-scaled padding. Its background stays steady
at 97% opacity, without a glow or voice-driven dimming.

Install the directory under `~/.config/voxtype/osd/shell-line/`, then select
`style = "shell-line"` and `palette = "omarchy"` in `[osd]`. Its manifest
selects the `shell` layout, so it is not necessary to change the config's
layout setting. Restart the OSD after selecting it.

Both the launcher and QML tree must support the new `shell` layout (newer
than the 1.1.1 release). The matching no-package preset is
`examples/osd-recipes/shell-line.toml`.

For voice-assistant dots in the same shell frame, use the sibling
`shell-dots` package.

A synthetic Capsule/Shell Line/Shell Dots comparison is in
`docs/public/quickshell-osd-modern.png`; it uses generated voice levels on a
plain background, not a microphone recording or a desktop capture.
