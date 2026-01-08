# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.4.x   | :white_check_mark: |
| < 0.4   | :x:                |

## Reporting a Vulnerability

If you discover a security vulnerability in Voxtype, please report it responsibly:

1. **Do not** open a public GitHub issue for security vulnerabilities
2. Email the maintainer directly (see GitHub profile for contact info)
3. Include a detailed description of the vulnerability
4. If possible, include steps to reproduce

## What to Expect

- Acknowledgment of your report within 48 hours
- Regular updates on the progress of addressing the vulnerability
- Credit in the security advisory (unless you prefer to remain anonymous)

## Security Considerations

### Recommended: Compositor Keybindings (No Special Permissions)

The preferred way to use voxtype is with **compositor keybindings** (Hyprland, Sway, River, etc.). This approach:

- Requires **no special permissions** or group membership
- Uses your compositor's native keybinding system
- Is more secure than the alternative

Configure your compositor to call `voxtype record start/stop/toggle`, and set `[hotkey] enabled = false` in your voxtype config. See the [User Manual](docs/USER_MANUAL.md#compositor-keybindings) for setup instructions.

### Alternative: Built-in Hotkey (Requires `input` Group)

If your desktop doesn't support key release events (GNOME, KDE, X11), voxtype can use its built-in evdev hotkey detection. This requires:

- **input group membership**: Grants read access to `/dev/input/event*` devices

**Security warning:** The `input` group grants access to **all keyboard input system-wide**. Any application running as your user with this permission can act as a keylogger. Only use this approach if compositor keybindings aren't available for your desktop.

### Text Injection

For text output, voxtype uses (in order of preference):
- **wtype**: Wayland-native, no special permissions
- **ydotool**: Requires the ydotoold daemon running
- **clipboard**: Falls back to copying text (user must paste)

## Scope

Security issues we care about:
- Vulnerabilities in Voxtype's code
- Unsafe handling of audio data or transcriptions
- Privilege escalation beyond documented requirements
- Dependencies with known vulnerabilities

Out of scope:
- Whisper model accuracy or transcription errors
- Third-party tools (ydotool, whisper.cpp) - report those upstream
