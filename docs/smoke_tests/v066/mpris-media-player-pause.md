# MPRIS Media Player Pause (#249)

Verifies the pause_media feature is wired up.

```bash
# CLI flag exists (it is a top-level flag on `voxtype`, not on `record start`)
voxtype --help 2>&1 | grep -i "pause.media"
# Expected: --pause-media flag shown

# Config field exists
grep -c "pause_media" src/config.rs
# Expected: 4+ references

# Module exists
test -f src/audio/media.rs && echo "media.rs exists" || echo "MISSING"
# Expected: media.rs exists

# Runtime test (requires playerctl and a media player):
# 1. Start playing music (Spotify, Firefox video, mpv, etc.)
# 2. playerctl status  # Should show "Playing"
# 3. Set [audio] pause_media = true in config.toml, restart daemon
# 4. voxtype record start
# 5. playerctl status  # Should show "Paused"
# 6. sleep 3 && voxtype record stop
# 7. Wait for transcription, then: playerctl status  # Should show "Playing"
```

