# Signal Handling

Test direct signal control of the daemon:

```bash
# Get daemon PID
DAEMON_PID=$(cat ~/.local/share/voxtype/voxtype.pid)

# Start recording via SIGUSR1
kill -USR1 $DAEMON_PID
voxtype status  # Should show "recording"
sleep 2

# Stop recording via SIGUSR2
kill -USR2 $DAEMON_PID
voxtype status  # Should show "transcribing" then "idle"

# Check logs:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "USR1|USR2|signal"
```

