# On-Demand Model Loading

Tests loading model only when needed (reduces idle memory):

```bash
# 1. Enable on_demand_loading in config.toml:
#    [whisper]
#    on_demand_loading = true

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Check memory before recording (model not loaded):
systemctl --user status voxtype | grep Memory

# 4. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 5. Check logs for model load/unload:
journalctl --user -u voxtype --since "1 minute ago" | grep -E "Loading|Unloading"
```

