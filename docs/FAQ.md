# Frequently Asked Questions

Common questions about Voxtype.

---

## General

### What is Voxtype?

Voxtype is a push-to-talk voice-to-text tool for Linux. Optimized for Wayland, works on X11 too. You hold a hotkey, speak, release the key, and your speech is transcribed and either typed at your cursor position or copied to the clipboard.

### Why another voice-to-text tool?

Most voice-to-text solutions for Linux either:
- Require internet/cloud services
- Are compositor or desktop-specific
- Don't support CJK (Korean, Chinese, Japanese) characters

Voxtype is designed to:
- Work on **any Linux desktop** (Wayland or X11)
- Be **fully offline** (uses local Whisper models)
- Use the **push-to-talk** paradigm (more predictable than continuous listening)
- Support **CJK characters** via wtype on Wayland

### Does it work on X11?

Yes! Voxtype works on both Wayland and X11. It uses evdev (kernel-level) for hotkey detection, which works everywhere. For text output, it uses wtype on Wayland (with CJK support), with dotool and ydotool as fallbacks.

### Does it require an internet connection?

No. All speech recognition is done locally using whisper.cpp. The only time network access is used is to download Whisper models during initial setup.

---

## Compatibility

### Which desktops are supported?

All of them! Voxtype is optimized for Wayland compositors with native keybinding support:

