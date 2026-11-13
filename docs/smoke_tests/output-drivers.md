# Output Drivers

The output fallback chain is: wtype -> dotool -> ydotool -> clipboard

For automated desktop checks, use an isolated compositor and an owned text
target. Before each insertion, reset the buffer through the target application
and verify that the buffer is empty, the window is active, and the text widget
has keyboard focus. Do not use an unverified Ctrl+A sequence as the reset.

A headless compositor may have no keyboard device and therefore no keyboard
focus until a virtual keyboard connects. Provide an input-ready seat before
checking focus. A persistent virtual keyboard that sends no characters can
supply this precondition without accessing a physical keyboard.

Compare direct `wtype -- TEXT` with Voxtype using identical target preconditions.
Check the exact resulting buffer, including its first character, at zero delay
and with uppercase and lowercase initial letters. Keep focus-precondition
failures separate from text-delivery failures. A delayed run appending a correct
suffix to an uncleared buffer is not a successful exact-text check.

```bash
# Test wtype (Wayland native, default)
# Should work by default on Wayland - check logs confirm wtype is used:
voxtype record start && sleep 2 && voxtype record stop
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "wtype|Text output"

# Test clipboard mode
# Edit config.toml: mode = "clipboard"
systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop
wl-paste  # Should show transcribed text

# Test paste mode (clipboard + Ctrl+V)
# Edit config.toml: mode = "paste"
systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop
```

