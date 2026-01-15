# Voxtype Troubleshooting Guide

Solutions to common issues when using Voxtype.

## Table of Contents

- [Modifier Key Interference (Hyprland/Sway/River)](#modifier-key-interference-hyprlandswayriver)
- [Permission Issues](#permission-issues)
- [Audio Problems](#audio-problems)
- [Transcription Issues](#transcription-issues)
- [Output Problems](#output-problems)
- [Performance Issues](#performance-issues)
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

#### 4. Context window optimization (rare)

If you experience accuracy issues specifically with short recordings, try disabling context window optimization:

```toml
[whisper]
context_window_optimization = false
```

Or via command line:
```bash
voxtype --no-whisper-context-optimization daemon
```

This is rarely needed. The optimization speeds up transcription for short clips and should not affect quality. Only try this if other solutions don't help and the issue is specific to short recordings.

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

---

## Output Problems

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
