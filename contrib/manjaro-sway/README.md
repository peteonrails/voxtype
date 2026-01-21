# Manjaro Sway Integration

This directory contains configuration files for integrating Voxtype into [Manjaro Sway](https://github.com/manjaro-sway/manjaro-sway) as the default voice-to-text tool.

## Overview

Voxtype provides push-to-talk voice-to-text for Wayland compositors. It uses OpenAI's Whisper for local, private speech recognition with no cloud services required.

**Why Voxtype for Manjaro Sway:**
- Single Rust binary with no runtime dependencies
- Native Wayland support via wtype
- Sway binding mode integration for reliable push-to-talk
- Built-in Waybar module with Nerd Font icons
- Already packaged in AUR (`voxtype`, `voxtype-bin`)

## Required PRs

### 1. manjaro-sway/iso-profiles

Add to `community/sway/Packages-Desktop`:

```
voxtype
wtype
```

### 2. manjaro-sway/desktop-settings

#### A. Waybar config template

Add to `community/sway/usr/share/sway/templates/waybar/config.jsonc`:

In `modules-right` array (suggested position after `pulseaudio`):
```json
"custom/voxtype",
```

Add module definition:
```json
"custom/voxtype": {
    "exec": "voxtype status --follow --format json --icon-theme nerd-font",
    "return-type": "json",
    "format": "{}",
    "tooltip": true,
    "on-click": "voxtype record toggle",
    "on-click-right": "systemctl --user restart voxtype"
},
```

#### B. Waybar styles

Add to `community/sway/usr/share/sway/templates/waybar/style.css`:

```css
#custom-voxtype {
    padding: 0 8px;
}

#custom-voxtype.idle {
    color: @theme_text_color;
}

#custom-voxtype.recording {
    color: @error_color;
    animation: blink-critical 1s ease-in-out infinite;
}

#custom-voxtype.transcribing {
    color: @warning_color;
}

#custom-voxtype.stopped {
    color: alpha(@theme_text_color, 0.5);
}
```

#### C. Sway keybindings

Create `community/sway/etc/sway/config.d/97-voxtype.conf`:

```bash
# Voxtype push-to-talk voice-to-text
# Hold ScrollLock while speaking, release to transcribe

bindsym --no-repeat Scroll_Lock exec voxtype record start; mode voxtype

mode voxtype {
    bindsym --release Scroll_Lock exec voxtype record stop; mode default
    bindsym Escape exec voxtype record cancel; mode default
}
```

#### D. Default config

Create `community/sway/etc/skel/.config/voxtype/config.toml`:

```toml
model = "base.en"
device = "default"
output_method = "wtype"
state_file = "auto"

[status]
icon_theme = "nerd-font"
```

#### E. Systemd user service autostart

Add to existing sway autostart config or create new file:

```bash
exec_always --no-startup-id systemctl --user start voxtype.service
```

### 3. manjaro-sway/packages (optional)

If they want to host the package in their repo instead of pulling from AUR:

Create a workflow to build from the existing PKGBUILD at:
https://aur.archlinux.org/packages/voxtype

## Testing

On a Manjaro Sway system:

```bash
# Install from AUR
yay -S voxtype wtype

# Copy configs
mkdir -p ~/.config/voxtype
cp skel/config.toml ~/.config/voxtype/

# Add keybindings to sway
mkdir -p ~/.config/sway/config.d
cp sway/voxtype-keybindings.conf ~/.config/sway/config.d/97-voxtype.conf

# Start service
systemctl --user enable --now voxtype

# Test
# Hold ScrollLock, speak, release
```

## File Manifest

```
contrib/manjaro-sway/
├── README.md                      # This file
├── waybar/
│   ├── voxtype-module.jsonc       # Waybar module definition
│   └── voxtype-style.css          # Waybar CSS styles
├── sway/
│   ├── voxtype-keybindings.conf   # Sway push-to-talk bindings
│   └── voxtype-autostart.conf     # Systemd service autostart
└── skel/
    └── config.toml                # Default user config
```

## Links

- Voxtype: https://voxtype.io
- GitHub: https://github.com/peteonrails/voxtype
- AUR: https://aur.archlinux.org/packages/voxtype
- Manjaro Sway: https://github.com/manjaro-sway/manjaro-sway