- **Hyprland, Sway, River** - Full push-to-talk via compositor keybindings (no special permissions needed)
- **niri, GNOME, KDE Plasma** - Toggle mode via a custom shortcut that runs `voxtype record toggle` (no special permissions needed)
- **X11 desktops (i3, etc.)** - Bind `voxtype record toggle` with your window manager or `xbindkeys`. The built-in evdev hotkey also works, but it needs the `input` group, which lets any program read every keystroke (see [the trade-off](USER_MANUAL.md#the-input-group-trade-off))

For text output, Voxtype uses:
- **wtype** on Wayland (best CJK/Unicode support, no daemon needed)
- **dotool** as fallback (supports keyboard layouts, no daemon needed)
- **ydotool** on X11 or as fallback (requires daemon)

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

### Does wtype work on KDE Plasma or GNOME?

No. KDE Plasma and GNOME (on Wayland) do not support the virtual keyboard protocol that wtype requires. You'll see the error: "Compositor does not support the virtual keyboard protocol."

**Solution:** Install eitype (needs no extra permissions), or dotool for non-US keyboard layouts. Voxtype automatically falls back to eitype, then dotool, then ydotool, when wtype fails.

dotool and ydotool need write access to `/dev/uinput`. Grant it through a dedicated group rather than the `input` group, which would also let your programs read every keystroke. See [Giving dotool and ydotool access to uinput](USER_MANUAL.md#giving-dotool-and-ydotool-access-to-uinput).

For ydotool (requires daemon):
```bash
systemctl --user enable --now ydotool
```

See [Troubleshooting](TROUBLESHOOTING.md#wtype-not-working-on-kde-plasma-or-gnome-wayland) for complete setup instructions.

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

Yes! Use `large-v3` which supports 99 languages:

**Transcribe in the spoken language** (speak French, output French):
```toml
[whisper]
model = "large-v3"
language = "auto"
translate = false
```

With GPU acceleration, `large-v3` achieves sub-second inference while supporting all languages.

### Can it translate to English?

Yes! Speak any language and get English output:
```toml
[whisper]
model = "large-v3"
language = "auto"
translate = true
```

### Can I transcribe audio files?

Yes, use the transcribe command:
```bash
voxtype transcribe recording.wav
```

### Does it add punctuation?

Whisper automatically adds punctuation based on context. For explicit punctuation, you can speak it (e.g., "period", "comma", "question mark").

### Can I customize the Waybar/status bar icons?

Yes! Voxtype supports 10 built-in icon themes plus custom icons. Configure in `~/.config/voxtype/config.toml`:

```toml
[status]
icon_theme = "nerd-font"
```

**Available themes:**

| Theme | Description | Requirements |
|-------|-------------|--------------|
| `emoji` | 🎙️ 🎤 ⏳ (default) | None |
| `nerd-font` | Nerd Font icons | [Nerd Font](https://www.nerdfonts.com/) |
| `material` | Material Design Icons | [MDI Font](https://materialdesignicons.com/) |
| `phosphor` | Phosphor Icons | [Phosphor Font](https://phosphoricons.com/) |
| `codicons` | VS Code icons | [Codicons](https://github.com/microsoft/vscode-codicons) |
| `omarchy` | Omarchy distro icons | Omarchy font |
| `minimal` | ○ ● ◐ × | None |
| `dots` | ◯ ⬤ ◔ ◌ | None |
| `arrows` | ▶ ● ↻ ■ | None |
| `text` | [MIC] [REC] [...] [OFF] | None |

You can also override individual icons or create custom theme files. See the [Waybar Integration Guide](WAYBAR.md#customizing-icons) for complete details.

---

## Technical

### Do I need to be in the 'input' group?

**Usually not, and you should avoid it.** With compositor or desktop keybindings (Hyprland, Sway, River, niri, KDE Plasma, GNOME), voxtype needs no special permissions.

Only voxtype's built-in evdev hotkey needs the `input` group, because it reads `/dev/input/event*` directly. Membership applies to every program you run, so all of them can then read every key you press, passwords included. Some distributions, Omarchy among them, remove users from the group for that reason. Use the built-in hotkey only where nothing else can run `voxtype record`, and read [The `input` group trade-off](USER_MANUAL.md#the-input-group-trade-off) first.

### Why does it need wtype/dotool/ydotool?

Neither Wayland nor X11 provide a universal way for applications to simulate keyboard input. Voxtype uses a fallback chain:
- **wtype** on Wayland - uses the virtual-keyboard protocol, supports CJK characters, no daemon needed
- **dotool** as fallback - uses the kernel's uinput interface, supports keyboard layouts, no daemon needed
- **ydotool** on X11 (or Wayland fallback) - uses the kernel's uinput interface, requires a daemon

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

Yes! Voxtype supports optional GPU acceleration:

- **Vulkan** - Works on AMD, NVIDIA, and Intel GPUs (included in packages)
- **CUDA** - NVIDIA GPUs (build from source)
- **Metal** - Apple Silicon (included in the macOS release)
- **HIP/ROCm** - AMD GPUs (build from source)

**Vulkan (easiest):** Packages include a pre-built Vulkan binary. Install the runtime and enable:

```bash
# Install Vulkan runtime (Arch: vulkan-icd-loader, Debian: libvulkan1, Fedora: vulkan-loader)
sudo voxtype setup gpu --enable
```

**Other backends:** Build from source with `cargo build --release --features gpu-cuda` (or `gpu-metal`, `gpu-hipblas`).

GPU acceleration dramatically improves inference speed, especially for larger models. The `large-v3` model can achieve sub-second inference with GPU acceleration.

### Is my voice data sent anywhere?

No. All processing happens locally on your machine. No audio or text is sent to any server.

---

## Troubleshooting

### It's not detecting my hotkey

**Using compositor keybindings (recommended):**
1. Verify your compositor config calls `voxtype record start` and `voxtype record stop`
2. Check that voxtype is running: `pgrep voxtype`
3. Test manually: `voxtype record start` then `voxtype record stop`

**Using built-in evdev hotkey** (a fallback; prefer a keybinding):
1. Make sure `enabled = true` in your config's `[hotkey]` section
2. Run `voxtype doctor`: it says whether the daemon can open a keyboard and why not. The built-in hotkey needs the `input` group; see [the trade-off](USER_MANUAL.md#the-input-group-trade-off) before adding it
3. Check the key name with `evtest`
4. Try running with debug: `voxtype -vv`

### No text is typed

**On Wayland:**
1. Check wtype is installed: `which wtype`
2. Test wtype directly: `wtype "test"`

**On X11:**
1. Check ydotool is running: `systemctl --user status ydotool`
2. Test ydotool directly: `ydotool type "test"`

**Fallback:**
Try clipboard mode: `voxtype --clipboard`

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

See the [Contributing Guide](https://github.com/peteonrails/voxtype/blob/main/CONTRIBUTING.md) for details. We welcome:
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
- Debug output (`voxtype -vv`). It includes the text of what you dictate while it runs, so read it before you post it

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
