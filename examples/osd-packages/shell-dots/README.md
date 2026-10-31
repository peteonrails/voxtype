# Shell Dots

Aura's five voice-assistant dots inside the same Omarchy Shell panel as
Shell Line. The dots stretch with your voice, ripple outward from the
center, and bounce in turn while transcribing. The frame follows live
Hyprland rounding, the theme's popup border and gradient, monospace text,
and shell-scaled padding. No rotating aura edge, glow, or voice-driven dimming.

This is a manifest-only style that reuses the built-in dots renderer. Install
the directory under `~/.config/voxtype/osd/shell-dots/`, then select
`style = "shell-dots"` and `palette = "omarchy"` in `[osd]`. The manifest
selects the `shell` layout, so the config's layout setting need not change.
Restart the OSD after selecting it.

Both the launcher and QML tree must support the `shell` layout (newer than
the 1.1.1 release). The matching no-package preset is
`examples/osd-recipes/shell-dots.toml`.

A synthetic Capsule/Shell Line/Shell Dots comparison is in
`docs/public/quickshell-osd-modern.png`. It uses generated voice levels on
a plain background, not microphone audio or a user desktop capture.
