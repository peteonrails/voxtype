# Frequently Asked Questions

Common questions about Voxtype.

---

## General

### What is Voxtype?

Voxtype is a push-to-talk voice-to-text tool for Wayland Linux. You hold a hotkey, speak, release the key, and your speech is transcribed and either typed at your cursor position or copied to the clipboard.

### Why another voice-to-text tool?

Most voice-to-text solutions for Linux either:
- Only work on X11 (not Wayland)
- Require internet/cloud services
- Are compositor-specific

Voxtype is designed to:
- Work on **any** Wayland compositor
- Be **fully offline** (uses local Whisper models)
- Use the **push-to-talk** paradigm (more predictable than continuous listening)

### Does it work on X11?

Voxtype is designed primarily for Wayland, but it should work on X11 as well since it uses evdev (kernel-level) for hotkey detection and ydotool (uinput) for keyboard simulation.

### Does it require an internet connection?

No. All speech recognition is done locally using whisper.cpp. The only time network access is used is to download Whisper models during initial setup.

---

## Compatibility

### Which Wayland compositors are supported?

All of them! Voxtype uses:
- **evdev** for hotkey detection (kernel-level, compositor-independent)
- **ydotool** for typing output (uinput, compositor-independent)

Tested on: GNOME, KDE Plasma, Sway, Hyprland, river, and more.

### Which audio systems are supported?

- PipeWire (recommended)
- PulseAudio
- ALSA (directly)

### Does it work with Bluetooth microphones?

Yes, as long as your Bluetooth microphone is recognized by PipeWire/PulseAudio as an audio source.

### Does it work in all applications?

For **type mode**: Most applications work. Some may have issues:
- Full-screen games may not receive input
- Some terminal emulators handle pasted input differently
- Electron apps occasionally have issues

For **clipboard mode**: Works universally (you just need to paste manually).

---

## Features

### Can I use a different hotkey?

Yes! Any key that shows up in `evtest` can be used. Common choices:
- ScrollLock (default)
- Pause/Break
- Right Alt
- F13-F24 (if your keyboard has them)

Configure in `~/.config/voxtype/config.toml`:
```toml
[hotkey]
key = "PAUSE"
```

### Can I use key combinations?

Yes, you can require modifier keys:
```toml
[hotkey]
key = "SCROLLLOCK"
modifiers = ["LEFTCTRL"]  # Ctrl+ScrollLock
```

### Does it support multiple languages?

Yes! Use a multilingual model (without `.en` suffix) and set the language:
```toml
[whisper]
model = "base"      # Multilingual model
language = "auto"   # Auto-detect language
```

### Can it translate to English?

Yes! Enable translation:
```toml
[whisper]
model = "base"
language = "auto"
translate = true    # Translate to English
```

### Can I transcribe audio files?

Yes, use the transcribe command:
```bash
voxtype transcribe recording.wav
```

### Does it add punctuation?

Whisper automatically adds punctuation based on context. For explicit punctuation, you can speak it (e.g., "period", "comma", "question mark").

---

## Technical

### Why do I need to be in the 'input' group?

Voxtype uses the Linux evdev subsystem to detect global hotkeys. This requires read access to `/dev/input/event*` devices, which is restricted to the `input` group for security reasons.

### Why does it need ydotool?

Wayland doesn't provide a standard way for applications to simulate keyboard input (unlike X11's XTEST extension). ydotool uses the kernel's uinput interface, which works universally.

### How much RAM does it use?

Depends on the Whisper model:
- tiny.en: ~400 MB
- base.en: ~500 MB
- small.en: ~1 GB
- medium.en: ~2.5 GB
- large-v3: ~4 GB

### How fast is transcription?

Depends on model and hardware. On a modern CPU:
- tiny.en: ~10x realtime (1 second of speech = 0.1 second to transcribe)
- base.en: ~7x realtime
- small.en: ~4x realtime
- medium.en: ~2x realtime
- large-v3: ~1x realtime

### Does it use GPU acceleration?

Currently, Voxtype uses CPU-only inference via whisper.cpp. GPU support (CUDA, Metal) may be added in future versions.

### Is my voice data sent anywhere?

No. All processing happens locally on your machine. No audio or text is sent to any server.

---

## Troubleshooting

### It's not detecting my hotkey

1. Verify you're in the `input` group: `groups | grep input`
2. Log out and back in after adding to the group
3. Check the key name with `evtest`
4. Try running with debug: `voxtype -vv`

### No text is typed

1. Check ydotool is running: `systemctl --user status ydotool`
2. Test ydotool directly: `ydotool type "test"`
3. Try clipboard mode: `voxtype --clipboard`

### Transcription is inaccurate

1. Use a larger model: `--model small.en`
2. Speak more clearly and at consistent volume
3. Reduce background noise
4. Use an `.en` model for English content

### It's too slow

1. Use a smaller model: `--model tiny.en`
2. Increase thread count in config
3. Keep recordings short

See the [Troubleshooting Guide](TROUBLESHOOTING.md) for more solutions.

---

## Privacy & Security

### Is it always listening?

No. Voxtype only records audio while you hold the hotkey. When you release the key, recording stops immediately.

### Where is my audio stored?

Audio is processed in memory and discarded after transcription. Nothing is saved to disk unless you use the `transcribe` command on a file.

### Can it be used by malware to record me?

Voxtype only records while the hotkey is actively held. However, any application with access to your microphone could potentially record audio. Voxtype doesn't add any new attack surface beyond what PipeWire/PulseAudio already provides.

### Is the transcription accurate enough for sensitive content?

Whisper is highly accurate but not perfect. For sensitive or important content:
- Use a larger model (medium.en or large-v3)
- Review the transcription before using it
- Consider that Whisper may occasionally "hallucinate" text

---

## Contributing

### How can I contribute?

See the [Contributing Guide](../CONTRIBUTING.md) for details. We welcome:
- Bug reports
- Feature requests
- Code contributions
- Documentation improvements
- Translations

### Where do I report bugs?

Open an issue at: https://github.com/peteonrails/voxtype/issues

Include:
- Voxtype version
- Linux distribution and version
- Wayland compositor
- Steps to reproduce
- Debug output (`voxtype -vv`)

### Can I request a feature?

Yes! Open a feature request issue at: https://github.com/peteonrails/voxtype/issues

Describe:
- What you want to accomplish
- Why existing features don't meet your needs
- How you envision it working

### How can I show my appreciation?

I don't accept donations, but if you find Voxtype useful, a star on the [GitHub repository](https://github.com/peteonrails/voxtype) would mean a lot and helps others discover the project!

---

## Feedback

We want to hear from you! Voxtype is a young project and your feedback helps make it better.

- **Something not working?** If Voxtype doesn't install cleanly, doesn't work on your system, or is buggy in any way, please [open an issue](https://github.com/peteonrails/voxtype/issues). I actively monitor and respond to issues.
