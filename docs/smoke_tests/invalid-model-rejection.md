# Invalid Model Rejection

Verify bad model names warn and fall back to default:

```bash
# Should warn, send notification, and fall back to default model
voxtype --model nonexistent record start
sleep 2
voxtype record cancel

# Expected behavior:
# 1. Warning logged: "Unknown model 'nonexistent', using default model 'base.en'"
# 2. Desktop notification via notify-send
# 3. Recording proceeds with the default model

# Check logs for warning:
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "unknown model"

# The setup --set command should still reject invalid models:
voxtype setup model --set nonexistent
# Expected: error about model not installed
```

