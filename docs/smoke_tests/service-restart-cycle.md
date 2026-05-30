# Service Restart Cycle

Test systemd service restarts:

```bash
# Multiple restart cycles
for i in {1..3}; do
    echo "Restart cycle $i..."
    systemctl --user restart voxtype
    sleep 2
    voxtype status
done

# Verify clean restarts in logs:
journalctl --user -u voxtype --since "1 minute ago" | grep -E "Starting|Ready|shutdown"
```

