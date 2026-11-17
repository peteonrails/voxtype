# Eager Processing

Tests parallel transcription of audio chunks during recording:

```bash
# 1. Enable eager processing in config.toml:
#    [whisper]
#    eager_processing = true
#    eager_chunk_secs = 3.0  # Use short chunks for visible testing

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record for 10+ seconds (to generate multiple chunks)
voxtype record start
sleep 12
voxtype record stop

# 4. Check logs for chunk processing. The per-chunk lines are debug level:
#    add voxtype::daemon=debug to the service's RUST_LOG to see them.
journalctl --user -u voxtype --since "1 minute ago" | grep -iE "eager|chunk"
# Expected: "Spawning eager transcription for chunk 0 (...s)"   (debug)
#           "Spawning eager transcription for chunk 1 (...s)"   (debug)
#           "Chunk 0 completed: ..."                            (debug)
#           "Combined eager transcription: ..."                 (info)

# 5. Verify combined output is coherent: no word duplicated where chunks
# meet, and a word said twice on purpose ("that that") is kept twice

# 5b. With [vad] enabled, record 5s of silence: expect no output, an info line
# "No speech detected (...); discarding the recording", and a "No speech
# detected" notification (unless [output.notification] on_no_speech = false)

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

