# Output Drivers

The output fallback chain is: wtype -> dotool -> ydotool -> clipboard

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

