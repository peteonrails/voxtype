# Compositor Hooks

```bash
# Verify hooks run (check Hyprland submap changes):
voxtype record start
hyprctl submap  # Should show voxtype_recording
sleep 2
voxtype record stop
hyprctl submap  # Should show empty (reset)
```

