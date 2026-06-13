# Voxtype Troubleshooting Guide

Solutions to common issues when using Voxtype.

## Table of Contents

- [Modifier Key Interference (Hyprland/Sway/River)](#modifier-key-interference-hyprlandswayriver)
- [Hotkey Detection on KDE Plasma](#hotkey-detection-on-kde-plasma)
- [Permission Issues](#permission-issues)
- [Audio Problems](#audio-problems)
- [Transcription Issues](#transcription-issues)
- [Voice Activity Detection (VAD)](#voice-activity-detection-vad)
- [Output Problems](#output-problems)
  - [wtype not working on KDE Plasma or GNOME Wayland](#wtype-not-working-on-kde-plasma-or-gnome-wayland)
  - [Text output not working on X11](#text-output-not-working-on-x11)
  - [Wrong characters on non-US keyboard layouts](#wrong-characters-on-non-us-keyboard-layouts-yz-swapped-qwertz-azerty)
- [Performance Issues](#performance-issues)
- [Soniox Backend Issues](#soniox-backend-issues)
- [Systemd Service Issues](#systemd-service-issues)
- [Debug Mode](#debug-mode)

---

## Modifier Key Interference (Hyprland/Sway/River)

### Typed text triggers window manager shortcuts instead of inserting text

**Symptoms:** When using compositor keybindings with modifiers (e.g., `SUPER+CTRL+X` or `SUPER+O`), releasing keys slowly causes typed output to trigger shortcuts instead of inserting text. For example, if you release X while still holding SUPER, the transcribed "hello" might trigger SUPER+h, SUPER+e, SUPER+l, etc.

**Cause:** The compositor tracks physical keyboard state. Even though voxtype types text, if you're still physically holding a modifier key, the compositor combines them.

**Solution:** Use the compositor setup command to automatically install a fix that blocks modifier keys during text output.

**For Hyprland:**
```bash
voxtype setup compositor hyprland
hyprctl reload
systemctl --user restart voxtype
```

**For Sway:**
```bash
voxtype setup compositor sway
swaymsg reload
systemctl --user restart voxtype
```

**For River:**
```bash
voxtype setup compositor river
# Restart River or source the new config
systemctl --user restart voxtype
```

**Note:** This command does NOT set up keybindings—it only installs the modifier interference fix. See the [User Manual](USER_MANUAL.md#compositor-keybindings) to set up your push-to-talk hotkey.

This command:
1. Writes a modifier-blocking submap/mode to `~/.config/hypr/conf.d/voxtype-submap.conf` (or `sway/conf.d/voxtype-mode.conf`, or `river/conf.d/voxtype-mode.sh`)
2. Adds pre/post output hooks to your voxtype config
3. Checks that your compositor config sources the conf.d directory

If voxtype crashes while typing, press **F12** to escape the submap and restore normal modifier behavior.

**Manual setup:** See `voxtype setup compositor hyprland --show` for the full configuration if you prefer to set it up manually.

**Alternative workaround:** If you can't use submaps, a simple delay before typing may help:

```toml
[output.post_process]
command = "sleep 0.3 && cat"
timeout_ms = 5000
```

### Compositors Without Mode/Submap Support

The automatic fix (`voxtype setup compositor`) only works on compositors that support input modes or submaps:

| Compositor | Supported | Why |
|------------|-----------|-----|
| Hyprland | Yes | Has submaps |
| Sway | Yes | Has modes |
| River | Yes | Has modes |
| Qtile | No | No mode/submap concept |
| Niri | No | No mode/submap concept |
| GNOME | No | No mode/submap concept |
| KDE | No | No mode/submap concept |

**For unsupported compositors, use one of these alternatives:**

1. **Use a dedicated key without modifiers** - Keys like ScrollLock, Pause, or F13-F24 don't have this problem since there are no modifiers to interfere:
   ```toml
   [hotkey]
   key = "SCROLLLOCK"
   ```

2. **Use the post-processor delay** (works on any compositor):
   ```toml
   [output.post_process]
   command = "sleep 0.3 && cat"
   timeout_ms = 5000
   ```
   This gives you 300ms to release all keys before typing starts.

3. **Use voxtype's built-in evdev hotkey** instead of compositor keybindings - release timing doesn't matter since voxtype controls the entire recording flow.

---

## Hotkey Detection on KDE Plasma

### Meta+modifier hotkeys not detected (evdev mode)

**Symptoms:** When using evdev hotkey detection (`[hotkey] enabled = true`) on KDE Plasma, hotkeys like Meta+Shift, Meta+Ctrl, or Meta+Alt are not detected. Voxtype shows no events in debug logs when these keys are pressed. Non-modifier keys with Meta work fine (e.g., Meta+RightAlt).

**Example:**
```toml
[hotkey]
enabled = true
key = "LEFTSHIFT"
modifiers = ["LEFTMETA"]
```

Pressing Meta+Shift produces no debug output. Changing to `key = "RIGHTALT"` with the same modifiers works correctly.

**Cause:** KDE Plasma grabs Meta+modifier combinations (Meta+Shift, Meta+Ctrl, Meta+Alt) at the compositor level for desktop switching, window management, and other shortcuts. The compositor handles these key combinations before they reach evdev, preventing voxtype from receiving the events. This is compositor behavior, not a voxtype bug.

The kernel delivers these events correctly (visible via `sudo evtest`), but KDE prevents them from reaching applications using evdev.

**Workarounds:**

**1. Use compositor keybindings instead of evdev (recommended)**

Disable evdev hotkey detection and bind voxtype commands directly in KDE System Settings:

```toml
[hotkey]
enabled = false  # Disable evdev, use compositor bindings instead
```

Then set up KDE shortcuts:
1. Open System Settings → Shortcuts → Custom Shortcuts
2. Create a new shortcut for `voxtype record toggle`
3. Assign it to your desired key combination (e.g., Meta+Shift)

This approach works reliably because the compositor processes the hotkey before any grabbing happens.

**2. Disable conflicting KDE shortcuts**

If you want to use evdev mode, remove KDE shortcuts that use your target hotkey:

1. Open System Settings → Shortcuts
2. Search for shortcuts using Meta+Shift (or your target combination)
3. Disable or rebind conflicting shortcuts
4. Test with `voxtype -vv` to verify events are received

**3. Use non-modifier target keys**

Keys like F13-F24, ScrollLock, or Pause are not grabbed by KDE:

```toml
[hotkey]
enabled = true
key = "SCROLLLOCK"
modifiers = ["LEFTMETA"]  # Meta+ScrollLock works
```

Or use a single non-modifier key without any modifiers:

```toml
[hotkey]
enabled = true
key = "SCROLLLOCK"  # No modifiers needed
```

**4. Use Alt+Shift with correct key ordering**

On KDE, Alt+Shift has strict ordering requirements. This configuration works reliably:

```toml
[hotkey]
enabled = true
key = "LEFTALT"
modifiers = ["LEFTSHIFT"]  # Press Shift first, then Alt
```

Press Shift first, then Alt while holding Shift. This avoids KDE keyboard layout switching and works consistently.

**Note:** This is not a voxtype limitation. Any application using evdev on KDE Plasma will experience the same behavior with Meta+modifier hotkeys. The compositor keybinding approach is the most reliable solution on KDE.

---

## Permission Issues

### "Cannot open input device" or "Permission denied"

**Cause:** User is not in the `input` group, required for evdev access.

**Solution:**
```bash
# Add user to input group
sudo usermod -aG input $USER

# IMPORTANT: Log out and back in for changes to take effect
# Verify membership
groups | grep input
```

### "Failed to access /dev/input/event*"

**Cause:** udev rules preventing access, or input group not applied.

**Solution:**
1. Verify group membership: `groups | grep input`
2. If not shown, log out and back in completely
3. If still failing, check udev rules:
```bash
ls -la /dev/input/event*
# Should show group 'input' with rw permissions
```

### "Unable to create uinput device" (ydotool)

**Cause:** uinput module not loaded or wrong permissions.

**Solution:**
```bash
# Load uinput module
sudo modprobe uinput

# Make it persistent
echo "uinput" | sudo tee /etc/modules-load.d/uinput.conf

# Check ydotool daemon
systemctl --user status ydotool
```

---

## Audio Problems

### "No audio captured" or empty transcriptions

**Possible causes and solutions:**

#### 1. Wrong audio device selected

```bash
# List available audio sources
pactl list sources short

# Test recording with system default
arecord -d 3 -f S16_LE -r 16000 test.wav
aplay test.wav
```

If test recording works, check your Voxtype config:
```toml
[audio]
device = "default"  # Or specific device name from pactl
```

#### 2. Microphone muted or volume too low

```bash
# Check PulseAudio/PipeWire volume
pavucontrol
# Or
pactl list sources | grep -A 10 "Default"
```

#### 3. PipeWire/PulseAudio not running

```bash
# Check audio server status
pactl info

# Restart if needed
systemctl --user restart pipewire pipewire-pulse
# Or for PulseAudio:
systemctl --user restart pulseaudio
```

### "Audio format not supported"

**Cause:** Audio device doesn't support 16kHz sample rate.

**Solution:** Voxtype handles resampling internally, but ensure your device works:
```bash
# Test at native rate
arecord -d 2 test.wav
aplay test.wav
```

### Recording stops unexpectedly

**Cause:** `max_duration_secs` limit reached.

**Solution:** Increase the limit:
```toml
[audio]
max_duration_secs = 120  # 2 minutes
```

### First word missing when dictation starts

**Symptom:** The first word (or first syllable) of an utterance is lost, but only when you start dictating after a few seconds of not using the microphone. Dictating again right away works fine.

**Cause:** The audio server suspends idle capture devices to save power (PipeWire/WirePlumber does this after about 5 seconds by default). A resuming device delivers around half a second of digital silence before real samples flow, so speech in that window is never captured.

Voxtype works around this with the `wait_for_device` gate (enabled by default): the recording-start sound, notification, and OSD are held back until the device delivers real audio. Wait for the cue before speaking and nothing is lost.

**If you still lose the first word:**
- Make sure `wait_for_device` has not been disabled in your config
- Wait for the start cue (sound or OSD) before speaking; enable audio feedback with `[audio.feedback] enabled = true` if you have no visible indicator

**If the start cue feels delayed instead:** your source may emit exact digital silence when the room is quiet (some noise-suppression filters do), which makes the gate wait its full 1.5-second timeout. Disable it:
```toml
[audio]
wait_for_device = false
```

---

## Transcription Issues

### "Model not found"

**Cause:** Whisper model not downloaded or wrong path.

**Solution:**
```bash
# Download the model
voxtype setup --download

# Or manually download
mkdir -p ~/.local/share/voxtype/models
curl -L -o ~/.local/share/voxtype/models/ggml-base.en.bin \
    https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
```

### Voxtype crashes during transcription (Linux)

**Cause:** On some Linux systems (particularly with glibc 2.42+ like Ubuntu 25.10), the whisper-rs FFI bindings crash due to C++ exceptions crossing the FFI boundary.

**Solution:** Use the CLI backend which runs whisper-cli as a subprocess:

```toml
[whisper]
backend = "cli"
```

This requires `whisper-cli` to be installed. Build it from [whisper.cpp](https://github.com/ggerganov/whisper.cpp):

```bash
git clone https://github.com/ggerganov/whisper.cpp
cd whisper.cpp
cmake -B build
cmake --build build --config Release
sudo cp build/bin/whisper-cli /usr/local/bin/
```

See [CLI Backend](USER_MANUAL.md#cli-backend-whisper-cli) in the User Manual for details.

### Poor transcription accuracy

**Possible causes:**

#### 1. Using wrong model
- For English: Use `.en` models (e.g., `base.en`)
- For accuracy: Use larger models (`small.en`, `medium.en`)

#### 2. Audio quality issues
- Use a quality microphone
- Reduce background noise
- Maintain consistent distance from mic

#### 3. Wrong language setting
```toml
[whisper]
model = "base.en"  # For English
language = "en"
```

#### 4. Context window optimization

Context window optimization is disabled by default because it can cause phrase repetition with some models (especially large-v3 and large-v3-turbo).

If you want faster transcription and your model works well with it, you can enable it:

```toml
[whisper]
context_window_optimization = true
```

Or via command line:
```bash
voxtype --whisper-context-optimization daemon
```

If you experience phrase repetition (e.g., "word word word"), make sure this setting is disabled (the default).

### Transcription includes "[BLANK_AUDIO]" or similar

**Cause:** Recording contains mostly silence.

**Solution:**
- Check microphone is working
- Increase microphone sensitivity
- Speak closer to the microphone

### Hallucinations (transcribed text not spoken)

**Cause:** Known Whisper behavior with silence or noise.

**Solutions:**
1. Use a larger model for better accuracy
2. Avoid recording ambient noise
3. Keep recordings short and speech-focused

### Phrase repetition (same words repeated multiple times)

**Cause:** Known issue with Whisper large-v3 models, especially when context window optimization is enabled.

**Example:** Saying "increase the limit" produces "increase the limit increase the limit increase the limit"

**Solutions:**
1. Ensure `context_window_optimization` is disabled (the default):
   ```toml
   [whisper]
   context_window_optimization = false
   ```
2. Try a different model (large-v3-turbo and large-v3 are most affected)
3. If using context optimization and experiencing issues, disable it

---

## Voice Activity Detection (VAD)

### VAD filters too aggressively (rejects recordings with speech)

**Symptom:** VAD rejects recordings that contain speech, showing "No speech detected" in notifications or logs.

**Cause:** The detection threshold is too high for your microphone or environment.

**Solution:** Lower the threshold. The default is 0.5. Values closer to 0.0 are more sensitive:

```toml
[vad]
enabled = true
threshold = 0.2  # More sensitive, less likely to reject speech
```

You can also reduce `min_speech_duration_ms` if very short utterances are being rejected:

```toml
[vad]
enabled = true
threshold = 0.3
min_speech_duration_ms = 50  # Accept shorter speech segments (default: 100)
```

### VAD model not found (Whisper VAD backend)

**Symptom:** Error about missing VAD model when using `backend = "whisper"` or `backend = "auto"` with the Whisper engine.

**Solution:** Download the Silero VAD model:

```bash
voxtype setup vad
```

Alternatively, switch to the energy backend which requires no model download:

```toml
[vad]
enabled = true
backend = "energy"
```

### VAD doesn't filter silence

**Symptom:** VAD is enabled but silent recordings still get transcribed, producing hallucinations.

**Possible causes:**
1. Background noise above the threshold. Lower your microphone gain or raise the threshold:
   ```toml
   [vad]
   enabled = true
   threshold = 0.7  # Require louder speech
   ```
2. The energy backend may be less accurate than the Whisper backend for borderline cases. Try switching:
   ```toml
   [vad]
   enabled = true
   backend = "whisper"  # More accurate, requires: voxtype setup vad
   ```

**Debugging:** Run with verbose logging to see VAD decisions:

```bash
voxtype -vv
```

Look for log messages about speech detection to understand what VAD is doing with your recordings.

---

## Output Problems

### wtype not working on KDE Plasma or GNOME Wayland

**Symptom:** wtype fails with "Compositor does not support the virtual keyboard protocol"

**Cause:** KDE Plasma and GNOME do not implement the `zwp_virtual_keyboard_v1` Wayland protocol that wtype requires. This is a compositor limitation, not a voxtype bug.

**What happens:** Voxtype detects this failure and automatically falls back to dotool, then ydotool. If neither is set up, it falls back to clipboard mode.

**Solution 1 (Recommended):** Install dotool. Unlike ydotool, direct dotool fallback does not require a daemon and supports keyboard layouts for non-US keyboards:

```bash
# 1. Install dotool (check your distribution's package manager)
# Arch (AUR):
yay -S dotool
# From source: https://sr.ht/~geb/dotool/

# 2. Add user to input group (required for uinput access)
sudo usermod -aG input $USER
# Log out and back in for group change to take effect

# 3. Configure keyboard layout if needed (non-US keyboards)
# Add to config.toml:
# [output]
# dotool_xkb_layout = "de"  # German, French ("fr"), etc.
```

**Solution 2:** Set up ydotool as your typing backend. Unlike dotool, ydotool requires a daemon to be running:

```bash
# 1. Install ydotool
# Arch:
sudo pacman -S ydotool
# Fedora:
sudo dnf install ydotool
# Ubuntu/Debian:
sudo apt install ydotool

# 2. Enable and start the daemon (required!)
systemctl --user enable --now ydotool

# 3. Verify it's running
systemctl --user status ydotool
```

**Important:** For ydotool, simply having it installed is not enough. The daemon must be running for the fallback to work.

**Alternative:** Use clipboard or paste mode instead of type mode:

```toml
[output]
mode = "clipboard"  # Copies to clipboard, you paste manually
# or
mode = "paste"      # Copies to clipboard, then simulates Ctrl+V
```

**Compositor compatibility:**

| Desktop | wtype | dotool | ydotool | Recommended |
|---------|-------|--------|---------|-------------|
| Hyprland | ✓ | ✓ | ✓ | wtype |
| Sway | ✓ | ✓ | ✓ | wtype |
| River | ✓ | ✓ | ✓ | wtype |
| KDE Plasma (Wayland) | ✗ | ✓ | ✓ | dotool |
| GNOME (Wayland) | ✗ | ✓ | ✓ | dotool |
| X11 (any desktop) | ✗ | ✓ | ✓ | dotool |

---

### Text output not working on X11

**Symptom:** You're running X11 (not Wayland) and see errors like:
```
WARN  wtype failed: Wayland connection failed
WARN  clipboard (wl-copy) failed: Text injection failed
ERROR Output failed: All output methods failed.
```

**Cause:** wtype and wl-copy are Wayland-only tools. On X11, voxtype needs dotool, ydotool, or xclip installed.

**Solution:** Install one of these X11-compatible tools:

**Option 1 (Recommended): Install dotool**

direct dotool works on X11, supports keyboard layouts, and doesn't need a daemon:

```bash
# Ubuntu/Debian (from source):
sudo apt install libxkbcommon-dev
git clone https://git.sr.ht/~geb/dotool
cd dotool && ./build.sh && sudo cp dotool /usr/local/bin/

# Arch (AUR):
yay -S dotool

# Add user to input group
sudo usermod -aG input $USER
# Log out and back in
```

**Option 2: Install ydotool**

ydotool works on X11 but requires a running daemon:

```bash
# Ubuntu/Debian:
sudo apt install ydotool

# Start the daemon (see "ydotool daemon not running" section for Fedora)
systemctl --user enable --now ydotool
```

**Option 3: Use clipboard mode with xclip**

For clipboard-only output (you paste manually with Ctrl+V):

```bash
# Ubuntu/Debian:
sudo apt install xclip
```

Then configure voxtype to use clipboard mode:
```toml
[output]
mode = "clipboard"
```

**Verify your setup:**

```bash
voxtype setup
```

This shows which output tools are installed and available.

---

### Wrong characters on non-US keyboard layouts (y/z swapped, QWERTZ, AZERTY)

**Symptom:** Transcribed text has wrong characters. For example, on a German keyboard, "Python" becomes "Pzthon" and "zebra" becomes "yebra" (y and z are swapped).

**Cause:** ydotool sends raw US keycodes and doesn't support keyboard layouts. When voxtype falls back to ydotool (e.g., on X11, Cinnamon, or when wtype fails), characters are typed as if you had a US keyboard layout.

**Solution:** Install dotool and configure your keyboard layout. Unlike
ydotool, direct dotool fallback supports keyboard layouts via XKB:

```bash
# 1. Install dotool
# Arch (AUR):
yay -S dotool
# Ubuntu/Debian (from source):
# See https://sr.ht/~geb/dotool/ for instructions
# Fedora (from source):
# See https://sr.ht/~geb/dotool/ for instructions

# 2. Add user to input group (required for uinput access)
sudo usermod -aG input $USER
# Log out and back in for group change to take effect

# 3. Configure your keyboard layout in config.toml:
```

Add to `~/.config/voxtype/config.toml`:

```toml
[output]
dotool_xkb_layout = "de"  # German QWERTZ
```

Then switch your desktop/compositor keyboard layout to the same layout before
dictating. dotool sends key events; it does not switch the active layout for
the focused app.

Common layout codes:
- `de` - German (QWERTZ)
- `fr` - French (AZERTY)
- `es` - Spanish
- `uk` - Ukrainian
- `ru` - Russian
- `pl` - Polish
- `it` - Italian
- `pt` - Portuguese

For layout variants (e.g., German without dead keys):

```toml
[output]
dotool_xkb_layout = "de"
dotool_xkb_variant = "nodeadkeys"
```

The active desktop layout must use the same variant.

For multilingual dictation, use per-language mappings so the variant only
applies to the language that needs it. For example, Russian phonetic typing:

```toml
[whisper]
language = ["en", "ru"]

[output.language_to_layout]
en = "us"
ru = "ru"

[output.language_to_variant]
ru = "phonetic"
```

Before dictating Russian through direct dotool fallback, switch the active
desktop layout to Russian phonetic. If the active layout is still English,
dotool will send the right key positions for Russian phonetic, but the focused
app will receive English letters such as `Probuem goworitx po-russki`.

**Alternative:** Use paste mode, which copies text to the clipboard and simulates Ctrl+V. This works regardless of keyboard layout:

```toml
[output]
mode = "paste"
```

**Note:** The keyboard layout fix requires voxtype v0.5.0 or later. If you're on an older version, upgrade first.

---

### Wrong characters when transcribing a second language

**Symptom:** With `language = ["en", "ru"]`, transcribing Russian on a US
system layout fails with eitype reporting `Character not found in keymap`, or
dotool/eitype prints garbled text. English transcriptions work fine.

**Cause:** Before voxtype v0.7.3, the daemon did not pass a layout hint to
the `eitype` binary. eitype fell back to the system's active XKB layout,
which lacked the keycodes for Cyrillic (or any non-system language) and so
either errored or typed the wrong characters. This is issue #180.

**Solution:** Upgrade to a version with per-language XKB hints. The daemon
reads the language Whisper picked for each transcription and passes a matching
layout/variant hint to eitype or direct dotool fallback for that call.

`dotoolc` does not work with variants and cannot receive voxtype's per-call XKB
hints, so voxtype uses direct `dotool` for these hinted calls. This hint only
controls dotool's text-to-key lookup. You must switch the active desktop layout
to the target language/variant before dictating. If you dictate Russian while
the active layout is English, the result can look transliterated
(`Probuem goworitx po-russki`) even though dotool sent the intended key
positions.

Verify:

```bash
voxtype daemon -vv  # debug logs
# After a Russian transcription you should see:
# DEBUG Auto layout for eitype: language='ru' -> layout='ru'
# DEBUG Auto variant for eitype: language='ru' -> variant='phonetic'
```

**Forcing a specific layout.** To pin eitype to a fixed layout regardless of
the detected language, set it explicitly:

```toml
[output]
driver_order = ["eitype"]
eitype_xkb_layout = "us"          # or "de", "ru", etc.
# eitype_xkb_variant = "dvorak"   # optional
```

**Customizing the language-to-layout and variant maps.** Voxtype ships built-in
layout defaults (`en->us`, `ru->ru`, `de->de`, etc.). Layouts that don't match
the language code (e.g. Brazilian Portuguese uses `br`, not `pt`) need an
override in config. Variants are empty by default because they are user-specific:

```toml
[output.language_to_layout]
en = "us"
pt = "br"
ru = "ru"

[output.language_to_variant]
ru = "phonetic"
```

See `docs/CONFIGURATION.md` for the full list of built-in defaults and merge
semantics (the user table replaces the defaults, so copy the entries you
want to keep).

---

### "ydotool daemon not running"

**Cause:** ydotool systemd service not started.

**Solution:**
```bash
# Enable and start ydotool
systemctl --user enable --now ydotool

# Verify it's running
systemctl --user status ydotool

# Check for errors
journalctl --user -u ydotool
```

### Text not typed / nothing happens

**Possible causes:**

#### 1. ydotool not working
```bash
# Test ydotool directly
ydotool type "test"
```

#### 2. Fallback to clipboard not working
```bash
# Test wl-copy
echo "test" | wl-copy
wl-paste
```

#### 3. Application blocking input
Some applications (terminals, games) may block simulated input.

**Solution:** Use clipboard mode:
```toml
[output]
mode = "clipboard"
```

### First CJK character dropped (wtype)

**Symptom:** When using wtype, the first Chinese, Japanese, or Korean character is missing from the output. This happens in some apps like Discord.

**Cause:** Some Wayland applications swallow the first character from wtype's virtual keyboard input, particularly with CJK text.

**Solution:** Enable the Shift prefix workaround:

```toml
[output]
wtype_shift_prefix = true
```

This prefixes each wtype command with a Shift key press and release (`-P Shift_L -p Shift_L`), which prevents the first character from being dropped. It only affects the wtype driver and has no visible effect on the output text.

You can also enable it via CLI flag (`--wtype-shift-prefix`) or environment variable (`VOXTYPE_WTYPE_SHIFT_PREFIX=true`).

### Characters dropped or garbled

**Cause:** Typing too fast for the application.

**Solution:** Increase typing delay:
```toml
[output]
type_delay_ms = 10  # Try 10-50ms
```

### Clipboard not working

**Cause:** wl-copy not installed or Wayland session issue.

**Solution:**
```bash
# Install wl-clipboard
# Arch: sudo pacman -S wl-clipboard
# Debian: sudo apt install wl-clipboard
# Fedora: sudo dnf install wl-clipboard

# Test it works
echo "test" | wl-copy
wl-paste
```

### Clipboard not restored after paste

**Cause:** The restore delay may be too short for your application, or `wl-paste` is not installed.

**Solution:**

1. Make sure `wl-clipboard` is installed (provides both `wl-copy` and `wl-paste`):
```bash
# Arch: sudo pacman -S wl-clipboard
# Debian: sudo apt install wl-clipboard
# Fedora: sudo dnf install wl-clipboard
```

2. If the clipboard is restored before the application reads it, increase the delay:
```toml
[output]
restore_clipboard_delay_ms = 500  # Try 300-500ms for slow applications
```

3. On X11, make sure `xclip` is installed for clipboard restoration support.

### No desktop notification

**Cause:** notify-send not installed or notifications disabled.

**Solution:**
```bash
# Install libnotify
# Arch: sudo pacman -S libnotify
# Debian: sudo apt install libnotify-bin
# Fedora: sudo dnf install libnotify

# Test
notify-send "Test" "This is a test"
```

---

## Performance Issues

### Slow transcription

**Solutions:**

1. **Use a smaller model:**
```toml
[whisper]
model = "tiny.en"  # Fastest
```

2. **Increase thread count:**
```toml
[whisper]
threads = 8  # Match your CPU cores
```

3. **Use English-only model:**
`.en` models are faster than multilingual models.

### Slow transcription on multi-GPU systems (Vulkan)

**Cause:** whisper.cpp 1.7+ enumerates integrated GPUs via Vulkan. On systems with both an integrated GPU (e.g., Intel UHD) and a discrete GPU (e.g., NVIDIA RTX), the integrated GPU gets index 0 and becomes the default. This can cause ~3x slower transcription.

**Solutions:**

1. **Use VOXTYPE_VULKAN_DEVICE** (recommended for different-vendor GPUs):

If your integrated and discrete GPUs are from different vendors (e.g., Intel iGPU + NVIDIA dGPU), this is the simplest fix. It filters out the unwanted vendor's Vulkan driver entirely.

```bash
# In your environment or systemd override:
VOXTYPE_VULKAN_DEVICE=nvidia
```

Valid values: `nvidia`, `amd`, `intel`. See `voxtype setup gpu` for detected GPUs.

2. **Set gpu_device in config** (for same-vendor GPUs or precise index control):

This passes a device index directly to whisper.cpp, useful when both GPUs are from the same vendor.

```toml
[whisper]
gpu_device = 1  # Use discrete GPU instead of integrated at index 0
```

3. **Or use the GGML_VK_VISIBLE_DEVICES env var:**
```bash
GGML_VK_VISIBLE_DEVICES=1 voxtype
```

**How to find the right GPU:** Run `voxtype setup gpu` to see detected GPUs, or `vulkaninfo --summary` to see the Vulkan device list and their indices.

### High CPU usage

**Cause:** Whisper inference is CPU-intensive.

**Solutions:**
1. Limit threads:
```toml
[whisper]
threads = 4  # Limit CPU usage
```

2. Use smaller model:
```toml
[whisper]
model = "tiny.en"
```

### High memory usage

**Cause:** Large Whisper models require significant RAM.

| Model | Approximate RAM |
|-------|-----------------|
| tiny.en | ~400 MB |
| base.en | ~500 MB |
| small.en | ~1 GB |
| medium.en | ~2.5 GB |
| large-v3 | ~4 GB |

**Solution:** Use a smaller model if RAM is limited.

### Hotkey lag / delayed recording start

**Cause:** System load or evdev latency.

**Solutions:**
1. Ensure voxtype is running with normal priority
2. Check for other applications using evdev
3. Try a different hotkey

---

## Soniox Backend Issues

### "Soniox API key required: set [soniox] api_key or SONIOX_API_KEY"

The backend can't find a credential. Either set the env var:

```bash
export SONIOX_API_KEY="your-key-here"
```

…or add it to `~/.config/voxtype/config.toml`:

```toml
[soniox]
api_key = "your-key-here"   # less safe — lands in dotfiles
```

The env var is preferred (no key in shell history, no key in config backups).

### "Soniox: WS connect failed: ..." or "connect timeout"

Network or DNS issue reaching `wss://stt-rt.soniox.com`. Check:
- Internet connectivity (`curl https://api.soniox.com`)
- Firewall / corporate proxy blocking outbound 443
- VPN that mangles WebSocket handshakes

Voxtype emits one `Streaming Error` notification and returns to idle. Press the hotkey again to retry once the network is back.

### 401 Unauthorized / 403 Forbidden

API key is invalid, revoked, or out of credit. Check the dashboard at https://console.soniox.com.

### Soniox typed text occasionally diverges from spoken words (realtime mode)

Soniox occasionally revises tail tokens between non-final and final states (`tévedések,` → `tévedések.`, `fejeztem` → `fejezte`). Voxtype emits a `StreamingEvent::Replace { backspace, text }` in this case so the cursor is patched up — but the patch only works if a backspace-capable driver is in the chain. The current backspace path tries `wtype`, then `dotool` (via `dotoolc` if the daemon is running), then `ydotool`. `eitype` does not have a backspace implementation.

If you see persistent duplication or wrong tails:
1. Check `journalctl --user -u voxtype` for `Soniox tail revision: backspace N chars, type … (lcp=N)` lines. If you see them, Replace is firing.
2. If you also see `Streaming replace: no backspace-capable backend available; skipping backspace and accepting cursor artifact`, none of wtype/dotool/ydotool was usable — the original tail stayed at the cursor and the corrected text appended. Install at least one of them (`pacman -S wtype` or `pacman -S dotool` on Arch).
3. Disable partial typing entirely: `[soniox] type_partials = false`. Finals are still typed, but no live cursor feedback. Trade-off: feels slower, zero divergence risk.

### Notifications spam during dictation (transient tray icon flicker on KDE)

If your KDE Plasma panel briefly shows an icon and re-layouts every ~150ms during streaming, the cause is usually the `eitype` driver. eitype connects via the XDG RemoteDesktop portal on each call, and KDE's security indicator briefly registers in the system tray.

**Fix:** prefer `dotool` (or `ydotool`, layout-permitting) ahead of `eitype` in `[output] driver_order`. dotool uses kernel uinput directly — no portal, no tray.

```toml
[output]
driver_order = ["dotool", "ydotool", "eitype", "clipboard"]
```

### Streaming is unusably slow (each typed segment takes ~1 second)

You're hitting dotool's uinput init cost (~700ms) on every output call. With 60+ partials per session this stacks into 40+ seconds.

**Fix:** run `dotoold` once at login. When there is no per-call XKB hint,
voxtype auto-detects its FIFO and routes through `dotoolc`, paying the init
cost once for the daemon's lifetime instead of per call. Sub-10ms per typed
segment.

See [Streaming performance: dotoold fast path](CONFIGURATION.md#streaming-performance-dotoold-fast-path) in CONFIGURATION.md for the systemd user unit template.

To verify the fast path is active after dictation:
```bash
journalctl --user -u voxtype --since "5 min ago" | grep "typed via"
```
- `Text typed via dotoolc (N chars)` — fast path
- `Text typed via dotool (N chars)` — direct path; daemon not running or voxtype had a per-call XKB hint

### Wrong keyboard layout when dotoold is running

dotool's layout setting applies to **the daemon, not the client**. When voxtype
has no per-call XKB hint, commands routed through `dotoolc` use whatever
layout dotoold inherited from its own environment.

**Fix for one fixed layout:** set `DOTOOL_XKB_LAYOUT` in dotoold's startup
environment and leave voxtype's dotool XKB fields unset:

```bash
# In your systemd user unit:
Environment=DOTOOL_XKB_LAYOUT=hu

# Then:
systemctl --user daemon-reload && systemctl --user restart dotoold
```

For per-language layouts or variants, configure `language_to_layout` /
`language_to_variant` in voxtype. `dotoolc` does not work with variants and
cannot receive voxtype's per-call XKB hints, so voxtype will use direct
`dotool` instead of `dotoolc` for those calls. You still need to switch the
active desktop layout to the same language/variant before dictating.

### PTT auto-promoted to toggle every time you start the daemon

Expected when `[soniox] streaming = true` (the default for the realtime backend). Live cursor typing while the PTT key is still held breaks libinput's held-key state tracking on Hyprland/Sway/River. Voxtype auto-promotes to toggle for the running session and warns.

To use Soniox with **real** push-to-talk, choose one of:
- `[soniox] streaming = false` — one-shot WebSocket on key release, no live partials
- `[soniox] async_api = true` — async REST API, slower but higher accuracy
- `[hotkey] mode = "toggle"` — accept toggle activation (silences the warning)

### Async API job stuck or "Soniox async: job ... did not complete within Ns"

The async API processing took longer than `async_max_wait_secs` (default 120). For very long recordings or during Soniox capacity spikes, bump the timeout:

```toml
[soniox]
async_api = true
async_max_wait_secs = 300
```

### Post-stop "Streaming Error: Soniox server error (408): Request timeout"

This notification used to appear when you released the hotkey and Soniox's server-side timer fired before the connection fully closed. Voxtype now suppresses 408s that arrive **after** you've signalled end-of-audio, so this should be silent. If you still see it, your build predates the fix (any release after v0.7.2 + soniox).

---

## Systemd Service Issues

### Service fails to start

```bash
# Check status
systemctl --user status voxtype

# View logs
journalctl --user -u voxtype -n 50

# Common issues:
# - Not in input group (log out/in after adding)
# - Model not downloaded
# - ydotool not running
```

### Service starts but doesn't work

**Cause:** Session environment not available.

**Solution:** Ensure you're running under a graphical session:
```bash
# Check environment
echo $XDG_RUNTIME_DIR
echo $WAYLAND_DISPLAY
```

### Service doesn't start on login

```bash
# Enable the service
systemctl --user enable voxtype

# Check if it's enabled
systemctl --user is-enabled voxtype

# Check startup targets
systemctl --user list-dependencies default.target
```

---

## Debug Mode

### Enable verbose logging

```bash
# Verbose
voxtype -v

# Debug (most verbose)
voxtype -vv

# Or via environment
RUST_LOG=debug voxtype
RUST_LOG=voxtype=trace voxtype
```

### Debug specific components

```bash
# Audio capture issues
RUST_LOG=voxtype::audio=debug voxtype

# Hotkey issues
RUST_LOG=voxtype::hotkey=debug voxtype

# Whisper issues
RUST_LOG=voxtype::transcribe=debug voxtype

# Output issues
RUST_LOG=voxtype::output=debug voxtype
```

### Log to file

```bash
voxtype -vv 2>&1 | tee voxtype.log
```

### Check system logs

```bash
# Kernel input messages
dmesg | grep -i input

# Audio system
journalctl --user -u pipewire -n 20
journalctl --user -u pulseaudio -n 20
```

---

## Getting Help

If you're still having issues:

1. **Run setup check:** `voxtype setup`
2. **Gather debug logs:** `voxtype -vv 2>&1 | tee debug.log`
3. **Check system info:**
   ```bash
   uname -a
   groups
   pactl info
   systemctl --user status ydotool
   ```
4. **Open an issue:** https://github.com/peteonrails/voxtype/issues

Include:
- Voxtype version (`voxtype --version`)
- Linux distribution and version
- Wayland compositor
- Debug log output
- Steps to reproduce

---

## Feedback

We want to hear from you! Voxtype is a young project and your feedback helps make it better.

- **Something not working?** If Voxtype doesn't install cleanly, doesn't work on your system, or is buggy in any way, please [open an issue](https://github.com/peteonrails/voxtype/issues). I actively monitor and respond to issues.
- **Like Voxtype?** I don't accept donations, but if you find it useful, a star on the [GitHub repository](https://github.com/peteonrails/voxtype) would mean a lot!
