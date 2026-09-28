# Hotkey Capture (withholding the chord)

Verifies `[hotkey] grab = true`: keyboards are held exclusively, events are
re-emitted through a virtual keyboard, and the hotkey chord never reaches the
application. Use a chord that an application acts on (Meta+V in Chromium types
"v") so a leak is obvious.

## Unit and integration tests

```bash
# Suppression rules (no hardware needed) and the kernel-level capture test.
# The capture test skips itself when /dev/uinput is unavailable or not writable.
cargo test --lib hotkey::evdev_listener -- --nocapture
```

Expected: `exclusive_capture_withholds_the_chord_and_mirrors_the_rest ... ok`.
It builds a fake keyboard, grabs it, and asserts that a Meta+V chord reaches
the listener while the mirror emits only the modifier, then that a later plain
"v" does reach the mirror.

## Structural checks

```bash
grep -c 'n.contains("voxtype")' src/hotkey/evdev_listener.rs      # 1 line, own mirrors skipped
grep -c "MIRROR_DEVICE_NAME" src/hotkey/evdev_listener.rs         # 4 (const, uinput name, log, test)
grep -c "hotkey.grab" src/config/schema.rs                        # 2 (spec + resolver)
```

## Runtime checklist

Config under test:

```toml
[hotkey]
key = "EVTEST_47"        # V as a kernel keycode
modifiers = ["LEFTMETA"]
grab = true
```

1. Start the daemon with `voxtype -v` (or `systemctl --user restart voxtype`).

```bash
journalctl --user -u voxtype --since "1 minute ago" | grep -i "exclusive capture"
```

   Expected: `Exclusive capture active: 5 of 5 keyboard(s) grabbed; the hotkey
   chord is withheld from applications`, with `Capturing "<node>" exclusively`
   lines above it, one per keyboard. A `no keyboard could be captured` line
   instead means `/dev/uinput` is missing or another program holds the keyboard
   (see TROUBLESHOOTING).

2. Confirm the mirror devices exist, one per captured keyboard.

```bash
xinput list | grep -c "voxtype key mirror"        # X11: one per keyboard
hyprctl devices | grep -c "voxtype key mirror"    # Wayland/Hyprland
```

3. Type into any window. Expected: text lands as usual, with no added latency a
   human can feel, and no keys are lost or doubled.

4. With Chromium focused and a text field active, press the chord. Expected:
   voxtype records, and Chromium shows nothing. Release, speak, and release:
   the transcript lands normally.

5. Check the daemon log shows no `Mirror write failed` or repeated
   `Skipping virtual injection keyboard` lines after startup. Expected: the
   mirror is skipped once per device, never re-opened or grabbed.

6. Stop the daemon. Expected: `xinput list` no longer lists the mirrors and
   typing keeps working (the kernel releases the grab with the file
   descriptors).

7. Failure paths worth one pass each:
   - `grab = false` with the same chord: the recording still starts, and
     Chromium inserts "v" again
   - `chmod 000 /dev/uinput` (or remove the `input` group): the daemon logs
     `cannot create the mirror device`, never grabs anything, and the hotkey
     still records
   - `grab = true` with no `modifiers`: the daemon warns at startup that every
     press of the hotkey key is withheld, not just a chord
