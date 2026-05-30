# Recording Cycle

```bash
# Basic record start/stop
voxtype record start
sleep 3
voxtype record stop

# Toggle mode
voxtype record toggle  # starts recording
sleep 3
voxtype record toggle  # stops and transcribes

# Cancel recording (should not transcribe)
voxtype record start
sleep 2
voxtype record cancel
# Verify no transcription in logs:
journalctl --user -u voxtype --since "30 seconds ago" | grep -i transcri
```

