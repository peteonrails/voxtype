# Rapid Successive Recordings

Stress test with quick start/stop cycles:

```bash
# Run multiple quick recordings in succession
for i in {1..5}; do
    echo "Recording $i..."
    voxtype record start
    sleep 1
    voxtype record cancel
done

# Verify daemon is still healthy
voxtype status
journalctl --user -u voxtype --since "1 minute ago" | grep -iE "error|panic"
```

