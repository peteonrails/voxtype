# Native X11 typing (x11 driver)

Verifies that the `x11` output driver types text the active keyboard layout
cannot produce (CJK, accented characters) into an X11 window, with no clipboard
and no external typing tool, and that the wait after a keymap change is what
makes it work.

Run this on a throwaway display: it injects keystrokes into whichever window has
focus.

## 1. Isolated display, with a client that prints what it received

```bash
Xvfb :99 -screen 0 1280x800x24 -nolisten tcp &
export DISPLAY=:99
openbox &                # any window manager, for focus handling
zenity --entry --title=probe >/tmp/x11-type.out &
sleep 2
xdotool windowactivate --sync "$(xdotool search --onlyvisible --class zenity | tail -1)"
```

`zenity --entry` prints its entry text when it is submitted, so `auto_submit`
makes the client itself report what arrived.

## 2. Sandbox daemon whose transcription is a fixed CJK string

`post_process` replaces whatever was transcribed with known text, so the check
does not depend on a model, an audio fixture, or a language.

```bash
SB=/tmp/voxtype-x11-smoke
mkdir -p "$SB" "$SB/run" && chmod 700 "$SB/run"
pactl load-module module-null-sink sink_name=vox_sb sink_properties=device.description=vox_sb

cat > "$SB/config.toml" <<EOF
state_file = "$SB/state"

[hotkey]
enabled = false

[audio]
device = "vox_sb.monitor"

[whisper]
model = "tiny.en"

[output]
mode = "type"
driver_order = ["x11"]
auto_submit = true
type_delay_ms = 0
fallback_to_clipboard = false

[output.post_process]
command = "printf '测试一二三四五六七八九十'"

[output.notification]
on_recording_start = false
on_recording_stop = false
on_transcription = false
EOF

XDG_RUNTIME_DIR=$SB/run voxtype -c $SB/config.toml daemon &
XDG_RUNTIME_DIR=$SB/run voxtype -c $SB/config.toml record start
paplay --device=vox_sb tests/fixtures/vad/speech_hello.wav
XDG_RUNTIME_DIR=$SB/run voxtype -c $SB/config.toml record stop
sleep 4
cat /tmp/x11-type.out
```

**Expected:** `测试一二三四五六七八九十` - all ten characters, in order.

## 3. The keymap settle is what does it

Repeat step 2 with `x11_keymap_settle_ms = 0`. The driver then presses each
character before X clients have read the changed keyboard mapping.

**Expected:** characters missing or replaced by the previous one - the first
run of the measurement that shaped this default typed `测试一二三四五六七八九十`
and the client reported `测试`. This is the failure the driver exists to avoid.

## 4. Opt-in, and clean on other sessions

```bash
# Default chain must not have changed: the x11 driver is not tried unless asked for
voxtype config show | grep driver_order

# With no X display the driver reports itself unavailable, so a chain that lists
# it falls through to the next entry instead of failing the transcription
DISPLAY= voxtype -c $SB/config.toml record start && sleep 1 && DISPLAY= voxtype -c $SB/config.toml record stop
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "x11"
```
