# Eager Processing

Tests parallel transcription of audio chunks during recording:

```bash
# 1. Enable eager processing in config.toml:
#    [whisper]
#    eager_processing = true
#    eager_chunk_secs = 3.0  # Use short chunks for visible testing
#    eager_overlap_secs = 0.5

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record for 10+ seconds (to generate multiple chunks)
voxtype record start
sleep 12
voxtype record stop

# 4. Check logs for chunk processing:
journalctl --user -u voxtype --since "1 minute ago" | grep -iE "eager|chunk"
# Expected: "Spawning eager transcription for chunk 0"
#           "Spawning eager transcription for chunk 1"
#           "Chunk 0 completed"
#           "Combined eager chunks"

# 5. Verify combined output is coherent (no obvious word duplication)
# The final transcription should read naturally

# 6. Test cancellation during eager recording
voxtype record start
sleep 5
voxtype record cancel
journalctl --user -u voxtype --since "30 seconds ago" | grep -iE "cancel|abort"
# Expected: chunk tasks are cancelled, no transcription output

# 7. Restore default (disabled) when done testing:
#    [whisper]
#    eager_processing = false
```

