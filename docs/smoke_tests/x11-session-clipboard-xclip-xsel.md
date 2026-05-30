# X11 Session Clipboard (xclip/xsel)

Verifies that voxtype dispatches to `xclip` or `xsel` under an X11 session
instead of the no-op `wl-copy` call. Regression test for GitHub #346.

```bash
# Requires: an X11 session (e.g. XLibre, Xorg). WAYLAND_DISPLAY must be unset.
# Install xclip (or xsel) via your package manager:
#   sudo pacman -S xclip   # Arch / Manjaro
#   sudo apt install xclip # Debian / Ubuntu

# 1. Confirm session is X11
echo "WAYLAND_DISPLAY=$WAYLAND_DISPLAY"  # should be empty
echo "DISPLAY=$DISPLAY"                  # should be set (e.g. :0)

# 2. Force clipboard mode and trigger a recording
voxtype --mode clipboard record start && sleep 2 && voxtype record stop

# 3. Verify the transcribed text landed in the X11 clipboard
xclip -selection clipboard -o
# Expected: the transcribed text (NOT empty, NOT a stale value)

# 4. Verify the log shows the correct dispatch
journalctl --user -u voxtype --since "30 seconds ago" | grep -iE "xclip|xsel|wl-copy"
# Expected: "Using xclip for X11 clipboard" (or xsel if xclip is missing)
# Not expected: "Text copied to clipboard" via wl-copy

# 5. xsel fallback (optional): hide xclip and rerun
sudo mv /usr/bin/xclip /usr/bin/xclip.bak
voxtype --mode clipboard record start && sleep 2 && voxtype record stop
xsel --clipboard --output
# Expected: transcribed text from xsel
sudo mv /usr/bin/xclip.bak /usr/bin/xclip
```

