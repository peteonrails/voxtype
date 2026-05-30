# Parakeet with Preloaded Model (v0.5.1)

Tests that Parakeet works correctly when `on_demand_loading = false` (the default):

```bash
# This test verifies the v0.5.1 bug fix where Parakeet would incorrectly
# use Whisper when on_demand_loading was disabled.

# 1. Verify Parakeet is configured
grep "engine" ~/.config/voxtype/config.toml
# Expected: engine = "parakeet"

# 2. Verify on_demand_loading is false (or absent, defaulting to false)
grep "on_demand_loading" ~/.config/voxtype/config.toml || echo "on_demand_loading not set (defaults to false)"

# 3. Restart daemon and check model loading
systemctl --user restart voxtype
journalctl --user -u voxtype --since "10 seconds ago" | grep -E "Loading|Parakeet"
# Expected: "Loading Parakeet Tdt model from..."
# Expected: "Parakeet Tdt model loaded in X.XXs"

# 4. Record and transcribe
voxtype record start && sleep 2 && voxtype record stop

# 5. Verify Parakeet was used (NOT Whisper)
journalctl --user -u voxtype --since "10 seconds ago" | grep -E "Transcribing.*Parakeet"
# Expected: "Transcribing X.XXs of audio (XXXXX samples) with Parakeet Tdt"

# 6. Verify NO whisper_init_state messages (indicates bug)
journalctl --user -u voxtype --since "1 minute ago" | grep -c "whisper_init_state"
# Expected: 0 (no Whisper initialization when using Parakeet)
```

