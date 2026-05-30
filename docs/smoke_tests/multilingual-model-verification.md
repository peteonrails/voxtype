# Multilingual Model Verification

Tests that non-.en models load correctly and detect language:

```bash
# Use a multilingual model (without .en suffix)
voxtype --model small record start
sleep 3
voxtype record stop

# Check logs for language auto-detection:
journalctl --user -u voxtype --since "30 seconds ago" | grep "auto-detected language"

# Verify model menu shows multilingual options:
echo "0" | voxtype setup model  # Should show tiny, base, small, medium (multilingual)
```

