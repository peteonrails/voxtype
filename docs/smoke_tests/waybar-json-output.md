# Waybar JSON Output

Test the status follower with JSON format for Waybar integration:

```bash
# Should output JSON status updates (Ctrl+C to stop)
timeout 3 voxtype status --follow --format json || true

# Expected output format:
# {"text":"idle","class":"idle","tooltip":"Voxtype: idle"}

# Test during recording:
voxtype record start &
sleep 1
timeout 2 voxtype status --follow --format json || true
voxtype record cancel
```

