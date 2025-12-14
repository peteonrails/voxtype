# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.2.x   | :white_check_mark: |
| < 0.2   | :x:                |

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

Voxtype requires elevated permissions by design:

- **input group membership**: Required to read keyboard events via evdev for hotkey detection. This is the standard Wayland-compatible approach for global hotkeys.
- **ydotool/uinput access**: Required to inject text at the cursor position.

These permissions are documented in the installation guide. Users should be aware that any application with input group access can read keyboard input.

## Scope

Security issues we care about:
- Vulnerabilities in Voxtype's code
- Unsafe handling of audio data or transcriptions
- Privilege escalation beyond documented requirements
- Dependencies with known vulnerabilities

Out of scope:
- Issues inherent to the permissions model (evdev requires input group)
- Whisper model accuracy or transcription errors
- Third-party tools (ydotool, whisper.cpp) - report those upstream
